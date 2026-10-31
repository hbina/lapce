use lapce_engine::sync::{DocumentSync, SyncOutcome};
use lapce_rpc::{
    RpcError,
    buffer::BufferId,
    proxy::{ProxyRequest, ProxyResponse},
};
use lapce_xi_rope::{Delta, Interval, Rope};

#[test]
fn disk_updates_are_ordered_and_revision_mapping_is_acknowledged() {
    let mut sync = DocumentSync::new("test".into(), BufferId::next(), 1, 10);
    sync.edit(
        Delta::simple_edit(Interval::new(0, 0), Rope::from("a"), 0),
        2,
    );
    sync.edit(
        Delta::simple_edit(Interval::new(1, 1), Rope::from("b"), 1),
        3,
    );
    let first = sync.next_effect().unwrap();
    assert!(matches!(
        first.request,
        ProxyRequest::UpdateBuffer { rev: 11, .. }
    ));
    assert!(sync.next_effect().is_none());
    assert_eq!(sync.remote_revision(), 10);
    assert_eq!(
        sync.acknowledge(
            first.id + 99,
            Ok(ProxyResponse::BufferUpdated {
                snapshot: Default::default(),
                rev: 11
            })
        ),
        SyncOutcome::Stale
    );
    assert_eq!(
        sync.acknowledge(
            first.id,
            Ok(ProxyResponse::BufferUpdated {
                snapshot: Default::default(),
                rev: 11
            })
        ),
        SyncOutcome::Edited
    );
    let effect = sync.next_effect().unwrap();
    assert!(matches!(
        effect.request,
        ProxyRequest::UpdateBuffer { rev: 12, .. }
    ));
    sync.acknowledge(
        effect.id,
        Ok(ProxyResponse::BufferUpdated {
            snapshot: Default::default(),
            rev: 12,
        }),
    );
    assert!(sync.is_idle());
}

#[test]
fn failure_pauses_queue_and_explicit_recovery_preserves_local_text_without_saving() {
    let id = BufferId::next();
    let mut sync = DocumentSync::new("test".into(), id, 0, 0);
    sync.edit(
        Delta::simple_edit(Interval::new(0, 0), Rope::from("local"), 0),
        1,
    );
    let old = sync.next_effect().unwrap();
    assert!(matches!(
        sync.acknowledge(
            old.id,
            Err(RpcError {
                code: 1,
                message: "conflict".into()
            })
        ),
        SyncOutcome::Failed(_)
    ));
    assert!(sync.next_effect().is_none());
    let snapshot = ProxyResponse::BufferSnapshot {
        snapshot: Default::default(),
        buffer_id: id,
        path: "test".into(),
        content: "remote".into(),
        read_only: false,
        rev: 5,
    };
    sync.recover_keep_local(snapshot, Rope::from("local🌍"), 2)
        .unwrap();
    let effect = sync.next_effect().unwrap();
    let ProxyRequest::UpdateBuffer { delta, rev, .. } = effect.request else {
        panic!("expected edit")
    };
    assert_eq!(rev, 6);
    assert_eq!(delta.apply(&Rope::from("remote")).to_string(), "local🌍");
    assert_eq!(
        sync.acknowledge(
            old.id,
            Ok(ProxyResponse::BufferUpdated {
                snapshot: Default::default(),
                rev: 1
            })
        ),
        SyncOutcome::Stale
    );
    assert_eq!(
        sync.acknowledge(
            effect.id,
            Ok(ProxyResponse::BufferUpdated {
                snapshot: Default::default(),
                rev: 6
            })
        ),
        SyncOutcome::Edited
    );
    assert!(sync.is_idle());
    assert!(sync.next_effect().is_none());
}

#[test]
fn bad_acknowledgements_and_foreign_snapshots_cannot_reset_synchronization() {
    let id = BufferId::next();
    let mut sync = DocumentSync::new("test".into(), id, 0, 0);
    sync.edit(
        Delta::simple_edit(Interval::new(0, 0), Rope::from("x"), 0),
        1,
    );
    let effect = sync.next_effect().unwrap();
    assert!(matches!(
        sync.acknowledge(
            effect.id,
            Ok(ProxyResponse::BufferUpdated {
                snapshot: Default::default(),
                rev: 9
            })
        ),
        SyncOutcome::Failed(_)
    ));
    for (buffer_id, path, read_only) in [
        (BufferId::next(), "test", false),
        (id, "other", false),
        (id, "test", true),
    ] {
        assert!(
            sync.recover_keep_local(
                ProxyResponse::BufferSnapshot {
                    snapshot: Default::default(),
                    buffer_id,
                    path: path.into(),
                    content: "".into(),
                    read_only,
                    rev: 4
                },
                Rope::from("x"),
                1
            )
            .is_err()
        );
        assert!(sync.error.is_some());
    }
}

#[test]
fn backend_versions_map_before_and_after_acknowledgement_without_assuming_equal_counters()
 {
    let mut sync = DocumentSync::new("test".into(), BufferId::next(), 5, 10);
    assert_eq!(sync.local_version(10), Some(5));
    assert_eq!(sync.local_version(11), None);
    sync.edit(
        Delta::simple_edit(Interval::new(0, 0), Rope::from("x"), 0),
        8,
    );
    let effect = sync.next_effect().unwrap();
    assert_eq!(sync.local_version(11), Some(8));
    sync.acknowledge(
        effect.id,
        Ok(ProxyResponse::BufferUpdated {
            snapshot: Default::default(),
            rev: 11,
        }),
    );
    assert_eq!(sync.local_version(11), Some(8));
    assert_eq!(sync.local_version(10), None);
}

#[test]
fn language_requests_wait_for_edits_but_do_not_block_subsequent_edits() {
    let mut sync = DocumentSync::new("test".into(), BufferId::next(), 0, 0);
    let edit = || Delta::simple_edit(Interval::new(0, 0), Rope::from("x"), 0);
    sync.edit(edit(), 1);
    let query = sync
        .request(
            ProxyRequest::GetDocumentSymbols {
                path: "test".into(),
            },
            1,
        )
        .unwrap();
    let first = sync.next_effect().unwrap();
    assert!(!first.detached);
    assert!(sync.next_effect().is_none());
    sync.acknowledge(
        first.id,
        Ok(ProxyResponse::BufferUpdated {
            snapshot: Default::default(),
            rev: 1,
        }),
    );
    let request = sync.next_effect().unwrap();
    assert_eq!(request.id, query);
    assert!(request.detached);
    sync.edit(
        Delta::simple_edit(Interval::new(1, 1), Rope::from("y"), 1),
        2,
    );
    let next = sync.next_effect().unwrap();
    assert!(matches!(
        next.request,
        ProxyRequest::UpdateBuffer { rev: 2, .. }
    ));
    assert_eq!(
        sync.acknowledge(
            query,
            Ok(ProxyResponse::GetDocumentSymbols {
                resp: lsp_types::DocumentSymbolResponse::Nested(Vec::new())
            })
        ),
        SyncOutcome::RequestCompleted {
            id: query,
            current: false
        }
    );
    assert_eq!(
        sync.acknowledge(
            next.id,
            Ok(ProxyResponse::BufferUpdated {
                snapshot: Default::default(),
                rev: 2
            })
        ),
        SyncOutcome::Edited
    );
    assert!(sync.is_idle());
}

#[test]
fn notifications_and_queries_preserve_order_after_acknowledged_edits() {
    use lapce_rpc::proxy::ProxyNotification;
    use lsp_types::Position;
    let mut sync = DocumentSync::new("test".into(), BufferId::next(), 0, 0);
    sync.edit(
        Delta::simple_edit(Interval::new(0, 0), Rope::from("x"), 0),
        1,
    );
    assert!(sync.notification(ProxyNotification::OpenFileChanged {
        path: "test".into()
    }));
    sync.request(
        ProxyRequest::GetHover {
            request_id: 4,
            path: "test".into(),
            position: Position::new(0, 1),
        },
        1,
    );
    assert!(sync.next_notification().is_none());
    let edit = sync.next_effect().unwrap();
    assert!(sync.next_notification().is_none());
    sync.acknowledge(
        edit.id,
        Ok(ProxyResponse::BufferUpdated {
            snapshot: Default::default(),
            rev: 1,
        }),
    );
    assert!(sync.next_effect().is_none());
    assert!(matches!(
        sync.next_notification(),
        Some(ProxyNotification::OpenFileChanged { .. })
    ));
    let query = sync.next_effect().unwrap();
    assert!(query.detached);
    assert_eq!(
        sync.acknowledge(
            query.id,
            Err(RpcError {
                code: 1,
                message: "no provider".into()
            })
        ),
        SyncOutcome::RequestCompleted {
            id: query.id,
            current: true
        }
    );
    assert!(sync.error.is_none());
    assert!(sync.is_idle());
}

#[test]
fn close_waits_for_mutations_and_rejects_later_operations() {
    let owner = BufferId::next();
    let mut sync = DocumentSync::new("test".into(), owner, 0, 0);
    assert!(sync.edit(
        Delta::simple_edit(Interval::new(0, 0), Rope::from("a"), 0),
        1
    ));
    let close = sync.close().unwrap();
    assert!(sync.close().is_none());
    assert!(!sync.edit(
        Delta::simple_edit(Interval::new(1, 1), Rope::from("b"), 1),
        2
    ));
    assert!(
        sync.request(
            ProxyRequest::GetBuffer {
                path: "test".into()
            },
            1
        )
        .is_none()
    );
    let edit = sync.next_effect().unwrap();
    assert_eq!(
        sync.acknowledge(
            edit.id,
            Ok(ProxyResponse::BufferUpdated {
                snapshot: Default::default(),
                rev: 1
            })
        ),
        SyncOutcome::Edited
    );
    let effect = sync.next_effect().unwrap();
    assert_eq!(effect.id, close);
    assert!(
        matches!(effect.request, ProxyRequest::CloseBuffer { buffer_id, .. } if buffer_id == owner)
    );
    assert!(!sync.is_closed());
    assert_eq!(
        sync.acknowledge(close, Ok(ProxyResponse::Success {})),
        SyncOutcome::Closed { id: close }
    );
    assert!(sync.is_closed());
    assert_eq!(sync.local_version(1), None);
    assert!(sync.next_effect().is_none());
}

#[test]
fn failed_close_preserves_state_and_recovery_cancels_closing() {
    let owner = BufferId::next();
    let mut sync = DocumentSync::new("test".into(), owner, 0, 0);
    let close = sync.close().unwrap();
    sync.next_effect().unwrap();
    assert!(matches!(
        sync.acknowledge(
            close,
            Err(RpcError {
                code: -1,
                message: "offline".into()
            })
        ),
        SyncOutcome::Failed(_)
    ));
    assert!(!sync.is_closed());
    assert!(sync.close().is_none());
    assert!(sync.next_effect().is_none());
    sync.recover_keep_local(
        ProxyResponse::BufferSnapshot {
            snapshot: Default::default(),
            buffer_id: owner,
            path: "test".into(),
            content: "".into(),
            read_only: false,
            rev: 0,
        },
        Rope::from(""),
        0,
    )
    .unwrap();
    assert!(sync.close().is_some());
}

#[test]
fn generic_queries_cannot_enqueue_mutation_requests() {
    let owner = BufferId::next();
    let mut sync = DocumentSync::new("test".into(), owner, 0, 0);
    for request in [
        ProxyRequest::Save {
            buffer_id: owner,
            path: "test".into(),
            rev: 0,
            create_parents: true,
        },
        ProxyRequest::GetDocumentFormatting {
            path: "test".into(),
        },
        ProxyRequest::CreateFile { path: "new".into() },
    ] {
        assert!(sync.request(request, 0).is_none());
        assert!(sync.next_effect().is_none());
        assert!(sync.is_idle());
    }
    assert!(
        sync.request(
            ProxyRequest::GetDocumentSymbols {
                path: "test".into()
            },
            0
        )
        .is_some()
    );
}

#[test]
fn document_notifications_reject_editing_debugging_and_foreign_paths() {
    use lapce_rpc::proxy::ProxyNotification;
    let mut sync = DocumentSync::new("test".into(), BufferId::next(), 0, 0);
    for notification in [
        ProxyNotification::Completion {
            request_id: 1,
            path: "test".into(),
            input: "x".into(),
            position: lsp_types::Position::new(0, 0),
        },
        ProxyNotification::DapStop {
            dap_id: Default::default(),
        },
        ProxyNotification::OpenFileChanged {
            path: "foreign".into(),
        },
        ProxyNotification::Shutdown {},
    ] {
        assert!(!sync.notification(notification));
        assert!(sync.is_idle());
    }
    assert!(sync.notification(ProxyNotification::LspCancel { id: 1 }));
    assert!(matches!(
        sync.next_notification(),
        Some(ProxyNotification::LspCancel { id: 1 })
    ));
}
