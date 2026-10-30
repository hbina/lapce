use std::{
    borrow::Cow,
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use lapce_rpc::{
    RpcError, plugin::PluginId, proxy::ProxyResponse, style::LineStyle,
};
use lapce_xi_rope::{Rope, RopeDelta};
use lsp_types::notification::Notification;
use lsp_types::{
    DidOpenTextDocumentParams, SemanticTokens, TextDocumentIdentifier,
    TextDocumentItem, VersionedTextDocumentIdentifier,
    notification::DidOpenTextDocument,
};
use parking_lot::Mutex;
use serde_json::Value;

use super::{
    PluginCatalogNotification, PluginCatalogRpcHandler,
    rpc::{CloneableCallback, PluginServerRpc, PluginServerRpcHandler, RpcCallback},
};
pub struct PluginCatalog {
    plugin_rpc: PluginCatalogRpcHandler,
    plugins: HashMap<PluginId, PluginServerRpcHandler>,
    open_files: HashMap<PathBuf, String>,
}

impl PluginCatalog {
    pub fn new(
        workspace: Option<PathBuf>,
        language_server_configurations: HashMap<
            String,
            HashMap<String, serde_json::Value>,
        >,
        plugin_rpc: PluginCatalogRpcHandler,
    ) -> Self {
        let plugin = Self {
            plugin_rpc: plugin_rpc.clone(),
            plugins: HashMap::new(),
            open_files: HashMap::new(),
        };

        let native_workspace = workspace;
        let native_configurations = language_server_configurations;
        thread::spawn(move || {
            if let Err(error) = super::native::start(
                native_workspace,
                &native_configurations,
                plugin_rpc.clone(),
            ) {
                plugin_rpc.core_rpc.show_message(
                    "Language server startup failed".into(),
                    lsp_types::ShowMessageParams {
                        typ: lsp_types::MessageType::ERROR,
                        message: error.to_string(),
                    },
                );
            }
        });

        plugin
    }

    #[allow(clippy::too_many_arguments)]
    pub fn handle_server_request(
        &mut self,
        plugin_id: Option<PluginId>,
        request_sent: Option<Arc<AtomicUsize>>,
        method: Cow<'static, str>,
        params: Value,
        language_id: Option<String>,
        path: Option<PathBuf>,
        check: bool,
        f: Box<dyn CloneableCallback<Value, RpcError>>,
    ) {
        if let Some(plugin_id) = plugin_id {
            if let Some(plugin) = self.plugins.get(&plugin_id) {
                plugin.server_request_async(
                    method,
                    params,
                    language_id,
                    path,
                    check,
                    move |result| {
                        f(plugin_id, result);
                    },
                );
            } else {
                f(
                    plugin_id,
                    Err(RpcError {
                        code: 0,
                        message: "plugin doesn't exist".to_string(),
                    }),
                );
            }
            return;
        }

        if let Some(request_sent) = request_sent {
            // if there are no plugins installed the callback of the client is not called
            // so check if plugins list is empty
            if self.plugins.is_empty() {
                // Add a request
                request_sent.fetch_add(1, Ordering::Relaxed);

                // make a direct callback with an "error"
                f(
                    lapce_rpc::plugin::PluginId(0),
                    Err(RpcError {
                        code: 0,
                        message: "no available plugin could make a callback, because the plugins list is empty".to_string(),
                    }),
                );
                return;
            } else {
                request_sent.fetch_add(self.plugins.len(), Ordering::Relaxed);
            }
        }
        for (plugin_id, plugin) in self.plugins.iter() {
            let f = dyn_clone::clone_box(&*f);
            let plugin_id = *plugin_id;
            plugin.server_request_async(
                method.clone(),
                params.clone(),
                language_id.clone(),
                path.clone(),
                check,
                move |result| {
                    f(plugin_id, result);
                },
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn handle_server_notification(
        &mut self,
        plugin_id: Option<PluginId>,
        method: impl Into<Cow<'static, str>>,
        params: Value,
        language_id: Option<String>,
        path: Option<PathBuf>,
        check: bool,
    ) {
        if let Some(plugin_id) = plugin_id {
            if let Some(plugin) = self.plugins.get(&plugin_id) {
                plugin.server_notification(method, params, language_id, path, check);
            }

            return;
        }

        // Otherwise send it to all plugins
        let method = method.into();
        for (_, plugin) in self.plugins.iter() {
            plugin.server_notification(
                method.clone(),
                params.clone(),
                language_id.clone(),
                path.clone(),
                check,
            );
        }
    }

    pub fn handle_did_close_text_document(&mut self, path: PathBuf) {
        let language_id = self.open_files.remove(&path);
        let Ok(uri) = lsp_types::Url::from_file_path(&path) else {
            return;
        };
        for plugin in self.plugins.values() {
            plugin.server_notification(
                <lsp_types::notification::DidCloseTextDocument as lsp_types::notification::Notification>::METHOD,
                lsp_types::DidCloseTextDocumentParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                },
                language_id.clone(),
                Some(path.clone()),
                true,
            );
        }
    }

    pub fn handle_did_open_text_document(&mut self, document: TextDocumentItem) {
        match document.uri.to_file_path() {
            Ok(path) => {
                self.open_files.insert(path, document.language_id.clone());
            }
            Err(err) => {
                tracing::error!("{:?}", err);
            }
        }

        let path = document.uri.to_file_path().ok();
        for (_, plugin) in self.plugins.iter() {
            plugin.server_notification(
                DidOpenTextDocument::METHOD,
                DidOpenTextDocumentParams {
                    text_document: document.clone(),
                },
                Some(document.language_id.clone()),
                path.clone(),
                true,
            );
        }
    }

    pub fn handle_did_save_text_document(
        &mut self,
        language_id: String,
        path: PathBuf,
        text_document: TextDocumentIdentifier,
        text: Rope,
    ) {
        for (_, plugin) in self.plugins.iter() {
            plugin.handle_rpc(PluginServerRpc::DidSaveTextDocument {
                language_id: language_id.clone(),
                path: path.clone(),
                text_document: text_document.clone(),
                text: text.clone(),
            });
        }
    }

    pub fn handle_did_change_text_document(
        &mut self,
        language_id: String,
        document: VersionedTextDocumentIdentifier,
        delta: RopeDelta,
        text: Rope,
        new_text: Rope,
    ) {
        let change = Arc::new(Mutex::new((None, None)));
        for (_, plugin) in self.plugins.iter() {
            plugin.handle_rpc(PluginServerRpc::DidChangeTextDocument {
                language_id: language_id.clone(),
                document: document.clone(),
                delta: delta.clone(),
                text: text.clone(),
                new_text: new_text.clone(),
                change: change.clone(),
            });
        }
    }

    pub fn format_semantic_tokens(
        &self,
        plugin_id: PluginId,
        tokens: SemanticTokens,
        text: Rope,
        f: Box<dyn RpcCallback<Vec<LineStyle>, RpcError>>,
    ) {
        if let Some(plugin) = self.plugins.get(&plugin_id) {
            plugin.handle_rpc(PluginServerRpc::FormatSemanticTokens {
                tokens,
                text,
                f,
            });
        } else {
            f.call(Err(RpcError {
                code: 0,
                message: "plugin doesn't exist".to_string(),
            }));
        }
    }

    pub fn handle_notification(&mut self, notification: PluginCatalogNotification) {
        use PluginCatalogNotification::*;
        match notification {
            PluginServerLoaded(plugin) => {
                // TODO: check if the server has did open registered
                match self.plugin_rpc.proxy_rpc.get_open_files_content() {
                    Ok(ProxyResponse::GetOpenFilesContentResponse { items }) => {
                        for item in items {
                            let language_id = Some(item.language_id.clone());
                            let path = item.uri.to_file_path().ok();
                            plugin.server_notification(
                                DidOpenTextDocument::METHOD,
                                DidOpenTextDocumentParams {
                                    text_document: item,
                                },
                                language_id,
                                path,
                                true,
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::error!("{:?}", err);
                    }
                }

                self.plugins.insert(plugin.plugin_id, plugin);
            }
            Shutdown => {
                for (_, plugin) in self.plugins.iter() {
                    plugin.shutdown();
                }
            }
        }
    }
}
