use std::{rc::Rc, sync::Arc};

use alacritty_terminal::{
    Term,
    grid::{Dimensions, Scroll},
    selection::{Selection, SelectionType},
    term::TermMode,
    vi_mode::ViMotion,
};
use floem::{
    keyboard::{Key, KeyEvent, Modifiers, NamedKey},
    reactive::{RwSignal, Scope, SignalGet, SignalUpdate, SignalWith},
    views::editor::text::SystemClipboard,
};
use lapce_core::{
    command::{EditCommand, FocusCommand, ScrollCommand},
    mode::{Mode, VisualMode},
    movement::{LinePosition, Movement},
    register::Clipboard,
};
use lapce_rpc::terminal::{TermId, TerminalProfile};
use parking_lot::RwLock;

use super::{
    event::TermEvent,
    raw::{EventProxy, RawTerminal},
};
use crate::{
    command::{CommandExecuted, CommandKind, InternalCommand},
    keypress::{KeyPressFocus, condition::Condition},
    window_tab::CommonData,
    workspace::LapceWorkspace,
};

#[derive(Clone, Debug)]
pub struct TerminalData {
    pub scope: Scope,
    pub term_id: TermId,
    pub workspace: Arc<LapceWorkspace>,
    pub title: RwSignal<String>,
    pub launch_error: crate::reactive_model::ModelField<
        lapce_engine::terminal::TerminalState,
        Option<String>,
    >,
    lifecycle: RwSignal<lapce_engine::terminal::TerminalState>,
    lifecycle_changed: RwSignal<()>,
    pub mode: RwSignal<Mode>,
    pub visual_mode: RwSignal<VisualMode>,
    pub raw: RwSignal<Arc<RwLock<RawTerminal>>>,
    pub common: Rc<CommonData>,
}

impl KeyPressFocus for TerminalData {
    fn get_mode(&self) -> Mode {
        self.mode.get_untracked()
    }

    fn check_condition(&self, condition: Condition) -> bool {
        matches!(condition, Condition::TerminalFocus | Condition::PanelFocus)
    }

    fn run_command(
        &self,
        command: &crate::command::LapceCommand,
        count: Option<usize>,
        _mods: Modifiers,
    ) -> crate::command::CommandExecuted {
        self.common.view_id.get_untracked().request_paint();
        let config = self.common.config.get_untracked();
        match &command.kind {
            CommandKind::Move(cmd) => {
                let movement = cmd.to_movement(count);
                let raw = self.raw.get_untracked();
                let mut raw = raw.write();
                let term = &mut raw.term;
                match movement {
                    Movement::Left => {
                        term.vi_motion(ViMotion::Left);
                    }
                    Movement::Right => {
                        term.vi_motion(ViMotion::Right);
                    }
                    Movement::Up => {
                        term.vi_motion(ViMotion::Up);
                    }
                    Movement::Down => {
                        term.vi_motion(ViMotion::Down);
                    }
                    Movement::FirstNonBlank => {
                        term.vi_motion(ViMotion::FirstOccupied);
                    }
                    Movement::StartOfLine => {
                        term.vi_motion(ViMotion::First);
                    }
                    Movement::EndOfLine => {
                        term.vi_motion(ViMotion::Last);
                    }
                    Movement::WordForward => {
                        term.vi_motion(ViMotion::SemanticRight);
                    }
                    Movement::WordEndForward => {
                        term.vi_motion(ViMotion::SemanticRightEnd);
                    }
                    Movement::WordBackward => {
                        term.vi_motion(ViMotion::SemanticLeft);
                    }
                    Movement::Line(line) => {
                        match line {
                            LinePosition::First => {
                                term.scroll_display(Scroll::Top);
                                term.vi_mode_cursor.point.line = term.topmost_line();
                            }
                            LinePosition::Last => {
                                term.scroll_display(Scroll::Bottom);
                                term.vi_mode_cursor.point.line =
                                    term.bottommost_line();
                            }
                            LinePosition::Line(_) => {}
                        };
                    }
                    _ => (),
                };
            }
            CommandKind::Edit(cmd) => match cmd {
                EditCommand::NormalMode => {
                    if !config.core.modal {
                        return CommandExecuted::Yes;
                    }
                    self.mode.set(Mode::Normal);
                    let raw = self.raw.get_untracked();
                    let mut raw = raw.write();
                    let term = &mut raw.term;
                    if !term.mode().contains(TermMode::VI) {
                        term.toggle_vi_mode();
                    }
                    term.selection = None;
                }
                EditCommand::ToggleVisualMode => {
                    self.toggle_visual(VisualMode::Normal);
                }
                EditCommand::ToggleLinewiseVisualMode => {
                    self.toggle_visual(VisualMode::Linewise);
                }
                EditCommand::ToggleBlockwiseVisualMode => {
                    self.toggle_visual(VisualMode::Blockwise);
                }
                EditCommand::InsertMode => {
                    self.mode.set(Mode::Terminal);
                    let raw = self.raw.get_untracked();
                    let mut raw = raw.write();
                    let term = &mut raw.term;
                    if term.mode().contains(TermMode::VI) {
                        term.toggle_vi_mode();
                    }
                    let scroll = alacritty_terminal::grid::Scroll::Bottom;
                    term.scroll_display(scroll);
                    term.selection = None;
                }
                EditCommand::ClipboardCopy => {
                    let mut clipboard = SystemClipboard::new();
                    if matches!(self.mode.get_untracked(), Mode::Visual(_)) {
                        self.mode.set(Mode::Normal);
                    }
                    let raw = self.raw.get_untracked();
                    let mut raw = raw.write();
                    let term = &mut raw.term;
                    if let Some(content) = term.selection_to_string() {
                        clipboard.put_string(content);
                    }
                    if self.mode.get_untracked() != Mode::Terminal {
                        term.selection = None;
                    }
                }
                EditCommand::ClipboardPaste => {
                    let mut clipboard = SystemClipboard::new();
                    let mut check_bracketed_paste: bool = false;
                    if self.mode.get_untracked() == Mode::Terminal {
                        let raw = self.raw.get_untracked();
                        let mut raw = raw.write();
                        let term = &mut raw.term;
                        term.selection = None;
                        if term.mode().contains(TermMode::BRACKETED_PASTE) {
                            check_bracketed_paste = true;
                        }
                    }
                    if let Some(s) = clipboard.get_string() {
                        if check_bracketed_paste {
                            self.receive_char("\x1b[200~");
                            self.receive_char(&s.replace('\x1b', ""));
                            self.receive_char("\x1b[201~");
                        } else {
                            self.receive_char(&s);
                        }
                    }
                }
                _ => return CommandExecuted::No,
            },
            CommandKind::Scroll(cmd) => match cmd {
                ScrollCommand::PageUp => {
                    let raw = self.raw.get_untracked();
                    let mut raw = raw.write();
                    let term = &mut raw.term;
                    let scroll_lines = term.screen_lines() as i32 / 2;
                    term.vi_mode_cursor =
                        term.vi_mode_cursor.scroll(term, scroll_lines);

                    term.scroll_display(alacritty_terminal::grid::Scroll::Delta(
                        scroll_lines,
                    ));
                }
                ScrollCommand::PageDown => {
                    let raw = self.raw.get_untracked();
                    let mut raw = raw.write();
                    let term = &mut raw.term;
                    let scroll_lines = -(term.screen_lines() as i32 / 2);
                    term.vi_mode_cursor =
                        term.vi_mode_cursor.scroll(term, scroll_lines);

                    term.scroll_display(alacritty_terminal::grid::Scroll::Delta(
                        scroll_lines,
                    ));
                }
                _ => return CommandExecuted::No,
            },
            CommandKind::Focus(cmd) => match cmd {
                FocusCommand::SplitVertical => {
                    self.common.internal_command.send(
                        InternalCommand::SplitTerminal {
                            term_id: self.term_id,
                        },
                    );
                }
                FocusCommand::SplitHorizontal => {
                    self.common.internal_command.send(
                        InternalCommand::SplitTerminal {
                            term_id: self.term_id,
                        },
                    );
                }
                FocusCommand::SplitLeft => {
                    self.common.internal_command.send(
                        InternalCommand::SplitTerminalPrevious {
                            term_id: self.term_id,
                        },
                    );
                }
                FocusCommand::SplitRight => {
                    self.common.internal_command.send(
                        InternalCommand::SplitTerminalNext {
                            term_id: self.term_id,
                        },
                    );
                }
                FocusCommand::SplitExchange => {
                    self.common.internal_command.send(
                        InternalCommand::SplitTerminalExchange {
                            term_id: self.term_id,
                        },
                    );
                }
                FocusCommand::SearchForward => {
                    // if let Some(search_string) = self.find.search_string.as_ref() {
                    //     let mut raw = self.terminal.raw.lock();
                    //     let term = &mut raw.term;
                    //     self.terminal.search_next(
                    //         term,
                    //         search_string,
                    //         Direction::Right,
                    //     );
                    // }
                }
                FocusCommand::SearchBackward => {
                    // if let Some(search_string) = self.find.search_string.as_ref() {
                    //     let mut raw = self.terminal.raw.lock();
                    //     let term = &mut raw.term;
                    //     self.terminal.search_next(
                    //         term,
                    //         search_string,
                    //         Direction::Left,
                    //     );
                    // }
                }
                _ => return CommandExecuted::No,
            },
            _ => return CommandExecuted::No,
        };
        CommandExecuted::Yes
    }

    fn receive_char(&self, c: &str) {
        if self.mode.get_untracked() == Mode::Terminal {
            let effect = self
                .lifecycle
                .with_untracked(|state| state.input(self.term_id, c.into()));
            let Ok(Some(effect)) = effect else {
                return;
            };
            self.common.proxy.notification(effect);
            self.raw
                .get_untracked()
                .write()
                .term
                .scroll_display(Scroll::Bottom);
        }
    }
}

impl TerminalData {
    pub fn new(
        cx: Scope,
        workspace: Arc<LapceWorkspace>,
        profile: Option<TerminalProfile>,
        common: Rc<CommonData>,
    ) -> Self {
        let cx = cx.create_child();
        let term_id = TermId::next();

        let title = if let Some(profile) = &profile {
            cx.create_rw_signal(profile.name.to_owned())
        } else {
            cx.create_rw_signal(String::from("Default"))
        };

        let lifecycle =
            cx.create_rw_signal(lapce_engine::terminal::TerminalState::default());
        let lifecycle_changed = cx.create_rw_signal(());
        let launch_error = crate::reactive_model::ModelField::new(
            lifecycle,
            lifecycle_changed,
            |state| &state.launch_error,
            |state| &mut state.launch_error,
            Some(|state| {
                if let Some(error) = state.launch_error.clone() {
                    state.failed(error);
                }
            }),
        );

        let raw =
            Self::new_raw_terminal(&workspace, term_id, profile, common.clone());

        let mode = cx.create_rw_signal(Mode::Terminal);
        let visual_mode = cx.create_rw_signal(VisualMode::Normal);
        let raw = cx.create_rw_signal(raw);

        Self {
            scope: cx,
            term_id,
            workspace,
            raw,
            title,
            mode,
            visual_mode,
            common,
            launch_error,
            lifecycle,
            lifecycle_changed,
        }
    }

    fn new_raw_terminal(
        workspace: &LapceWorkspace,
        term_id: TermId,
        profile: Option<TerminalProfile>,
        common: Rc<CommonData>,
    ) -> Arc<RwLock<RawTerminal>> {
        let raw = Arc::new(RwLock::new(RawTerminal::new(
            term_id,
            common.proxy.clone(),
            common.term_notification_tx.clone(),
        )));

        let mut profile = profile.unwrap_or_default();

        if profile.workdir.is_none() {
            profile.workdir = url::Url::from_file_path(
                workspace.path.as_ref().cloned().unwrap_or_default(),
            )
            .ok();
        }

        {
            let raw = raw.clone();
            if let Err(err) =
                common.term_tx.send((term_id, TermEvent::NewTerminal(raw)))
            {
                tracing::error!("{:?}", err);
            }
            common.proxy.new_terminal(term_id, profile);
        }

        raw
    }

    pub fn send_keypress(&self, key: &KeyEvent) -> bool {
        if let Some(command) = Self::resolve_key_event(key) {
            self.receive_char(command);
            true
        } else if key.modifiers == Modifiers::ALT
            && matches!(&key.key.logical_key, Key::Character(_))
        {
            if let Key::Character(c) = &key.key.logical_key {
                // In terminal emulators, when the Alt key is combined with another character
                // (such as Alt+a), a leading ESC (Escape, ASCII code 0x1B) character is usually
                // sent followed by a sequence of that character. For example,
                // Alt+a sends \x1Ba.
                self.receive_char("\x1b");
                self.receive_char(c.as_str());
            }
            true
        } else {
            false
        }
    }

    pub fn resolve_key_event(key: &KeyEvent) -> Option<&str> {
        let key = key.clone();

        // Generates a `Modifiers` value to check against.
        macro_rules! modifiers {
            (ctrl) => {
                Modifiers::CONTROL
            };

            (alt) => {
                Modifiers::ALT
            };

            (shift) => {
                Modifiers::SHIFT
            };

            ($mod:ident $(| $($mods:ident)|+)?) => {
                modifiers!($mod) $(| modifiers!($($mods)|+) )?
            };
        }

        // Generates modifier values for ANSI sequences.
        macro_rules! modval {
            (shift) => {
                // 1
                "2"
            };
            (alt) => {
                // 2
                "3"
            };
            (alt | shift) => {
                // 1 + 2
                "4"
            };
            (ctrl) => {
                // 4
                "5"
            };
            (ctrl | shift) => {
                // 1 + 4
                "6"
            };
            (alt | ctrl) => {
                // 2 + 4
                "7"
            };
            (alt | ctrl | shift) => {
                // 1 + 2 + 4
                "8"
            };
        }

        // Generates ANSI sequences to move the cursor by one position.
        macro_rules! term_sequence {
            // Generate every modifier combination (except meta)
            ([all], $evt:ident, $no_mod:literal, $pre:literal, $post:literal) => {
                {
                    term_sequence!([], $evt, $no_mod);
                    term_sequence!([shift, alt, ctrl], $evt, $pre, $post);
                    term_sequence!([alt | shift, ctrl | shift, alt | ctrl], $evt, $pre, $post);
                    term_sequence!([alt | ctrl | shift], $evt, $pre, $post);
                    return None;
                }
            };
            // No modifiers
            ([], $evt:ident, $no_mod:literal) => {
                if $evt.modifiers.is_empty() {
                    return Some($no_mod);
                }
            };
            // A single modifier combination
            ([$($mod:ident)|+], $evt:ident, $pre:literal, $post:literal) => {
                if $evt.modifiers == modifiers!($($mod)|+) {
                    return Some(concat!($pre, modval!($($mod)|+), $post));
                }
            };
            // Break down multiple modifiers into a series of single combination branches
            ([$($($mod:ident)|+),+], $evt:ident, $pre:literal, $post:literal) => {
                $(
                    term_sequence!([$($mod)|+], $evt, $pre, $post);
                )+
            };
        }

        match key.key.logical_key {
            Key::Character(ref c) => {
                if key.modifiers == Modifiers::CONTROL {
                    // Convert the character into its index (into a control character).
                    // In essence, this turns `ctrl+h` into `^h`
                    let str = match c.as_str() {
                        "@" => "\x00",
                        "a" => "\x01",
                        "b" => "\x02",
                        "c" => "\x03",
                        "d" => "\x04",
                        "e" => "\x05",
                        "f" => "\x06",
                        "g" => "\x07",
                        "h" => "\x08",
                        "i" => "\x09",
                        "j" => "\x0a",
                        "k" => "\x0b",
                        "l" => "\x0c",
                        "m" => "\x0d",
                        "n" => "\x0e",
                        "o" => "\x0f",
                        "p" => "\x10",
                        "q" => "\x11",
                        "r" => "\x12",
                        "s" => "\x13",
                        "t" => "\x14",
                        "u" => "\x15",
                        "v" => "\x16",
                        "w" => "\x17",
                        "x" => "\x18",
                        "y" => "\x19",
                        "z" => "\x1a",
                        "[" => "\x1b",
                        "\\" => "\x1c",
                        "]" => "\x1d",
                        "^" => "\x1e",
                        "_" => "\x1f",
                        _ => return None,
                    };

                    Some(str)
                } else {
                    None
                }
            }
            Key::Named(NamedKey::Backspace) => {
                Some(if key.modifiers.control() {
                    "\x08" // backspace
                } else if key.modifiers.alt() {
                    "\x1b\x7f"
                } else {
                    "\x7f"
                })
            }

            Key::Named(NamedKey::Tab) => Some("\x09"),
            Key::Named(NamedKey::Enter) => Some("\r"),
            Key::Named(NamedKey::Escape) => Some("\x1b"),

            // The following either expands to `\x1b[X` or `\x1b[1;NX` where N is a modifier value
            Key::Named(NamedKey::ArrowUp) => {
                term_sequence!([all], key, "\x1b[A", "\x1b[1;", "A")
            }
            Key::Named(NamedKey::ArrowDown) => {
                term_sequence!([all], key, "\x1b[B", "\x1b[1;", "B")
            }
            Key::Named(NamedKey::ArrowRight) => {
                term_sequence!([all], key, "\x1b[C", "\x1b[1;", "C")
            }
            Key::Named(NamedKey::ArrowLeft) => {
                term_sequence!([all], key, "\x1b[D", "\x1b[1;", "D")
            }
            Key::Named(NamedKey::Home) => {
                term_sequence!([all], key, "\x1bOH", "\x1b[1;", "H")
            }
            Key::Named(NamedKey::End) => {
                term_sequence!([all], key, "\x1bOF", "\x1b[1;", "F")
            }
            Key::Named(NamedKey::Insert) => {
                term_sequence!([all], key, "\x1b[2~", "\x1b[2;", "~")
            }
            Key::Named(NamedKey::Delete) => {
                term_sequence!([all], key, "\x1b[3~", "\x1b[3;", "~")
            }
            Key::Named(NamedKey::PageUp) => {
                term_sequence!([all], key, "\x1b[5~", "\x1b[5;", "~")
            }
            Key::Named(NamedKey::PageDown) => {
                term_sequence!([all], key, "\x1b[6~", "\x1b[6;", "~")
            }
            _ => None,
        }
    }

    pub fn wheel_scroll(&self, delta: f64) {
        let config = self.common.config.get_untracked();
        let step = config.terminal_line_height() as f64;
        let raw = self.raw.get_untracked();
        let mut raw = raw.write();
        raw.scroll_delta -= delta;
        let delta = (raw.scroll_delta / step) as i32;
        raw.scroll_delta -= delta as f64 * step;
        if delta != 0 {
            let scroll = alacritty_terminal::grid::Scroll::Delta(delta);
            raw.term.scroll_display(scroll);
        }
    }

    fn toggle_visual(&self, visual_mode: VisualMode) {
        let config = self.common.config.get_untracked();
        if !config.core.modal {
            return;
        }

        match self.mode.get_untracked() {
            Mode::Normal => {
                self.mode.set(Mode::Visual(visual_mode));
                self.visual_mode.set(visual_mode);
            }
            Mode::Visual(_) => {
                if self.visual_mode.get_untracked() == visual_mode {
                    self.mode.set(Mode::Normal);
                } else {
                    self.visual_mode.set(visual_mode);
                }
            }
            _ => (),
        }

        let raw = self.raw.get_untracked();
        let mut raw = raw.write();
        let term = &mut raw.term;
        if !term.mode().contains(TermMode::VI) {
            term.toggle_vi_mode();
        }
        let ty = match visual_mode {
            VisualMode::Normal => SelectionType::Simple,
            VisualMode::Linewise => SelectionType::Lines,
            VisualMode::Blockwise => SelectionType::Block,
        };
        let point = term.renderable_content().cursor.point;
        self.toggle_selection(
            term,
            ty,
            point,
            alacritty_terminal::index::Side::Left,
        );
        if let Some(selection) = term.selection.as_mut() {
            selection.include_all();
        }
    }

    pub fn toggle_selection(
        &self,
        term: &mut Term<EventProxy>,
        ty: SelectionType,
        point: alacritty_terminal::index::Point,
        side: alacritty_terminal::index::Side,
    ) {
        match &mut term.selection {
            Some(selection) if selection.ty == ty && !selection.is_empty() => {
                term.selection = None;
            }
            Some(selection) if !selection.is_empty() => {
                selection.ty = ty;
            }
            _ => self.start_selection(term, ty, point, side),
        }
    }

    fn start_selection(
        &self,
        term: &mut Term<EventProxy>,
        ty: SelectionType,
        point: alacritty_terminal::index::Point,
        side: alacritty_terminal::index::Side,
    ) {
        term.selection = Some(Selection::new(ty, point, side));
    }

    pub fn observe_process_id(&self, process_id: Option<u32>) -> bool {
        let accepted = self
            .lifecycle
            .try_update(|state| state.started(process_id))
            .unwrap_or(false);
        if accepted {
            self.lifecycle_changed.set(());
        }
        accepted
    }
    pub fn observe_exit(&self, exit_code: Option<i32>) -> bool {
        let accepted = self
            .lifecycle
            .try_update(|state| state.stopped(exit_code))
            .unwrap_or(false);
        if accepted {
            self.lifecycle_changed.set(());
        }
        accepted
    }
    pub fn observe_failure(&self, error: &str) {
        if self
            .lifecycle
            .try_update(|state| state.failed(error.into()))
            .unwrap_or(false)
        {
            self.lifecycle_changed.set(());
        }
    }
    pub fn stop(&self) {
        self.common.proxy.terminal_close(self.term_id);
    }
}
