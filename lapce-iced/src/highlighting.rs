//! Rendering adapter for backend-produced scopes. No parsing runs here.
use iced::{
    Color, Font, Theme,
    advanced::text::highlighter::{Format, Highlighter},
};
use lapce_rpc::highlighting::{HighlightSnapshot, TextSnapshot};
use std::{ops::Range, sync::Arc};

type Line = Vec<(Range<usize>, String)>;

#[derive(Clone, Default)]
pub struct Settings {
    key: Option<(TextSnapshot, u64)>,
    lines: Arc<Vec<Line>>,
}
impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Settings {
    pub fn matches(&self, highlights: Option<&HighlightSnapshot>) -> bool {
        self.key
            .as_ref()
            .map(|(snapshot, generation)| (snapshot, *generation))
            == highlights.map(|h| (&h.snapshot, h.generation))
    }
    pub fn new(text: &str, highlights: Option<&HighlightSnapshot>) -> Self {
        let Some(highlights) = highlights else {
            // A style removal must clear previous colors, even without new text.
            return Self {
                key: None,
                lines: Arc::new(text.split('\n').map(|_| Vec::new()).collect()),
            };
        };
        let mut lines = Vec::new();
        let mut start = 0;
        let mut indices = [0, 0];
        for line in text.split('\n') {
            let visible = line.strip_suffix('\r').unwrap_or(line);
            let end = start + visible.len();
            let mut spans = Vec::new();
            // Brackets are a backend-computed overlay, applied after base scopes.
            for (index, source) in [&highlights.spans, &highlights.brackets]
                .into_iter()
                .enumerate()
            {
                while indices[index] < source.len()
                    && source[indices[index]].end <= start
                {
                    indices[index] += 1;
                }
                for span in &source[indices[index]..] {
                    if span.start >= end {
                        break;
                    }
                    let left = span.start.max(start) - start;
                    let right = span.end.min(end) - start;
                    if left < right
                        && visible.is_char_boundary(left)
                        && visible.is_char_boundary(right)
                    {
                        spans.push((left..right, span.scope.clone()));
                    }
                }
            }
            lines.push(spans);
            start += line.len() + 1;
        }
        Self {
            key: Some((highlights.snapshot.clone(), highlights.generation)),
            lines: Arc::new(lines),
        }
    }
}

pub struct BackendHighlighter {
    settings: Settings,
    line: usize,
}
impl Highlighter for BackendHighlighter {
    type Settings = Settings;
    type Highlight = String;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, String)>;
    fn new(settings: &Settings) -> Self {
        Self {
            settings: settings.clone(),
            line: 0,
        }
    }
    fn update(&mut self, settings: &Settings) {
        self.settings = settings.clone();
        self.line = 0;
    }
    fn change_line(&mut self, line: usize) {
        self.line = self.line.min(line);
    }
    fn current_line(&self) -> usize {
        // iced's default editor has zero lines until text is installed. Its
        // highlighter must follow PlainText's sentinel behavior in that state.
        if self.settings.lines.is_empty() {
            usize::MAX
        } else {
            self.line
        }
    }
    fn highlight_line(&mut self, _text: &str) -> Self::Iterator<'_> {
        let spans = self
            .settings
            .lines
            .get(self.line)
            .cloned()
            .unwrap_or_default();
        self.line += 1;
        spans.into_iter()
    }
}

pub fn format(scope: &String, theme: &Theme) -> Format<Font> {
    let dark = theme.extended_palette().is_dark;
    let rgb = match scope.split('.').next().unwrap_or(scope) {
        "keyword" => {
            if dark {
                0xc586c0
            } else {
                0x8b008b
            }
        }
        "string" | "escape" => {
            if dark {
                0xce9178
            } else {
                0xa31515
            }
        }
        "comment" => {
            if dark {
                0x82a96c
            } else {
                0x3b6b37
            }
        }
        "function" | "constructor" | "method" => {
            if dark {
                0xdcdcaa
            } else {
                0x795e26
            }
        }
        "type" | "struct" | "enum" | "interface" | "typeAlias" | "typeParameter" => {
            if dark {
                0x4ec9b0
            } else {
                0x267f99
            }
        }
        "constant" | "number" => {
            if dark {
                0xb5cea8
            } else {
                0x098658
            }
        }
        "variable" | "property" | "field" | "parameter" => {
            if dark {
                0x9cdcfe
            } else {
                0x175b86
            }
        }
        "bracket" => match scope.as_str() {
            "bracket.color.1" => 0xd7a800,
            "bracket.color.2" => 0xb070d0,
            _ => 0x208bc0,
        },
        _ => return Format::default(),
    };
    Format {
        color: Some(Color::from_rgb8(
            (rgb >> 16) as u8,
            (rgb >> 8) as u8,
            rgb as u8,
        )),
        font: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lapce_rpc::highlighting::{HighlightSpan, HighlightStatus};
    #[test]
    fn style_update_after_scrolling_keeps_the_offscreen_caret_layout() {
        use iced::advanced::{
            graphics::text::Editor,
            text::{
                LineHeight, Wrapping,
                editor::{Action, Editor as _},
            },
        };
        let source = "fn line() {}\n".repeat(50);
        let mut editor = Editor::with_text(&source);
        let mut highlighter = BackendHighlighter::new(&Settings::default());
        let layout = |editor: &mut Editor, highlighter: &mut BackendHighlighter| {
            editor.update(
                iced::Size::new(800.0, 100.0),
                Font::MONOSPACE,
                16.0.into(),
                LineHeight::default(),
                Wrapping::default(),
                highlighter,
            )
        };
        layout(&mut editor, &mut highlighter);
        editor.perform(Action::Scroll { lines: 20 });
        layout(&mut editor, &mut highlighter);
        assert!(editor.buffer().scroll().line > 0);
        let before = editor.buffer().scroll();
        let cursor = editor.cursor();
        let mut lines = vec![vec![(0..2, "keyword".into())]; 50];
        lines.push(Vec::new());
        highlighter.update(&Settings {
            key: Some((
                TextSnapshot {
                    instance: "one".into(),
                    revision: 1,
                },
                1,
            )),
            lines: Arc::new(lines),
        });
        editor.highlight(Font::MONOSPACE, &mut highlighter, |scope| {
            format(scope, &Theme::Dark)
        });
        let _ = editor.selection();
        assert_eq!(editor.cursor(), cursor);
        assert_eq!(editor.buffer().scroll(), before);
        // A later resize invalidates layouts without changing syntax settings.
        editor.update(
            iced::Size::new(640.0, 100.0),
            Font::MONOSPACE,
            16.0.into(),
            LineHeight::default(),
            Wrapping::default(),
            &mut highlighter,
        );
        let _ = editor.selection();
        assert_eq!(editor.cursor(), cursor);
        assert_eq!(editor.buffer().scroll(), before);
    }

    #[test]
    fn renderer_layout_survives_highlighting_empty_and_loaded_documents() {
        use iced::advanced::{
            graphics::text::Editor,
            text::{LineHeight, Wrapping, editor::Editor as _},
        };
        for content in [None, Some(""), Some("fn main() {}\n"), Some("\n")] {
            let mut editor = content.map(Editor::with_text).unwrap_or_default();
            let mut highlighter = BackendHighlighter::new(&Settings::default());
            let layout =
                |editor: &mut Editor, highlighter: &mut BackendHighlighter| {
                    editor.update(
                        iced::Size::new(800.0, 600.0),
                        Font::MONOSPACE,
                        16.0.into(),
                        LineHeight::default(),
                        Wrapping::default(),
                        highlighter,
                    )
                };
            layout(&mut editor, &mut highlighter);
            editor.highlight(Font::MONOSPACE, &mut highlighter, |scope| {
                format(scope, &Theme::Dark)
            });
            if content.is_some() {
                let _ = editor.selection();
            }
            if content == Some("fn main() {}\n") {
                highlighter.update(&Settings {
                    key: Some((
                        TextSnapshot {
                            instance: "one".into(),
                            revision: 1,
                        },
                        1,
                    )),
                    lines: Arc::new(vec![vec![(0..2, "keyword".into())], vec![]]),
                });
                layout(&mut editor, &mut highlighter);
                editor.highlight(Font::MONOSPACE, &mut highlighter, |scope| {
                    format(scope, &Theme::Dark)
                });
                // Cursor cache is cleared by layout on a subsequent frame.
                layout(&mut editor, &mut highlighter);
                let _ = editor.selection();
                highlighter.update(&Settings::new(content.unwrap(), None));
                layout(&mut editor, &mut highlighter);
                editor.highlight(Font::MONOSPACE, &mut highlighter, |scope| {
                    format(scope, &Theme::Dark)
                });
                layout(&mut editor, &mut highlighter);
                let _ = editor.selection();
                assert!(highlighter.settings.lines.iter().all(Vec::is_empty));
            }
        }
    }

    #[test]
    fn splits_multiline_utf8_spans_and_rewinds_on_style_only_update() {
        let text = "é\\\"first\r\nsecond\\\"\r\n";
        let mut snapshot = HighlightSnapshot {
            language: "rust".into(),
            path: "a.rs".into(),
            snapshot: TextSnapshot {
                instance: "one".into(),
                revision: 1,
            },
            text_len: text.len(),
            generation: 1,
            status: HighlightStatus::Ready,
            brackets: vec![],
            structure: vec![],
            normal_lines: vec![],
            spans: vec![HighlightSpan {
                start: 2,
                end: text.len(),
                scope: "string".into(),
            }],
        };
        let settings = Settings::new(text, Some(&snapshot));
        let mut adapter = BackendHighlighter::new(&settings);
        assert_eq!(
            adapter.highlight_line("é\\\"first").collect::<Vec<_>>(),
            vec![(2..9, "string".into())]
        );
        assert_eq!(
            adapter.highlight_line("second\\\"").collect::<Vec<_>>(),
            vec![(0..8, "string".into())]
        );
        snapshot.generation += 1;
        assert!(!settings.matches(Some(&snapshot)));
        adapter.update(&Settings::new(text, Some(&snapshot)));
        assert_eq!(adapter.current_line(), 0);
    }
}
