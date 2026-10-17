use lapce_core::buffer::{Buffer, rope_text::RopeTextRef};
use lapce_engine::{
    completion::{CompletionState, completion_lens_text},
    inline_completion::{
        InlineCompletionItem, InlineCompletionState, InlineCompletionText,
        inline_completion_text,
    },
};
use lapce_rpc::{plugin::PluginId, proxy::ProxyNotification};
use lapce_xi_rope::Rope;
use lsp_types::{
    CompletionItem, CompletionResponse, CompletionTextEdit,
    InlineCompletionItem as LspInlineItem, InsertTextFormat, Position, Range,
    TextEdit,
};

fn candidate(label: &str) -> CompletionItem {
    CompletionItem {
        label: label.into(),
        ..Default::default()
    }
}

#[test]
fn completion_lifecycle_filters_candidates_and_ignores_stale_responses() {
    let mut state = CompletionState::default();
    state.begin("file.rs".into(), 0, "pr".into());
    let request = state.request("file.rs".into(), "".into(), Position::new(0, 0));
    let ProxyNotification::Completion { request_id, .. } = request else {
        panic!("expected completion");
    };
    state.receive(
        request_id,
        "",
        &CompletionResponse::Array(vec![candidate("println"), candidate("other")]),
        PluginId(1),
    );
    assert_eq!(state.filtered_items.len(), 1);
    assert_eq!(state.current_item(0).unwrap().item.label, "println");
    state.begin("other.rs".into(), 0, "".into());
    state.receive(
        request_id,
        "",
        &CompletionResponse::Array(vec![candidate("stale")]),
        PluginId(1),
    );
    assert!(state.filtered_items.is_empty());
    let active_request = state.request_id;
    state.cancel();
    state.receive(
        active_request,
        "",
        &CompletionResponse::Array(vec![candidate("cancelled")]),
        PluginId(1),
    );
    assert!(state.filtered_items.is_empty());
}

#[test]
fn completion_highlight_indices_are_characters_even_with_unicode_prefix() {
    let mut state = CompletionState::default();
    state.begin("file.rs".into(), 0, "pr".into());
    let item = CompletionItem {
        label: "🌍 print".into(),
        filter_text: Some("print".into()),
        ..Default::default()
    };
    state.receive(
        state.request_id,
        "",
        &CompletionResponse::Array(vec![item]),
        PluginId(1),
    );
    assert_eq!(state.current_item(0).unwrap().indices, vec![2, 3]);
}

#[test]
fn completion_preview_uses_snippet_text_and_reports_unchanged() {
    let mut state = CompletionState::default();
    state.begin("file.rs".into(), 0, "pr".into());
    let item = CompletionItem {
        label: "println".into(),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range: Range::new(Position::new(0, 0), Position::new(0, 2)),
            new_text: "println(${1:value})$0".into(),
        })),
        ..Default::default()
    };
    state.receive(
        state.request_id,
        "",
        &CompletionResponse::Array(vec![item]),
        PluginId(1),
    );
    let rope = Rope::from("pr");
    assert_eq!(
        completion_lens_text(RopeTextRef::new(&rope), 2, &state, 0, None),
        Some(Some("intln(value)".into()))
    );
    assert_eq!(
        completion_lens_text(
            RopeTextRef::new(&rope),
            2,
            &state,
            0,
            Some("intln(value)")
        ),
        Some(None)
    );
}

fn inline_item(text: &str) -> InlineCompletionItem {
    InlineCompletionItem {
        insert_text: text.into(),
        filter_text: None,
        range: None,
        command: None,
        insert_text_format: None,
    }
}

#[test]
fn inline_response_requires_current_request_and_document_revision() {
    let mut state = InlineCompletionState::default();
    let old = state.begin("file.rs".into(), 0, 1);
    let current = state.begin("file.rs".into(), 1, 2);
    assert!(!state.receive(old, 1, im::vector![inline_item("stale")]));
    assert!(!state.receive(current, 1, im::vector![inline_item("wrong revision")]));
    assert!(state.receive(current, 2, im::vector![inline_item("current")]));
    assert_eq!(state.current_item(0).unwrap().insert_text, "current");
    state.cancel();
    assert!(!state.receive(current, 2, im::vector![inline_item("cancelled")]));
}

#[test]
fn inline_utf16_ranges_and_previews_use_actual_cursor_prefix() {
    let buffer = Buffer::new("🌍pri tail");
    let item = InlineCompletionItem::from_lsp(
        &buffer,
        LspInlineItem {
            insert_text: "println(${1:value})".into(),
            filter_text: None,
            range: Some(Range::new(Position::new(0, 2), Position::new(0, 5))),
            command: None,
            insert_text_format: Some(InsertTextFormat::SNIPPET),
        },
    );
    assert_eq!(item.range, Some(4..7));
    let rope = Rope::from("🌍pri tail");
    assert_eq!(
        inline_completion_text(RopeTextRef::new(&rope), 7, 7, &item, None),
        InlineCompletionText::Set("ntln(value)".into(), 7)
    );
    assert_eq!(
        inline_completion_text(
            RopeTextRef::new(&rope),
            7,
            7,
            &item,
            Some("ntln(value)")
        ),
        InlineCompletionText::Unchanged
    );
}
