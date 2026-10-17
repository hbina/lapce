use crate::reactive_model::ModelField;
use floem::reactive::{RwSignal, Scope, SignalUpdate, SignalWith};
use lapce_engine::diagnostics::DiagnosticState;
use lapce_xi_rope::{Rope, RopeDelta, spans::Spans};
use lsp_types::{Diagnostic, DiagnosticSeverity};

#[derive(Clone, Debug)]
pub struct DiagnosticData {
    pub expanded: RwSignal<bool>,
    pub diagnostics: ModelField<DiagnosticState, im::Vector<Diagnostic>>,
    pub diagnostics_span: ModelField<DiagnosticState, Spans<Diagnostic>>,
    state: RwSignal<DiagnosticState>,
    changed: RwSignal<()>,
}

impl DiagnosticData {
    pub fn publish_prepared(
        &self,
        publication: lapce_engine::diagnostics::DiagnosticPublication,
    ) -> bool {
        let accepted = self
            .state
            .try_update(|state| publication.apply(state))
            .unwrap_or(false);
        if accepted {
            self.changed.set(());
        }
        accepted
    }
    pub fn counts(&self) -> (usize, usize) {
        self.diagnostics.track();
        self.state.with_untracked(DiagnosticState::counts)
    }
    pub fn new(scope: Scope) -> Self {
        let state = scope.create_rw_signal(DiagnosticState::default());
        let changed = scope.create_rw_signal(());
        Self {
            expanded: scope.create_rw_signal(true),
            state,
            changed,
            diagnostics: ModelField::new(
                state,
                changed,
                |state| &state.items,
                |state| &mut state.items,
                None,
            ),
            diagnostics_span: ModelField::new(
                state,
                changed,
                |state| &state.spans,
                |state| &mut state.spans,
                None,
            ),
        }
    }

    pub fn publish(
        &self,
        diagnostics: Vec<Diagnostic>,
        version: Option<i32>,
        document: Option<(&Rope, u64)>,
    ) -> bool {
        let accepted = self
            .state
            .try_update(|state| state.publish(diagnostics, version, document))
            .unwrap_or(false);
        if accepted {
            self.changed.set(());
        }
        accepted
    }

    pub fn bind(&self, text: &Rope, revision: u64) {
        if self
            .state
            .try_update(|state| state.bind(text, revision))
            .is_some()
        {
            self.changed.set(());
        }
    }

    pub fn apply_delta(&self, delta: &RopeDelta) {
        if self
            .state
            .try_update(|state| state.apply_delta(delta))
            .is_some()
        {
            self.changed.set(());
        }
    }

    pub fn entries(
        &self,
        severity: DiagnosticSeverity,
        tracked: bool,
    ) -> im::Vector<lapce_engine::diagnostics::EditorDiagnostic> {
        if tracked {
            self.diagnostics.track();
        }
        self.state.with_untracked(|state| state.entries(severity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range};
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn gui_diagnostic_consumers_share_state_and_ignore_stale_publications() {
        let scope = Scope::new();
        let data = DiagnosticData::new(scope);
        let other_consumer = data.clone();
        let reads = Rc::new(Cell::new(0));
        scope.create_effect({
            let data = data.clone();
            let reads = reads.clone();
            move |_| {
                data.counts();
                reads.set(reads.get() + 1);
            }
        });
        let text = Rope::from("hello");
        let mut issue = Diagnostic::new_simple(
            Range::new(Position::new(0, 0), Position::new(0, 1)),
            "issue".into(),
        );
        issue.severity = Some(DiagnosticSeverity::ERROR);
        assert!(data.publish(vec![issue], Some(3), Some((&text, 3))));
        assert_eq!(
            other_consumer.entries(DiagnosticSeverity::ERROR, false)[0].range,
            Some((0, 1))
        );
        assert_eq!(reads.get(), 2);
        assert!(!data.publish(vec![], Some(2), Some((&text, 3))));
        assert_eq!(reads.get(), 2);
        assert_eq!(other_consumer.counts(), (1, 0));
        // Expansion is view state and cannot invalidate the semantic model.
        data.expanded.set(false);
        assert_eq!(reads.get(), 2);
        scope.dispose();
    }
}
