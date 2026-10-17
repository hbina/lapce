use lapce_engine::{
    global_search::WorkspaceSearchQuery,
    workspace::{WorkspaceActions, WorkspaceOutcome},
};
use lapce_rpc::proxy::ProxyResponse;

fn query(pattern: &str) -> WorkspaceSearchQuery {
    WorkspaceSearchQuery {
        pattern: pattern.into(),
        ..Default::default()
    }
}
#[test]
fn delayed_search_completions_cannot_update_another_query_or_workspace() {
    let mut workspace = WorkspaceActions::default();
    let old = workspace.search(query("old")).unwrap();
    let current = workspace.search(query("current")).unwrap();
    let response = || {
        Ok(ProxyResponse::GlobalSearchResponse {
            matches: Default::default(),
        })
    };
    let mut foreign = WorkspaceActions::default();
    foreign.search(query("current"));
    assert!(matches!(
        foreign.complete(current.clone(), response()),
        WorkspaceOutcome::Stale
    ));
    assert!(foreign.search.pending());
    assert!(matches!(
        workspace.complete(old, response()),
        WorkspaceOutcome::Stale
    ));
    assert!(workspace.search.pending());
    assert!(matches!(
        workspace.complete(current.clone(), response()),
        WorkspaceOutcome::SearchFinished(Ok(()))
    ));
    assert!(matches!(
        workspace.complete(current, response()),
        WorkspaceOutcome::Stale
    ));
    let pending = workspace.search(query("pending")).unwrap();
    assert!(workspace.search(query("")).is_none());
    assert!(matches!(
        workspace.complete(pending, response()),
        WorkspaceOutcome::Stale
    ));
}
