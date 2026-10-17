use lapce_client::leases::{BufferLeases, FrontendId};
use lapce_rpc::buffer::BufferId;
use std::path::Path;

#[test]
fn colliding_frontend_ids_share_backend_owner_and_close_independently() {
    let mut leases = BufferLeases::default();
    let first = FrontendId::next();
    let second = FrontendId::next();
    let path = Path::new("source.rs");
    let local = BufferId(0);
    let one = leases.acquire(first, path.into(), local).unwrap();
    let two = leases.acquire(second, path.into(), local).unwrap();
    assert!(one.first_owner);
    assert!(!two.first_owner);
    assert_eq!(one.backend_id, two.backend_id);
    assert!(leases.backend_id(FrontendId::next(), path, local).is_err());
    assert!(leases.acquire(first, path.into(), BufferId(99)).is_err());
    assert!(leases.acquire(first, "other.rs".into(), local).is_err());
    assert_eq!(leases.release(first, path, local).unwrap(), None);
    assert_eq!(
        leases.backend_id(second, path, local).unwrap(),
        one.backend_id
    );
    assert_eq!(
        leases.release(second, path, local).unwrap(),
        Some(one.backend_id)
    );
    assert!(leases.local_id(second, path).is_none());
}

#[test]
fn disconnect_closes_only_unshared_files_and_failed_open_can_release_reservation() {
    let mut leases = BufferLeases::default();
    let first = FrontendId::next();
    let second = FrontendId::next();
    let shared = Path::new("shared.rs");
    let private = Path::new("private.rs");
    leases.acquire(first, shared.into(), BufferId(1)).unwrap();
    leases.acquire(second, shared.into(), BufferId(1)).unwrap();
    let owned = leases.acquire(first, private.into(), BufferId(2)).unwrap();
    assert_eq!(
        leases.disconnect(first),
        vec![(private.into(), owned.backend_id)]
    );
    assert_eq!(leases.local_id(second, shared), Some(BufferId(1)));
    let failed = leases
        .acquire(first, "failed.rs".into(), BufferId(3))
        .unwrap();
    assert_eq!(
        leases
            .release(first, Path::new("failed.rs"), BufferId(3))
            .unwrap(),
        Some(failed.backend_id)
    );
    assert!(
        leases
            .acquire(first, "failed.rs".into(), BufferId(4))
            .unwrap()
            .first_owner
    );
}
