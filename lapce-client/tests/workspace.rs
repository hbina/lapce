#![cfg(feature = "embedded-backend")]

use std::{
    path::Path,
    time::{Duration, Instant},
};

use lapce_client::{
    BackendConnection, ConnectionOptions, workspace::LapceWorkspace,
};
use lapce_core::xi_rope::{Delta, Interval, Rope};
use lapce_rpc::{
    buffer::BufferId,
    core::CoreNotification,
    proxy::{PathRequest, ProxyRequest, ProxyResponse, ProxyStatus},
};

fn request(connection: &BackendConnection, request: ProxyRequest) -> ProxyResponse {
    connection
        .proxy_rpc
        .request_timeout(request, Duration::from_secs(10))
        .unwrap()
}

fn connect(path: &Path) -> BackendConnection {
    BackendConnection::connect(ConnectionOptions {
        workspace: LapceWorkspace {
            path: Some(path.to_owned()),
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn concurrent_workspace_searches_are_isolated_between_sessions() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let contents = format!("needle\n{}", "unrelated contents\n".repeat(1000));
    for workspace in [first.path(), second.path()] {
        for index in 0..100 {
            std::fs::write(workspace.join(format!("file{index}.txt")), &contents)
                .unwrap();
        }
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = [first.path(), second.path()]
        .into_iter()
        .map(|workspace| {
            let connection = connect(workspace);
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                match request(
                    &connection,
                    ProxyRequest::GlobalSearch {
                        pattern: "needle".into(),
                        case_sensitive: true,
                        whole_word: true,
                        is_regex: false,
                    },
                ) {
                    ProxyResponse::GlobalSearchResponse { matches } => {
                        assert_eq!(matches.len(), 100);
                        assert!(matches.values().all(|matches| matches.len() == 1));
                    }
                    response => panic!("unexpected response: {response:?}"),
                }
                connection.shutdown_and_wait().unwrap();
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn repository_mutations_are_rejected_without_creating_a_repository() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("file.txt");
    std::fs::write(&path, "unchanged").unwrap();
    let connection = connect(workspace.path());
    for operation in [
        ProxyRequest::GitInit {},
        ProxyRequest::GitCheckout {
            reference: "main".into(),
        },
        ProxyRequest::GitCommit {
            message: "forbidden".into(),
            diffs: vec![lapce_rpc::source_control::FileDiff::Added(path.clone())],
        },
        ProxyRequest::GitDiscardFilesChanges {
            files: vec![path.clone()],
        },
        ProxyRequest::GitDiscardWorkspaceChanges {},
    ] {
        assert!(
            connection
                .proxy_rpc
                .request_timeout(operation, Duration::from_secs(10))
                .is_err()
        );
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), "unchanged");
    assert!(!workspace.path().join(".git").exists());
    connection.shutdown_and_wait().unwrap();
}

#[test]
fn directory_listing_is_semantic_and_naturally_sorted() {
    let workspace = tempfile::tempdir().unwrap();
    for name in ["file10.txt", "file2.txt"] {
        std::fs::write(workspace.path().join(name), "").unwrap();
    }
    std::fs::create_dir(workspace.path().join("z-directory")).unwrap();
    let connection = connect(workspace.path());
    match request(
        &connection,
        ProxyRequest::ReadDir {
            path: PathRequest::Path(workspace.path().to_owned()),
        },
    ) {
        ProxyResponse::ReadDirResponse { items, .. } => {
            assert_eq!(items.len(), 3);
            assert!(items[0].is_dir);
            assert_eq!(items[0].path.file_name().unwrap(), "z-directory");
            assert_eq!(items[1].path.file_name().unwrap(), "file2.txt");
            assert_eq!(items[2].path.file_name().unwrap(), "file10.txt");
            assert!(!items[1].is_dir);
        }
        response => panic!("unexpected response: {response:?}"),
    }
    connection.shutdown_and_wait().unwrap();
}

#[test]
fn read_only_document_lifecycle_preserves_ownership_and_reload_validation() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("example.txt");
    std::fs::write(&path, "hello 🌍\n").unwrap();
    let connection = connect(workspace.path());
    let buffer_id = BufferId::next();
    let rev = match request(
        &connection,
        ProxyRequest::NewBuffer {
            buffer_id,
            path: path.clone(),
        },
    ) {
        ProxyResponse::NewBufferResponse {
            content,
            read_only: true,
            rev,
        } => {
            assert_eq!(content, "hello 🌍\n");
            rev
        }
        response => panic!("unexpected response: {response:?}"),
    };
    let malformed = Delta {
        base_len: "hello 🌍\n".len(),
        els: vec![lapce_core::xi_rope::DeltaElement::Copy(0, 8)],
    };
    assert!(
        connection
            .proxy_rpc
            .request_timeout(
                ProxyRequest::UpdateBuffer {
                    buffer_id,
                    path: path.clone(),
                    delta: malformed,
                    rev: rev + 1
                },
                Duration::from_secs(10)
            )
            .is_err()
    );
    let foreign_id = BufferId::next();
    for operation in [
        ProxyRequest::NewBuffer {
            buffer_id: foreign_id,
            path: path.clone(),
        },
        ProxyRequest::CloseBuffer {
            buffer_id: foreign_id,
            path: path.clone(),
        },
        ProxyRequest::UpdateBuffer {
            buffer_id: foreign_id,
            path: path.clone(),
            delta: Delta::simple_edit(
                Interval::new(0, 0),
                Rope::from("x"),
                "hello 🌍\n".len(),
            ),
            rev: rev + 1,
        },
    ] {
        assert!(
            connection
                .proxy_rpc
                .request_timeout(operation, Duration::from_secs(10))
                .is_err()
        );
    }
    assert!(
        matches!(request(&connection, ProxyRequest::NewBuffer { buffer_id, path: path.clone() }), ProxyResponse::NewBufferResponse { rev: current, read_only: true, .. } if current == rev)
    );
    assert!(
        matches!(request(&connection, ProxyRequest::GetBuffer { path: path.clone() }), ProxyResponse::BufferSnapshot { buffer_id: owner, rev: current, content, .. } if owner == buffer_id && current == rev && content == "hello 🌍\n")
    );
    request(
        &connection,
        ProxyRequest::CloseBuffer {
            buffer_id,
            path: path.clone(),
        },
    );
    assert!(
        connection
            .proxy_rpc
            .request_timeout(
                ProxyRequest::GetBuffer { path: path.clone() },
                Duration::from_secs(10)
            )
            .is_err()
    );
    // Rejected operations do not terminate the dispatcher.
    request(&connection, ProxyRequest::NewBuffer { buffer_id, path });
    connection.shutdown();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let event = connection
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if matches!(
            event,
            CoreNotification::ProxyStatus {
                status: ProxyStatus::Disconnected
            }
        ) {
            break;
        }
    }
    let error = connection
        .proxy_rpc
        .request_timeout(
            ProxyRequest::GetOpenFilesContent {},
            Duration::from_secs(1),
        )
        .unwrap_err();
    assert_eq!(error.code, -32001);
    connection.shutdown_and_wait().unwrap();
}

#[test]
fn filesystem_resource_requests_cannot_change_open_documents() {
    use lapce_rpc::resource::ResourceOperation;
    let workspace = tempfile::tempdir().unwrap();
    let connection = connect(workspace.path());
    let source = workspace.path().join("source.txt");
    let target = workspace.path().join("target.txt");
    std::fs::write(&source, "source text").unwrap();
    assert!(
        connection
            .proxy_rpc
            .request_timeout(
                ProxyRequest::ApplyResourceOperation {
                    operation: ResourceOperation::Create {
                        path: target.clone(),
                        overwrite: false,
                        ignore_if_exists: false
                    }
                },
                Duration::from_secs(10)
            )
            .is_err()
    );
    assert!(!target.exists());
    request(
        &connection,
        ProxyRequest::NewBuffer {
            buffer_id: BufferId::next(),
            path: source.clone(),
        },
    );
    std::fs::create_dir(workspace.path().join("subdirectory")).unwrap();
    for operation in [
        ResourceOperation::Create {
            path: source.clone(),
            overwrite: true,
            ignore_if_exists: false,
        },
        ResourceOperation::Create {
            path: workspace.path().join("subdirectory/../source.txt"),
            overwrite: true,
            ignore_if_exists: false,
        },
        ResourceOperation::Rename {
            from: source.clone(),
            to: target,
            overwrite: false,
            ignore_if_exists: false,
        },
        ResourceOperation::Delete {
            path: workspace.path().to_owned(),
            recursive: true,
            ignore_if_missing: false,
        },
    ] {
        assert!(
            connection
                .proxy_rpc
                .request_timeout(
                    ProxyRequest::ApplyResourceOperation { operation },
                    Duration::from_secs(10)
                )
                .is_err()
        );
    }
    assert!(source.exists());
    assert!(matches!(
        request(&connection, ProxyRequest::GetBuffer { path: source }),
        ProxyResponse::BufferSnapshot { .. }
    ));
    connection.shutdown_and_wait().unwrap();
}
