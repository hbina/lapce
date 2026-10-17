use lapce_engine::source_control::SourceControlState;
use lapce_rpc::source_control::{DiffInfo, FileDiff};
use std::path::Path;

#[test]
fn repository_refresh_replaces_metadata_and_preserves_backend_diff_order() {
    let mut state = SourceControlState::default();
    state.apply_diff(&DiffInfo {
        head: "main".into(),
        branches: vec!["main".into()],
        tags: vec!["v1".into()],
        diffs: vec![FileDiff::Modified("old.rs".into())],
    });
    state.apply_diff(&DiffInfo {
        head: "feature".into(),
        branches: vec!["feature".into(), "main".into()],
        tags: vec![],
        diffs: vec![
            FileDiff::Added("new.rs".into()),
            FileDiff::Renamed("old.rs".into(), "moved.rs".into()),
            FileDiff::Deleted("gone.rs".into()),
        ],
    });
    assert_eq!(state.branch, "feature");
    assert_eq!(
        state
            .branches
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["feature", "main"]
    );
    assert!(state.tags.is_empty());
    assert_eq!(
        state
            .file_diffs
            .keys()
            .map(|path| path.to_str().unwrap())
            .collect::<Vec<_>>(),
        ["new.rs", "moved.rs", "gone.rs"]
    );
    assert!(!state.file_diffs.contains_key(Path::new("old.rs")));
    assert_eq!(
        state.file_diffs[Path::new("moved.rs")],
        FileDiff::Renamed("old.rs".into(), "moved.rs".into())
    );
}

#[test]
fn repository_disappearance_clears_metadata_and_changed_files() {
    let mut state = SourceControlState::default();
    state.apply_diff(&DiffInfo {
        head: "main".into(),
        diffs: vec![FileDiff::Added("file.rs".into())],
        ..Default::default()
    });
    state.apply_diff(&DiffInfo::default());
    assert!(state.branch.is_empty());
    assert!(state.file_diffs.is_empty());
    assert!(state.branches.is_empty());
    assert!(state.tags.is_empty());
}
