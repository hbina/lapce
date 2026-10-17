use lapce_engine::{
    global_search::WorkspaceSearchQuery,
    runtime::{RequestExecutor, WorkspaceSession},
};
use lapce_rpc::{
    RpcError,
    proxy::{ProxyRequest, ProxyResponse},
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

#[derive(Clone, Default)]
struct Backend {
    requests: Rc<RefCell<Vec<ProxyRequest>>>,
    replies: Rc<RefCell<VecDeque<Result<ProxyResponse, RpcError>>>>,
}
impl RequestExecutor for Backend {
    fn request(&self, request: ProxyRequest) -> Result<ProxyResponse, RpcError> {
        self.requests.borrow_mut().push(request);
        self.replies
            .borrow_mut()
            .pop_front()
            .expect("unexpected request")
    }
}
fn failure() -> Result<ProxyResponse, RpcError> {
    Err(RpcError {
        code: 0,
        message: "backend failed".into(),
    })
}

#[test]
fn workspace_search_clears_empty_queries_and_recovers_from_backend_errors() {
    let backend = Backend::default();
    backend.replies.borrow_mut().extend([
        failure(),
        Ok(ProxyResponse::GlobalSearchResponse {
            matches: Default::default(),
        }),
    ]);
    let mut session = WorkspaceSession::new(backend.clone());
    let query = WorkspaceSearchQuery {
        pattern: "needle".into(),
        ..Default::default()
    };
    assert_eq!(
        session.search_workspace(query.clone()).unwrap_err(),
        "backend failed"
    );
    assert!(!session.search.pending());
    assert_eq!(session.search.error.as_deref(), Some("backend failed"));
    session.search_workspace(query).unwrap();
    assert!(session.search.error.is_none());
    session.search_workspace(Default::default()).unwrap();
    assert_eq!(backend.requests.borrow().len(), 2);
}
