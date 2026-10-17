use lapce_engine::session::WorkspaceDocuments;
use std::rc::Rc;

#[test]
fn pending_reopen_excludes_duplicate_attempts_and_failure_allows_retry() {
    use lapce_engine::{
        document::{DocContent, Document},
        session::SessionDocument,
    };
    let mut session = SessionDocument::new(Document::empty(DocContent::File {
        path: "file.rs".into(),
        read_only: true,
    }));
    let first = session.begin_reopen().unwrap();
    assert!(!session.can_reopen_backend());
    assert!(session.begin_reopen().is_err());
    assert!(
        first
            .finish(
                &mut session,
                Err(lapce_rpc::RpcError {
                    code: -1,
                    message: "unavailable".into()
                })
            )
            .is_err()
    );
    assert!(session.can_reopen_backend());
    assert!(session.begin_reopen().is_ok());
    session.backend_disconnected();
    assert!(session.can_reopen_backend());
}

#[test]
fn shared_close_policy_retains_ownership_until_acknowledgement() {
    use lapce_engine::{
        document::{DocContent, Document},
        session::{DocumentClose, SessionDocument},
    };
    use lapce_rpc::proxy::ProxyResponse;
    use lapce_xi_rope::Rope;
    let mut session = SessionDocument::new(Document::new(
        DocContent::File {
            path: "file.rs".into(),
            read_only: true,
        },
        Rope::from("disk"),
    ));
    assert!(matches!(
        session.begin_close().unwrap(),
        DocumentClose::Local
    ));
    session.initialize_sync(0);
    let DocumentClose::Backend { operation_id } = session.begin_close().unwrap()
    else {
        panic!("attached document must close remotely");
    };
    assert!(session.begin_close().is_err());
    assert!(!session.can_close_locally());
    let sync = session.synchronization.as_mut().unwrap();
    let effect = sync.next_effect().unwrap();
    assert_eq!(effect.id, operation_id);
    assert!(matches!(
        sync.acknowledge(effect.id, Ok(ProxyResponse::Success {})),
        lapce_engine::sync::SyncOutcome::Closed { .. }
    ));
    assert!(matches!(
        session.begin_close().unwrap(),
        DocumentClose::Local
    ));
}

#[test]
fn interrupted_readonly_initial_load_can_reopen_an_empty_file() {
    use lapce_engine::{
        document::{DocContent, Document},
        session::{ReopenDocument, SessionDocument},
    };
    use lapce_rpc::proxy::ProxyResponse;
    let mut session = SessionDocument::new(Document::empty(DocContent::File {
        path: "empty.rs".into(),
        read_only: true,
    }));
    session.document.fail_loading("backend disconnected".into());
    let id = session.id;
    ReopenDocument::capture(&session)
        .unwrap()
        .finish(
            &mut session,
            Ok(ProxyResponse::NewBufferResponse {
                content: String::new(),
                read_only: false,
                rev: 0,
            }),
        )
        .unwrap();
    assert!(session.loading.loaded);
    assert!(session.loading.error.is_none());
    assert!(session.backend_ready());
    assert_eq!(session.id, id);
    assert!(session.content.read_only());
}

#[test]
fn watcher_reload_requires_live_buffer_ownership() {
    use lapce_engine::{
        document::{DocContent, Document},
        session::{ReopenDocument, SessionDocument},
    };
    use lapce_rpc::proxy::ProxyResponse;
    use lapce_xi_rope::Rope;
    let mut session = SessionDocument::new(Document::new(
        DocContent::File {
            path: "file.rs".into(),
            read_only: true,
        },
        Rope::from("cached"),
    ));
    assert!(session.reload_from_disk(Rope::from("stale")).is_none());
    assert_eq!(session.buffer.to_string(), "cached");
    ReopenDocument::capture(&session)
        .unwrap()
        .finish(
            &mut session,
            Ok(ProxyResponse::NewBufferResponse {
                content: "disk".into(),
                read_only: true,
                rev: 0,
            }),
        )
        .unwrap();
    assert!(
        session
            .reload_from_disk(Rope::from("updated🌍"))
            .unwrap()
            .is_some()
    );
    assert_eq!(session.buffer.to_string(), "updated🌍");
    let mut closing = session.clone();
    closing.synchronization.as_mut().unwrap().close();
    assert!(
        closing
            .reload_from_disk(Rope::from("late close event"))
            .is_none()
    );
    assert_eq!(closing.buffer.to_string(), "updated🌍");
    session.backend_disconnected();
    assert!(session.reload_from_disk(Rope::from("old stream")).is_none());
    assert_eq!(session.buffer.to_string(), "updated🌍");
}

#[test]
fn views_reuse_one_document_and_removal_allows_a_fresh_open() {
    let mut documents = WorkspaceDocuments::default();
    let path = "example.txt".into();
    let (first, created) = documents.file(path, || Rc::new("first"));
    assert!(created);
    let (second, created) =
        documents.file("example.txt".into(), || panic!("duplicate open"));
    assert!(!created);
    assert!(Rc::ptr_eq(&first, &second));
    documents
        .files
        .remove(&std::path::PathBuf::from("example.txt"));
    let (replacement, created) =
        documents.file("example.txt".into(), || Rc::new("replacement"));
    assert!(created);
    assert!(!Rc::ptr_eq(&first, &replacement));
}
#[test]
fn scratch_names_ignore_other_names_and_file_paths() {
    let mut documents = WorkspaceDocuments::<()>::default();
    documents.files.insert("Untitled-100".into(), ());
    documents.scratch.insert("custom scratch".into(), ());
    documents.scratch.insert("Untitled-2".into(), ());
    documents.scratch.insert("Untitled-9".into(), ());
    assert_eq!(documents.next_scratch_name(), "Untitled-10");
}

#[test]
fn session_document_queues_local_edits_and_recovers_from_its_owned_text() {
    use lapce_core::{buffer::rope_text::RopeText, cursor::Cursor};
    use lapce_engine::{
        document::{DocContent, Document},
        session::SessionDocument,
    };
    use lapce_rpc::proxy::{ProxyRequest, ProxyResponse};
    use lapce_xi_rope::Rope;
    let mut session = SessionDocument::new(Document::new(
        DocContent::File {
            path: "example.txt".into(),
            read_only: false,
        },
        Rope::from("hello"),
    ));
    assert!(session.initialize_sync(7));
    let deltas =
        session
            .document
            .insert(&mut Cursor::origin(false), "🌍", false, false);
    assert!(session.synchronize_edits(&deltas));
    let effect = session
        .synchronization
        .as_mut()
        .unwrap()
        .next_effect()
        .unwrap();
    assert!(
        matches!(effect.request, ProxyRequest::UpdateBuffer { buffer_id, rev: 8, .. } if buffer_id == session.document.id)
    );
    session
        .synchronization
        .as_mut()
        .unwrap()
        .acknowledge(effect.id, Ok(ProxyResponse::BufferUpdated { rev: 8 }));
    session
        .recover_sync(ProxyResponse::BufferSnapshot {
            buffer_id: session.document.id,
            path: "example.txt".into(),
            content: "external".into(),
            read_only: false,
            rev: 40,
        })
        .unwrap();
    let effect = session
        .synchronization
        .as_mut()
        .unwrap()
        .next_effect()
        .unwrap();
    let ProxyRequest::UpdateBuffer { delta, rev: 41, .. } = effect.request else {
        panic!("expected recovery edit");
    };
    assert_eq!(
        delta.apply(&Rope::from("external")).to_string(),
        session.document.buffer.text().to_string()
    );
}

#[test]
fn reopening_uses_latest_local_text_and_preserves_undo_history() {
    use lapce_core::{
        buffer::rope_text::RopeText,
        command::EditCommand,
        cursor::Cursor,
        editor::EditConf,
        register::{Clipboard, Register},
    };
    use lapce_engine::{
        document::{DocContent, Document},
        session::{ReopenDocument, SessionDocument},
    };
    use lapce_rpc::proxy::{ProxyRequest, ProxyResponse};
    use lapce_xi_rope::Rope;
    struct EmptyClipboard;
    impl Clipboard for EmptyClipboard {
        fn get_string(&mut self) -> Option<String> {
            None
        }
        fn put_string(&mut self, _: impl AsRef<str>) {}
    }
    let mut session = SessionDocument::new(Document::new(
        DocContent::File {
            path: "example.txt".into(),
            read_only: false,
        },
        Rope::from("hello"),
    ));
    let plan = ReopenDocument::capture(&session).unwrap();
    let owner = session.document.id;
    let mut cursor = Cursor::origin(false);
    session.document.insert(&mut cursor, "🌍", false, false);
    let revision = session.document.buffer.rev();
    plan.finish(
        &mut session,
        Ok(ProxyResponse::NewBufferResponse {
            content: "hello".into(),
            read_only: false,
            rev: 20,
        }),
    )
    .unwrap();
    assert_eq!(session.document.id, owner);
    assert_eq!(session.document.buffer.rev(), revision);
    assert!(!session.document.is_pristine());
    let effect = session
        .synchronization
        .as_mut()
        .unwrap()
        .next_effect()
        .unwrap();
    let ProxyRequest::UpdateBuffer { delta, rev: 21, .. } = effect.request else {
        panic!("expected reattachment edit");
    };
    assert_eq!(delta.apply(&Rope::from("hello")).to_string(), "🌍hello");
    session.document.edit_command(
        &mut cursor,
        &EditCommand::Undo,
        &mut EmptyClipboard,
        &mut Register::default(),
        EditConf {
            comment_token: "",
            modal: false,
            smart_tab: false,
            keep_indent: true,
            auto_indent: true,
        },
    );
    assert_eq!(session.document.buffer.text().to_string(), "hello");
    assert!(session.document.is_pristine());
}

#[test]
fn reopening_rejects_stale_owners_and_readonly_conflicts_without_losing_text() {
    use lapce_core::buffer::rope_text::RopeText;
    use lapce_engine::{
        document::{DocContent, Document},
        session::{ReopenDocument, SessionDocument},
    };
    use lapce_rpc::proxy::ProxyResponse;
    use lapce_xi_rope::Rope;
    let mut session = SessionDocument::new(Document::new(
        DocContent::File {
            path: "example.txt".into(),
            read_only: false,
        },
        Rope::from("local"),
    ));
    let first = ReopenDocument::capture(&session).unwrap();
    assert!(
        first
            .finish(
                &mut session,
                Ok(ProxyResponse::NewBufferResponse {
                    content: "remote".into(),
                    read_only: true,
                    rev: 1
                })
            )
            .is_err()
    );
    assert_eq!(session.document.buffer.text().to_string(), "local");
    assert!(session.synchronization.is_none());
    let first = ReopenDocument::capture(&session).unwrap();
    let stale = ReopenDocument::capture(&session).unwrap();
    first
        .finish(
            &mut session,
            Ok(ProxyResponse::NewBufferResponse {
                content: "local".into(),
                read_only: true,
                rev: 1,
            }),
        )
        .unwrap();
    assert!(session.document.content.read_only());
    assert!(
        stale
            .finish(
                &mut session,
                Ok(ProxyResponse::NewBufferResponse {
                    content: "wrong".into(),
                    read_only: false,
                    rev: 2
                })
            )
            .is_err()
    );
    assert_eq!(session.document.buffer.text().to_string(), "local");
}

#[test]
fn retargeting_requires_close_and_invalidates_old_reopen_attempts() {
    use lapce_engine::{
        document::{DocContent, Document},
        session::{ReopenDocument, SessionDocument},
    };
    use lapce_rpc::proxy::ProxyResponse;
    use lapce_xi_rope::Rope;
    let mut session = SessionDocument::new(Document::new(
        DocContent::File {
            path: "old.txt".into(),
            read_only: false,
        },
        Rope::from("local"),
    ));
    session.initialize_sync(1);
    assert!(session.retarget_closed("new.rs".into()).is_err());
    let sync = session.synchronization.as_mut().unwrap();
    let close = sync.close().unwrap();
    sync.next_effect().unwrap();
    sync.acknowledge(close, Ok(ProxyResponse::Success {}));
    let stale = ReopenDocument::capture(&session).unwrap();
    let revision = session.document.buffer.rev();
    session.retarget_closed("new.rs".into()).unwrap();
    assert_eq!(session.document.buffer.rev(), revision);
    assert_eq!(
        session.document.content.path().unwrap(),
        &std::path::PathBuf::from("new.rs")
    );
    assert!(
        stale
            .finish(
                &mut session,
                Ok(ProxyResponse::NewBufferResponse {
                    content: "local".into(),
                    read_only: false,
                    rev: 1
                })
            )
            .is_err()
    );
    assert!(
        matches!(ReopenDocument::capture(&session).unwrap().request(), lapce_rpc::proxy::ProxyRequest::NewBuffer { path, .. } if path == std::path::PathBuf::from("new.rs"))
    );
}

#[test]
fn readonly_reopen_refreshes_disk_text_without_uploading_cached_text() {
    use lapce_engine::{
        document::{DocContent, Document},
        session::{ReopenDocument, SessionDocument},
    };
    use lapce_rpc::proxy::ProxyResponse;
    use lapce_xi_rope::Rope;
    let mut session = SessionDocument::new(Document::new(
        DocContent::File {
            path: "file.rs".into(),
            read_only: true,
        },
        Rope::from("cached"),
    ));
    let id = session.id;
    let plan = ReopenDocument::capture(&session).unwrap();
    plan.finish(
        &mut session,
        Ok(ProxyResponse::NewBufferResponse {
            content: "disk🌍".into(),
            read_only: false,
            rev: 20,
        }),
    )
    .unwrap();
    assert_eq!(session.id, id);
    assert_eq!(session.buffer.to_string(), "disk🌍");
    assert!(session.content.read_only());
    assert!(session.is_pristine());
    assert!(
        session
            .synchronization
            .as_mut()
            .unwrap()
            .next_effect()
            .is_none()
    );
}

#[test]
fn old_backend_callback_cannot_acknowledge_reused_operation_id() {
    use lapce_engine::{
        document::{DocContent, Document},
        session::SessionDocument,
        sync::SyncOutcome,
    };
    use lapce_rpc::proxy::{ProxyRequest, ProxyResponse};
    use lapce_xi_rope::Rope;
    let mut document = SessionDocument::new(Document::new(
        DocContent::File {
            path: "file.rs".into(),
            read_only: true,
        },
        Rope::from("text"),
    ));
    document.initialize_sync(1);
    let old_generation = document.backend_generation();
    let request = || ProxyRequest::GetDocumentSymbols {
        path: "file.rs".into(),
    };
    let revision = document.buffer.rev();
    document
        .synchronization
        .as_mut()
        .unwrap()
        .request(request(), revision);
    let old = document
        .synchronization
        .as_mut()
        .unwrap()
        .next_effect()
        .unwrap();
    document.backend_disconnected();
    document.initialize_sync(2);
    let current_generation = document.backend_generation();
    document
        .synchronization
        .as_mut()
        .unwrap()
        .request(request(), revision);
    let current = document
        .synchronization
        .as_mut()
        .unwrap()
        .next_effect()
        .unwrap();
    assert_eq!(old.id, current.id);
    let response = || {
        Ok(ProxyResponse::GetDocumentSymbols {
            resp: lsp_types::DocumentSymbolResponse::Nested(vec![]),
        })
    };
    assert!(matches!(
        document
            .acknowledge_backend(old_generation, old.id, response())
            .outcome,
        SyncOutcome::Stale
    ));
    assert!(matches!(
        document
            .acknowledge_backend(current_generation, current.id, response())
            .outcome,
        SyncOutcome::RequestCompleted { .. }
    ));
}
