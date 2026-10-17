use indexmap::IndexMap;
use lapce_engine::global_search::{WorkspaceSearch, WorkspaceSearchQuery};
use lapce_rpc::{
    RpcError,
    proxy::{ProxyRequest, ProxyResponse, SearchMatch},
};

fn query(pattern: &str) -> WorkspaceSearchQuery {
    WorkspaceSearchQuery {
        pattern: pattern.into(),
        ..Default::default()
    }
}
fn response() -> Result<ProxyResponse, RpcError> {
    Ok(ProxyResponse::GlobalSearchResponse {
        matches: IndexMap::from([(
            "file.rs".into(),
            vec![SearchMatch {
                line: 1,
                start: 0,
                end: 4,
                line_content: "text".into(),
            }],
        )]),
    })
}

#[test]
fn query_changes_and_clearing_reject_stale_results() {
    let mut state = WorkspaceSearch::default();
    let (old, _) = state.begin(query("old")).unwrap();
    let mut options = query("new");
    options.whole_word = true;
    options.case_sensitive = true;
    options.is_regex = true;
    let (current, request) = state.begin(options.clone()).unwrap();
    assert!(
        matches!(request, ProxyRequest::GlobalSearch { pattern, case_sensitive: true, whole_word: true, is_regex: true } if pattern == "new")
    );
    assert!(!state.finish(old, response()));
    assert!(state.pending());
    assert!(state.finish(current, response()));
    assert_eq!(state.query, options);
    assert_eq!(state.matches.len(), 1);
    assert!(!state.pending());
    assert!(!state.finish(current, response()));
    let (pending, _) = state.begin(query("pending")).unwrap();
    assert!(state.begin(query("")).is_none());
    assert!(!state.finish(pending, response()));
    assert!(state.matches.is_empty());
    assert!(!state.pending());
}

#[test]
fn foreign_results_and_failed_queries_cannot_repopulate_matches() {
    let mut state = WorkspaceSearch::default();
    let mut other = WorkspaceSearch::default();
    let (ticket, _) = state.begin(query("[")).unwrap();
    let (foreign, _) = other.begin(query("[")).unwrap();
    assert!(!state.finish(foreign, response()));
    assert!(state.finish(
        ticket,
        Err(RpcError {
            code: 0,
            message: "invalid regex".into()
        })
    ));
    assert_eq!(state.error.as_deref(), Some("invalid regex"));
    assert!(state.matches.is_empty());
    let (retry, _) = state.begin(query("text")).unwrap();
    assert!(state.error.is_none());
    assert!(state.finish(retry, response()));
    assert_eq!(state.matches.len(), 1);
    let (ticket, _) = state.begin(query("text")).unwrap();
    assert!(state.finish(ticket, Ok(ProxyResponse::Success {})));
    assert!(state.error.is_some());
}
