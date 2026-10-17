//! Floem scheduling adapter for engine-owned workspace effects.
use crate::window_tab::CommonData;
use floem::{
    ext_event::create_ext_action,
    reactive::{RwSignal, SignalUpdate},
};
use lapce_engine::workspace::{WorkspaceActions, WorkspaceEffect, WorkspaceOutcome};
use std::rc::Rc;

pub fn dispatch(
    common: Rc<CommonData>,
    effect: WorkspaceEffect,
    finished: Rc<dyn Fn(WorkspaceOutcome)>,
) {
    let request = effect.request.clone();
    let source = common.clone();
    let send = create_ext_action(common.scope, move |response| {
        let Some(outcome) = complete(
            source.workspace_actions,
            source.workspace_changed,
            effect,
            response,
        ) else {
            return;
        };
        finished(outcome);
    });
    common
        .proxy
        .request_async(request, move |response| send(response));
}

fn complete(
    state: RwSignal<WorkspaceActions>,
    changed: RwSignal<()>,
    effect: WorkspaceEffect,
    response: Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError>,
) -> Option<WorkspaceOutcome> {
    let outcome = state.try_update(|state| state.complete(effect, response))?;
    if matches!(outcome, WorkspaceOutcome::Stale) {
        return None;
    }
    changed.set(());
    Some(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive_model::ModelField;
    use floem::reactive::Scope;
    use lapce_engine::global_search::WorkspaceSearchQuery;
    use lapce_rpc::proxy::ProxyResponse;
    use std::cell::Cell;

    #[test]
    fn completion_notifies_projected_state_after_mutation_and_ignores_stale_replies()
    {
        let scope = Scope::new();
        let state = scope.create_rw_signal(WorkspaceActions::default());
        let changed = scope.create_rw_signal(());
        let search =
            ModelField::new(state, changed, |s| &s.search, |s| &mut s.search, None);
        let observed = Rc::new(Cell::new(0));
        scope.create_effect({
            let observed = observed.clone();
            move |_| {
                search.with(|s| {
                    assert!(!s.pending());
                });
                observed.set(observed.get() + 1);
            }
        });
        let effect = state
            .try_update(|s| {
                s.search(WorkspaceSearchQuery {
                    pattern: "needle".into(),
                    ..Default::default()
                })
            })
            .flatten()
            .unwrap();
        let response = || {
            Ok(ProxyResponse::GlobalSearchResponse {
                matches: Default::default(),
            })
        };
        assert!(matches!(
            complete(state, changed, effect.clone(), response()),
            Some(WorkspaceOutcome::SearchFinished(Ok(())))
        ));
        assert_eq!(observed.get(), 2);
        assert!(complete(state, changed, effect, response()).is_none());
        assert_eq!(observed.get(), 2);
        let late = state
            .try_update(|s| {
                s.search(WorkspaceSearchQuery {
                    pattern: "late".into(),
                    ..Default::default()
                })
            })
            .flatten()
            .unwrap();
        scope.dispose();
        assert!(complete(state, changed, late, response()).is_none());
        assert_eq!(observed.get(), 2);
    }
}
