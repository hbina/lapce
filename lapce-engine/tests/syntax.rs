use lapce_core::buffer::rope_text::RopeText;
use lapce_engine::{
    document::{DocContent, Document},
    session::SessionDocument,
};
use lapce_rpc::highlighting::{
    HighlightSnapshot, HighlightSpan, HighlightStatus, TextSnapshot,
};
use lapce_xi_rope::Rope;

fn fixture() -> (SessionDocument, HighlightSnapshot) {
    let mut document = SessionDocument::new(Document::new(
        DocContent::File {
            path: "a.rs".into(),
            read_only: true,
        },
        Rope::from("// é\nfn main() {}"),
    ));
    document.initialize_sync(7);
    let snapshot = TextSnapshot {
        instance: "backend-buffer-1".into(),
        revision: 7,
    };
    document.document.backend_snapshot = Some(snapshot.clone());
    let result = HighlightSnapshot {
        path: "a.rs".into(),
        language: "rust".into(),
        snapshot,
        generation: 1,
        text_len: document.buffer.len(),
        status: HighlightStatus::Ready,
        spans: vec![HighlightSpan {
            start: 0,
            end: 5,
            scope: "comment".into(),
        }],
        brackets: vec![],
        structure: vec![],
        normal_lines: vec![],
    };
    (document, result)
}

#[test]
fn presentation_accepts_only_its_current_backend_snapshot() {
    let (mut document, result) = fixture();
    assert!(
        document.syntax.layers.is_none(),
        "frontend must not initialize a parser"
    );
    for bad in [
        {
            let mut bad = result.clone();
            bad.path = "other.rs".into();
            bad
        },
        {
            let mut bad = result.clone();
            bad.snapshot.instance = "retired-backend".into();
            bad
        },
        {
            let mut bad = result.clone();
            bad.snapshot.revision += 1;
            bad
        },
        {
            let mut bad = result.clone();
            bad.text_len += 1;
            bad
        },
        {
            let mut bad = result.clone();
            bad.spans[0].end = 4;
            bad
        }, // inside é
        {
            let mut bad = result.clone();
            bad.spans.push(bad.spans[0].clone());
            bad
        },
    ] {
        assert!(!document.accept_highlights(bad));
    }
    assert!(document.accept_highlights(result.clone()));
    assert!(!document.accept_highlights(result.clone()));
    let mut enrichment = result;
    enrichment.generation += 1;
    enrichment.spans.push(HighlightSpan {
        start: 6,
        end: 8,
        scope: "keyword".into(),
    });
    assert!(document.accept_highlights(enrichment));
    assert!(document.syntax.layers.is_none());
}

#[test]
fn reload_and_disconnect_clear_styles_and_reject_delayed_results() {
    let (mut document, result) = fixture();
    assert!(document.accept_highlights(result.clone()));
    document
        .document
        .reload(Rope::from("// é\nfn next() {}"), true);
    assert!(document.highlights.is_none());
    assert!(document.syntax.styles.is_none());
    assert!(!document.accept_highlights(result));
    let (mut document, result) = fixture();
    assert!(document.accept_highlights(result.clone()));
    document.backend_disconnected();
    assert!(document.highlights.is_none());
    assert!(!document.accept_highlights(result));
}
