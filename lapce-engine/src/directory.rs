//! Read-only directory queries shared by every frontend.
use lapce_rpc::{
    RpcError,
    file::DirectoryEntry,
    proxy::{PathRequest, ProxyRequest, ProxyResponse},
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

/// Validate a complete snapshot before any tree state is changed.
pub fn validate_entries(
    path: &Path,
    items: &[DirectoryEntry],
) -> Result<(), String> {
    let mut paths = HashSet::new();
    for item in items {
        if item.path.parent() != Some(path)
            || item.path.file_name().is_none()
            || !paths.insert(&item.path)
        {
            return Err("Invalid or duplicate directory entry".into());
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct DirectoryQuery {
    path: PathBuf,
}
impl DirectoryQuery {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn request(&self) -> ProxyRequest {
        ProxyRequest::ReadDir {
            path: PathRequest::Path(self.path.clone()),
        }
    }
    /// Validate ownership before exposing entries; sort independently of UI layout.
    pub fn finish(
        &self,
        response: Result<ProxyResponse, RpcError>,
    ) -> Result<Vec<DirectoryEntry>, String> {
        let ProxyResponse::ReadDirResponse { dir, mut items } =
            response.map_err(|error| error.message)?
        else {
            return Err("Unexpected directory response".into());
        };
        if dir.path != self.path || !dir.is_dir {
            return Err("Directory response belongs to another path".into());
        }
        validate_entries(&self.path, &items)?;
        items.sort_by(DirectoryEntry::compare);
        Ok(items)
    }
}

/// Adapter for frontend-specific expansion and row state.
pub trait DirectoryNode: From<DirectoryEntry> {
    fn is_directory(&self) -> bool;
    fn is_loaded(&self) -> bool;
}

/// Replace a directory snapshot without discarding unchanged expansion state.
/// Type changes reset the node, since a former directory's children are invalid.
/// The returned paths are loaded subdirectories that the host should refresh.
pub fn reconcile_children<N: DirectoryNode>(
    children: &mut std::collections::HashMap<PathBuf, N>,
    entries: Vec<DirectoryEntry>,
) -> Vec<PathBuf> {
    let paths: HashSet<_> = entries.iter().map(|entry| entry.path.clone()).collect();
    children.retain(|path, _| paths.contains(path));
    let mut refresh = Vec::new();
    for entry in entries {
        let unchanged = children
            .get(&entry.path)
            .is_some_and(|node| node.is_directory() == entry.is_dir);
        if unchanged {
            let node = &children[&entry.path];
            if node.is_directory() && node.is_loaded() {
                refresh.push(entry.path);
            }
        } else {
            children.insert(entry.path.clone(), entry.into());
        }
    }
    refresh.sort();
    refresh
}
