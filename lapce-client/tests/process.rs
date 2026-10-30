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
    rpc.initialize(Some(directory.path().into()), Default::default());
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
#[ignore = "requires a built lapce-proxy executable"]
fn shared_queues_deliver_disk_events_and_acknowledge_each_frontend_revision() {
    use lapce_client::{shared::SharedBackend, workspace::LapceWorkspace};
    use lapce_rpc::{buffer::BufferId, core::FileChanged, proxy::ProxyResponse};
    use lapce_xi_rope::{Rope, RopeDelta};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("queue.rs");
    let original = "before🌍\n";
    let changed = "after🌍 externally\n";
    std::fs::write(&path, original).unwrap();
    let host = SharedBackend::connect(ConnectionOptions {
        workspace: LapceWorkspace {
            path: Some(directory.path().into()),
            ..Default::default()
        },
        local_backend: LocalBackend::Process {
            executable: std::env::var_os("LAPCE_TEST_PROXY")
                .expect("set LAPCE_TEST_PROXY")
                .into(),
        },
        ..Default::default()
    })
    .unwrap();
    let peers = [host.attach().unwrap(), host.attach().unwrap()];
    let call = |index: usize, request| {
        peers[index]
            .proxy_rpc
            .request_timeout(request, Duration::from_secs(10))
    };
    let revisions = [0, 1].map(|index| {
        match call(
            index,
            ProxyRequest::NewBuffer {
                buffer_id: BufferId(0),
                path: path.clone(),
            },
        )
        .unwrap()
        {
            ProxyResponse::NewBufferResponse {
                content,
                read_only: true,
                rev,
            } => {
                assert_eq!(content, original);
                rev
            }
            response => panic!("unexpected open response: {response:?}"),
        }
    });
    // A client cannot disguise an edit as a disk reload.
    let update = |text: &str, rev| ProxyRequest::UpdateBuffer {
        buffer_id: BufferId(0),
        path: path.clone(),
        delta: RopeDelta::simple_edit(
            0..original.len(),
            Rope::from(text),
            original.len(),
        ),
        rev,
    };
    assert!(call(0, update("forged edit", revisions[0] + 1)).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    std::fs::write(&path, changed).unwrap();
    // Both event queues must receive the same watcher payload before either
    // frontend acknowledges it. Neither frontend consumes the other's event.
    for peer in &peers {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let event = peer
                .events
                .recv_timeout(
                    deadline.saturating_duration_since(std::time::Instant::now()),
                )
                .expect("missing external-change event");
            if let CoreNotification::OpenFileChanged {
                path: event_path,
                content: FileChanged::Change(content),
            } = event
            {
                if event_path == path && content == changed {
                    break;
                }
            }
        }
    }
    for index in 0..2 {
        let rev = revisions[index] + 1;
        assert!(matches!(call(index, update(changed, rev)).unwrap(),
            ProxyResponse::BufferUpdated { rev: accepted } if accepted == rev));
        // A duplicate acknowledgement cannot advance the local revision again.
        assert!(call(index, update(changed, rev)).is_err());
        assert!(
            matches!(call(index, ProxyRequest::GetBuffer { path: path.clone() }).unwrap(),
            ProxyResponse::BufferSnapshot { buffer_id: BufferId(0), content, read_only: true, .. }
                if content == changed)
        );
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
    drop(peers);
    host.shutdown_and_wait().unwrap();
}

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

#[test]
#[ignore = "requires a built lapce-proxy executable"]
fn two_frontends_share_one_process_and_close_buffers_independently() {
    use lapce_client::{shared::SharedBackend, workspace::LapceWorkspace};
    use lapce_rpc::{buffer::BufferId, proxy::ProxyResponse};
    let executable =
        std::env::var_os("LAPCE_TEST_PROXY").expect("set LAPCE_TEST_PROXY");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file.rs");
    std::fs::write(&path, "shared🌍 needle").unwrap();
    let host = SharedBackend::connect(ConnectionOptions {
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
    let call = |rpc: &lapce_rpc::proxy::ProxyRpcHandler, query| {
        rpc.request_timeout(query, Duration::from_secs(10))
    };
    for frontend in [&first, &second] {
        assert!(
            matches!(call(&frontend.proxy_rpc, ProxyRequest::NewBuffer { buffer_id: BufferId(0), path: path.clone() }).unwrap(), ProxyResponse::NewBufferResponse { content, read_only: true, .. } if content == "shared🌍 needle")
        );
    }
    assert!(
        call(
            &first.proxy_rpc,
            ProxyRequest::CloseBuffer {
                buffer_id: BufferId(99),
                path: path.clone()
            }
        )
        .is_err()
    );
    call(
        &first.proxy_rpc,
        ProxyRequest::CloseBuffer {
            buffer_id: BufferId(0),
            path: path.clone(),
        },
    )
    .unwrap();
    assert!(
        call(
            &first.proxy_rpc,
            ProxyRequest::GetBuffer { path: path.clone() }
        )
        .is_err()
    );
    assert!(
        matches!(call(&second.proxy_rpc, ProxyRequest::GetBuffer { path: path.clone() }).unwrap(), ProxyResponse::BufferSnapshot { buffer_id: BufferId(0), content, .. } if content == "shared🌍 needle")
    );
    let workers: Vec<_> = [&first, &second]
        .into_iter()
        .map(|frontend| {
            let rpc = frontend.proxy_rpc.clone();
            std::thread::spawn(move || {
                rpc.request_timeout(
                    ProxyRequest::GlobalSearch {
                        pattern: "needle".into(),
                        case_sensitive: false,
                        whole_word: false,
                        is_regex: false,
                    },
                    Duration::from_secs(10),
                )
                .unwrap()
            })
        })
        .collect();
    for worker in workers {
        assert!(
            matches!(worker.join().unwrap(), ProxyResponse::GlobalSearchResponse { matches } if matches.len() == 1)
        );
    }
    assert!(
        call(
            &second.proxy_rpc,
            ProxyRequest::Save {
                buffer_id: BufferId(0),
                rev: 1,
                path: path.clone(),
                create_parents: false
            }
        )
        .is_err()
    );
    call(
        &first.proxy_rpc,
        ProxyRequest::NewBuffer {
            buffer_id: BufferId(0),
            path: path.clone(),
        },
    )
    .unwrap();
    drop(first);
    assert!(matches!(
        call(
            &second.proxy_rpc,
            ProxyRequest::GetBuffer { path: path.clone() }
        )
        .unwrap(),
        ProxyResponse::BufferSnapshot { .. }
    ));
    call(
        &second.proxy_rpc,
        ProxyRequest::CloseBuffer {
            buffer_id: BufferId(0),
            path: path.clone(),
        },
    )
    .unwrap();
    let third = host.attach().unwrap();
    assert!(matches!(
        call(
            &third.proxy_rpc,
            ProxyRequest::NewBuffer {
                buffer_id: BufferId(0),
                path: path.clone()
            }
        )
        .unwrap(),
        ProxyResponse::NewBufferResponse { .. }
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "shared🌍 needle");
    drop(second);
    drop(third);
    host.shutdown_and_wait().unwrap();
}

#[test]
#[ignore = "requires a built lapce-proxy executable"]
fn socket_frontends_share_backend_and_disconnect_independently() {
    use lapce_client::{shared::SharedBackend, workspace::LapceWorkspace};
    use lapce_rpc::{buffer::BufferId, proxy::ProxyResponse};
    use std::{os::unix::net::UnixListener, sync::Arc};
    let executable =
        std::env::var_os("LAPCE_TEST_PROXY").expect("set LAPCE_TEST_PROXY");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.rs");
    std::fs::write(&path, "socket🌍").unwrap();
    let socket = directory.path().join("backend.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let host = Arc::new(
        SharedBackend::connect(ConnectionOptions {
            workspace: LapceWorkspace {
                path: Some(directory.path().into()),
                ..Default::default()
            },
            local_backend: LocalBackend::Process {
                executable: executable.into(),
            },
            ..Default::default()
        })
        .unwrap(),
    );
    let server_host = host.clone();
    let server = std::thread::spawn(move || {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            let frontend = server_host.attach().unwrap();
            workers.push(std::thread::spawn(move || {
                lapce_client::socket::serve_frontend(frontend, stream).unwrap()
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    let connect = || {
        BackendConnection::connect(ConnectionOptions {
            local_backend: LocalBackend::SharedSocket {
                path: socket.clone(),
            },
            ..Default::default()
        })
        .unwrap()
    };
    let first = connect();
    let second = connect();
    let open = |connection: &BackendConnection| {
        connection
            .proxy_rpc
            .request_timeout(
                ProxyRequest::NewBuffer {
                    buffer_id: BufferId(0),
                    path: path.clone(),
                },
                Duration::from_secs(10),
            )
            .unwrap()
    };
    for connection in [&first, &second] {
        assert!(
            matches!(open(connection), ProxyResponse::NewBufferResponse { content, read_only: true, .. } if content == "socket🌍")
        );
    }
    first.shutdown_and_wait().unwrap();
    assert!(matches!(
        second
            .proxy_rpc
            .request_timeout(
                ProxyRequest::GetBuffer { path },
                Duration::from_secs(10)
            )
            .unwrap(),
        ProxyResponse::BufferSnapshot {
            buffer_id: BufferId(0),
            ..
        }
    ));
    second.shutdown_and_wait().unwrap();
    server.join().unwrap();
    Arc::try_unwrap(host)
        .ok()
        .unwrap()
        .shutdown_and_wait()
        .unwrap();
}
