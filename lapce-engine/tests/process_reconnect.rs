use lapce_client::{
    BackendConnection, ConnectionOptions, LocalBackend, workspace::LapceWorkspace,
};
use lapce_engine::runtime::{RpcExecutor, WorkspaceSession};
use std::time::Duration;

#[test]
#[ignore = "requires LAPCE_TEST_PROXY pointing to a built backend"]
fn shared_frontends_reload_one_external_disk_change_independently() {
    let executable =
        std::env::var_os("LAPCE_TEST_PROXY").expect("set LAPCE_TEST_PROXY");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.rs");
    std::fs::write(&path, "before🌍").unwrap();
    let host = lapce_client::shared::SharedBackend::connect(ConnectionOptions {
        workspace: LapceWorkspace {
            path: Some(directory.path().into()),
            ..Default::default()
        },
        local_backend: LocalBackend::Process {
            executable: executable.into(),
        },
        ..Default::default()
    })
    .unwrap();
    let first = host.attach().unwrap();
    let second = host.attach().unwrap();
    let mut sessions = [&first, &second].map(|frontend| {
        WorkspaceSession::new(RpcExecutor {
            rpc: frontend.proxy_rpc.clone(),
            timeout: Duration::from_secs(10),
        })
    });
    for session in &mut sessions {
        session.open(path.clone()).unwrap();
    }
    // Drain initial status before changing disk.
    for (session, frontend) in sessions.iter_mut().zip([&first, &second]) {
        let generation = session.backend_generation();
        for event in frontend.events.try_iter() {
            session.handle_backend_event(generation, event).unwrap();
        }
    }
    std::fs::write(&path, "after🌍 and more").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        for (session, frontend) in sessions.iter_mut().zip([&first, &second]) {
            let generation = session.backend_generation();
            for event in frontend.events.try_iter() {
                session.handle_backend_event(generation, event).unwrap();
            }
        }
        if sessions.iter().all(|session| {
            session.document(&path).unwrap().buffer.to_string() == "after🌍 and more"
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    for session in &mut sessions {
        assert_eq!(
            session.document(&path).unwrap().buffer.to_string(),
            "after🌍 and more"
        );
        session.close(&path).unwrap();
    }
    host.shutdown_and_wait().unwrap();
}

#[test]
#[ignore = "requires LAPCE_TEST_PROXY pointing to a built backend"]
fn reconnect_to_real_process_refreshes_read_only_file_and_preserves_identity() {
    let executable =
        std::env::var_os("LAPCE_TEST_PROXY").expect("set LAPCE_TEST_PROXY");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file.rs");
    std::fs::write(&path, "before🌍").unwrap();
    let connect = || {
        BackendConnection::connect(ConnectionOptions {
            workspace: LapceWorkspace {
                path: Some(directory.path().into()),
                ..Default::default()
            },
            local_backend: LocalBackend::Process {
                executable: executable.clone().into(),
            },
            ..Default::default()
        })
        .unwrap()
    };
    let executor = |connection: &BackendConnection| RpcExecutor {
        rpc: connection.proxy_rpc.clone(),
        timeout: Duration::from_secs(10),
    };
    let first = connect();
    let mut session = WorkspaceSession::new(executor(&first));
    session.open(path.clone()).unwrap();
    let id = session.document(&path).unwrap().id;
    first.shutdown_and_wait().unwrap();
    std::fs::write(&path, "after🌍").unwrap();
    let second = connect();
    let outcomes = session.reconnect(executor(&second));
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].1.is_ok(), "{:?}", outcomes[0].1);
    let document = session.document(&path).unwrap();
    assert_eq!(document.id, id);
    assert_eq!(document.buffer.to_string(), "after🌍");
    assert!(document.content.read_only());
    assert!(document.backend_ready());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "after🌍");
    session.close(&path).unwrap();
    second.shutdown_and_wait().unwrap();
}
