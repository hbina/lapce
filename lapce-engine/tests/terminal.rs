use lapce_engine::{
    runtime::{RequestExecutor, SessionEvent, WorkspaceSession},
    terminal::{TerminalPhase, TerminalState},
};
use lapce_rpc::{
    RpcError,
    core::CoreNotification,
    proxy::{ProxyRequest, ProxyResponse},
    terminal::TermId,
};

struct NoRequests;
impl RequestExecutor for NoRequests {
    fn request(&self, _: ProxyRequest) -> Result<ProxyResponse, RpcError> {
        panic!("events must not issue requests")
    }
}
#[test]
fn process_lifecycle_rejects_duplicate_exit_and_late_start_until_restart() {
    let mut state = TerminalState::default();
    assert!(state.started(Some(42)));
    assert!(state.stopped(Some(1)));
    assert!(!state.stopped(Some(0)));
    assert!(!state.started(Some(43)));
    assert!(!state.failed("late error".into()));
    assert_eq!(state.exit_code, Some(1));
    state.restart();
    assert!(state.failed("launch failed".into()));
    assert_eq!(state.phase, TerminalPhase::Failed);
    assert!(!state.started(Some(44)));
    state.restart();
    assert!(state.started(None));
    assert!(state.launch_error.is_none());
    assert_eq!(state.exit_code, None);
}
#[test]
fn terminal_routing_keeps_binary_output_and_unknown_events_intact() {
    let mut session = WorkspaceSession::new(NoRequests);
    let id = TermId::next();
    let bytes = vec![0, 255, 27, b'[', b'm'];
    assert!(
        matches!(session.handle_event(CoreNotification::UpdateTerminal { term_id: id, content: bytes.clone() }).unwrap(), SessionEvent::Unhandled(CoreNotification::UpdateTerminal { content, .. }) if content == bytes)
    );
    assert!(session.register_terminal(id));
    assert!(!session.register_terminal(id));
    assert!(
        matches!(session.handle_event(CoreNotification::UpdateTerminal { term_id: id, content: bytes.clone() }).unwrap(), SessionEvent::TerminalOutput { content, .. } if content == bytes)
    );
    session
        .handle_event(CoreNotification::TerminalProcessId {
            term_id: id,
            process_id: Some(42),
        })
        .unwrap();
    assert_eq!(session.terminals[&id].process_id, Some(42));
    assert!(matches!(
        session
            .handle_event(CoreNotification::TerminalProcessStopped {
                term_id: id,
                exit_code: Some(0)
            })
            .unwrap(),
        SessionEvent::TerminalChanged { accepted: true, .. }
    ));
    assert!(matches!(
        session
            .handle_event(CoreNotification::TerminalProcessStopped {
                term_id: id,
                exit_code: Some(1)
            })
            .unwrap(),
        SessionEvent::TerminalChanged {
            accepted: false,
            ..
        }
    ));
    assert_eq!(session.terminals[&id].exit_code, Some(0));
}

#[test]
fn unsupported_terminal_launch_does_not_leave_phantom_ownership() {
    let mut session = WorkspaceSession::new(NoRequests);
    assert!(session.launch_terminal(Default::default()).is_err());
    assert!(session.terminals.is_empty());
}

#[test]
fn input_and_resize_obey_lifecycle_and_pty_dimension_contract() {
    use lapce_rpc::proxy::ProxyNotification;
    let id = TermId::next();
    let mut state = TerminalState::default();
    assert!(state.input(id, String::new()).unwrap().is_none());
    assert!(
        matches!(state.input(id, "🌍\n".into()).unwrap(), Some(ProxyNotification::TerminalWrite { content, .. }) if content == "🌍\n")
    );
    assert!(matches!(
        state.resize(id, usize::MAX, 0).unwrap(),
        ProxyNotification::TerminalResize {
            width: 65535,
            height: 1,
            ..
        }
    ));
    state.stopped(Some(0));
    assert!(state.input(id, "ignored".into()).is_err());
    assert!(state.resize(id, 80, 24).is_err());
    let session = WorkspaceSession::new(NoRequests);
    assert!(session.terminal_input(id, "unknown".into()).is_err());
    assert!(session.terminal_resize(id, 80, 24).is_err());
}
