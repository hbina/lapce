#![cfg(feature = "embedded-backend")]
use lapce_client::{
    BackendConnection, ConnectionOptions, workspace::LapceWorkspace,
};
use lapce_rpc::{
    buffer::BufferId,
    proxy::{ProxyRequest, ProxyResponse},
};
use lapce_xi_rope::{Delta, Interval, Rope};
use std::time::Duration;

#[test]
fn backend_browses_files_but_rejects_edits_saves_resources_and_debugging() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.rs");
    std::fs::write(&path, "original🌍").unwrap();
    let connection = BackendConnection::connect(ConnectionOptions {
        workspace: LapceWorkspace {
            path: directory.path().into(),
            ..Default::default()
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
    let original = Rope::from("original🌍");
    assert!(matches!(
        request(ProxyRequest::NewBuffer {
            buffer_id: id,
            path: path.clone()
        }),
        Ok(ProxyResponse::NewBufferResponse {
            read_only: true,
            rev: 1,
            ..
        })
    ));
    let replacement = Delta::simple_edit(
        Interval::new(0, original.len()),
        Rope::from("changed"),
        original.len(),
    );
    assert!(
        request(ProxyRequest::UpdateBuffer {
            buffer_id: id,
            path: path.clone(),
            delta: replacement,
            rev: 2
        })
        .is_err()
    );
    let new_path = directory.path().join("new.rs");
    for operation in [
        ProxyRequest::Save {
            buffer_id: id,
            path: path.clone(),
            rev: 1,
            create_parents: true,
        },
        ProxyRequest::SaveBufferAs {
            buffer_id: id,
            path: new_path.clone(),
            rev: 1,
            content: "new".into(),
            create_parents: true,
        },
        ProxyRequest::CreateFile {
            path: new_path.clone(),
        },
        ProxyRequest::TrashPath { path: path.clone() },
        ProxyRequest::ApplyResourceOperation {
            operation: lapce_rpc::resource::ResourceOperation::Delete {
                path: path.clone(),
                recursive: false,
                ignore_if_missing: false,
            },
        },
        ProxyRequest::GitInit {},
        ProxyRequest::DapVariable {
            dap_id: Default::default(),
            reference: 1,
        },
    ] {
        assert!(request(operation).is_err());
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original🌍");
    assert!(!new_path.exists());
    assert!(!directory.path().join(".git").exists());
    assert!(
        matches!(request(ProxyRequest::GetBuffer { path: path.clone() }), Ok(ProxyResponse::BufferSnapshot { read_only: true, rev: 1, content, .. }) if content == "original🌍")
    );
    // External tools can change files; the browser can synchronize that disk text.
    std::fs::write(&path, "external").unwrap();
    let reload = Delta::simple_edit(
        Interval::new(0, original.len()),
        Rope::from("external"),
        original.len(),
    );
    assert!(matches!(
        request(ProxyRequest::UpdateBuffer {
            buffer_id: id,
            path: path.clone(),
            delta: reload,
            rev: 2
        }),
        Ok(ProxyResponse::BufferUpdated { rev: 2 })
    ));
    assert!(
        request(ProxyRequest::CloseBuffer {
            buffer_id: id,
            path
        })
        .is_ok()
    );
    connection.shutdown_and_wait().unwrap();
}
