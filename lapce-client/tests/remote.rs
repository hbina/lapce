//! Opt-in runtime checks against an existing remote host and read-only fixture.
//! LAPCE_TEST_SSH_HOST accepts user@host:port; LAPCE_TEST_SSH_FILE must name
//! an existing absolute remote file. Normal SSH authentication must be set up.
//! LAPCE_TEST_SSH_PROXY optionally selects an existing remote backend; this
//! avoids installing or downloading a backend on the validation host.
use lapce_client::workspace::{LapceWorkspace, LapceWorkspaceType, SshHost};
use lapce_client::{BackendConnection, ConnectionOptions};
use lapce_rpc::{
    buffer::BufferId,
    proxy::{ProxyRequest, ProxyResponse},
};
use std::{path::PathBuf, time::Duration};

#[test]
#[ignore = "requires LAPCE_TEST_SSH_HOST and LAPCE_TEST_SSH_FILE"]
fn ssh_backend_opens_queries_and_closes_read_only_file() {
    let host =
        std::env::var("LAPCE_TEST_SSH_HOST").expect("set LAPCE_TEST_SSH_HOST");
    let path = PathBuf::from(
        std::env::var_os("LAPCE_TEST_SSH_FILE").expect("set LAPCE_TEST_SSH_FILE"),
    );
    exercise_remote(
        LapceWorkspaceType::RemoteSSH(SshHost::from_string(&host)),
        path,
        std::env::var("LAPCE_TEST_SSH_PROXY").ok(),
    );
}

fn exercise_remote(
    kind: LapceWorkspaceType,
    path: PathBuf,
    remote_backend_executable: Option<String>,
) {
    assert!(
        path.is_absolute(),
        "fixture must be an absolute remote path"
    );
    let connection = BackendConnection::connect(ConnectionOptions {
        workspace: LapceWorkspace {
            kind,
            path: path
                .parent()
                .expect("remote file has a parent")
                .to_path_buf(),
            ..Default::default()
        },
        remote_backend_executable,
        ..Default::default()
    })
    .unwrap();
    let call = |request| {
        connection
            .proxy_rpc
            .request_timeout(request, Duration::from_secs(60))
    };
    // Preserve cleanup even if a remote request fails or has an unexpected type.
    let opened = call(ProxyRequest::NewBuffer {
        buffer_id: BufferId(0),
        path: path.clone(),
    });
    let snapshot = call(ProxyRequest::GetBuffer { path: path.clone() });
    let save = call(ProxyRequest::Save {
        buffer_id: BufferId(0),
        path: path.clone(),
        rev: 0,
        create_parents: false,
    });
    let closed = call(ProxyRequest::CloseBuffer {
        buffer_id: BufferId(0),
        path: path.clone(),
    });
    connection.shutdown_and_wait().unwrap();
    let ProxyResponse::NewBufferResponse {
        content,
        read_only: true,
        rev,
    } = opened.unwrap()
    else {
        panic!("remote backend must open a read-only buffer");
    };
    assert!(matches!(snapshot.unwrap(), ProxyResponse::BufferSnapshot {
        buffer_id: BufferId(0), path: returned_path, content: returned_content, read_only: true, rev: returned_rev
    } if returned_path == path && returned_content == content && returned_rev == rev));
    assert!(save.is_err(), "remote backend accepted a save request");
    assert!(closed.is_ok(), "remote close failed: {closed:?}");
}
