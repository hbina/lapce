use lapce_client::{BackendConnection, ConnectionOptions, LocalBackend};
use lapce_rpc::{
    core::CoreNotification,
    proxy::{ProxyRequest, ProxyStatus},
};
use std::time::Duration;

#[test]
#[ignore = "requires a built lapce-proxy executable"]
fn remote_launcher_serves_real_backend_requests_over_stdio_queues() {
    use lapce_client::remote::{Remote, start_remote};
    use lapce_rpc::{
        buffer::BufferId,
        core::CoreRpcHandler,
        proxy::{ProxyResponse, ProxyRpcHandler},
    };
    use std::{
        path::{Path, PathBuf},
        process::Command,
    };
    struct LocalTransport(PathBuf);
    impl Remote for LocalTransport {
        fn command_builder(&self) -> Command {
            let mut command = Command::new("sh");
            command.args(["-c", "exec \"$@\"", "remote-test"]);
            command
        }
        fn proxy_command(&self, _: &str) -> anyhow::Result<Command> {
            Ok(Command::new(&self.0))
        }
        fn upload_file(&self, _: impl AsRef<Path>, _: &str) -> anyhow::Result<()> {
            panic!("the installed test backend must not trigger a download")
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("remote source.rs");
    std::fs::write(&path, "remote🌍\n").unwrap();
    let rpc = ProxyRpcHandler::new();
    rpc.initialize(
        Some(directory.path().into()),
        vec![],
        vec![],
        Default::default(),
    );
    let driver_rpc = rpc.clone();
    let worker = std::thread::spawn(move || {
        start_remote(
            LocalTransport(
                std::env::var_os("LAPCE_TEST_PROXY")
                    .expect("set LAPCE_TEST_PROXY")
                    .into(),
            ),
            CoreRpcHandler::new(),
            driver_rpc,
        )
    });
    let result = rpc.request_timeout(
        ProxyRequest::NewBuffer {
            buffer_id: BufferId(0),
            path: path.clone(),
        },
        Duration::from_secs(10),
    );
    rpc.shutdown();
    worker.join().unwrap().unwrap();
    assert!(matches!(result.unwrap(), ProxyResponse::NewBufferResponse {
        content, read_only: true, ..
    } if content == "remote🌍\n"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "remote🌍\n");
}

/// Exercise the frontend queue contract without a renderer or session model.

#[test]
fn missing_backend_reports_failure_and_releases_requests() {
    let directory = tempfile::tempdir().unwrap();
    let connection = BackendConnection::connect(ConnectionOptions {
        local_backend: LocalBackend::Process {
            executable: directory.path().join("missing-backend"),
        },
        ..Default::default()
    })
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    connection.proxy_rpc.request_async(
        ProxyRequest::GetOpenFilesContent {},
        move |result| {
            tx.send(result).unwrap();
        },
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap_err()
            .code,
        -32001
    );
    let mut failure = false;
    loop {
        match connection
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            CoreNotification::ShowMessage { title, message } => {
                assert_eq!(title, "Backend connection failed");
                assert_eq!(message.typ, lsp_types::MessageType::ERROR);
                assert!(!message.message.is_empty());
                failure = true;
            }
            CoreNotification::ProxyStatus {
                status: ProxyStatus::Disconnected,
            } => break,
            _ => {}
        }
    }
    assert!(failure);
    assert_eq!(
        connection
            .proxy_rpc
            .request_timeout(
                ProxyRequest::GetOpenFilesContent {},
                Duration::from_secs(1)
            )
            .unwrap_err()
            .code,
        -32001
    );
    connection.shutdown_and_wait().unwrap();
}

/// Build lapce-proxy first and set LAPCE_TEST_PROXY to its absolute path.
#[test]
#[ignore = "requires a built lapce-proxy executable"]
fn real_process_browses_workspace_and_enforces_read_only() {
    use lapce_client::workspace::LapceWorkspace;
    use lapce_rpc::{
        buffer::BufferId,
        proxy::{PathRequest, ProxyResponse},
    };
    let executable = std::env::var_os("LAPCE_TEST_PROXY")
        .expect("set LAPCE_TEST_PROXY to the backend executable");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.rs");
    let text = "fn needle() {} // 🌍\n";
    std::fs::write(&path, text).unwrap();
    let connection = BackendConnection::connect(ConnectionOptions {
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
    let request = |request| {
        connection
            .proxy_rpc
            .request_timeout(request, Duration::from_secs(10))
    };
    let id = BufferId::next();
    assert!(
        matches!(request(ProxyRequest::NewBuffer { buffer_id: id, path: path.clone() }).unwrap(), ProxyResponse::NewBufferResponse { content, read_only: true, .. } if content == text)
    );
    assert!(
        matches!(request(ProxyRequest::ReadDir { path: PathRequest::Path(directory.path().into()) }).unwrap(), ProxyResponse::ReadDirResponse { items, .. } if items.iter().any(|entry| entry.path == path))
    );
    assert!(
        matches!(request(ProxyRequest::GlobalSearch { pattern: "needle".into(), case_sensitive: false, whole_word: false, is_regex: false }).unwrap(), ProxyResponse::GlobalSearchResponse { matches } if matches.get(&path).is_some_and(|items| items.len() == 1))
    );
    assert!(
        request(ProxyRequest::Save {
            buffer_id: id,
            rev: 1,
            path: path.clone(),
            create_parents: false
        })
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    connection.shutdown_and_wait().unwrap();
}

#[test]
#[ignore = "requires a built lapce-proxy executable"]
fn managed_restart_preserves_frontend_endpoint_and_replaces_backend() {
    use lapce_client::{managed::ManagedBackend, workspace::LapceWorkspace};
    use lapce_rpc::{buffer::BufferId, proxy::ProxyResponse};
    let executable =
        std::env::var_os("LAPCE_TEST_PROXY").expect("set LAPCE_TEST_PROXY");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.rs");
    std::fs::write(&path, "before🌍").unwrap();
    let connection = ManagedBackend::connect(ConnectionOptions {
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
    let rpc = connection.proxy_rpc.clone();
    let wait_connected = || {
        loop {
            let event = connection
                .events
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            if matches!(
                event,
                CoreNotification::ProxyStatus {
                    status: ProxyStatus::Connected
                }
            ) {
                break;
            }
        }
    };
    wait_connected();
    let id = BufferId::next();
    let open = || {
        rpc.request_timeout(
            ProxyRequest::NewBuffer {
                buffer_id: id,
                path: path.clone(),
            },
            Duration::from_secs(10),
        )
        .unwrap()
    };
    assert!(
        matches!(open(), ProxyResponse::NewBufferResponse { content, read_only: true, .. } if content == "before🌍")
    );
    std::fs::write(&path, "after🌍").unwrap();
    connection.control.restart().unwrap();
    let mut disconnected = false;
    loop {
        match connection
            .events
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
        {
            CoreNotification::ProxyStatus {
                status: ProxyStatus::Disconnected,
            } => disconnected = true,
            CoreNotification::ProxyStatus {
                status: ProxyStatus::Connected,
            } => {
                assert!(disconnected);
                break;
            }
            _ => {}
        }
    }
    assert!(
        matches!(open(), ProxyResponse::NewBufferResponse { content, read_only: true, .. } if content == "after🌍")
    );
    assert!(
        rpc.request_timeout(
            ProxyRequest::CloseBuffer {
                buffer_id: id,
                path
            },
            Duration::from_secs(10)
        )
        .is_ok()
    );
    connection.shutdown_and_wait().unwrap();
}
