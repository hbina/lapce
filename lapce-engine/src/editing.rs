//! Editing operations independent of frontend scheduling and rendering.
use lapce_core::{
    buffer::{Buffer, InvalLines, rope_text::RopeText},
    char_buffer::CharBuffer,
    command::EditCommand,
    cursor::Cursor,
    editor::{Action, EditConf},
    register::{Clipboard, Register},
    rope_text_pos::RopeTextPosition,
    selection::Selection,
    syntax::Syntax,
    word::WordCursor,
};
use lapce_xi_rope::{Rope, RopeDelta};
use lsp_types::TextEdit;

pub type DocumentDelta = (Rope, RopeDelta, InvalLines);

pub fn insert(
    buffer: &mut Buffer,
    syntax: &Syntax,
    cursor: &mut Cursor,
    text: &str,
    auto_closing_matching_pairs: bool,
    auto_surround: bool,
) -> Vec<DocumentDelta> {
    let old_cursor = cursor.mode.clone();
    let deltas = Action::insert(
        cursor,
        buffer,
        text,
        &|buffer, c, offset| {
            if syntax.layers.is_some() {
                syntax.find_tag(offset, true, &CharBuffer::new(c))
            } else {
                WordCursor::new(buffer.text(), offset).previous_unmatched(c)
            }
        },
        auto_closing_matching_pairs,
        auto_surround,
    );
    buffer.set_cursor_before(old_cursor);
    buffer.set_cursor_after(cursor.mode.clone());
    deltas
}

pub fn edit_command(
    buffer: &mut Buffer,
    cursor: &mut Cursor,
    command: &EditCommand,
    clipboard: &mut impl Clipboard,
    register: &mut Register,
    config: EditConf<'_>,
) -> Vec<DocumentDelta> {
    let old_cursor = cursor.mode.clone();
    let deltas =
        Action::do_edit(cursor, buffer, command, clipboard, register, config);
    if !deltas.is_empty() {
        buffer.set_cursor_before(old_cursor);
        buffer.set_cursor_after(cursor.mode.clone());
    }
    deltas
}

/// Convert LSP UTF-16 ranges to the UTF-8 byte selections used by the buffer.
pub fn text_edits<'a>(
    buffer: &Buffer,
    edits: &'a [TextEdit],
) -> Vec<(Selection, &'a str)> {
    edits
        .iter()
        .map(|edit| {
            (
                Selection::region(
                    buffer.offset_of_position(&edit.range.start),
                    buffer.offset_of_position(&edit.range.end),
                ),
                edit.new_text.as_str(),
            )
        })
        .collect()
}

/// An asynchronous save must not mark newer, unsaved changes as pristine.
pub fn mark_saved(buffer: &mut Buffer, saved_revision: u64) -> bool {
    if buffer.rev() != saved_revision {
        return false;
    }
    buffer.set_pristine();
    true
}
