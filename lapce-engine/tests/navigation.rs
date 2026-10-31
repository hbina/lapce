use lapce_engine::{
    document::{DocContent, Document},
    navigation::{NavigationPlan, NavigationQuery},
    runtime::{RequestExecutor, WorkspaceSession},
};
use lapce_rpc::{
    RpcError,
    proxy::{ProxyRequest, ProxyResponse},
};
use lapce_xi_rope::Rope;
use std::{cell::RefCell, path::Path, rc::Rc};

#[test]
fn identical_text_after_reconnect_cannot_accept_retired_navigation() {
    use lapce_engine::{
        navigation::SessionNavigationPlan,
        session::{ReopenDocument, SessionDocument},
    };
    let mut session = SessionDocument::new(document());
    session.initialize_sync(0);
    let old = SessionNavigationPlan::capture(
        &session,
        NavigationQuery::References { offset: 4 },
    )
    .unwrap();
    let text_revision = session.buffer.rev();
    session.backend_disconnected();
    assert!(
        SessionNavigationPlan::capture(&session, NavigationQuery::Symbols).is_err()
    );
    ReopenDocument::capture(&session)
        .unwrap()
        .finish(
            &mut session,
            Ok(ProxyResponse::NewBufferResponse {
                snapshot: Default::default(),
                content: "🌍x\nnext".into(),
                read_only: true,
                rev: 0,
            }),
        )
        .unwrap();
    assert_eq!(session.buffer.rev(), text_revision);
    assert!(
        matches!(old.finish(&session, references()), Err(error) if error.code == -32801)
    );
    let current = SessionNavigationPlan::capture(
        &session,
        NavigationQuery::References { offset: 4 },
    )
    .unwrap();
    assert!(current.finish(&session, references()).is_ok());
}

fn document() -> Document {
    Document::new(
        DocContent::File {
            path: "file.rs".into(),
            read_only: true,
        },
        Rope::from("🌍x\nnext"),
    )
}
fn references() -> Result<ProxyResponse, RpcError> {
    Ok(ProxyResponse::GetReferencesResponse { references: vec![] })
}

#[test]
fn read_only_navigation_maps_unicode_and_rejects_invalid_offsets() {
    let document = document();
    let plan = NavigationPlan::capture(
        &document,
        NavigationQuery::References { offset: 4 },
    )
    .unwrap();
    assert!(
        matches!(plan.request(), ProxyRequest::GetReferences { path, position } if path == Path::new("file.rs") && position == lsp_types::Position::new(0, 2))
    );
    let plan = NavigationPlan::capture(
        &document,
        NavigationQuery::Definition { offset: 6 },
    )
    .unwrap();
    assert!(
        matches!(plan.request(), ProxyRequest::GetDefinition { request_id: 6, position, .. } if position == lsp_types::Position::new(1, 0))
    );
    for offset in [1, 2, 3, 100] {
        assert!(
            NavigationPlan::capture(&document, NavigationQuery::Hover { offset })
                .is_err()
        );
    }
    assert!(
        NavigationPlan::capture(&document, NavigationQuery::Symbols)
            .unwrap()
            .finish(
                &document,
                Ok(ProxyResponse::GetDocumentSymbols {
                    resp: lsp_types::DocumentSymbolResponse::Nested(vec![])
                })
            )
            .is_ok()
    );
}

#[test]
fn changed_retargeted_and_foreign_documents_cannot_accept_navigation() {
    let mut document = document();
    let plan = NavigationPlan::capture(
        &document,
        NavigationQuery::References { offset: 0 },
    )
    .unwrap();
    document.reload(Rope::from("new text"), true);
    assert!(plan.finish(&document, references()).is_err());
    let plan = NavigationPlan::capture(
        &document,
        NavigationQuery::References { offset: 0 },
    )
    .unwrap();
    document.content = DocContent::File {
        path: "moved.rs".into(),
        read_only: true,
    };
    assert!(plan.finish(&document, references()).is_err());
    let plan = NavigationPlan::capture(
        &document,
        NavigationQuery::References { offset: 0 },
    )
    .unwrap();
    let foreign = Document::new(document.content.clone(), Rope::from("new text"));
    assert!(plan.finish(&foreign, references()).is_err());
    let plan = NavigationPlan::capture(
        &document,
        NavigationQuery::References { offset: 0 },
    )
    .unwrap();
    assert!(
        plan.finish(&document, Ok(ProxyResponse::SaveResponse {}))
            .is_err()
    );
}

#[test]
fn local_and_unloaded_documents_cannot_issue_language_navigation() {
    let unloaded = Document::empty(DocContent::File {
        path: "file.rs".into(),
        read_only: true,
    });
    assert!(NavigationPlan::capture(&unloaded, NavigationQuery::Symbols).is_err());
    let local = Document::new(DocContent::Local, Rope::from("query"));
    assert!(NavigationPlan::capture(&local, NavigationQuery::Symbols).is_err());
}

#[derive(Clone, Default)]
struct Backend(Rc<RefCell<Vec<ProxyRequest>>>);
impl RequestExecutor for Backend {
    fn request(&self, request: ProxyRequest) -> Result<ProxyResponse, RpcError> {
        self.0.borrow_mut().push(request.clone());
        match request {
            ProxyRequest::NewBuffer { .. } => Ok(ProxyResponse::NewBufferResponse {
                snapshot: Default::default(),
                content: "🌍x\nnext".into(),
                read_only: true,
                rev: 9,
            }),
            ProxyRequest::GetReferences { .. } => references(),
            ProxyRequest::GetWorkspaceSymbols { .. } => {
                Ok(ProxyResponse::GetWorkspaceSymbols { symbols: vec![] })
            }
            ProxyRequest::GetInlayHints { .. } => {
                Ok(ProxyResponse::GetInlayHints { hints: vec![] })
            }
            ProxyRequest::CloseBuffer { .. } => Ok(ProxyResponse::Success {}),
            _ => panic!("unexpected request: {request:?}"),
        }
    }
}
#[test]
fn headless_navigation_uses_owned_read_only_documents_without_editing_or_saving() {
    let backend = Backend::default();
    let mut session = WorkspaceSession::new(backend.clone());
    let path = Path::new("file.rs");
    session.open(path.into()).unwrap();
    let revision = session.document(path).unwrap().buffer.rev();
    assert!(
        session
            .navigate(path, NavigationQuery::References { offset: 4 })
            .is_ok()
    );
    assert!(
        session
            .navigate(path, NavigationQuery::References { offset: 1 })
            .is_err()
    );
    assert!(
        session
            .navigate(Path::new("unknown"), NavigationQuery::Symbols)
            .is_err()
    );
    assert_eq!(session.document(path).unwrap().buffer.rev(), revision);
    assert_eq!(backend.0.borrow().len(), 2);
    assert!(
        matches!(&backend.0.borrow()[1], ProxyRequest::GetReferences { position, .. } if *position == lsp_types::Position::new(0, 2))
    );
    session.close(path).unwrap();
    assert!(session.navigate(path, NavigationQuery::Symbols).is_err());
}

#[test]
fn empty_definition_links_are_valid_empty_navigation_results() {
    use lapce_engine::navigation::first_definition;
    use lsp_types::{GotoDefinitionResponse, LocationLink, Position, Range, Url};
    assert!(first_definition(GotoDefinitionResponse::Array(vec![])).is_none());
    assert!(first_definition(GotoDefinitionResponse::Link(vec![])).is_none());
    let range = Range::new(Position::new(2, 3), Position::new(2, 7));
    let uri = Url::parse("file:///source.rs").unwrap();
    let result =
        first_definition(GotoDefinitionResponse::Link(vec![LocationLink {
            origin_selection_range: None,
            target_uri: uri.clone(),
            target_range: Range::new(Position::new(0, 0), Position::new(10, 0)),
            target_selection_range: range,
        }]))
        .unwrap();
    assert_eq!(result.uri, uri);
    assert_eq!(result.range, range);
}

#[test]
fn folding_ranges_validate_response_and_document_revision() {
    let mut document = document();
    let plan =
        NavigationPlan::capture(&document, NavigationQuery::FoldingRanges).unwrap();
    assert!(
        matches!(plan.request(), ProxyRequest::LspFoldingRange { path } if path == Path::new("file.rs"))
    );
    let response = || {
        Ok(ProxyResponse::LspFoldingRangeResponse {
            plugin_id: lapce_rpc::plugin::PluginId(1),
            resp: Some(vec![]),
        })
    };
    assert!(plan.finish(&document, response()).is_ok());
    let plan =
        NavigationPlan::capture(&document, NavigationQuery::FoldingRanges).unwrap();
    assert!(plan.finish(&document, references()).is_err());
    let plan =
        NavigationPlan::capture(&document, NavigationQuery::FoldingRanges).unwrap();
    document.reload(Rope::from("changed"), true);
    assert!(plan.finish(&document, response()).is_err());
}

#[test]
fn inlay_hints_require_matching_response_and_current_snapshot() {
    let mut document = document();
    let capture = |document: &Document| {
        NavigationPlan::capture(document, NavigationQuery::InlayHints).unwrap()
    };
    let plan = capture(&document);
    assert!(
        matches!(plan.request(), ProxyRequest::GetInlayHints { path } if path == Path::new("file.rs"))
    );
    assert!(
        plan.finish(
            &document,
            Ok(ProxyResponse::GetInlayHints { hints: vec![] })
        )
        .is_ok()
    );
    assert!(capture(&document).finish(&document, references()).is_err());
    let plan = capture(&document);
    document.reload(Rope::from("new"), true);
    assert!(
        plan.finish(
            &document,
            Ok(ProxyResponse::GetInlayHints { hints: vec![] })
        )
        .is_err()
    );
}

#[test]
fn headless_frontend_can_request_inlay_hints() {
    let backend = Backend::default();
    let mut session = WorkspaceSession::new(backend.clone());
    let path = Path::new("file.rs");
    session.open(path.into()).unwrap();
    assert!(
        matches!(session.navigate(path, NavigationQuery::InlayHints).unwrap(), ProxyResponse::GetInlayHints { hints } if hints.is_empty())
    );
    assert!(
        matches!(&backend.0.borrow()[1], ProxyRequest::GetInlayHints { path: requested } if requested == path)
    );
}

#[test]
fn workspace_symbols_are_available_without_an_open_document() {
    let backend = Backend::default();
    let session = WorkspaceSession::new(backend.clone());
    assert!(
        session
            .workspace_symbols("Widget".into())
            .unwrap()
            .is_empty()
    );
    assert!(
        matches!(&backend.0.borrow()[0], ProxyRequest::GetWorkspaceSymbols { query } if query == "Widget")
    );
    let plan = lapce_engine::workspace::WorkspaceSymbolsQuery::new(String::new());
    assert!(plan.finish(references()).is_err());
}

#[test]
fn document_highlights_share_unicode_and_response_validation() {
    let mut document = document();
    let plan = NavigationPlan::capture(
        &document,
        NavigationQuery::Highlights { offset: 4 },
    )
    .unwrap();
    assert!(
        matches!(plan.request(), ProxyRequest::GetDocumentHighlights { position, .. } if position == lsp_types::Position::new(0, 2))
    );
    assert!(
        plan.finish(
            &document,
            Ok(ProxyResponse::GetDocumentHighlights { highlights: None })
        )
        .is_ok()
    );
    assert!(
        NavigationPlan::capture(
            &document,
            NavigationQuery::Highlights { offset: 1 }
        )
        .is_err()
    );
    let plan = NavigationPlan::capture(
        &document,
        NavigationQuery::Highlights { offset: 0 },
    )
    .unwrap();
    assert!(plan.finish(&document, references()).is_err());
    let plan = NavigationPlan::capture(
        &document,
        NavigationQuery::Highlights { offset: 0 },
    )
    .unwrap();
    document.reload(Rope::from("changed"), true);
    assert!(
        plan.finish(
            &document,
            Ok(ProxyResponse::GetDocumentHighlights {
                highlights: Some(vec![])
            })
        )
        .is_err()
    );
}

#[test]
fn semantic_tokens_validate_snapshot_metadata() {
    let document = document();
    let capture = || {
        NavigationPlan::capture(&document, NavigationQuery::SemanticTokens).unwrap()
    };
    let response = |path: &str, len| {
        Ok(ProxyResponse::GetSemanticTokens {
            styles: lapce_rpc::style::SemanticStyles {
                rev: 0,
                path: path.into(),
                len,
                styles: vec![],
            },
        })
    };
    assert!(
        matches!(capture().request(), ProxyRequest::GetSemanticTokens { path } if path == Path::new("file.rs"))
    );
    assert!(capture().finish(&document, response("file.rs", 10)).is_ok());
    assert!(
        capture()
            .finish(&document, response("foreign.rs", 10))
            .is_err()
    );
    assert!(capture().finish(&document, response("file.rs", 9)).is_err());
    assert!(capture().finish(&document, references()).is_err());
}

#[test]
#[allow(deprecated)]
fn symbol_trees_preserve_flat_symbols_and_nested_children() {
    use lapce_engine::navigation::document_symbol_tree;
    use lsp_types::{
        DocumentSymbol, DocumentSymbolResponse, Location, Position, Range,
        SymbolInformation, SymbolKind, Url,
    };
    let range = Range::new(Position::new(1, 2), Position::new(3, 4));
    let flat = document_symbol_tree(DocumentSymbolResponse::Flat(vec![
        SymbolInformation {
            name: "method".into(),
            kind: SymbolKind::METHOD,
            tags: None,
            deprecated: None,
            location: Location {
                uri: Url::parse("file:///source.rs").unwrap(),
                range,
            },
            container_name: Some("Widget".into()),
        },
    ]));
    assert_eq!(flat.len(), 1);
    assert_eq!(flat[0].name, "method");
    assert_eq!(flat[0].detail.as_deref(), Some("Widget"));
    assert_eq!(flat[0].selection_range, range);
    let parent = DocumentSymbol {
        name: "Widget".into(),
        detail: None,
        kind: SymbolKind::CLASS,
        tags: None,
        deprecated: None,
        range,
        selection_range: range,
        children: Some(flat),
    };
    let nested = document_symbol_tree(DocumentSymbolResponse::Nested(vec![parent]));
    assert_eq!(nested[0].children.as_ref().unwrap()[0].name, "method");
    assert!(document_symbol_tree(DocumentSymbolResponse::Flat(vec![])).is_empty());
}

#[test]
fn disconnect_preserves_loaded_text_but_revokes_navigation_ownership() {
    use lapce_rpc::{core::CoreNotification, proxy::ProxyStatus};
    let backend = Backend::default();
    let mut session = WorkspaceSession::new(backend.clone());
    let path = Path::new("file.rs");
    session.open(path.into()).unwrap();
    assert!(session.document(path).unwrap().backend_ready());
    session
        .handle_event(CoreNotification::ProxyStatus {
            status: ProxyStatus::Disconnected,
        })
        .unwrap();
    let document = session.document(path).unwrap();
    assert!(!document.backend_ready());
    assert_eq!(document.buffer.to_string(), "🌍x\nnext");
    assert!(document.loading.loaded);
    assert!(session.navigate(path, NavigationQuery::Symbols).is_err());
    assert_eq!(backend.0.borrow().len(), 1);
    session.close(path).unwrap();
    assert!(session.document(path).is_none());
    assert_eq!(backend.0.borrow().len(), 1);
}

#[test]
fn replacing_backend_reattaches_read_only_files_with_same_identity() {
    let backend = Backend::default();
    let mut session = WorkspaceSession::new(backend);
    let path = Path::new("file.rs");
    session.open(path.into()).unwrap();
    let id = session.document(path).unwrap().id;
    let replacement = Backend::default();
    let results = session.reconnect(replacement.clone());
    assert_eq!(results.len(), 1);
    assert!(results[0].1.is_ok());
    assert_eq!(session.document(path).unwrap().id, id);
    assert!(session.document(path).unwrap().backend_ready());
    assert!(
        session
            .navigate(path, NavigationQuery::References { offset: 4 })
            .is_ok()
    );
    assert_eq!(replacement.0.borrow().len(), 2);
}

#[test]
fn old_and_foreign_event_streams_cannot_disconnect_reconnected_documents() {
    use lapce_engine::runtime::SessionEvent;
    use lapce_rpc::{core::CoreNotification, proxy::ProxyStatus};
    let mut session = WorkspaceSession::new(Backend::default());
    session.open("file.rs".into()).unwrap();
    let old = session.backend_generation();
    let foreign = WorkspaceSession::new(Backend::default()).backend_generation();
    assert!(session.reconnect(Backend::default())[0].1.is_ok());
    let current = session.backend_generation();
    for generation in [old, foreign] {
        assert!(matches!(
            session
                .handle_backend_event(
                    generation,
                    CoreNotification::ProxyStatus {
                        status: ProxyStatus::Disconnected
                    }
                )
                .unwrap(),
            SessionEvent::StaleBackend
        ));
        assert!(
            session
                .document(Path::new("file.rs"))
                .unwrap()
                .backend_ready()
        );
    }
    assert!(matches!(
        session
            .handle_backend_event(
                current,
                CoreNotification::ProxyStatus {
                    status: ProxyStatus::Disconnected
                }
            )
            .unwrap(),
        SessionEvent::StatusChanged
    ));
    assert!(
        !session
            .document(Path::new("file.rs"))
            .unwrap()
            .backend_ready()
    );
}
