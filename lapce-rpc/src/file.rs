use std::{cmp::Ordering, path::PathBuf};

use serde::{Deserialize, Serialize};

/// Decode a local or remote file URI without requiring its path to be native
/// with percent-encoded workspace paths.
pub fn file_url_path(url: &url::Url) -> Option<PathBuf> {
    if url.scheme() != "file" {
        return None;
    }
    if let Ok(path) = url.to_file_path() {
        return Some(path);
    }
    // Preserve native file URL conversion. Do not silently
    // discard an unsupported authority on other hosts.
    if url.host_str().is_some_and(|host| host != "localhost") {
        return None;
    }
    let path = percent_encoding::percent_decode_str(url.path())
        .decode_utf8()
        .ok()?;
    Some(PathBuf::from(path.into_owned()))
}

#[cfg(test)]
mod file_url_tests {
    use super::*;
    #[test]
    fn remote_posix_paths_and_unicode_survive_frontend_conversion() {
        let uri =
            url::Url::parse("file:///workspace/a%20b/%F0%9F%8C%8D.rs").unwrap();
        assert_eq!(
            file_url_path(&uri),
            Some(PathBuf::from("/workspace/a b/🌍.rs"))
        );
        assert!(
            file_url_path(&url::Url::parse("https://example.test/file.rs").unwrap())
                .is_none()
        );
    }
}

/// UTF8 line and column-offset
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct LineCol {
    pub line: usize,
    pub column: usize,
}

#[derive(
    Default, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct PathObject {
    pub path: PathBuf,
    pub linecol: Option<LineCol>,
    pub is_dir: bool,
}

impl PathObject {
    pub fn new(
        path: PathBuf,
        is_dir: bool,
        line: usize,
        column: usize,
    ) -> PathObject {
        PathObject {
            path,
            is_dir,
            linecol: Some(LineCol { line, column }),
        }
    }

    pub fn from_path(path: PathBuf, is_dir: bool) -> PathObject {
        PathObject {
            path,
            is_dir,
            linecol: None,
        }
    }
}

/// A filesystem entry. Tree expansion, naming and row layout are frontend state.
/// Unknown legacy explorer fields are ignored when reading an older proxy response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub is_dir: bool,
}

impl DirectoryEntry {
    /// Directories first, then natural ordering of names, independent of a view.
    pub fn compare(&self, other: &Self) -> Ordering {
        match (self.is_dir, other.is_dir) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => {
                let names = [&self.path, &other.path].map(|path| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                });
                human_sort::compare(&names[0], &names[1])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_entry_wire_format_contains_only_filesystem_data() {
        let entry = DirectoryEntry {
            path: "folder".into(),
            is_dir: true,
        };
        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "path": "folder", "is_dir": true })
        );
        let legacy = serde_json::json!({ "path": "folder", "is_dir": true,
            "read": true, "open": true, "children": {}, "children_open_count": 123 });
        assert_eq!(
            serde_json::from_value::<DirectoryEntry>(legacy).unwrap(),
            entry
        );
    }
}
