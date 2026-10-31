use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use lapce_rpc::file::DirectoryEntry;

/// Presentation state only; directory contents come from WorkspaceSession.
#[derive(Default, Debug, Clone)]
pub struct DirectoryTree {
    children: BTreeMap<PathBuf, Vec<DirectoryEntry>>,
    expanded: BTreeSet<PathBuf>,
}

impl DirectoryTree {
    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    pub fn collapse(&mut self, path: &Path) {
        self.expanded.remove(path);
    }

    pub fn expand(&mut self, path: PathBuf, entries: Vec<DirectoryEntry>) {
        // Discard cached descendants of deleted or replaced directories.
        self.children.retain(|cached, _| {
            cached == &path
                || !cached.starts_with(&path)
                || entries
                    .iter()
                    .any(|entry| entry.is_dir && cached.starts_with(&entry.path))
        });
        self.expanded.retain(|expanded| {
            expanded == &path
                || !expanded.starts_with(&path)
                || entries
                    .iter()
                    .any(|entry| entry.is_dir && expanded.starts_with(&entry.path))
        });
        self.expanded.insert(path.clone());
        self.children.insert(path, entries);
    }

    /// Flatten expanded branches in the engine's directory-first natural order.
    pub fn rows(&self, root: &Path) -> Vec<(&DirectoryEntry, usize)> {
        fn visit<'a>(
            tree: &'a DirectoryTree,
            path: &Path,
            depth: usize,
            rows: &mut Vec<(&'a DirectoryEntry, usize)>,
        ) {
            if tree.is_expanded(path)
                && let Some(children) = tree.children.get(path)
            {
                for entry in children {
                    rows.push((entry, depth));
                    if entry.is_dir {
                        visit(tree, &entry.path, depth + 1, rows);
                    }
                }
            }
        }
        let mut rows = Vec::new();
        visit(self, root, 1, &mut rows);
        rows
    }
}
