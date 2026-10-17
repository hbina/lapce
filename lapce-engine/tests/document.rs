use lapce_core::{
    buffer::rope_text::RopeText,
    command::EditCommand,
    cursor::Cursor,
    editor::EditConf,
    register::{Clipboard, Register},
};
use lapce_engine::document::{DocContent, Document};
use lapce_xi_rope::Rope;
use lsp_types::{Position, Range, TextEdit};

#[derive(Default)]
struct MemoryClipboard(String);
impl Clipboard for MemoryClipboard {
    fn get_string(&mut self) -> Option<String> {
        Some(self.0.clone())
    }
    fn put_string(&mut self, text: impl AsRef<str>) {
        self.0 = text.as_ref().into();
    }
}

#[test]
fn edits_undo_redo_and_save_revision_work_without_gui() {
    let mut document = Document::new(DocContent::Local, Rope::from(""));
    let mut cursor = Cursor::origin(false);
    document.insert(&mut cursor, "hello", false, false);
    let inserted_revision = document.buffer.rev();
    assert_eq!(document.buffer.text().to_string(), "hello");
    let mut clipboard = MemoryClipboard::default();
    let mut register = Register::default();
    for (command, expected) in
        [(EditCommand::Undo, ""), (EditCommand::Redo, "hello")]
    {
        document.edit_command(
            &mut cursor,
            &command,
            &mut clipboard,
            &mut register,
            EditConf {
                comment_token: "",
                modal: false,
                smart_tab: false,
                keep_indent: true,
                auto_indent: true,
            },
        );
        assert_eq!(document.buffer.text().to_string(), expected);
    }
    assert!(!document.mark_saved(inserted_revision));
    assert!(document.mark_saved(document.buffer.rev()));
    assert!(document.buffer.is_pristine());
}

#[test]
fn lsp_edits_use_utf16_positions_and_respect_read_only() {
    let edits = [TextEdit {
        range: Range::new(Position::new(0, 1), Position::new(0, 3)),
        new_text: "世界".into(),
    }];
    let mut document = Document::new(DocContent::Local, Rope::from("a🌍b\n"));
    document.apply_text_edits(&edits).unwrap();
    assert_eq!(document.buffer.text().to_string(), "a世界b\n");
    document.content = DocContent::File {
        path: "example.txt".into(),
        read_only: true,
    };
    assert!(document.apply_text_edits(&edits).is_none());
    assert!(
        document
            .insert(&mut Cursor::origin(false), "x", false, false)
            .is_empty()
    );
}

#[test]
fn raw_edits_respect_access_but_host_reloads_can_update_read_only_documents() {
    let mut document = Document::new(
        DocContent::File {
            path: "example.rs".into(),
            read_only: true,
        },
        Rope::from("old"),
    );
    let selection = lapce_core::selection::Selection::region(0, 3);
    assert!(
        document
            .raw_edit(
                &[(selection, "edit")],
                lapce_core::editor::EditType::Completion
            )
            .is_none()
    );
    assert_eq!(document.buffer.text().to_string(), "old");
    document.reload(Rope::from("host change"), true);
    assert_eq!(document.buffer.text().to_string(), "host change");
    assert!(document.buffer.is_pristine());
}

#[test]
fn deferred_loading_preserves_document_identity_and_selects_history_language() {
    let mut document =
        Document::empty(DocContent::History(lapce_engine::document::DocHistory {
            path: "example.rs".into(),
            version: "HEAD".into(),
        }));
    let id = document.id;
    assert_eq!(
        document.syntax.language,
        lapce_core::language::LapceLanguage::Rust
    );
    document.load(Rope::from("fn main() {}\n"));
    assert_eq!(document.id, id);
    assert!(document.buffer.is_pristine());
}

#[test]
fn modal_range_edits_cannot_modify_read_only_documents() {
    let mut document = Document::new(
        DocContent::History(lapce_engine::document::DocHistory {
            path: "example.rs".into(),
            version: "HEAD".into(),
        }),
        Rope::from("abc"),
    );
    let mut cursor = Cursor::origin(true);
    let mut register = Register::default();
    let deltas = document.execute_motion_mode(
        &mut cursor,
        lapce_core::mode::MotionMode::Delete { count: 1 },
        0..1,
        false,
        &mut register,
    );
    assert!(deltas.is_empty());
    assert_eq!(document.buffer.text().to_string(), "abc");
}

#[test]
fn modal_range_edits_have_shared_undo_history() {
    let mut document = Document::new(DocContent::Local, Rope::from("abc"));
    let mut cursor = Cursor::origin(true);
    let mut register = Register::default();
    assert!(
        !document
            .execute_motion_mode(
                &mut cursor,
                lapce_core::mode::MotionMode::Delete { count: 1 },
                0..1,
                false,
                &mut register
            )
            .is_empty()
    );
    assert_eq!(document.buffer.text().to_string(), "bc");
    document.edit_command(
        &mut cursor,
        &EditCommand::Undo,
        &mut MemoryClipboard::default(),
        &mut register,
        EditConf {
            comment_token: "",
            modal: true,
            smart_tab: false,
            keep_indent: true,
            auto_indent: true,
        },
    );
    assert_eq!(document.buffer.text().to_string(), "abc");
}

#[test]
fn loading_is_owned_by_the_headless_document() {
    let mut document = lapce_engine::document::Document::empty(
        lapce_engine::document::DocContent::File {
            path: "example.txt".into(),
            read_only: false,
        },
    );
    assert!(!document.loading.loaded);
    document.fail_loading("offline".into());
    assert_eq!(document.loading.error.as_deref(), Some("offline"));
    document.load(lapce_xi_rope::Rope::from("retry"));
    assert!(document.loading.loaded);
    assert!(document.loading.error.is_none());
    document.fail_loading("late".into());
    assert!(document.loading.error.is_none());
    assert!(
        lapce_engine::document::Document::empty(
            lapce_engine::document::DocContent::Local
        )
        .loading
        .loaded
    );
}
