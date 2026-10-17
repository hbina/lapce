//! Read-only repository metadata and changed-file browsing shared by frontends.
use indexmap::IndexMap;
use lapce_rpc::source_control::{DiffInfo, FileDiff};
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct SourceControlState {
    pub file_diffs: IndexMap<PathBuf, FileDiff>,
    pub branch: String,
    pub branches: im::Vector<String>,
    pub tags: im::Vector<String>,
}
impl SourceControlState {
    pub fn apply_diff(&mut self, diff: &DiffInfo) {
        self.branch = diff.head.clone();
        self.branches = diff.branches.iter().cloned().collect();
        self.tags = diff.tags.iter().cloned().collect();
        self.file_diffs = diff
            .diffs
            .iter()
            .cloned()
            .map(|diff| (diff.path().clone(), diff))
            .collect();
    }
}
