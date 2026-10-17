use lapce_engine::{document::DocContent, view_state::DocumentViewState};
#[test]
fn file_locations_preserve_navigation_and_drop_unsaved_content() {
    let state = DocumentViewState::capture(
        DocContent::File {
            path: "source.rs".into(),
            read_only: false,
        },
        Some("obsolete edits".into()),
        42,
        (3.0, 12.0),
        Some(true),
    );
    assert!(state.content.read_only());
    assert!(state.unsaved.is_none());
    let value = serde_json::to_value(&state).unwrap();
    assert_eq!(value["offset"], 42);
    assert_eq!(value["scroll_offset"], serde_json::json!([3.0, 12.0]));
    assert!(value["unsaved"].is_null());
    let restored: DocumentViewState = serde_json::from_value(value).unwrap();
    assert_eq!(restored.confirmed, Some(true));
    assert_eq!(
        restored.content.path().unwrap().to_str().unwrap(),
        "source.rs"
    );
}
#[test]
fn legacy_location_schema_remains_readable() {
    let state: DocumentViewState = serde_json::from_value(serde_json::json!({
        "content": {"File": {"path":"old.rs", "read_only":false}},
        "unsaved":"old edits", "offset":7, "scroll_offset":[0.0,4.0], "confirmed":null
    })).unwrap();
    assert!(state.content.read_only());
    assert!(state.unsaved.is_none());
    assert_eq!(state.offset, 7);
}

#[test]
fn serialization_normalizes_legacy_fields_even_after_direct_mutation() {
    let state = DocumentViewState {
        content: DocContent::File {
            path: "file.rs".into(),
            read_only: false,
        },
        unsaved: Some("local edits".into()),
        offset: 0,
        scroll_offset: (0.0, 0.0),
        confirmed: None,
    };
    let value = serde_json::to_value(state).unwrap();
    assert_eq!(value["content"]["File"]["read_only"], true);
    assert!(value["unsaved"].is_null());
}

#[test]
fn read_only_file_tabs_never_prompt_to_save_even_when_missing_on_disk() {
    for read_only in [false, true] {
        let content = DocContent::File {
            path: "deleted.rs".into(),
            read_only,
        };
        for pristine in [false, true] {
            assert!(!DocumentViewState::requires_close_confirmation(
                &content, pristine
            ));
        }
    }
    assert!(!DocumentViewState::requires_close_confirmation(
        &DocContent::Local,
        false
    ));
}
