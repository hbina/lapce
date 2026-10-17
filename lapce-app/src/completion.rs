use std::{path::PathBuf, sync::Arc};

use floem::{
    peniko::kurbo::Rect,
    reactive::{ReadSignal, RwSignal, Scope, SignalGet, SignalUpdate},
    views::editor::{id::EditorId, text::Document},
};
use lapce_core::movement::Movement;

use lsp_types::Position;

use crate::{config::LapceConfig, editor::EditorData};

use lapce_engine::completion::{CompletionState, completion_lens_text};
pub use lapce_engine::completion::{CompletionStatus, ScoredCompletionItem};

#[derive(Clone)]
pub struct CompletionData {
    pub state: CompletionState,
    pub active: RwSignal<usize>,
    pub layout_rect: Rect,
    pub latest_editor_id: Option<EditorId>,
    config: ReadSignal<Arc<LapceConfig>>,
}

impl std::ops::Deref for CompletionData {
    type Target = CompletionState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
impl std::ops::DerefMut for CompletionData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl CompletionData {
    pub fn new(cx: Scope, config: ReadSignal<Arc<LapceConfig>>) -> Self {
        Self {
            state: CompletionState::default(),
            active: cx.create_rw_signal(0),
            layout_rect: Rect::ZERO,
            latest_editor_id: None,
            config,
        }
    }

    pub fn request(
        &mut self,
        editor_id: EditorId,
        doc: &crate::doc::Doc,
        path: PathBuf,
        input: String,
        position: Position,
    ) {
        self.latest_editor_id = Some(editor_id);
        doc.notification_document(self.state.request(path, input, position));
    }

    pub fn cancel(&mut self) {
        self.state.cancel();
        self.latest_editor_id = None;
        self.active.set(0);
    }

    pub fn update_input(&mut self, input: String) {
        if self.state.status == CompletionStatus::Inactive {
            return;
        }
        self.state.update_input(input);
        self.active.set(0);
    }

    /// Move down in the list of items.
    pub fn next(&mut self) {
        let active = self.active.get_untracked();
        let new =
            Movement::Down.update_index(active, self.filtered_items.len(), 1, true);
        self.active.set(new);
    }

    /// Move up in the list of items.
    pub fn previous(&mut self) {
        let active = self.active.get_untracked();
        let new =
            Movement::Up.update_index(active, self.filtered_items.len(), 1, true);
        self.active.set(new);
    }

    /// The amount of items that can be displayed in the current layout.
    fn display_count(&self) -> usize {
        let config = self.config.get_untracked();
        ((self.layout_rect.size().height / config.editor.line_height() as f64)
            .floor() as usize)
            .saturating_sub(1)
    }

    /// Move to the next page of items.
    pub fn next_page(&mut self) {
        let count = self.display_count();
        let active = self.active.get_untracked();
        let new = Movement::Down.update_index(
            active,
            self.filtered_items.len(),
            count,
            false,
        );
        self.active.set(new);
    }

    /// Move to the previous page of items.
    pub fn previous_page(&mut self) {
        let count = self.display_count();
        let active = self.active.get_untracked();
        let new = Movement::Up.update_index(
            active,
            self.filtered_items.len(),
            count,
            false,
        );
        self.active.set(new);
    }

    /// The currently selected/active item.
    pub fn current_item(&self) -> Option<&ScoredCompletionItem> {
        self.state.current_item(self.active.get_untracked())
    }

    /// Update the completion lens of the document with the active completion item.
    pub fn update_document_completion(
        &self,
        editor_data: &EditorData,
        cursor_offset: usize,
    ) {
        let doc = editor_data.doc();

        if !doc.content.with_untracked(|content| content.is_file()) {
            return;
        }

        let config = self.config.get_untracked();

        if !config.editor.enable_completion_lens {
            doc.clear_completion_lens();
            return;
        }

        let completion_lens = completion_lens_text(
            doc.rope_text(),
            cursor_offset,
            &self.state,
            self.active.get_untracked(),
            doc.completion_lens().as_deref(),
        );
        match completion_lens {
            Some(Some(lens)) => {
                let offset = self.offset + self.input.len();
                // TODO: will need to be adjusted to use visual line.
                //   Could just store the offset in doc.
                let (line, col) = editor_data.editor.offset_to_line_col(offset);

                doc.set_completion_lens(lens, line, col);
            }
            // Unchanged
            Some(None) => {}
            None => {
                doc.clear_completion_lens();
            }
        }
    }
}
