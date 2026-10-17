//! Serializable document location shared by frontend session persistence.
use crate::document::DocContent;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(from = "StoredDocumentViewState", into = "StoredDocumentViewState")]
pub struct DocumentViewState {
    pub content: DocContent,
    /// Legacy scratch content; file snapshots never carry unsaved text.
    pub unsaved: Option<String>,
    pub offset: usize,
    pub scroll_offset: (f64, f64),
    pub confirmed: Option<bool>,
}

// Keep the persisted schema while enforcing browser policy in both directions.
#[derive(Serialize, Deserialize)]
struct StoredDocumentViewState {
    content: DocContent,
    unsaved: Option<String>,
    offset: usize,
    scroll_offset: (f64, f64),
    confirmed: Option<bool>,
}
impl From<StoredDocumentViewState> for DocumentViewState {
    fn from(state: StoredDocumentViewState) -> Self {
        Self::capture(
            state.content,
            state.unsaved,
            state.offset,
            state.scroll_offset,
            state.confirmed,
        )
    }
}
impl From<DocumentViewState> for StoredDocumentViewState {
    fn from(state: DocumentViewState) -> Self {
        let state = DocumentViewState::capture(
            state.content,
            state.unsaved,
            state.offset,
            state.scroll_offset,
            state.confirmed,
        );
        Self {
            content: state.content,
            unsaved: state.unsaved,
            offset: state.offset,
            scroll_offset: state.scroll_offset,
            confirmed: state.confirmed,
        }
    }
}
impl DocumentViewState {
    /// Disk-backed browser tabs have no local edits to save on close.
    pub fn requires_close_confirmation(
        content: &DocContent,
        pristine: bool,
    ) -> bool {
        matches!(content, DocContent::Scratch { .. }) && !pristine
    }
    pub fn capture(
        mut content: DocContent,
        mut unsaved: Option<String>,
        offset: usize,
        scroll_offset: (f64, f64),
        confirmed: Option<bool>,
    ) -> Self {
        if let DocContent::File { read_only, .. } = &mut content {
            *read_only = true;
            unsaved = None;
        }
        Self {
            content,
            unsaved,
            offset,
            scroll_offset,
            confirmed,
        }
    }
}
