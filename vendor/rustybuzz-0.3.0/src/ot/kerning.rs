use ttf_parser::GlyphId;

use crate::{Face, Mask};
use crate::buffer::{Buffer, BufferScratchFlags};
use crate::plan::ShapePlan;
use crate::tables::gsubgpos::LookupFlags;
use super::TableIndex;
use super::apply::ApplyContext;
use super::matching::SkippyIter;
use crate::ot::attach_type;

pub fn has_kerning(face: &Face) -> bool {
    face.kern.is_some()
}

pub fn has_machine_kerning(face: &Face) -> bool {
    match face.kern {
        Some(mut kern) => {
            kern.any(|s| s.has_state_machine())
        }
        None => false,
    }
}

pub fn has_cross_kerning(face: &Face) -> bool {
    match face.kern {
        Some(mut kern) => {
            kern.any(|s| s.has_cross_stream())
        }
        None => false,
    }
}

pub fn kern(plan: &ShapePlan, face: &Face, buffer: &mut Buffer) {
    let subtables = match face.kern {
        Some(subtables) => subtables,
        None => return,
    };

    let mut seen_cross_stream = false;
    for subtable in subtables {
        if subtable.is_variable() {
            continue;
        }

        if buffer.direction.is_horizontal() != subtable.is_horizontal() {
            continue;
        }

        let reverse = buffer.direction.is_backward();

        if !seen_cross_stream && subtable.has_cross_stream() {
            seen_cross_stream = true;

            // Attach all glyphs into a chain.
            for pos in &mut buffer.pos {
                pos.set_attach_type(attach_type::CURSIVE);
                pos.set_attach_chain(if buffer.direction.is_forward() { -1 } else { 1 });
                // We intentionally don't set BufferScratchFlags::HAS_GPOS_ATTACHMENT,
                // since there needs to be a non-zero attachment for post-positioning to
                // be needed.
            }
        }

        if reverse {
            buffer.reverse();
        }

        if subtable.has_state_machine() {
            apply_state_machine_kerning(&subtable, plan.kern_mask, buffer);
        } else {
            if !plan.requested_kerning {
                continue;
            }

            apply_simple_kerning(&subtable, &face, plan.kern_mask, buffer);
        }

        if reverse {
            buffer.reverse();
        }
    }
}

// TODO: remove
fn machine_kern(
    face: &Face,
    buffer: &mut Buffer,
    kern_mask: Mask,
    cross_stream: bool,
    get_kerning: impl Fn(u32, u32) -> i32,
) {
    let mut ctx = ApplyContext::new(TableIndex::GPOS, face, buffer);
    ctx.lookup_mask = kern_mask;
    ctx.lookup_props = u32::from(LookupFlags::IGNORE_MARKS.bits());

    let horizontal = ctx.buffer.direction.is_horizontal();

    let mut i = 0;
    while i < ctx.buffer.len {
        if (ctx.buffer.info[i].mask & kern_mask) == 0 {
            i += 1;
            continue;
        }

        let mut iter = SkippyIter::new(&ctx, i, 1, false);
        if !iter.next() {
            i += 1;
            continue;
        }

        let j = iter.index();

        let info = &ctx.buffer.info;
        let kern = get_kerning(info[i].codepoint, info[j].codepoint);

        let pos = &mut ctx.buffer.pos;
        if kern != 0 {
            if horizontal {
                if cross_stream {
                    pos[j].y_offset = kern;
                    ctx.buffer.scratch_flags |= BufferScratchFlags::HAS_GPOS_ATTACHMENT;
                } else {
                    let kern1 = kern >> 1;
                    let kern2 = kern - kern1;
                    pos[i].x_advance += kern1;
                    pos[j].x_advance += kern2;
                    pos[j].x_offset += kern2;
                }
            } else {
                if cross_stream {
                    pos[j].x_offset = kern;
                    ctx.buffer.scratch_flags |= BufferScratchFlags::HAS_GPOS_ATTACHMENT;
                } else {
                    let kern1 = kern >> 1;
                    let kern2 = kern - kern1;
                    pos[i].y_advance += kern1;
                    pos[j].y_advance += kern2;
                    pos[j].y_offset += kern2;
                }
            }

            ctx.buffer.unsafe_to_break(i, j + 1)
        }

        i = j;
    }
}

fn apply_simple_kerning(
    subtable: &crate::tables::kern::Subtable,
    face: &Face,
    kern_mask: Mask,
    buffer: &mut Buffer,
) {
    machine_kern(face, buffer, kern_mask, subtable.has_cross_stream(), |left, right| {
        subtable.glyphs_kerning(GlyphId(left as u16), GlyphId(right as u16))
            .map(|v| i32::from(v)).unwrap_or(0)
    });
}

struct StateMachineDriver {
    stack: [usize; 8],
    depth: usize,
}

fn apply_state_machine_kerning(
    subtable: &crate::tables::kern::Subtable,
    kern_mask: Mask,
    buffer: &mut Buffer,
) {
    use crate::tables::kern::state_machine;

    let machine = match subtable.state_machine() {
        Some(v) => v,
        None => return,
    };

    let mut driver = StateMachineDriver {
        stack: [0; 8],
        depth: 0,
    };

    let mut state = state_machine::state::START_OF_TEXT;
    buffer.idx = 0;
    loop {
        let class = if buffer.idx < buffer.len {
            machine.class(buffer.info[buffer.idx].as_glyph()).unwrap_or(1)
        } else {
            state_machine::class::END_OF_TEXT
        };

        let entry = match machine.entry(state, class) {
            Some(v) => v,
            None => break,
        };

        // Unsafe-to-break before this if not in state 0, as things might
        // go differently if we start from state 0 here.
        if state != state_machine::state::START_OF_TEXT &&
            buffer.backtrack_len() != 0 &&
            buffer.idx < buffer.len
        {
            // If there's no value and we're just epsilon-transitioning to state 0, safe to break.
            if entry.has_offset() ||
                !(entry.new_state() == state_machine::state::START_OF_TEXT && !entry.has_advance())
            {
                buffer.unsafe_to_break_from_outbuffer(buffer.backtrack_len() - 1, buffer.idx + 1);
            }
        }

        // Unsafe-to-break if end-of-text would kick in here.
        if buffer.idx + 2 <= buffer.len {
            let end_entry = match machine.entry(state, state_machine::class::END_OF_TEXT) {
                Some(v) => v,
                None => break,
            };

            if end_entry.has_offset() {
                buffer.unsafe_to_break(buffer.idx, buffer.idx + 2);
            }
        }

        state_machine_transition(entry, subtable.has_cross_stream(), kern_mask,
                                 &machine, &mut driver, buffer);

        state = machine.new_state(entry.new_state());

        if buffer.idx >= buffer.len {
            break;
        }

        buffer.max_ops -= 1;
        if entry.has_advance() || buffer.max_ops <= 0 {
            buffer.next_glyph();
        }
    }
}

fn state_machine_transition(
    entry: crate::tables::kern::state_machine::Entry,
    has_cross_stream: bool,
    kern_mask: Mask,
    machine: &crate::tables::kern::state_machine::Machine,
    driver: &mut StateMachineDriver,
    buffer: &mut Buffer,
) {
    if entry.has_push() {
        if driver.depth < driver.stack.len() {
            driver.stack[driver.depth] = buffer.idx;
            driver.depth += 1;
        } else {
            driver.depth = 0; // Probably not what CoreText does, but better?
        }
    }

    if entry.has_offset() && driver.depth != 0 {
        let mut value_offset = entry.value_offset();
        let mut value = match machine.kerning(value_offset) {
            Some(v) => v,
            None => {
                driver.depth = 0;
                return;
            }
        };

        // From Apple 'kern' spec:
        // "Each pops one glyph from the kerning stack and applies the kerning value to it.
        // The end of the list is marked by an odd value...
        let mut last = false;
        while !last && driver.depth != 0 {
            driver.depth -= 1;
            let idx = driver.stack[driver.depth];
            let mut v = value as i32;
            value_offset = value_offset.next();
            value = machine.kerning(value_offset).unwrap_or(0);
            if idx >= buffer.len {
                continue;
            }

            // "The end of the list is marked by an odd value..."
            last = v & 1 != 0;
            v &= !1;

            // Testing shows that CoreText only applies kern (cross-stream or not)
            // if none has been applied by previous subtables. That is, it does
            // NOT seem to accumulate as otherwise implied by specs.

            let mut has_gpos_attachment = false;
            let glyph_mask = buffer.info[idx].mask;
            let pos = &mut buffer.pos[idx];

            if buffer.direction.is_horizontal() {
                if has_cross_stream {
                    // The following flag is undocumented in the spec, but described
                    // in the 'kern' table example.
                    if v == -0x8000 {
                        pos.set_attach_type(0);
                        pos.set_attach_chain(0);
                        pos.y_offset = 0;
                    } else if pos.attach_type() != 0 {
                        pos.y_offset += v;
                        has_gpos_attachment = true;
                    }
                } else if glyph_mask & kern_mask != 0 {
                    pos.x_advance += v;
                    pos.x_offset += v;
                }
            } else {
                if has_cross_stream {
                    // CoreText doesn't do crossStream kerning in vertical. We do.
                    if v == -0x8000 {
                        pos.set_attach_type(0);
                        pos.set_attach_chain(0);
                        pos.x_offset = 0;
                    } else if pos.attach_type() != 0 {
                        pos.x_offset += v;
                        has_gpos_attachment = true;
                    }
                } else if glyph_mask & kern_mask != 0 {
                    if pos.y_offset == 0 {
                        pos.y_advance += v;
                        pos.y_offset += v;
                    }
                }
            }

            if has_gpos_attachment {
                buffer.scratch_flags |= BufferScratchFlags::HAS_GPOS_ATTACHMENT;
            }
        }
    }
}
