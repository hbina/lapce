use lapce_core::buffer::rope_text::RopeText;
use lapce_engine::runtime::{RequestExecutor, WorkspaceSession};
use lapce_rpc::{
    RpcError,
    proxy::{ProxyRequest, ProxyResponse},
};
use std::{cell::RefCell, path::Path, rc::Rc};

#[derive(Clone, Default)]
struct Backend {
    requests: Rc<RefCell<Vec<ProxyRequest>>>,
    fail_close: Rc<RefCell<bool>>,
    filesystem: bool,
}
impl RequestExecutor for Backend {
    fn request(&self, request: ProxyRequest) -> Result<ProxyResponse, RpcError> {
        self.requests.borrow_mut().push(request.clone());
        match request {
            ProxyRequest::NewBuffer { path, .. } => {
                let content = if self.filesystem {
                    std::fs::read_to_string(path).unwrap_or_default()
                } else {
                    "hello".into()
                };
                Ok(ProxyResponse::NewBufferResponse {
                    snapshot: Default::default(),
                    rev: u64::from(!content.is_empty()),
                    content,
                    read_only: false,
                })
            }
            ProxyRequest::UpdateBuffer { rev, .. } => {
                Ok(ProxyResponse::BufferUpdated {
                    snapshot: Default::default(),
                    rev,
                })
            }
            ProxyRequest::CloseBuffer { .. } if *self.fail_close.borrow() => {
                Err(RpcError {
                    code: 0,
                    message: "close failed".into(),
                })
            }
            ProxyRequest::CloseBuffer { .. } => Ok(ProxyResponse::Success {}),
            _ => panic!("unexpected request"),
        }
    }
}
#[test]
fn session_reuses_read_only_documents_and_closes_the_owned_buffer() {
    let backend = Backend::default();
    let mut session = WorkspaceSession::new(backend.clone());
    let path = Path::new("file");
    session.open(path.into()).unwrap();
    let owner = session.document(path).unwrap().id;
    session.open(path.into()).unwrap();
    assert_eq!(backend.requests.borrow().len(), 1);
    // Even a legacy backend advertising editable content cannot change policy.
    assert!(session.document(path).unwrap().content.read_only());
    assert_eq!(
        session.document(path).unwrap().buffer.text().to_string(),
        "hello"
    );
    session.close(path).unwrap();
    assert!(session.document(path).is_none());
    assert!(
        matches!(&backend.requests.borrow()[1], ProxyRequest::CloseBuffer { buffer_id, .. } if *buffer_id == owner)
    );
}
#[test]
fn failed_close_preserves_owned_document_and_reports_paused_sync() {
    let backend = Backend::default();
    *backend.fail_close.borrow_mut() = true;
    let mut session = WorkspaceSession::new(backend);
    let path = Path::new("file");
    session.open(path.into()).unwrap();
    let owner = session.document(path).unwrap().id;
    assert_eq!(session.close(path).unwrap_err(), "close failed");
    assert_eq!(session.document(path).unwrap().id, owner);
    assert_eq!(
        session.document(path).unwrap().buffer.text().to_string(),
        "hello"
    );
    assert_eq!(session.flush(path).unwrap_err(), "close failed");
}

#[test]
fn disk_events_reload_read_only_documents_and_preserve_deleted_text() {
    use lapce_engine::runtime::SessionEvent;
    use lapce_rpc::core::{CoreNotification, FileChanged};
    let backend = Backend::default();
    let mut session = WorkspaceSession::new(backend.clone());
    let path = Path::new("file");
    session.open(path.into()).unwrap();
    let owner = session.document(path).unwrap().id;
    let event = |content| CoreNotification::OpenFileChanged {
        path: path.into(),
        content,
    };
    for text in ["external 🌍", "new disk", ""] {
        assert!(matches!(
            session
                .handle_event(event(FileChanged::Change(text.into())))
                .unwrap(),
            SessionEvent::DocumentChanged { reloaded: true, .. }
        ));
        let document = session.document(path).unwrap();
        assert_eq!(document.buffer.text().to_string(), text);
        assert_eq!(document.id, owner);
        assert!(document.is_pristine());
        assert!(document.content.read_only());
        assert!(document.backend_ready());
    }
    session.handle_event(event(FileChanged::Delete)).unwrap();
    assert!(session.document(path).unwrap().missing_on_disk());
    session
        .handle_event(event(FileChanged::Change("restored".into())))
        .unwrap();
    assert!(!session.document(path).unwrap().missing_on_disk());
    assert_eq!(
        session.document(path).unwrap().buffer.text().to_string(),
        "restored"
    );
    assert!(
        backend
            .requests
            .borrow()
            .iter()
            .all(ProxyRequest::allowed_in_code_browser)
    );
}

#[test]
fn session_routes_repository_and_status_events_without_dropping_other_events() {
    use lapce_engine::runtime::SessionEvent;
    use lapce_rpc::{
        core::CoreNotification, proxy::ProxyStatus, source_control::DiffInfo,
    };
    let mut session = WorkspaceSession::new(Backend::default());
    assert!(matches!(
        session
            .handle_event(CoreNotification::ProxyStatus {
                status: ProxyStatus::Connected
            })
            .unwrap(),
        SessionEvent::StatusChanged
    ));
    assert!(matches!(session.status, Some(ProxyStatus::Connected)));
    assert!(matches!(
        session
            .handle_event(CoreNotification::DiffInfo {
                diff: DiffInfo {
                    head: "main".into(),
                    ..Default::default()
                }
            })
            .unwrap(),
        SessionEvent::RepositoryChanged
    ));
    assert_eq!(session.source_control.branch, "main");
    assert!(matches!(
        session
            .handle_event(CoreNotification::WorkspaceFileChange)
            .unwrap(),
        SessionEvent::Unhandled(CoreNotification::WorkspaceFileChange)
    ));
}

#[test]
fn diagnostic_events_bind_unopened_files_and_reject_old_backend_revisions() {
    use lapce_engine::runtime::SessionEvent;
    use lapce_rpc::core::CoreNotification;
    use lsp_types::{
        Diagnostic, DiagnosticSeverity, Position, PublishDiagnosticsParams, Range,
        Url,
    };
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("file");
    let uri = Url::from_file_path(&path).unwrap();
    let mut diagnostic = Diagnostic::new_simple(
        Range::new(Position::new(0, 0), Position::new(0, 5)),
        "message".into(),
    );
    diagnostic.severity = Some(DiagnosticSeverity::ERROR);
    let event = |version| CoreNotification::PublishDiagnostics {
        diagnostics: PublishDiagnosticsParams {
            uri: uri.clone(),
            version: Some(version),
            diagnostics: vec![diagnostic.clone()],
        },
    };
    let mut session = WorkspaceSession::new(Backend::default());
    session.handle_event(event(1)).unwrap();
    assert_eq!(
        session.diagnostics[&path].entries(DiagnosticSeverity::ERROR)[0].range,
        None
    );
    session.open(path.clone()).unwrap();
    assert_eq!(
        session.diagnostics[&path].entries(DiagnosticSeverity::ERROR)[0].range,
        Some((0, 5))
    );
    session
        .handle_event(CoreNotification::OpenFileChanged {
            path: path.clone(),
            content: lapce_rpc::core::FileChanged::Change("🌍hello".into()),
        })
        .unwrap();
    assert!(matches!(
        session.handle_event(event(1)).unwrap(),
        SessionEvent::DiagnosticsChanged {
            accepted: false,
            ..
        }
    ));
    // A whole-file reload invalidates old diagnostic locations until the
    // language server publishes diagnostics for the acknowledged revision.
    assert!(
        session.diagnostics[&path]
            .entries(DiagnosticSeverity::ERROR)
            .is_empty()
    );
    assert!(matches!(
        session.handle_event(event(2)).unwrap(),
        SessionEvent::DiagnosticsChanged { accepted: true, .. }
    ));
    assert!(matches!(
        session.handle_event(event(1)).unwrap(),
        SessionEvent::DiagnosticsChanged {
            accepted: false,
            ..
        }
    ));
}

#[test]
fn failed_open_does_not_publish_a_document_and_can_be_retried() {
    struct RecoveringBackend(RefCell<bool>);
    impl RequestExecutor for RecoveringBackend {
        fn request(&self, request: ProxyRequest) -> Result<ProxyResponse, RpcError> {
            assert!(matches!(request, ProxyRequest::NewBuffer { .. }));
            if self.0.replace(false) {
                // A malformed reply must not leave a phantom owned document.
                Ok(ProxyResponse::Success {})
            } else {
                Ok(ProxyResponse::NewBufferResponse {
                    snapshot: Default::default(),
                    content: "disk".into(),
                    read_only: false,
                    rev: 7,
                })
            }
        }
    }
    let path = Path::new("file");
    let mut session = WorkspaceSession::new(RecoveringBackend(RefCell::new(true)));
    assert_eq!(
        session.open(path.into()).unwrap_err(),
        "Unexpected document-open response"
    );
    assert!(session.document(path).is_none());
    session.open(path.into()).unwrap();
    let document = session.document(path).unwrap();
    assert_eq!(document.buffer.text().to_string(), "disk");
    assert!(document.content.read_only());
    assert!(document.backend_ready());
}
