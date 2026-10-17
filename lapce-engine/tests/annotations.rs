use lapce_core::buffer::rope_text::RopeText;
use lapce_engine::annotations::{diagnostic_spans, transform_completion_lens};
use lapce_xi_rope::{Delta, Interval, Rope};
use lsp_types::{Diagnostic, Position, Range};

#[test]
fn diagnostics_map_utf16_and_ignore_invalid_ranges() {
    let text = Rope::from("a🌍b");
    let valid = Diagnostic::new_simple(
        Range::new(Position::new(0, 1), Position::new(0, 3)),
        "valid".into(),
    );
    let reversed = Diagnostic::new_simple(
        Range::new(Position::new(0, 3), Position::new(0, 1)),
        "reversed".into(),
    );
    let surrogate = Diagnostic::new_simple(
        Range::new(Position::new(0, 2), Position::new(0, 3)),
        "surrogate".into(),
    );
    let spans = diagnostic_spans(&text, [reversed, surrogate, valid]);
    let spans: Vec<_> = spans.iter().collect();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].0, Interval::new(1, 5));
    assert_eq!(spans[0].1.message, "valid");
}

#[test]
fn completion_preview_tracks_each_edit_and_consumes_unicode_prefixes() {
    let before = Rope::from("hello world");
    let delta =
        Delta::simple_edit(Interval::new(0, 0), Rope::from("\n"), before.len());
    let (preview, position) =
        transform_completion_lens(&before, &delta, (0, 6), "🌍rest").unwrap();
    assert_eq!(position, (1, 6));
    let before = delta.apply(&before);
    let delta =
        Delta::simple_edit(Interval::new(7, 7), Rope::from("🌍"), before.len());
    let (preview, position) =
        transform_completion_lens(&before, &delta, position, &preview).unwrap();
    assert_eq!(preview, "rest");
    assert_eq!(position, (1, 10));
}

#[test]
fn mismatched_insertions_hide_the_preview_without_slicing_a_unicode_character() {
    let before = Rope::from("a");
    let delta =
        Delta::simple_edit(Interval::new(1, 1), Rope::from("x"), before.len());
    assert!(transform_completion_lens(&before, &delta, (0, 1), "🌍").is_none());
}

#[test]
fn engine_documents_own_hint_positions_and_reject_stale_hint_responses() {
    use lapce_engine::document::{DocContent, Document};
    let mut document = Document::new(DocContent::Local, Rope::from("abc"));
    let hint: lsp_types::InlayHint = serde_json::from_value(serde_json::json!({
        "position":{"line":0,"character":2},"label":"hint"
    }))
    .unwrap();
    let revision = document.buffer.rev();
    assert!(document.set_inlay_hints(revision, vec![hint.clone()]));
    document
        .raw_edit(
            &[(lapce_core::selection::Selection::caret(0), "🌍")],
            lapce_core::editor::EditType::Completion,
        )
        .unwrap();
    assert_eq!(document.buffer.text().to_string(), "🌍abc");
    let spans: Vec<_> = document.inlay_hints.as_ref().unwrap().iter().collect();
    assert_eq!(spans[0].0, Interval::new(6, 7));
    assert!(!document.set_inlay_hints(revision, vec![hint]));
    assert_eq!(
        document
            .inlay_hints
            .as_ref()
            .unwrap()
            .iter()
            .next()
            .unwrap()
            .0,
        Interval::new(6, 7)
    );
}

#[test]
fn inlay_construction_sorts_hints_and_ignores_invalid_utf16_positions() {
    let hints = [3, 2, 1].into_iter().map(|character| serde_json::from_value(
        serde_json::json!({"position":{"line":0,"character":character},"label":"hint"})
    ).unwrap()).collect();
    let spans = lapce_engine::annotations::inlay_spans(&Rope::from("a🌍b"), hints);
    let ranges: Vec<_> = spans.iter().map(|(interval, _)| interval).collect();
    assert_eq!(ranges, [Interval::new(1, 2), Interval::new(5, 6)]);
}

#[test]
fn highlights_validate_utf16_ranges_and_handle_overlaps() {
    use lapce_engine::annotations::{DocumentHighlight, document_highlights};
    let text = Rope::from("a🌍b");
    let make = |start, end| lsp_types::DocumentHighlight {
        range: Range::new(Position::new(0, start), Position::new(0, end)),
        kind: None,
    };
    let ranges = document_highlights(
        &text,
        [make(1, 3), make(0, 4), make(2, 3), make(3, 1), make(0, 99)],
    );
    assert_eq!(
        ranges,
        vec![
            DocumentHighlight { start: 0, end: 6 },
            DocumentHighlight { start: 1, end: 5 }
        ]
    );
    assert_eq!(DocumentHighlight::find_for_offset(&ranges, 6), Some(0));
    assert_eq!(DocumentHighlight::find_for_offset(&ranges, 7), None);
    let overlaps = vec![
        DocumentHighlight { start: 0, end: 20 },
        DocumentHighlight { start: 1, end: 2 },
        DocumentHighlight { start: 3, end: 4 },
    ];
    assert_eq!(DocumentHighlight::find_for_offset(&overlaps, 10), Some(0));
}

#[test]
fn semantic_spans_sort_validate_cancel_and_clear_empty_results() {
    use lapce_engine::annotations::semantic_spans;
    use lapce_rpc::style::{LineStyle, Style};
    let text = Rope::from("a🌍b");
    let make = |start, end| LineStyle {
        start,
        end,
        style: Style {
            fg_color: Some("type".into()),
        },
    };
    let styles = semantic_spans(
        &text,
        [
            make(5, 6),
            make(2, 5),
            make(1, 5),
            make(0, 1),
            make(0, 99),
            make(5, 4),
            make(1, 1),
        ],
        || false,
    )
    .unwrap();
    let ranges: Vec<_> = styles.iter().map(|(range, _)| range).collect();
    assert_eq!(
        ranges,
        vec![
            Interval::new(0, 1),
            Interval::new(1, 5),
            Interval::new(5, 6)
        ]
    );
    assert!(
        semantic_spans(&text, [], || false)
            .unwrap()
            .iter()
            .next()
            .is_none()
    );
    assert!(semantic_spans(&text, [make(0, 1)], || true).is_none());
    let mut calls = 0;
    assert!(
        semantic_spans(&text, [make(0, 1)], || {
            calls += 1;
            calls >= 3
        })
        .is_none()
    );
}
