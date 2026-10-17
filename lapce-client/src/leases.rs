//! Frontend-local document identities mapped to shared backend buffer owners.
use lapce_rpc::buffer::BufferId;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrontendId(u64);
impl FrontendId {
    pub fn next() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferLease {
    pub backend_id: BufferId,
    pub first_owner: bool,
}
struct SharedBuffer {
    backend_id: BufferId,
    owners: HashMap<FrontendId, BufferId>,
}
#[derive(Default)]
pub struct BufferLeases {
    files: HashMap<PathBuf, SharedBuffer>,
}
impl BufferLeases {
    /// Reserve ownership before opening; release it if backend opening fails.
    pub fn acquire(
        &mut self,
        frontend: FrontendId,
        path: PathBuf,
        local_id: BufferId,
    ) -> Result<BufferLease, String> {
        if self.files.iter().any(|(owned_path, buffer)| {
            owned_path != &path && buffer.owners.get(&frontend) == Some(&local_id)
        }) {
            return Err("Frontend document already owns another path".into());
        }
        if let Some(buffer) = self.files.get_mut(&path) {
            if buffer
                .owners
                .get(&frontend)
                .is_some_and(|id| *id != local_id)
            {
                return Err(
                    "Frontend already owns this path with another document".into()
                );
            }
            buffer.owners.insert(frontend, local_id);
            return Ok(BufferLease {
                backend_id: buffer.backend_id,
                first_owner: false,
            });
        }
        // Allocate independently of frontend IDs, which may collide across processes.
        let backend_id = BufferId::next();
        self.files.insert(
            path,
            SharedBuffer {
                backend_id,
                owners: HashMap::from([(frontend, local_id)]),
            },
        );
        Ok(BufferLease {
            backend_id,
            first_owner: true,
        })
    }
    pub fn backend_id(
        &self,
        frontend: FrontendId,
        path: &Path,
        local_id: BufferId,
    ) -> Result<BufferId, String> {
        let buffer = self.files.get(path).ok_or("File is not open")?;
        if buffer.owners.get(&frontend) != Some(&local_id) {
            return Err("Frontend document does not own this file".into());
        }
        Ok(buffer.backend_id)
    }
    pub fn is_last_owner(
        &self,
        frontend: FrontendId,
        path: &Path,
        local_id: BufferId,
    ) -> Result<bool, String> {
        self.backend_id(frontend, path, local_id)?;
        Ok(self.files[path].owners.len() == 1)
    }
    pub fn local_id(&self, frontend: FrontendId, path: &Path) -> Option<BufferId> {
        self.files.get(path)?.owners.get(&frontend).copied()
    }
    /// Returns the backend owner only when its final frontend lease is released.
    pub fn release(
        &mut self,
        frontend: FrontendId,
        path: &Path,
        local_id: BufferId,
    ) -> Result<Option<BufferId>, String> {
        let backend_id = self.backend_id(frontend, path, local_id)?;
        let buffer = self.files.get_mut(path).unwrap();
        buffer.owners.remove(&frontend);
        if buffer.owners.is_empty() {
            self.files.remove(path);
            return Ok(Some(backend_id));
        }
        Ok(None)
    }
    /// Release all documents on frontend disconnect, keeping other owners intact.
    pub fn disconnect(&mut self, frontend: FrontendId) -> Vec<(PathBuf, BufferId)> {
        let paths: Vec<_> = self
            .files
            .iter()
            .filter_map(|(path, buffer)| {
                buffer.owners.get(&frontend).map(|id| (path.clone(), *id))
            })
            .collect();
        let mut closed = Vec::new();
        for (path, id) in paths {
            if let Ok(Some(backend_id)) = self.release(frontend, &path, id) {
                closed.push((path, backend_id));
            }
        }
        closed.sort_by(|a, b| a.0.cmp(&b.0));
        closed
    }
}
