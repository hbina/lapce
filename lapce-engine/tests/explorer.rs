use lapce_engine::explorer::{FileNodeItem, FileNodeViewKind};
use lapce_rpc::file::DirectoryEntry;
use std::path::Path;
#[test]
fn visible_slices_are_exclusive_and_match_expanded_tree_order() {
    let mut root = FileNodeItem::from(DirectoryEntry {
        path: "/root".into(),
        is_dir: true,
    });
    root.open = true;
    root.add_child(Path::new("/root/folder"), true).unwrap();
    root.add_child(Path::new("/root/folder/leaf"), false)
        .unwrap();
    root.get_file_node_mut(Path::new("/root/folder"))
        .unwrap()
        .open = true;
    root.add_child(Path::new("/root/file10"), false).unwrap();
    root.add_child(Path::new("/root/file2"), false).unwrap();
    root.update_node_count_recursive(Path::new("/root/folder"));
    assert_eq!(root.children_open_count, 4);
    let mut full = vec![];
    root.append_view_slice(&mut full, 0, 5, 0, 1);
    assert_eq!(
        full.iter()
            .map(|row| row.kind.path().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "/root",
            "/root/folder",
            "/root/folder/leaf",
            "/root/file2",
            "/root/file10"
        ]
    );
    for start in 0..=5 {
        for end in start..=5 {
            let mut slice = vec![];
            root.append_view_slice(&mut slice, start, end, 0, 1);
            assert_eq!(
                slice.iter().map(|row| &row.kind).collect::<Vec<_>>(),
                full[start..end]
                    .iter()
                    .map(|row| &row.kind)
                    .collect::<Vec<_>>()
            );
        }
    }
    root.get_file_node_mut(Path::new("/root/folder"))
        .unwrap()
        .open = false;
    root.update_node_count_recursive(Path::new("/root/folder"));
    assert_eq!(root.children_open_count, 3);
    let mut collapsed = vec![];
    root.append_view_slice(&mut collapsed, 0, 4, 0, 1);
    assert!(
        !collapsed.iter().any(
            |row| row.kind == FileNodeViewKind::Path("/root/folder/leaf".into())
        )
    );
}

#[test]
fn directory_transitions_preserve_collapsed_state_and_ignore_removed_nodes() {
    let root_path = Path::new("/root");
    let folder = Path::new("/root/folder");
    let mut root = FileNodeItem::from(DirectoryEntry {
        path: root_path.into(),
        is_dir: true,
    });
    assert_eq!(root.toggle_directory(root_path), Some(true));
    assert_eq!(root.toggle_directory(root_path), Some(false));
    // A late read should populate the cache without reopening a collapsed folder.
    assert_eq!(
        root.refresh_directory(
            root_path,
            vec![DirectoryEntry {
                path: folder.into(),
                is_dir: true
            }]
        ),
        Some(vec![])
    );
    assert!(!root.open);
    assert_eq!(root.children_open_count, 0);
    assert_eq!(root.toggle_directory(root_path), Some(false));
    assert_eq!(root.children_open_count, 1);
    assert_eq!(root.toggle_directory(folder), Some(true));
    root.refresh_directory(folder, vec![]).unwrap();
    assert_eq!(
        root.refresh_directory(
            root_path,
            vec![DirectoryEntry {
                path: folder.into(),
                is_dir: true
            }]
        ),
        Some(vec![folder.to_path_buf()])
    );
    root.refresh_directory(root_path, vec![]).unwrap();
    assert!(root.refresh_directory(folder, vec![]).is_none());
    assert!(root.toggle_directory(folder).is_none());
}

#[test]
fn reveal_reads_ancestors_then_exposes_a_visible_row_or_missing_path() {
    use lapce_engine::explorer::RevealStep;
    let root_path = Path::new("/root");
    let folder = Path::new("/root/folder");
    let file = Path::new("/root/folder/file.rs");
    let mut root = FileNodeItem::from(DirectoryEntry {
        path: root_path.into(),
        is_dir: true,
    });
    assert_eq!(root.reveal(Path::new("/outside/file")), RevealStep::Missing);
    assert_eq!(
        root.reveal(file),
        RevealStep::ReadDirectory(root_path.into())
    );
    root.refresh_directory(
        root_path,
        vec![DirectoryEntry {
            path: folder.into(),
            is_dir: true,
        }],
    )
    .unwrap();
    assert_eq!(root.reveal(file), RevealStep::ReadDirectory(folder.into()));
    root.refresh_directory(
        folder,
        vec![DirectoryEntry {
            path: file.into(),
            is_dir: false,
        }],
    )
    .unwrap();
    root.toggle_directory(root_path);
    assert_eq!(root.reveal(file), RevealStep::Visible { row: 2 });
    assert_eq!(root.children_open_count, 2);
    assert_eq!(
        root.reveal(Path::new("/root/folder/absent")),
        RevealStep::Missing
    );
    assert_eq!(root.reveal(root_path), RevealStep::Visible { row: 0 });
}

#[test]
fn invalid_refresh_preserves_the_entire_previous_snapshot() {
    let path = Path::new("/root");
    let mut root = FileNodeItem::from(DirectoryEntry {
        path: path.into(),
        is_dir: true,
    });
    root.toggle_directory(path);
    root.refresh_directory(
        path,
        vec![DirectoryEntry {
            path: "/root/keep".into(),
            is_dir: false,
        }],
    )
    .unwrap();
    let before = root.clone();
    for entries in [
        vec![DirectoryEntry {
            path: "/foreign".into(),
            is_dir: false,
        }],
        vec![DirectoryEntry {
            path: "/root/nested/file".into(),
            is_dir: false,
        }],
        vec![
            DirectoryEntry {
                path: "/root/dup".into(),
                is_dir: false,
            },
            DirectoryEntry {
                path: "/root/dup".into(),
                is_dir: true,
            },
        ],
    ] {
        assert!(root.refresh_directory(path, entries).is_none());
        assert_eq!(root, before);
    }
}
