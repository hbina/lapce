//! Syntax jobs are executor-independent snapshots of an engine document.
use lapce_core::syntax::{Syntax, edit::SyntaxEdit};
use lapce_rpc::buffer::BufferId;
use lapce_xi_rope::Rope;

pub fn edits_from_deltas(
    deltas: &[crate::editing::DocumentDelta],
) -> Vec<SyntaxEdit> {
    deltas
        .iter()
        .map(|(before, delta, _)| SyntaxEdit::from_delta(before, delta.clone()))
        .collect()
}
use std::sync::atomic::Ordering;

pub struct SyntaxJob {
    pub(crate) document_id: BufferId,
    pub(crate) revision: u64,
    pub(crate) generation: u64,
    pub(crate) syntax: Syntax,
    pub(crate) text: Rope,
    pub(crate) edits: Option<Vec<SyntaxEdit>>,
}

pub struct SyntaxResult {
    pub(crate) document_id: BufferId,
    pub(crate) revision: u64,
    pub(crate) generation: u64,
    pub(crate) syntax: Syntax,
}

impl SyntaxJob {
    /// Cancellation can happen before or during parsing. Cancelled jobs publish
    /// no result; document identity/revision/generation are checked on acceptance.
    pub fn run(mut self) -> Option<SyntaxResult> {
        if self.syntax.cancel_flag.load(Ordering::Relaxed) != 0 {
            return None;
        }
        self.syntax
            .parse(self.revision, self.text, self.edits.as_deref());
        if self.syntax.cancel_flag.load(Ordering::Relaxed) != 0 {
            return None;
        }
        Some(SyntaxResult {
            document_id: self.document_id,
            revision: self.revision,
            generation: self.generation,
            syntax: self.syntax,
        })
    }
}
