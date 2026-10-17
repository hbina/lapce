use lapce_rpc::resource::ResourceOperation;
use lapce_services::resource::apply;

#[test]
fn creation_and_rename_preserve_existing_files_unless_overwrite_is_requested() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let target = directory.path().join("target.txt");
    std::fs::write(&source, "source").unwrap();
    std::fs::write(&target, "target").unwrap();
    assert!(
        !apply(&ResourceOperation::Create {
            path: target.clone(),
            overwrite: false,
            ignore_if_exists: true
        })
        .unwrap()
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "target");
    assert!(
        apply(&ResourceOperation::Rename {
            from: source.clone(),
            to: target.clone(),
            overwrite: false,
            ignore_if_exists: false
        })
        .is_err()
    );
    assert!(
        !apply(&ResourceOperation::Rename {
            from: source.clone(),
            to: target.clone(),
            overwrite: false,
            ignore_if_exists: true
        })
        .unwrap()
    );
    assert!(
        apply(&ResourceOperation::Rename {
            from: source.clone(),
            to: target.clone(),
            overwrite: true,
            ignore_if_exists: true
        })
        .unwrap()
    );
    assert!(!source.exists());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "source");
    assert!(
        apply(&ResourceOperation::Create {
            path: target.clone(),
            overwrite: true,
            ignore_if_exists: true
        })
        .unwrap()
    );
    assert_eq!(std::fs::read_to_string(target).unwrap(), "");
}

#[test]
fn deletion_requires_recursive_permission_and_can_ignore_missing_resources() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("child.txt"), "retained").unwrap();
    assert!(
        apply(&ResourceOperation::Delete {
            path: folder.clone(),
            recursive: false,
            ignore_if_missing: false
        })
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("child.txt")).unwrap(),
        "retained"
    );
    assert!(
        apply(&ResourceOperation::Delete {
            path: folder.clone(),
            recursive: true,
            ignore_if_missing: false
        })
        .unwrap()
    );
    assert!(
        !apply(&ResourceOperation::Delete {
            path: folder.clone(),
            recursive: true,
            ignore_if_missing: true
        })
        .unwrap()
    );
    assert!(
        apply(&ResourceOperation::Delete {
            path: folder,
            recursive: true,
            ignore_if_missing: false
        })
        .is_err()
    );
}

#[test]
fn recursive_deletion_of_a_symlink_preserves_its_target() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    let link = directory.path().join("link");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("child"), "retained").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    apply(&ResourceOperation::Delete {
        path: link.clone(),
        recursive: true,
        ignore_if_missing: false,
    })
    .unwrap();
    assert!(!link.exists());
    assert_eq!(
        std::fs::read_to_string(target.join("child")).unwrap(),
        "retained"
    );
}
