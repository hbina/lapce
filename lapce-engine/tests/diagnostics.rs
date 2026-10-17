use lapce_engine::diagnostics::DiagnosticState;
use lapce_xi_rope::{Delta, Interval, Rope};
use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

fn diagnostic(severity: DiagnosticSeverity, start: u32, end: u32) -> Diagnostic {
    let mut diagnostic = Diagnostic::new_simple(
        Range::new(Position::new(0, start), Position::new(0, end)),
        "issue".into(),
    );
    diagnostic.severity = Some(severity);
    diagnostic
}

#[test]
fn unopened_diagnostics_gain_utf16_offsets_and_follow_live_edits() {
    let mut state = DiagnosticState::default();
    assert!(state.publish(
        vec![diagnostic(DiagnosticSeverity::ERROR, 1, 3)],
        Some(1),
        None
    ));
    assert_eq!(state.entries(DiagnosticSeverity::ERROR)[0].range, None);
    let text = Rope::from("a🌍b");
    assert!(state.bind(&text, 1));
    assert_eq!(
        state.entries(DiagnosticSeverity::ERROR)[0].range,
        Some((1, 5))
    );
    state.apply_delta(&Delta::simple_edit(
        Interval::new(0, 0),
        Rope::from("hi"),
        text.len(),
    ));
    assert_eq!(
        state.entries(DiagnosticSeverity::ERROR)[0].range,
        Some((3, 7))
    );
    assert_eq!(state.counts(), (1, 0));
}

#[test]
fn stale_diagnostic_publications_cannot_replace_a_current_snapshot() {
    let mut state = DiagnosticState::default();
    let text = Rope::from("hello");
    assert!(state.publish(
        vec![diagnostic(DiagnosticSeverity::ERROR, 0, 1)],
        Some(5),
        Some((&text, 5))
    ));
    assert!(!state.publish(
        vec![diagnostic(DiagnosticSeverity::WARNING, 1, 2)],
        Some(4),
        Some((&text, 5))
    ));
    assert_eq!(state.version, Some(5));
    assert_eq!(state.counts(), (1, 0));
    assert_eq!(
        state.entries(DiagnosticSeverity::ERROR)[0].range,
        Some((0, 1))
    );
    assert!(state.publish(vec![], Some(5), Some((&text, 5))));
    assert_eq!(state.counts(), (0, 0));
    assert!(state.entries(DiagnosticSeverity::ERROR).is_empty());
}

#[test]
fn opening_a_different_version_does_not_map_stale_unopened_diagnostics() {
    let mut state = DiagnosticState::default();
    state.publish(
        vec![diagnostic(DiagnosticSeverity::ERROR, 0, 1)],
        Some(2),
        None,
    );
    assert!(!state.bind(&Rope::from("hello"), 3));
    assert!(state.items.is_empty());
    assert!(state.entries(DiagnosticSeverity::ERROR).is_empty());
}

#[test]
fn unversioned_diagnostics_remain_supported_and_filter_by_severity() {
    let mut state = DiagnosticState::default();
    let text = Rope::from("hello");
    state.publish(
        vec![
            diagnostic(DiagnosticSeverity::WARNING, 2, 3),
            diagnostic(DiagnosticSeverity::ERROR, 0, 1),
        ],
        None,
        Some((&text, 10)),
    );
    assert_eq!(state.counts(), (1, 1));
    assert_eq!(
        state.entries(DiagnosticSeverity::WARNING)[0].range,
        Some((2, 3))
    );
    assert_eq!(
        state.entries(DiagnosticSeverity::ERROR)[0].range,
        Some((0, 1))
    );
}
