//! Read-only iced adapter over the shared workspace engine.
mod highlighting;
mod tree;
mod worker;

use iced::{
    Element, Length, Subscription, Task,
    widget::{
        button, column, container, row, scrollable, text, text_editor, text_input,
    },
};
use std::{path::PathBuf, sync::mpsc, time::Duration};
use worker::{Command, Snapshot};

struct Browser {
    worker: Option<std::thread::JoinHandle<()>>,
    commands: mpsc::SyncSender<Command>,
    events: mpsc::Receiver<Result<Snapshot, String>>,
    snapshot: Snapshot,
    content: text_editor::Content,
    highlight_settings: highlighting::Settings,
    path_input: String,
    query: String,
    error: String,
}
#[derive(Debug, Clone)]
enum Message {
    Tick,
    Path(String),
    Query(String),
    Send(Command),
    Open,
    Search,
    Action(text_editor::Action),
    Navigate(worker::Navigation),
}
impl Browser {
    fn new(options: worker::Options) -> Self {
        let path_input = options
            .files
            .first()
            .unwrap_or(&options.root)
            .display()
            .to_string();
        let (commands, events, thread) = worker::spawn(options);
        Self {
            worker: Some(thread),
            commands,
            events,
            snapshot: Snapshot::default(),
            // iced needs a real buffer line before cursor/layout rendering.
            content: text_editor::Content::with_text(""),
            highlight_settings: highlighting::Settings::default(),
            path_input,
            query: String::new(),
            error: String::new(),
        }
    }
    fn send(&mut self, command: Command) {
        match self.commands.try_send(command) {
            Ok(()) => self.error.clear(),
            Err(error) => {
                self.error = format!("Backend busy or unavailable: {error}")
            }
        }
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                while let Ok(event) = self.events.try_recv() {
                    match event {
                        Ok(snapshot) => {
                            if self.snapshot.path != snapshot.path
                                || self.snapshot.content != snapshot.content
                            {
                                self.content = text_editor::Content::with_text(
                                    &snapshot.content,
                                );
                                self.path_input =
                                    snapshot.path.display().to_string();
                            }
                            if snapshot.jump != self.snapshot.jump
                                && let Some((_, line)) = snapshot.jump
                            {
                                self.content.move_to(text_editor::Cursor {
                                    position: text_editor::Position {
                                        line,
                                        column: 0,
                                    },
                                    selection: None,
                                });
                            }
                            if !self
                                .highlight_settings
                                .matches(snapshot.highlights.as_deref())
                            {
                                self.highlight_settings =
                                    highlighting::Settings::new(
                                        &snapshot.content,
                                        snapshot.highlights.as_deref(),
                                    );
                            }
                            self.snapshot = snapshot;
                        }
                        Err(error) => self.error = error,
                    }
                }
            }
            Message::Path(path) => self.path_input = path,
            Message::Query(query) => self.query = query,
            Message::Open => {
                self.send(Command::Open(PathBuf::from(&self.path_input)))
            }
            Message::Search => self.send(Command::Search(self.query.clone())),
            Message::Send(command) => self.send(command),
            Message::Action(action) => {
                if !matches!(action, text_editor::Action::Edit(_)) {
                    self.content.perform(action);
                }
            }
            Message::Navigate(kind) => {
                // iced/cosmic-text columns and engine offsets both use UTF-8 bytes.
                let position = self.content.cursor().position;
                let offset = byte_offset(
                    &self.snapshot.content,
                    position.line,
                    position.column,
                );
                self.send(Command::Navigate {
                    kind,
                    offset,
                    path: self.snapshot.path.clone(),
                    revision: self.snapshot.revision,
                });
            }
        }
        Task::none()
    }
    fn view(&self) -> Element<'_, Message> {
        let mut files = column![text("EXPLORER").size(12)].spacing(0);
        if !self.snapshot.directory.as_os_str().is_empty() {
            files = files.push(self.tree_row(&self.snapshot.directory, true, 0));
        }
        for (entry, depth) in self.snapshot.tree.rows(&self.snapshot.directory) {
            files = files.push(self.tree_row(&entry.path, entry.is_dir, depth));
        }
        let mut open_files = row![].spacing(2);
        for path in &self.snapshot.open_files {
            open_files = open_files.push(
                button(text(path.to_string_lossy().into_owned()))
                    .padding([3, 6])
                    .on_press(Message::Send(Command::Open(path.clone()))),
            );
        }
        let mut results = column![text(&self.snapshot.diagnostics)].spacing(2);
        for (path, line, label) in &self.snapshot.results {
            results = results.push(
                button(text(format!("{}:{}  {}", path.display(), line + 1, label)))
                    .padding([2, 6])
                    .on_press(Message::Send(Command::Jump(path.clone(), *line))),
            );
        }
        results = results.push(text(&self.snapshot.detail));
        container(
            column![
                text("Lapce / iced — read-only code browser").size(14),
                row![
                    text_input("File path", &self.path_input)
                        .padding([3, 6])
                        .on_input(Message::Path)
                        .on_submit(Message::Open),
                    button("Open").padding([3, 6]).on_press(Message::Open)
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
                row![
                    text_input("Workspace search", &self.query)
                        .padding([3, 6])
                        .on_input(Message::Query)
                        .on_submit(Message::Search),
                    button("Search").padding([3, 6]).on_press(Message::Search),
                    button("Definition")
                        .padding([3, 6])
                        .on_press(Message::Navigate(worker::Navigation::Definition)),
                    button("References")
                        .padding([3, 6])
                        .on_press(Message::Navigate(worker::Navigation::References)),
                    button("Hover")
                        .padding([3, 6])
                        .on_press(Message::Navigate(worker::Navigation::Hover)),
                    button("Symbols")
                        .padding([3, 6])
                        .on_press(Message::Navigate(worker::Navigation::Symbols))
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
                scrollable(open_files).direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::new()
                )),
                row![
                    scrollable(files).width(220).height(Length::Fill),
                    text_editor(&self.content)
                        .highlight_with::<highlighting::BackendHighlighter>(
                            self.highlight_settings.clone(),
                            highlighting::format
                        )
                        .on_action(Message::Action)
                        .font(iced::Font::MONOSPACE)
                        .size(16)
                        .height(Length::Fill)
                ]
                .spacing(6)
                .height(Length::Fill),
                scrollable(results).height(110),
                text(format!("{}  {}", self.snapshot.status, self.error)).size(12),
            ]
            .spacing(4),
        )
        .padding(6)
        .into()
    }

    fn tree_row<'a>(
        &'a self,
        path: &std::path::Path,
        is_dir: bool,
        depth: usize,
    ) -> Element<'a, Message> {
        let marker = if is_dir {
            if self.snapshot.tree.is_expanded(path) {
                "▾"
            } else {
                "▸"
            }
        } else {
            " "
        };
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let command = if is_dir {
            Command::ToggleDirectory(path.to_path_buf())
        } else {
            Command::Open(path.to_path_buf())
        };
        let selected = !is_dir && path == self.snapshot.path;
        let entry =
            button(row![text(marker).width(12), text(name.into_owned())].spacing(2))
                .on_press(Message::Send(command))
                .padding([2, 4])
                .style(if selected {
                    button::primary
                } else {
                    button::text
                })
                .width(Length::Fill);
        container(entry)
            .padding(iced::Padding {
                left: (depth * 12) as f32,
                ..Default::default()
            })
            .width(Length::Fill)
            .into()
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        // Disconnect both queues before joining: a worker publishing a final
        // snapshot must not block while the UI is shutting down.
        let (sender, _) = mpsc::sync_channel(1);
        drop(std::mem::replace(&mut self.commands, sender));
        let (_, receiver) = mpsc::channel();
        drop(std::mem::replace(&mut self.events, receiver));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn byte_offset(content: &str, line: usize, column: usize) -> usize {
    let start: usize = content.split_inclusive('\n').take(line).map(str::len).sum();
    let start = start.min(content.len());
    let current = content[start..].split('\n').next().unwrap_or("");
    let mut column = column.min(current.len());
    while !current.is_char_boundary(column) {
        column -= 1;
    }
    start + column
}
/// Launch the read-only iced frontend using command-line arguments.
pub fn launch() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "Usage: lapce-iced [--proxy EXECUTABLE | --socket SOCKET] [--lsp RUST_ANALYZER] [FILE_OR_DIRECTORY]..."
        );
        return Ok(());
    }
    let (_reload, _guard) = lapce_services::logging::init("lapce-iced");
    let options = worker::Options::parse(std::env::args_os().skip(1))?;
    iced::application(
        move || Browser::new(options.clone()),
        Browser::update,
        Browser::view,
    )
    .title("Lapce iced — read-only")
    .settings(iced::Settings {
        default_text_size: 13.into(),
        ..Default::default()
    })
    .subscription(|_| -> Subscription<Message> {
        iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick)
    })
    .window_size((1200.0, 850.0))
    .run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "embedded-backend")]
    fn directory_startup_initializes_an_empty_editor_line() {
        let directory = tempfile::tempdir().unwrap();
        let browser = Browser::new(worker::Options {
            root: directory.path().into(),
            files: vec![],
            backend: lapce_client::LocalBackend::Embedded,
            lsp: None,
        });
        assert_eq!(browser.content.line_count(), 1);
        assert!(browser.content.is_empty());
    }

    #[test]
    fn cursor_offsets_are_utf8_and_clamped() {
        assert_eq!(byte_offset("a🌍b\nλx", 0, 5), 5);
        assert_eq!(byte_offset("a🌍b\nλx", 1, 2), 9);
        assert_eq!(byte_offset("a🌍b\nλx", 99, 99), 10);
    }
    #[test]
    fn highlight_only_updates_preserve_editor_cursor_selection_and_text() {
        use lapce_rpc::highlighting::{
            HighlightSnapshot, HighlightSpan, HighlightStatus, TextSnapshot,
        };
        let (commands, _input) = mpsc::sync_channel(1);
        let (output, events) = mpsc::channel();
        let mut browser = Browser {
            worker: None,
            commands,
            events,
            snapshot: Snapshot::default(),
            content: text_editor::Content::new(),
            highlight_settings: highlighting::Settings::default(),
            path_input: String::new(),
            query: String::new(),
            error: String::new(),
        };
        let mut snapshot = Snapshot {
            path: "a.rs".into(),
            content: "fn main() {}\n".into(),
            ..Default::default()
        };
        output.send(Ok(snapshot.clone())).unwrap();
        let _ = browser.update(Message::Tick);
        browser.content.move_to(text_editor::Cursor {
            position: text_editor::Position { line: 0, column: 7 },
            selection: Some(text_editor::Position { line: 0, column: 3 }),
        });
        let before_cursor = browser.content.cursor();
        let before_text = browser.content.text();
        snapshot.highlights = Some(std::sync::Arc::new(HighlightSnapshot {
            path: snapshot.path.clone(),
            language: "Rust".into(),
            snapshot: TextSnapshot {
                instance: "one".into(),
                revision: 1,
            },
            text_len: snapshot.content.len(),
            generation: 1,
            status: HighlightStatus::Ready,
            spans: vec![HighlightSpan {
                start: 0,
                end: 2,
                scope: "keyword".into(),
            }],
            brackets: vec![],
            structure: vec![],
            normal_lines: vec![],
        }));
        output.send(Ok(snapshot.clone())).unwrap();
        let _ = browser.update(Message::Tick);
        assert_eq!(browser.content.cursor(), before_cursor);
        assert_eq!(browser.content.text(), before_text);
        assert!(
            browser
                .highlight_settings
                .matches(snapshot.highlights.as_deref())
        );
    }

    #[test]
    fn editing_actions_cannot_change_code() {
        let (commands, _input) = mpsc::sync_channel(1);
        let (_output, events) = mpsc::channel();
        let mut browser = Browser {
            worker: None,
            commands,
            events,
            snapshot: Snapshot::default(),
            content: text_editor::Content::with_text("read only"),
            highlight_settings: highlighting::Settings::default(),
            path_input: String::new(),
            query: String::new(),
            error: String::new(),
        };
        for edit in [
            text_editor::Edit::Insert('x'),
            text_editor::Edit::Backspace,
            text_editor::Edit::Delete,
        ] {
            let _ = browser.update(Message::Action(text_editor::Action::Edit(edit)));
        }
        assert_eq!(browser.content.text(), "read only");
    }
}
