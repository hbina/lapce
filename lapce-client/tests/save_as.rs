#![cfg(feature = "embedded-backend")]
use lapce_client::{
    BackendConnection, ConnectionOptions, workspace::LapceWorkspace,
};
use lapce_rpc::{buffer::BufferId, proxy::ProxyRequest};
use std::time::Duration;

#[test]
fn save_as_cannot_create_or_overwrite_files_or_register_buffers() {
    let directory = tempfile::tempdir().unwrap();
    let existing = directory.path().join("existing.txt");
    std::fs::write(&existing, "original🌍").unwrap();
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
    let absent = directory.path().join("nested/new.txt");
    for path in [existing.clone(), absent.clone()] {
        assert!(
            request(ProxyRequest::SaveBufferAs {
                buffer_id: BufferId::next(),
                path: path.clone(),
                rev: 1,
                content: "overwrite".into(),
                create_parents: true
            })
            .is_err()
        );
        assert!(request(ProxyRequest::GetBuffer { path }).is_err());
    }
    assert_eq!(std::fs::read_to_string(existing).unwrap(), "original🌍");
    assert!(!absent.exists());
    assert!(!directory.path().join("nested").exists());
    connection.shutdown_and_wait().unwrap();
}
