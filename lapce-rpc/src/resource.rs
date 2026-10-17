//! Filesystem operations independent of language-server and frontend types.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceOperation {
    Create {
        path: PathBuf,
        overwrite: bool,
        ignore_if_exists: bool,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        overwrite: bool,
        ignore_if_exists: bool,
    },
    Delete {
        path: PathBuf,
        recursive: bool,
        ignore_if_missing: bool,
    },
}
impl ResourceOperation {
    pub fn paths(&self) -> Vec<&Path> {
        match self {
            Self::Create { path, .. } | Self::Delete { path, .. } => vec![path],
            Self::Rename { from, to, .. } => vec![from, to],
        }
    }
}
