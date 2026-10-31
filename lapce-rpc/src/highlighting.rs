//! Theme-independent highlighting for one authoritative backend text snapshot.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Never translated into frontend-local revisions by a shared connection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextSnapshot {
    pub instance: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightSpan {
    /// Half-open UTF-8 byte offsets into the complete document.
    pub start: usize,
    pub end: usize,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HighlightStatus {
    Pending,
    Ready,
    Unsupported,
    Failed(String),
}

/// Backend-computed structural answers for a half-open interval of cursor offsets.
/// The interval partition avoids sending parser trees or parsing in the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureRange {
    pub start: usize,
    pub end: usize,
    pub matching: Option<usize>,
    pub parent: Option<usize>,
    pub enclosing: Option<(usize, usize)>,
    pub parentheses: Option<(usize, usize)>,
    pub headers: Vec<usize>,
    /// Answers for (), {}, [] in that order.
    pub previous: [Option<usize>; 6],
    pub next: [Option<usize>; 6],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightSnapshot {
    pub path: PathBuf,
    pub language: String,
    pub snapshot: TextSnapshot,
    pub text_len: usize,
    pub generation: u64,
    pub status: HighlightStatus,
    /// Ordered, non-overlapping spans. Colors are resolved by each frontend.
    pub spans: Vec<HighlightSpan>,
    pub brackets: Vec<HighlightSpan>,
    pub structure: Vec<StructureRange>,
    pub normal_lines: Vec<usize>,
}

impl HighlightSnapshot {
    pub fn valid_for(&self, snapshot: &TextSnapshot, text: &str) -> bool {
        if &self.snapshot != snapshot || self.text_len != text.len() {
            return false;
        }
        fn valid_spans(spans: &[HighlightSpan], text: &str) -> bool {
            let mut previous_end = 0;
            spans.iter().all(|span| {
                let valid = previous_end <= span.start
                    && span.start < span.end
                    && span.end <= text.len()
                    && text.is_char_boundary(span.start)
                    && text.is_char_boundary(span.end);
                previous_end = span.end;
                valid
            })
        }
        let offset =
            |offset: usize| offset <= text.len() && text.is_char_boundary(offset);
        let optional = |value: Option<usize>| value.is_none_or(&offset);
        let pair = |value: Option<(usize, usize)>| {
            value.is_none_or(|(start, end)| {
                start < end && offset(start) && offset(end)
            })
        };
        let mut previous = 0;
        let valid_structure = self.structure.iter().all(|range| {
            let valid = range.start == previous
                && range.start < range.end
                && offset(range.start)
                && (range.end == text.len() + 1 || offset(range.end))
                && optional(range.matching)
                && optional(range.parent)
                && pair(range.enclosing)
                && pair(range.parentheses)
                && range.headers.iter().all(|value| offset(*value))
                && range
                    .previous
                    .iter()
                    .chain(range.next.iter())
                    .all(|value| optional(*value));
            previous = range.end;
            valid
        }) && (self.structure.is_empty()
            || previous == text.len() + 1);
        let last_line = text.bytes().filter(|byte| *byte == b'\n').count();
        valid_spans(&self.spans, text)
            && valid_spans(&self.brackets, text)
            && valid_structure
            && self
                .normal_lines
                .windows(2)
                .all(|lines| lines[0] < lines[1])
            && self.normal_lines.iter().all(|line| *line <= last_line)
    }
}

/// A transient code block (for example from a hover), without a disk buffer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeBlock {
    pub language: String,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeHighlights {
    pub code: CodeBlock,
    pub spans: Vec<HighlightSpan>,
}
