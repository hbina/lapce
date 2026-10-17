//! Browsing tree, expansion and visible-row projection shared by frontends.
use lapce_rpc::file::DirectoryEntry;
use serde::{Deserialize, Serialize};
use std::{
    cmp::{Ord, Ordering, PartialOrd},
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileNodeViewKind {
    Path(PathBuf),
}
impl FileNodeViewKind {
    pub fn path(&self) -> Option<&Path> {
        let Self::Path(path) = self;
        Some(path)
    }
}

#[derive(Debug, Clone)]
pub struct FileNodeViewData {
    pub kind: FileNodeViewKind,
    pub is_dir: bool,
    pub is_root: bool,
    pub open: bool,
    pub level: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RevealStep {
    Visible { row: usize },
    ReadDirectory(PathBuf),
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileNodeItem {
    pub path: PathBuf,
    pub is_dir: bool,
    /// Whether the directory's children have been read.
    /// Does nothing if not a directory.
    pub read: bool,
    /// Whether the directory is open in the explorer view.
    pub open: bool,
    pub children: HashMap<PathBuf, FileNodeItem>,
    /// The number of child (directories) that are open themselves
    /// Used for sizing of the explorer list
    pub children_open_count: usize,
}

impl PartialOrd for FileNodeItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FileNodeItem {
    fn cmp(&self, other: &Self) -> Ordering {
        DirectoryEntry {
            path: self.path.clone(),
            is_dir: self.is_dir,
        }
        .compare(&DirectoryEntry {
            path: other.path.clone(),
            is_dir: other.is_dir,
        })
    }
}

impl FileNodeItem {
    /// Expand known ancestors or request the next missing directory snapshot.
    pub fn reveal(&mut self, path: &Path) -> RevealStep {
        if !path.starts_with(&self.path) {
            return RevealStep::Missing;
        }
        if self.get_file_node(path).is_some() {
            for ancestor in path.ancestors() {
                if let Some(node) = self.get_file_node_mut(ancestor) {
                    if node.is_dir {
                        node.open = true;
                    }
                }
            }
            self.update_node_count_recursive(path);
            if path == self.path {
                return RevealStep::Visible { row: 0 };
            }
            let (found, row) = self.find_file_at_line(path);
            return if found {
                RevealStep::Visible { row: row as usize }
            } else {
                RevealStep::Missing
            };
        }
        for ancestor in path.ancestors() {
            if let Some(node) = self.get_file_node_mut(ancestor) {
                if !node.is_dir || node.read {
                    return RevealStep::Missing;
                }
                node.open = true;
                self.update_node_count_recursive(ancestor);
                return RevealStep::ReadDirectory(ancestor.to_path_buf());
            }
        }
        RevealStep::Missing
    }
    /// Toggle a known directory and tell the host whether it needs a read.
    pub fn toggle_directory(&mut self, path: &Path) -> Option<bool> {
        let node = self.get_file_node_mut(path)?;
        if !node.is_dir {
            return None;
        }
        node.open = !node.open;
        let needs_read = node.open && !node.read;
        self.update_node_count_recursive(path);
        Some(needs_read)
    }

    /// Apply a validated directory snapshot and return loaded children to refresh.
    /// Exclusions are supplied by the host before applying the snapshot.
    pub fn refresh_directory(
        &mut self,
        path: &Path,
        entries: Vec<DirectoryEntry>,
    ) -> Option<Vec<PathBuf>> {
        crate::directory::validate_entries(path, &entries).ok()?;
        let node = self.get_file_node_mut(path)?;
        if !node.is_dir {
            return None;
        }
        node.read = true;
        let refresh =
            crate::directory::reconcile_children(&mut node.children, entries);
        self.update_node_count_recursive(path);
        Some(refresh)
    }
    /// Collect the children, sorted by name.
    /// Note: this will be empty if the directory has not been read.
    pub fn sorted_children(&self) -> Vec<&FileNodeItem> {
        let mut children = self.children.values().collect::<Vec<&FileNodeItem>>();
        children.sort();
        children
    }

    /// Collect the children, sorted by name.
    /// Note: this will be empty if the directory has not been read.
    pub fn sorted_children_mut(&mut self) -> Vec<&mut FileNodeItem> {
        let mut children = self
            .children
            .values_mut()
            .collect::<Vec<&mut FileNodeItem>>();
        children.sort();
        children
    }

    /// Returns an iterator over the ancestors of `path`, starting with the first descendant of `prefix`.
    ///
    /// # Example:
    /// (ignored because the function is private but I promise this passes)
    /// ```rust,ignore
    /// # use lapce_app::file_explorer::model::FileNodeItem;
    /// # use std::path::{Path, PathBuf};
    /// # use std::collections::HashMap;
    /// #
    /// let node_item = FileNodeItem {
    ///     path_buf: PathBuf::from("/pre/fix"),
    ///     // ...
    /// #    is_dir: true,
    /// #    read: false,
    /// #    open: false,
    /// #    children: HashMap::new(),
    /// #    children_open_count: 0,
    ///};
    /// let mut iter = node_item.ancestors_rev(Path::new("/pre/fix/foo/bar")).unwrap();
    /// assert_eq!(Some(Path::new("/pre/fix/foo")), iter.next());
    /// assert_eq!(Some(Path::new("/pre/fix/foo/bar")), iter.next());
    /// ```
    fn ancestors_rev<'a>(
        &self,
        path: &'a Path,
    ) -> Option<impl Iterator<Item = &'a Path> + use<'a>> {
        let take = if let Ok(suffix) = path.strip_prefix(&self.path) {
            suffix.components().count()
        } else {
            return None;
        };

        #[allow(clippy::needless_collect)] // Ancestors is not reversible
        let ancestors = path.ancestors().take(take).collect::<Vec<&Path>>();
        Some(ancestors.into_iter().rev())
    }

    /// Recursively get the node at `path`.
    pub fn get_file_node(&self, path: &Path) -> Option<&FileNodeItem> {
        self.ancestors_rev(path)?
            .try_fold(self, |node, path| node.children.get(path))
    }

    /// Recursively get the (mutable) node at `path`.
    pub fn get_file_node_mut(&mut self, path: &Path) -> Option<&mut FileNodeItem> {
        self.ancestors_rev(path)?
            .try_fold(self, |node, path| node.children.get_mut(path))
    }

    /// Remove a specific child from the node.
    /// The path is recursive and will remove the child from parent indicated by the path.
    pub fn remove_child(&mut self, path: &Path) -> Option<FileNodeItem> {
        let parent = path.parent()?;
        let node = self.get_file_node_mut(parent)?;
        let node = node.children.remove(path)?;
        for p in path.ancestors() {
            self.update_node_count(p);
        }

        Some(node)
    }

    /// Add a new (unread & unopened) child to the node.
    pub fn add_child(&mut self, path: &Path, is_dir: bool) -> Option<()> {
        let parent = path.parent()?;
        let node = self.get_file_node_mut(parent)?;
        node.children.insert(
            PathBuf::from(path),
            FileNodeItem {
                path: PathBuf::from(path),
                is_dir,
                read: false,
                open: false,
                children: HashMap::new(),
                children_open_count: 0,
            },
        );
        for p in path.ancestors() {
            self.update_node_count(p);
        }

        Some(())
    }

    /// Set the children of the node.
    /// Note: this opens the node.
    pub fn set_item_children(
        &mut self,
        path: &Path,
        children: HashMap<PathBuf, FileNodeItem>,
    ) {
        if let Some(node) = self.get_file_node_mut(path) {
            node.open = true;
            node.read = true;
            node.children = children;
        }

        for p in path.ancestors() {
            self.update_node_count(p);
        }
    }

    pub fn update_node_count_recursive(&mut self, path: &Path) {
        for current_path in path.ancestors() {
            self.update_node_count(current_path);
        }
    }

    pub fn update_node_count(&mut self, path: &Path) -> Option<()> {
        let node = self.get_file_node_mut(path)?;
        if node.is_dir {
            node.children_open_count = if node.open {
                node.children
                    .values()
                    .map(|item| item.children_open_count + 1)
                    .sum::<usize>()
            } else {
                0
            };
        }
        None
    }

    pub fn append_view_slice(
        &self,
        view_items: &mut Vec<FileNodeViewData>,
        min: usize,
        max: usize,
        current: usize,
        level: usize,
    ) -> usize {
        if current >= max {
            return current;
        }
        if current + self.children_open_count < min {
            return current + self.children_open_count;
        }

        if current >= min {
            let kind = FileNodeViewKind::Path(self.path.clone());
            view_items.push(FileNodeViewData {
                kind,
                is_dir: self.is_dir,
                is_root: level == 1,
                open: self.open,
                level,
            });
        }

        self.append_children_view_slice(view_items, min, max, current, level)
    }

    /// Calculate the row where the file resides
    pub fn find_file_at_line(&self, file_path: &Path) -> (bool, f64) {
        let mut line = 0.0;
        if !self.open {
            return (false, line);
        }
        for item in self.sorted_children() {
            line += 1.0;
            match (item.is_dir, item.open, item.path == file_path) {
                (_, _, true) => {
                    return (true, line);
                }
                (true, true, _) => {
                    let (found, item_position) = item.find_file_at_line(file_path);
                    line += item_position;
                    if found {
                        return (true, line);
                    }
                }
                _ => {}
            }
        }
        (false, line)
    }

    /// Append the children of this item with the given level
    pub fn append_children_view_slice(
        &self,
        view_items: &mut Vec<FileNodeViewData>,
        min: usize,
        max: usize,
        mut i: usize,
        level: usize,
    ) -> usize {
        if !self.open {
            return i;
        }
        for item in self.sorted_children() {
            i = item.append_view_slice(view_items, min, max, i + 1, level + 1);
            if i >= max {
                return i;
            }
        }

        i
    }
}

impl From<DirectoryEntry> for FileNodeItem {
    fn from(entry: DirectoryEntry) -> Self {
        Self {
            path: entry.path,
            is_dir: entry.is_dir,
            read: false,
            open: false,
            children: HashMap::new(),
            children_open_count: 0,
        }
    }
}

impl crate::directory::DirectoryNode for FileNodeItem {
    fn is_directory(&self) -> bool {
        self.is_dir
    }
    fn is_loaded(&self) -> bool {
        self.read
    }
}

#[cfg(test)]
mod directory_adapter_tests {
    use super::*;
    #[test]
    fn type_changes_drop_expanded_children_and_recompute_visible_rows() {
        let root_path = PathBuf::from("/workspace");
        let folder = root_path.join("folder");
        let leaf = folder.join("leaf");
        let mut root = FileNodeItem::from(DirectoryEntry {
            path: root_path.clone(),
            is_dir: true,
        });
        root.open = true;
        root.add_child(&folder, true).unwrap();
        root.add_child(&leaf, false).unwrap();
        let child = root.get_file_node_mut(&folder).unwrap();
        child.read = true;
        child.open = true;
        root.update_node_count_recursive(&folder);
        assert_eq!(root.children_open_count, 2);
        let refresh = crate::directory::reconcile_children(
            &mut root.children,
            vec![DirectoryEntry {
                path: folder.clone(),
                is_dir: false,
            }],
        );
        root.update_node_count_recursive(&root_path);
        assert!(refresh.is_empty());
        assert_eq!(root.children_open_count, 1);
        assert!(root.get_file_node(&leaf).is_none());
        assert!(!root.get_file_node(&folder).unwrap().open);
    }
}
