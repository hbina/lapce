use lapce_engine::directory::DirectoryQuery;
use lapce_rpc::{file::DirectoryEntry, proxy::ProxyResponse};
fn entry(path: &str, is_dir: bool) -> DirectoryEntry {
    DirectoryEntry {
        path: path.into(),
        is_dir,
    }
}
fn response(
    dir: &str,
    items: Vec<DirectoryEntry>,
) -> Result<ProxyResponse, lapce_rpc::RpcError> {
    Ok(ProxyResponse::ReadDirResponse {
        dir: entry(dir, true),
        items,
    })
}
#[test]
fn directory_query_sorts_folders_and_natural_names() {
    let query = DirectoryQuery::new("/workspace".into());
    assert!(query.request().allowed_in_code_browser());
    let entries = query
        .finish(response(
            "/workspace",
            vec![
                entry("/workspace/file10", false),
                entry("/workspace/file2", false),
                entry("/workspace/z", true),
            ],
        ))
        .unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.path.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["/workspace/z", "/workspace/file2", "/workspace/file10"]
    );
}
#[test]
fn directory_query_rejects_foreign_nested_and_duplicate_entries() {
    let query = DirectoryQuery::new("/workspace".into());
    assert!(query.finish(response("/other", vec![])).is_err());
    for items in [
        vec![entry("/outside/file", false)],
        vec![entry("/workspace/sub/file", false)],
        vec![
            entry("/workspace/file", false),
            entry("/workspace/file", true),
        ],
    ] {
        assert!(query.finish(response("/workspace", items)).is_err());
    }
    assert!(query.finish(Ok(ProxyResponse::Success {})).is_err());
    assert!(
        query
            .finish(response("/workspace", vec![]))
            .unwrap()
            .is_empty()
    );
}

#[derive(Debug)]
struct Node {
    is_dir: bool,
    loaded: bool,
    expansion: bool,
}
impl From<DirectoryEntry> for Node {
    fn from(entry: DirectoryEntry) -> Self {
        Self {
            is_dir: entry.is_dir,
            loaded: false,
            expansion: false,
        }
    }
}
impl lapce_engine::directory::DirectoryNode for Node {
    fn is_directory(&self) -> bool {
        self.is_dir
    }
    fn is_loaded(&self) -> bool {
        self.loaded
    }
}
#[test]
fn refresh_preserves_expansion_removes_missing_nodes_and_resets_type_changes() {
    use lapce_engine::directory::reconcile_children;
    use std::{collections::HashMap, path::PathBuf};
    let mut nodes: HashMap<PathBuf, Node> = HashMap::from([
        (
            "/root/keep".into(),
            Node {
                is_dir: true,
                loaded: true,
                expansion: true,
            },
        ),
        (
            "/root/change".into(),
            Node {
                is_dir: true,
                loaded: true,
                expansion: true,
            },
        ),
        ("/root/gone".into(), Node::from(entry("/root/gone", false))),
    ]);
    let refresh = reconcile_children(
        &mut nodes,
        vec![
            entry("/root/keep", true),
            entry("/root/change", false),
            entry("/root/new", true),
        ],
    );
    assert_eq!(refresh, vec![PathBuf::from("/root/keep")]);
    assert!(nodes[&PathBuf::from("/root/keep")].expansion);
    assert!(!nodes[&PathBuf::from("/root/change")].is_dir);
    assert!(!nodes[&PathBuf::from("/root/change")].loaded);
    assert!(!nodes[&PathBuf::from("/root/change")].expansion);
    assert!(!nodes.contains_key(&PathBuf::from("/root/gone")));
    assert!(!nodes[&PathBuf::from("/root/new")].loaded);
    assert!(reconcile_children(&mut nodes, vec![]).is_empty());
    assert!(nodes.is_empty());
}
