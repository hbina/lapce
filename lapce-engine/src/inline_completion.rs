//! Inline completion lifecycle and preview behavior, without GUI scheduling.
use crate::snippet::Snippet;
use lapce_core::{
    buffer::{Buffer, rope_text::RopeText},
    rope_text_pos::RopeTextPosition,
    selection::Selection,
};
use lsp_types::InsertTextFormat;
use std::{borrow::Cow, ops::Range, path::PathBuf, str::FromStr};

/// Redefinition of lsp types inline completion item with offset range
#[derive(Debug, Clone)]
pub struct InlineCompletionItem {
    /// The text to replace the range with.
    pub insert_text: String,
    /// Text used to decide if this inline completion should be shown.
    pub filter_text: Option<String>,
    /// The range (of offsets) to replace
    pub range: Option<Range<usize>>,
    pub command: Option<lsp_types::Command>,
    pub insert_text_format: Option<InsertTextFormat>,
}
impl InlineCompletionItem {
    pub fn from_lsp(buffer: &Buffer, item: lsp_types::InlineCompletionItem) -> Self {
        let range = item.range.map(|r| {
            let start = buffer.offset_of_position(&r.start);
            let end = buffer.offset_of_position(&r.end);
            start..end
        });
        Self {
            insert_text: item.insert_text,
            filter_text: item.filter_text,
            range,
            command: item.command,
            insert_text_format: item.insert_text_format,
        }
    }

    pub fn selection(&self, start_offset: usize) -> Selection {
        self.range
            .as_ref()
            .map(|range| Selection::region(range.start, range.end))
            .unwrap_or_else(|| Selection::caret(start_offset))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineCompletionStatus {
    /// The inline completion is not active.
    Inactive,
    /// The inline completion is active and is waiting for the server to respond.
    Started,
    /// The inline completion is active and has received a response from the server.
    Active,
}

#[derive(Clone)]
pub struct InlineCompletionState {
    pub status: InlineCompletionStatus,
    pub items: im::Vector<InlineCompletionItem>,
    pub start_offset: usize,
    pub path: PathBuf,
    request_id: u64,
    revision: u64,
}

impl Default for InlineCompletionState {
    fn default() -> Self {
        Self {
            status: InlineCompletionStatus::Inactive,
            items: im::Vector::new(),
            start_offset: 0,
            path: PathBuf::new(),
            request_id: 0,
            revision: 0,
        }
    }
}

impl InlineCompletionState {
    pub fn begin(
        &mut self,
        path: PathBuf,
        start_offset: usize,
        revision: u64,
    ) -> u64 {
        if self.path != path || self.start_offset != start_offset {
            self.items.clear();
        }
        self.request_id += 1;
        self.revision = revision;
        self.path = path;
        self.start_offset = start_offset;
        self.status = InlineCompletionStatus::Started;
        self.request_id
    }

    pub fn receive(
        &mut self,
        request_id: u64,
        revision: u64,
        items: im::Vector<InlineCompletionItem>,
    ) -> bool {
        if self.status == InlineCompletionStatus::Inactive
            || self.request_id != request_id
            || self.revision != revision
        {
            return false;
        }
        self.items = items;
        self.status = InlineCompletionStatus::Active;
        true
    }

    pub fn current_item(&self, selected: usize) -> Option<&InlineCompletionItem> {
        self.items.get(selected)
    }

    pub fn cancel(&mut self) {
        self.items.clear();
        self.status = InlineCompletionStatus::Inactive;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum InlineCompletionText {
    Hide,
    Unchanged,
    Set(String, usize),
}

/// Get the text of the inline completion item
pub fn inline_completion_text(
    rope_text: impl RopeText,
    start_offset: usize,
    cursor_offset: usize,
    item: &InlineCompletionItem,
    current_completion: Option<&str>,
) -> InlineCompletionText {
    let text_format = item
        .insert_text_format
        .unwrap_or(InsertTextFormat::PLAIN_TEXT);

    let text = match text_format {
        InsertTextFormat::PLAIN_TEXT => Cow::Borrowed(&item.insert_text),
        InsertTextFormat::SNIPPET => {
            let Ok(snippet) = Snippet::from_str(&item.insert_text) else {
                return InlineCompletionText::Hide;
            };
            let text = snippet.text();

            Cow::Owned(text)
        }
        _ => {
            // We don't know how to support this text format
            return InlineCompletionText::Hide;
        }
    };

    let anchor = item
        .range
        .as_ref()
        .map(|range| range.start)
        .unwrap_or(start_offset);
    if anchor > cursor_offset
        || cursor_offset > rope_text.len()
        || rope_text.line_of_offset(anchor)
            != rope_text.line_of_offset(cursor_offset)
    {
        return InlineCompletionText::Hide;
    }
    let prefix = rope_text.slice_to_cow(anchor..cursor_offset);
    // We strip the prefix of the current input from the label.
    // So that, for example `p` with a completion of `println` will show `rintln`.
    let Some(text) = text.strip_prefix(prefix.as_ref()) else {
        return InlineCompletionText::Hide;
    };

    if Some(text) == current_completion {
        InlineCompletionText::Unchanged
    } else {
        InlineCompletionText::Set(text.to_string(), cursor_offset)
    }
}
