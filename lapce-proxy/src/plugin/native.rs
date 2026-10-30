//! Native language-server configuration owned by the backend, independent of a UI.
use super::{PluginCatalogRpcHandler, lsp::LspClient};
use lsp_types::{DocumentFilter, Url};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, path::PathBuf};

#[derive(Deserialize)]
struct NativeServer {
    language: String,
    executable: PathBuf,
    #[serde(default)]
    arguments: Vec<String>,
    #[serde(default)]
    options: Option<Value>,
}
pub(super) fn start(
    workspace: Option<PathBuf>,
    configurations: &HashMap<String, HashMap<String, Value>>,
    rpc: PluginCatalogRpcHandler,
) -> anyhow::Result<()> {
    let Some(servers) = configurations
        .get("lapce-native-lsp")
        .and_then(|config| config.get("servers"))
    else {
        return Ok(());
    };
    let servers: Vec<NativeServer> = serde_json::from_value(servers.clone())?;
    for server in servers {
        let uri = Url::from_file_path(&server.executable).map_err(|_| {
            anyhow::anyhow!(
                "Native language-server executable must be an absolute path"
            )
        })?;
        anyhow::ensure!(
            !server.language.is_empty(),
            "Native language-server language cannot be empty"
        );
        LspClient::start(
            rpc.clone(),
            vec![DocumentFilter {
                language: Some(server.language.clone()),
                scheme: Some("file".into()),
                pattern: None,
            }],
            workspace.clone(),
            format!("Native {} language server", server.language),
            uri,
            server.arguments,
            server.options,
        )?;
    }
    Ok(())
}
