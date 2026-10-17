//! Completion request lifecycle, candidates and fuzzy filtering without a UI loop.
use crate::snippet::Snippet;
use lapce_core::{buffer::rope_text::RopeText, rope_text_pos::RopeTextPosition};
use lapce_rpc::{plugin::PluginId, proxy::ProxyNotification};
use lsp_types::{
    CompletionItem, CompletionResponse, CompletionTextEdit, InsertTextFormat,
    Position,
};
use nucleo::Utf32Str;
use parking_lot::Mutex;
use std::{borrow::Cow, path::PathBuf, str::FromStr, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CompletionStatus {
    Inactive,
    Started,
    Done,
}

#[derive(Clone, PartialEq)]
pub struct ScoredCompletionItem {
    pub item: CompletionItem,
    pub plugin_id: PluginId,
    pub score: u32,
    pub label_score: u32,
    pub indices: Vec<usize>,
}

#[derive(Clone)]
pub struct CompletionState {
    pub status: CompletionStatus,
    pub request_id: usize,
    pub input_id: usize,
    pub path: PathBuf,
    pub offset: usize,
    pub input: String,
    pub input_items: im::HashMap<String, im::Vector<ScoredCompletionItem>>,
    pub filtered_items: im::Vector<ScoredCompletionItem>,
    matcher: Arc<Mutex<nucleo::Matcher>>,
}

impl Default for CompletionState {
    fn default() -> Self {
        Self {
            status: CompletionStatus::Inactive,
            request_id: 0,
            input_id: 0,
            path: PathBuf::new(),
            offset: 0,
            input: String::new(),
            input_items: im::HashMap::new(),
            filtered_items: im::Vector::new(),
            matcher: Arc::new(Mutex::new(nucleo::Matcher::new(
                nucleo::Config::DEFAULT,
            ))),
        }
    }
}

impl CompletionState {
    pub fn begin(&mut self, path: PathBuf, offset: usize, input: String) {
        self.path = path;
        self.offset = offset;
        self.input = input;
        self.status = CompletionStatus::Started;
        self.input_items.clear();
        self.filtered_items.clear();
        self.request_id += 1;
    }

    pub fn request(
        &mut self,
        path: PathBuf,
        input: String,
        position: Position,
    ) -> ProxyNotification {
        self.input_items.insert(input.clone(), im::Vector::new());
        ProxyNotification::Completion {
            request_id: self.request_id,
            path,
            input,
            position,
        }
    }

    pub fn cancel(&mut self) {
        self.status = CompletionStatus::Inactive;
        self.input_id = 0;
        self.input.clear();
        self.input_items.clear();
        self.filtered_items.clear();
    }

    pub fn update_input(&mut self, input: String) {
        if self.status == CompletionStatus::Inactive {
            return;
        }
        self.input = input;
        self.filter_items();
    }

    pub fn current_item(&self, selected: usize) -> Option<&ScoredCompletionItem> {
        self.filtered_items.get(selected)
    }

    /// Handle the response to a completion request.
    pub fn receive(
        &mut self,
        request_id: usize,
        input: &str,
        resp: &CompletionResponse,
        plugin_id: PluginId,
    ) {
        // If we've been canceled or the request id is old, ignore the response.
        if self.status == CompletionStatus::Inactive || self.request_id != request_id
        {
            return;
        }

        let items = match resp {
            CompletionResponse::Array(items) => items,
            // TODO: Possibly handle the 'is_incomplete' field on List.
            CompletionResponse::List(list) => &list.items,
        };
        let items: im::Vector<ScoredCompletionItem> = items
            .iter()
            .map(|i| ScoredCompletionItem {
                item: i.to_owned(),
                plugin_id,
                score: 0,
                label_score: 0,
                indices: Vec::new(),
            })
            .collect();
        self.input_items.insert(input.to_string(), items);
        self.filter_items();
    }

    fn all_items(&self) -> im::Vector<ScoredCompletionItem> {
        self.input_items
            .get(&self.input)
            .cloned()
            .filter(|items| !items.is_empty())
            .unwrap_or_else(move || {
                self.input_items.get("").cloned().unwrap_or_default()
            })
    }

    pub fn filter_items(&mut self) {
        self.input_id += 1;
        if self.input.is_empty() {
            self.filtered_items = self.all_items();
            return;
        }

        // Filter the items by the fuzzy matching with the input text.
        let all_items = self.all_items();
        let mut matcher_guard = self.matcher.lock();
        let matcher = &mut *matcher_guard;
        let mut items: im::Vector<ScoredCompletionItem> = {
            let pattern = nucleo::pattern::Pattern::parse(
                &self.input,
                nucleo::pattern::CaseMatching::Ignore,
                nucleo::pattern::Normalization::Smart,
            );
            all_items
                .iter()
                .filter_map(|i| {
                    let filter_text =
                        i.item.filter_text.as_ref().unwrap_or(&i.item.label);
                    let shift = i
                        .item
                        .label
                        .match_indices(filter_text)
                        .next()
                        .map(|(shift, _)| i.item.label[..shift].chars().count())
                        .unwrap_or(0);
                    let mut indices = Vec::new();
                    let mut filter_text_buf = Vec::new();
                    let filter_text =
                        Utf32Str::new(filter_text, &mut filter_text_buf);
                    if let Some(score) =
                        pattern.indices(filter_text, matcher, &mut indices)
                    {
                        if shift > 0 {
                            for idx in indices.iter_mut() {
                                *idx += shift as u32;
                            }
                        }
                        let mut item = i.clone();
                        item.score = score;
                        item.label_score = score;
                        item.indices =
                            indices.into_iter().map(|i| i as usize).collect();

                        let mut label_buf = Vec::new();
                        let label_text =
                            Utf32Str::new(&i.item.label, &mut label_buf);
                        if let Some(score) = pattern.score(label_text, matcher) {
                            item.label_score = score;
                        }
                        Some(item)
                    } else {
                        None
                    }
                })
                .collect()
        };
        // Sort all the items by their score, then their label score, then their length.
        items.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| b.label_score.cmp(&a.label_score))
                .then_with(|| a.item.label.len().cmp(&b.item.label.len()))
        });
        self.filtered_items = items;
    }
}

/// Get the text of the completion lens for the given completion item.
/// Returns `None` if the completion lens should be hidden.
/// Returns `Some(None)` if the completion lens should be shown, but not changed.
/// Returns `Some(Some(text))` if the completion lens should be shown and changed to the given text.
pub fn completion_lens_text(
    rope_text: impl RopeText,
    cursor_offset: usize,
    completion: &CompletionState,
    selected: usize,
    current_completion: Option<&str>,
) -> Option<Option<String>> {
    let item = &completion.current_item(selected)?.item;

    let item: Cow<str> = if let Some(edit) = &item.text_edit {
        // A text edit is used, because that is what will actually be inserted.

        let text_format = item
            .insert_text_format
            .unwrap_or(InsertTextFormat::PLAIN_TEXT);

        // We don't display insert and replace
        let CompletionTextEdit::Edit(edit) = edit else {
            return None;
        };
        // The completion offset can be different from the current cursor offset.
        let completion_offset = completion.offset;

        let start_offset = rope_text.prev_code_boundary(cursor_offset);
        let edit_start = rope_text.offset_of_position(&edit.range.start);

        // If the start of the edit isn't where the cursor currently is,
        // and it is not at the start of the completion, then we ignore it.
        // This captures most cases that we want, even if it skips over some
        // displayable edits.
        if start_offset != edit_start && completion_offset != edit_start {
            return None;
        }

        match text_format {
            InsertTextFormat::PLAIN_TEXT => {
                // This is not entirely correct because it assumes that the position is
                // `{start,end}_offset` when it may not necessarily be.
                Cow::Borrowed(&edit.new_text)
            }
            InsertTextFormat::SNIPPET => {
                // Parse the snippet. Bail if it's invalid.
                let snippet = Snippet::from_str(&edit.new_text).ok()?;

                let text = snippet.text();

                Cow::Owned(text)
            }
            _ => {
                // We don't know how to support this text format.
                return None;
            }
        }
    } else {
        // There's no specific text edit, so we just use the label.
        Cow::Borrowed(&item.label)
    };
    // We strip the prefix of the current input from the label.
    // So that, for example, `p` with a completion of `println` only sets the lens text to `rintln`.
    // If the text does not include a prefix in the expected position, then we do not display it.
    let item = item.as_ref().strip_prefix(&completion.input)?;

    // Get only the first line of text, because Lapce does not currently support
    // multi-line phantom text.
    let item = item.lines().next().unwrap_or(item);

    if Some(item) == current_completion {
        // If the item is the same as the current completion, then we don't display it.
        Some(None)
    } else {
        Some(Some(item.to_string()))
    }
}
