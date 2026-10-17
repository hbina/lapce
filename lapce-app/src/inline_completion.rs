//! Floem adapter for the headless inline completion controller.
use crate::{config::LapceConfig, doc::Doc, editor::EditorData};
use floem::reactive::{RwSignal, Scope, SignalGet, SignalUpdate, SignalWith};
use lapce_core::buffer::rope_text::{RopeText, RopeTextRef};
pub use lapce_engine::inline_completion::{
    InlineCompletionItem, InlineCompletionStatus,
};
use lapce_engine::inline_completion::{
    InlineCompletionState, InlineCompletionText, inline_completion_text,
};
use lsp_types::InsertTextFormat;

pub fn apply_item(
    item: &InlineCompletionItem,
    editor: &EditorData,
    start_offset: usize,
) -> anyhow::Result<()> {
    let text_format = item
        .insert_text_format
        .unwrap_or(InsertTextFormat::PLAIN_TEXT);

    let selection = item.selection(start_offset);

    match text_format {
        InsertTextFormat::PLAIN_TEXT => editor.do_edit(
            &selection,
            &[(selection.clone(), item.insert_text.as_str())],
        ),
        InsertTextFormat::SNIPPET => {
            editor.completion_apply_snippet(
                &item.insert_text,
                &selection,
                Vec::new(),
                start_offset,
            )?;
        }
        _ => {
            // We don't know how to support this text format
        }
    }

    Ok(())
}

#[derive(Clone)]
pub struct InlineCompletionData {
    pub state: InlineCompletionState,
    pub active: RwSignal<usize>,
}

impl std::ops::Deref for InlineCompletionData {
    type Target = InlineCompletionState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
impl std::ops::DerefMut for InlineCompletionData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl InlineCompletionData {
    pub fn new(cx: Scope) -> Self {
        Self {
            state: InlineCompletionState::default(),
            active: cx.create_rw_signal(0),
        }
    }
    pub fn current_item(&self) -> Option<&InlineCompletionItem> {
        self.state.current_item(self.active.get_untracked())
    }
    pub fn next(&mut self) {
        if !self.items.is_empty() {
            self.active
                .set((self.active.get_untracked() + 1) % self.items.len());
        }
    }
    pub fn previous(&mut self) {
        if !self.items.is_empty() {
            let active = self.active.get_untracked();
            self.active.set(if active == 0 {
                self.items.len() - 1
            } else {
                active - 1
            });
        }
    }
    pub fn cancel(&mut self) {
        self.state.cancel();
        self.active.set(0);
    }
    pub fn receive(
        &mut self,
        request_id: u64,
        revision: u64,
        items: im::Vector<InlineCompletionItem>,
    ) -> bool {
        let accepted = self.state.receive(request_id, revision, items);
        if accepted {
            self.active.set(0);
        }
        accepted
    }
    pub fn update_doc(&self, doc: &Doc, offset: usize) {
        if self.status != InlineCompletionStatus::Active || self.items.is_empty() {
            doc.clear_inline_completion();
            return;
        }
        if self.active.get_untracked() >= self.items.len() {
            self.active.set(0);
        }
        self.update_inline_completion(
            &doc.common.config.get_untracked(),
            doc,
            offset,
        );
    }

    pub fn update_inline_completion(
        &self,
        config: &LapceConfig,
        doc: &Doc,
        cursor_offset: usize,
    ) {
        if !config.editor.enable_inline_completion {
            doc.clear_inline_completion();
            return;
        }

        let text = doc.buffer.with_untracked(|buffer| buffer.text().clone());
        let text = RopeTextRef::new(&text);
        let Some(item) = self.current_item() else {
            // TODO(minor): should we cancel completion
            return;
        };

        let completion = doc.inline_completion.with_untracked(|cur| {
            let cur = cur.as_deref();
            inline_completion_text(text, self.start_offset, cursor_offset, item, cur)
        });

        match completion {
            InlineCompletionText::Hide => {
                doc.clear_inline_completion();
            }
            InlineCompletionText::Unchanged => {}
            InlineCompletionText::Set(new, offset) => {
                let (line, col) = text.offset_to_line_col(offset);
                doc.set_inline_completion(new, line, col);
            }
        }
    }
}
