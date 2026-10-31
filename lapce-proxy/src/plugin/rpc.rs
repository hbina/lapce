use std::{
    borrow::Cow,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender};
use dyn_clone::DynClone;
use floem_editor_core::buffer::rope_text::{RopeText, RopeTextRef};
use jsonrpc_lite::{Id, JsonRpc, Params};
use lapce_core::rope_text_pos::RopeTextPosition;
use lapce_rpc::{
    RpcError,
    core::{CoreRpcHandler, ServerStatusParams},
    plugin::PluginId,
    style::{LineStyle, Style},
};
use lapce_xi_rope::{Rope, RopeDelta};
use lsp_types::{
    CancelParams, CodeActionProviderCapability, DidChangeTextDocumentParams,
    DidSaveTextDocumentParams, DocumentSelector, FoldingRangeProviderCapability,
    HoverProviderCapability, ImplementationProviderCapability, InitializeResult,
    LogMessageParams, MessageType, OneOf, ProgressParams, PublishDiagnosticsParams,
    Range, Registration, RegistrationParams, SemanticTokens, SemanticTokensLegend,
    SemanticTokensServerCapabilities, ServerCapabilities, ShowMessageParams,
    TextDocumentContentChangeEvent, TextDocumentIdentifier,
    TextDocumentSaveRegistrationOptions, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncSaveOptions,
    VersionedTextDocumentIdentifier,
    notification::{
        Cancel, DidChangeTextDocument, DidOpenTextDocument, DidSaveTextDocument,
        Initialized, LogMessage, Notification, Progress, PublishDiagnostics,
        ShowMessage,
    },
    request::{
        CallHierarchyIncomingCalls, CallHierarchyPrepare, CodeActionRequest,
        CodeActionResolveRequest, CodeLensRequest, CodeLensResolve, Completion,
        DocumentHighlightRequest, DocumentSymbolRequest, FoldingRangeRequest,
        Formatting, GotoDefinition, GotoImplementation, GotoTypeDefinition,
        HoverRequest, Initialize, InlayHintRequest, InlineCompletionRequest,
        PrepareRenameRequest, References, RegisterCapability, Rename, Request,
        ResolveCompletionItem, SelectionRangeRequest, SemanticTokensFullRequest,
        SignatureHelpRequest, WorkDoneProgressCreate, WorkspaceSymbolRequest,
    },
};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;

use super::{PluginCatalogRpcHandler, lsp::DocumentFilter};

pub enum ResponseHandler<Resp, Error> {
    Chan(Sender<Result<Resp, Error>>),
    Callback(Box<dyn RpcCallback<Resp, Error>>),
}

impl<Resp, Error> ResponseHandler<Resp, Error> {
    pub fn invoke(self, result: Result<Resp, Error>) {
        match self {
            ResponseHandler::Chan(tx) => {
                if let Err(err) = tx.send(result) {
                    tracing::error!("{:?}", err);
                }
            }
            ResponseHandler::Callback(f) => f.call(result),
        }
    }
}

pub trait CloneableCallback<Resp, Error>:
    FnOnce(PluginId, Result<Resp, Error>) + Send + DynClone
{
}

impl<Resp, Error, F: Send + FnOnce(PluginId, Result<Resp, Error>) + DynClone>
    CloneableCallback<Resp, Error> for F
{
}

pub trait RpcCallback<Resp, Error>: Send {
    fn call(self: Box<Self>, result: Result<Resp, Error>);
}

impl<Resp, Error, F: Send + FnOnce(Result<Resp, Error>)> RpcCallback<Resp, Error>
    for F
{
    fn call(self: Box<F>, result: Result<Resp, Error>) {
        (*self)(result)
    }
}

#[allow(clippy::large_enum_variant)]
pub enum PluginHandlerNotification {
    Initialize,
    InitializeResult(InitializeResult),
    Shutdown,
}

#[allow(clippy::large_enum_variant)]
pub enum PluginServerRpc {
    Shutdown,
    Handler(PluginHandlerNotification),
    ServerRequest {
        id: Id,
        method: Cow<'static, str>,
        params: Params,
        language_id: Option<String>,
        path: Option<PathBuf>,
        rh: ResponseHandler<Value, RpcError>,
    },
    ServerNotification {
        method: Cow<'static, str>,
        params: Params,
        language_id: Option<String>,
        path: Option<PathBuf>,
    },
    HostRequest {
        id: Id,
        method: String,
        params: Params,
        resp: ResponseSender,
    },
    HostNotification {
        method: String,
        params: Params,
        from: String,
    },
    DidSaveTextDocument {
        language_id: String,
        path: PathBuf,
        text_document: TextDocumentIdentifier,
        text: Rope,
    },
    DidChangeTextDocument {
        language_id: String,
        document: VersionedTextDocumentIdentifier,
        delta: RopeDelta,
        text: Rope,
        new_text: Rope,
        change: Arc<
            Mutex<(
                Option<TextDocumentContentChangeEvent>,
                Option<TextDocumentContentChangeEvent>,
            )>,
        >,
    },
    FormatSemanticTokens {
        tokens: SemanticTokens,
        text: Rope,
        f: Box<dyn RpcCallback<Vec<LineStyle>, RpcError>>,
    },
}

#[derive(Clone)]
pub struct PluginServerRpcHandler {
    pub plugin_id: PluginId,
    rpc_tx: Sender<PluginServerRpc>,
    rpc_rx: Receiver<PluginServerRpc>,
    io_tx: Sender<JsonRpc>,
    id: Arc<AtomicU64>,
    server_pending: Arc<Mutex<HashMap<Id, ResponseHandler<Value, RpcError>>>>,
}

#[derive(Clone)]
pub struct ResponseSender {
    tx: Sender<Result<Value, RpcError>>,
}
impl ResponseSender {
    pub fn new(tx: Sender<Result<Value, RpcError>>) -> Self {
        Self { tx }
    }

    pub fn send(&self, result: impl Serialize) {
        let result = serde_json::to_value(result).map_err(|e| RpcError {
            code: 0,
            message: e.to_string(),
        });
        if let Err(err) = self.tx.send(result) {
            tracing::error!("{:?}", err);
        }
    }

    pub fn send_null(&self) {
        if let Err(err) = self.tx.send(Ok(Value::Null)) {
            tracing::error!("{:?}", err);
        }
    }

    pub fn send_err(&self, code: i64, message: impl Into<String>) {
        if let Err(err) = self.tx.send(Err(RpcError {
            code,
            message: message.into(),
        })) {
            tracing::error!("{:?}", err);
        }
    }
}

pub trait PluginServerHandler {
    fn document_supported(
        &mut self,
        language_id: Option<&str>,
        path: Option<&Path>,
    ) -> bool;
    fn method_registered(&mut self, method: &str) -> bool;
    fn handle_host_notification(
        &mut self,
        method: String,
        params: Params,
        from: String,
    );
    fn handle_host_request(
        &mut self,
        id: Id,
        method: String,
        params: Params,
        chan: ResponseSender,
    );
    fn handle_handler_notification(
        &mut self,
        notification: PluginHandlerNotification,
    );
    fn handle_did_save_text_document(
        &self,
        language_id: String,
        path: PathBuf,
        text_document: TextDocumentIdentifier,
        text: Rope,
    );
    fn handle_did_change_text_document(
        &mut self,
        language_id: String,
        document: VersionedTextDocumentIdentifier,
        delta: RopeDelta,
        text: Rope,
        new_text: Rope,
        change: Arc<
            Mutex<(
                Option<TextDocumentContentChangeEvent>,
                Option<TextDocumentContentChangeEvent>,
            )>,
        >,
    );
    fn format_semantic_tokens(
        &self,
        tokens: SemanticTokens,
        text: Rope,
        f: Box<dyn RpcCallback<Vec<LineStyle>, RpcError>>,
    );
}

impl PluginServerRpcHandler {
    pub fn new(io_tx: Sender<JsonRpc>) -> Self {
        let (rpc_tx, rpc_rx) = crossbeam_channel::unbounded();

        let rpc = Self {
            plugin_id: PluginId::next(),
            rpc_tx,
            rpc_rx,
            io_tx,
            id: Arc::new(AtomicU64::new(0)),
            server_pending: Arc::new(Mutex::new(HashMap::new())),
        };

        rpc.initialize();
        rpc
    }

    fn initialize(&self) {
        self.handle_rpc(PluginServerRpc::Handler(
            PluginHandlerNotification::Initialize,
        ));
    }

    fn send_server_request(
        &self,
        id: Id,
        method: &str,
        params: Params,
        rh: ResponseHandler<Value, RpcError>,
    ) {
        {
            let mut pending = self.server_pending.lock();
            pending.insert(id.clone(), rh);
        }
        let msg = JsonRpc::request_with_params(id, method, params);
        self.send_server_rpc(msg);
    }

    fn send_server_notification(&self, method: &str, params: Params) {
        let msg = JsonRpc::notification_with_params(method, params);
        self.send_server_rpc(msg);
    }

    fn send_server_rpc(&self, msg: JsonRpc) {
        if let Err(err) = self.io_tx.send(msg) {
            tracing::error!("{:?}", err);
        }
    }

    pub fn handle_rpc(&self, rpc: PluginServerRpc) {
        if let Err(err) = self.rpc_tx.send(rpc) {
            tracing::error!("{:?}", err);
        }
    }

    /// Send a notification.
    /// The callback is called when the function is actually sent.
    pub fn server_notification<P: Serialize>(
        &self,
        method: impl Into<Cow<'static, str>>,
        params: P,
        language_id: Option<String>,
        path: Option<PathBuf>,
        check: bool,
    ) {
        let params = Params::from(serde_json::to_value(params).unwrap());
        let method = method.into();

        if check {
            if let Err(err) = self.rpc_tx.send(PluginServerRpc::ServerNotification {
                method,
                params,
                language_id,
                path,
            }) {
                tracing::error!("{:?}", err);
            }
        } else {
            self.send_server_notification(&method, params);
        }
    }

    /// Make a request to plugin/language server and get the response.
    ///
    /// When check is true, the request will be in the handler mainloop to
    /// do checks like if the server has the capability of the request.
    ///
    /// When check is false, the request will be sent out straight away.
    pub fn server_request<P: Serialize>(
        &self,
        method: impl Into<Cow<'static, str>>,
        params: P,
        language_id: Option<String>,
        path: Option<PathBuf>,
        check: bool,
    ) -> Result<Value, RpcError> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.server_request_common(
            method.into(),
            params,
            language_id,
            path,
            check,
            ResponseHandler::Chan(tx),
        );
        rx.recv().unwrap_or_else(|_| {
            Err(RpcError {
                code: 0,
                message: "io error".to_string(),
            })
        })
    }

    pub fn server_request_async<P: Serialize>(
        &self,
        method: impl Into<Cow<'static, str>>,
        params: P,
        language_id: Option<String>,
        path: Option<PathBuf>,
        check: bool,
        f: impl RpcCallback<Value, RpcError> + 'static,
    ) {
        self.server_request_common(
            method.into(),
            params,
            language_id,
            path,
            check,
            ResponseHandler::Callback(Box::new(f)),
        );
    }

    fn server_request_common<P: Serialize>(
        &self,
        method: Cow<'static, str>,
        params: P,
        language_id: Option<String>,
        path: Option<PathBuf>,
        check: bool,
        rh: ResponseHandler<Value, RpcError>,
    ) {
        let id = self.id.fetch_add(1, Ordering::Relaxed);
        let params = Params::from(serde_json::to_value(params).unwrap());
        if check {
            if let Err(err) = self.rpc_tx.send(PluginServerRpc::ServerRequest {
                id: Id::Num(id as i64),
                method,
                params,
                language_id,
                path,
                rh,
            }) {
                tracing::error!("{:?}", err);
            }
        } else {
            self.send_server_request(Id::Num(id as i64), &method, params, rh);
        }
    }

    pub fn handle_server_response(&self, id: Id, result: Result<Value, RpcError>) {
        if let Some(handler) = { self.server_pending.lock().remove(&id) } {
            handler.invoke(result);
        }
    }

    pub fn shutdown(&self) {
        // to kill lsp
        self.handle_rpc(PluginServerRpc::Handler(
            PluginHandlerNotification::Shutdown,
        ));
        // to end PluginServerRpcHandler::mainloop
        self.handle_rpc(PluginServerRpc::Shutdown);
    }

    pub fn mainloop<H>(&self, handler: &mut H)
    where
        H: PluginServerHandler,
    {
        for msg in &self.rpc_rx {
            match msg {
                PluginServerRpc::ServerRequest {
                    id,
                    method,
                    params,
                    language_id,
                    path,
                    rh,
                } => {
                    if handler
                        .document_supported(language_id.as_deref(), path.as_deref())
                        && handler.method_registered(&method)
                    {
                        self.send_server_request(id, &method, params, rh);
                    } else {
                        rh.invoke(Err(RpcError {
                            code: 0,
                            message: "server not capable".to_string(),
                        }));
                    }
                }
                PluginServerRpc::ServerNotification {
                    method,
                    params,
                    language_id,
                    path,
                } => {
                    if handler
                        .document_supported(language_id.as_deref(), path.as_deref())
                        && handler.method_registered(&method)
                    {
                        self.send_server_notification(&method, params);
                    }
                }
                PluginServerRpc::HostRequest {
                    id,
                    method,
                    params,
                    resp,
                } => {
                    handler.handle_host_request(id, method, params, resp);
                }
                PluginServerRpc::HostNotification {
                    method,
                    params,
                    from,
                } => {
                    handler.handle_host_notification(method, params, from);
                }
                PluginServerRpc::DidSaveTextDocument {
                    language_id,
                    path,
                    text_document,
                    text,
                } => {
                    handler.handle_did_save_text_document(
                        language_id,
                        path,
                        text_document,
                        text,
                    );
                }
                PluginServerRpc::DidChangeTextDocument {
                    language_id,
                    document,
                    delta,
                    text,
                    new_text,
                    change,
                } => {
                    handler.handle_did_change_text_document(
                        language_id,
                        document,
                        delta,
                        text,
                        new_text,
                        change,
                    );
                }
                PluginServerRpc::FormatSemanticTokens { tokens, text, f } => {
                    handler.format_semantic_tokens(tokens, text, f);
                }
                PluginServerRpc::Handler(notification) => {
                    handler.handle_handler_notification(notification)
                }
                PluginServerRpc::Shutdown => {
                    return;
                }
            }
        }
    }
}

pub fn handle_plugin_server_message(
    server_rpc: &PluginServerRpcHandler,
    message: &str,
    from: &str,
) -> Option<JsonRpc> {
    match JsonRpc::parse(message) {
        Ok(value @ JsonRpc::Request(_)) => {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let id = value.get_id().unwrap();
            let rpc = PluginServerRpc::HostRequest {
                id: id.clone(),
                method: value.get_method().unwrap().to_string(),
                params: value.get_params().unwrap_or(Params::None(())),
                resp: ResponseSender::new(tx),
            };
            server_rpc.handle_rpc(rpc);
            let result = rx.recv().unwrap();
            let resp = match result {
                Ok(v) => JsonRpc::success(id, &v),
                Err(e) => JsonRpc::error(
                    id,
                    jsonrpc_lite::Error {
                        code: e.code,
                        message: e.message,
                        data: None,
                    },
                ),
            };
            Some(resp)
        }
        Ok(value @ JsonRpc::Notification(_)) => {
            let rpc = PluginServerRpc::HostNotification {
                method: value.get_method().unwrap().to_string(),
                params: value.get_params().unwrap_or(Params::None(())),
                from: from.to_string(),
            };
            server_rpc.handle_rpc(rpc);
            None
        }
        Ok(value @ JsonRpc::Success(_)) => {
            let result = value.get_result().unwrap().clone();
            server_rpc.handle_server_response(value.get_id().unwrap(), Ok(result));
            None
        }
        Ok(value @ JsonRpc::Error(_)) => {
            let error = value.get_error().unwrap();
            server_rpc.handle_server_response(
                value.get_id().unwrap(),
                Err(RpcError {
                    code: error.code,
                    message: error.message.clone(),
                }),
            );
            None
        }
        Err(err) => {
            eprintln!("parse error {err} message {message}");
            None
        }
    }
}

struct SaveRegistration {
    include_text: bool,
    filters: Vec<DocumentFilter>,
}

#[derive(Default)]
struct ServerRegistrations {
    save: Option<SaveRegistration>,
}

pub struct PluginHostHandler {
    server_name: String,
    document_selector: Vec<DocumentFilter>,
    core_rpc: CoreRpcHandler,
    catalog_rpc: PluginCatalogRpcHandler,
    pub server_rpc: PluginServerRpcHandler,
    pub server_capabilities: ServerCapabilities,
    server_registrations: ServerRegistrations,
}

impl PluginHostHandler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        server_name: String,
        document_selector: DocumentSelector,
        core_rpc: CoreRpcHandler,
        server_rpc: PluginServerRpcHandler,
        catalog_rpc: PluginCatalogRpcHandler,
    ) -> Self {
        let document_selector = document_selector
            .iter()
            .map(DocumentFilter::from_lsp_filter_loose)
            .collect();
        Self {
            server_name,
            document_selector,
            core_rpc,
            catalog_rpc,
            server_rpc,
            server_capabilities: ServerCapabilities::default(),
            server_registrations: ServerRegistrations::default(),
        }
    }

    pub fn document_supported(
        &self,
        language_id: Option<&str>,
        path: Option<&Path>,
    ) -> bool {
        match language_id {
            Some(language_id) => {
                for filter in self.document_selector.iter() {
                    if (filter.language_id.is_none()
                        || filter.language_id.as_deref() == Some(language_id))
                        && (path.is_none()
                            || filter.pattern.is_none()
                            || filter
                                .pattern
                                .as_ref()
                                .unwrap()
                                .is_match(path.as_ref().unwrap()))
                    {
                        return true;
                    }
                }
                false
            }
            None => true,
        }
    }

    pub fn method_registered(&mut self, method: &str) -> bool {
        match method {
            Initialize::METHOD => true,
            Initialized::METHOD => true,
            Completion::METHOD => {
                self.server_capabilities.completion_provider.is_some()
            }
            ResolveCompletionItem::METHOD => self
                .server_capabilities
                .completion_provider
                .as_ref()
                .and_then(|c| c.resolve_provider)
                .unwrap_or(false),
            DidOpenTextDocument::METHOD => {
                match &self.server_capabilities.text_document_sync {
                    Some(TextDocumentSyncCapability::Kind(kind)) => {
                        kind != &TextDocumentSyncKind::NONE
                    }
                    Some(TextDocumentSyncCapability::Options(options)) => options
                        .open_close
                        .or_else(|| {
                            options
                                .change
                                .map(|kind| kind != TextDocumentSyncKind::NONE)
                        })
                        .unwrap_or(false),
                    None => false,
                }
            }
            DidChangeTextDocument::METHOD => {
                match &self.server_capabilities.text_document_sync {
                    Some(TextDocumentSyncCapability::Kind(kind)) => {
                        kind != &TextDocumentSyncKind::NONE
                    }
                    Some(TextDocumentSyncCapability::Options(options)) => options
                        .change
                        .map(|kind| kind != TextDocumentSyncKind::NONE)
                        .unwrap_or(false),
                    None => false,
                }
            }
            SignatureHelpRequest::METHOD => {
                self.server_capabilities.signature_help_provider.is_some()
            }
            HoverRequest::METHOD => self
                .server_capabilities
                .hover_provider
                .as_ref()
                .map(|c| match c {
                    HoverProviderCapability::Simple(is_capable) => *is_capable,
                    HoverProviderCapability::Options(_) => true,
                })
                .unwrap_or(false),
            GotoDefinition::METHOD => self
                .server_capabilities
                .definition_provider
                .as_ref()
                .map(|d| match d {
                    OneOf::Left(is_capable) => *is_capable,
                    OneOf::Right(_) => true,
                })
                .unwrap_or(false),
            GotoTypeDefinition::METHOD => {
                self.server_capabilities.type_definition_provider.is_some()
            }
            References::METHOD => self
                .server_capabilities
                .references_provider
                .as_ref()
                .map(|r| match r {
                    OneOf::Left(is_capable) => *is_capable,
                    OneOf::Right(_) => true,
                })
                .unwrap_or(false),
            GotoImplementation::METHOD => self
                .server_capabilities
                .implementation_provider
                .as_ref()
                .map(|r| match r {
                    ImplementationProviderCapability::Simple(is_capable) => {
                        *is_capable
                    }
                    ImplementationProviderCapability::Options(_) => {
                        // todo
                        false
                    }
                })
                .unwrap_or(false),
            FoldingRangeRequest::METHOD => self
                .server_capabilities
                .folding_range_provider
                .as_ref()
                .map(|r| match r {
                    FoldingRangeProviderCapability::Simple(support) => *support,
                    FoldingRangeProviderCapability::FoldingProvider(_) => {
                        // todo
                        true
                    }
                    FoldingRangeProviderCapability::Options(_) => {
                        // todo
                        true
                    }
                })
                .unwrap_or(false),
            CodeActionRequest::METHOD => self
                .server_capabilities
                .code_action_provider
                .as_ref()
                .map(|a| match a {
                    CodeActionProviderCapability::Simple(is_capable) => *is_capable,
                    CodeActionProviderCapability::Options(_) => true,
                })
                .unwrap_or(false),
            Formatting::METHOD => self
                .server_capabilities
                .document_formatting_provider
                .as_ref()
                .map(|f| match f {
                    OneOf::Left(is_capable) => *is_capable,
                    OneOf::Right(_) => true,
                })
                .unwrap_or(false),
            SemanticTokensFullRequest::METHOD => {
                self.server_capabilities.semantic_tokens_provider.is_some()
            }
            InlayHintRequest::METHOD => {
                self.server_capabilities.inlay_hint_provider.is_some()
            }
            InlineCompletionRequest::METHOD => self
                .server_capabilities
                .inline_completion_provider
                .is_some(),
            DocumentSymbolRequest::METHOD => {
                self.server_capabilities.document_symbol_provider.is_some()
            }
            WorkspaceSymbolRequest::METHOD => {
                self.server_capabilities.workspace_symbol_provider.is_some()
            }
            PrepareRenameRequest::METHOD => {
                self.server_capabilities.rename_provider.is_some()
            }
            Rename::METHOD => self.server_capabilities.rename_provider.is_some(),
            SelectionRangeRequest::METHOD => {
                self.server_capabilities.selection_range_provider.is_some()
            }
            CodeActionResolveRequest::METHOD => {
                self.server_capabilities.code_action_provider.is_some()
            }
            CodeLensRequest::METHOD => {
                self.server_capabilities.code_lens_provider.is_some()
            }
            CodeLensResolve::METHOD => self
                .server_capabilities
                .code_lens_provider
                .as_ref()
                .and_then(|x| x.resolve_provider)
                .unwrap_or(false),
            CallHierarchyPrepare::METHOD => {
                self.server_capabilities.call_hierarchy_provider.is_some()
            }
            CallHierarchyIncomingCalls::METHOD => {
                self.server_capabilities.call_hierarchy_provider.is_some()
            }
            DocumentHighlightRequest::METHOD => self
                .server_capabilities
                .document_highlight_provider
                .is_some(),
            _ => false,
        }
    }

    fn check_save_capability(&self, language_id: &str, path: &Path) -> (bool, bool) {
        if self.document_supported(Some(language_id), Some(path)) {
            let (should_send, include_text) = self
                .server_capabilities
                .text_document_sync
                .as_ref()
                .and_then(|sync| match sync {
                    TextDocumentSyncCapability::Kind(_) => None,
                    TextDocumentSyncCapability::Options(options) => Some(options),
                })
                .and_then(|o| o.save.as_ref())
                .map(|o| match o {
                    TextDocumentSyncSaveOptions::Supported(is_supported) => {
                        (*is_supported, true)
                    }
                    TextDocumentSyncSaveOptions::SaveOptions(options) => {
                        (true, options.include_text.unwrap_or(false))
                    }
                })
                .unwrap_or((false, false));
            if should_send {
                return (true, include_text);
            }
        }

        if let Some(options) = self.server_registrations.save.as_ref() {
            if options.filters.is_empty()
                && self.document_supported(Some(language_id), Some(path))
            {
                return (true, options.include_text);
            }
            for filter in options.filters.iter() {
                if (filter.language_id.is_none()
                    || filter.language_id.as_deref() == Some(language_id))
                    && (filter.pattern.is_none()
                        || filter.pattern.as_ref().unwrap().is_match(path))
                {
                    return (true, options.include_text);
                }
            }
        }

        (false, false)
    }

    fn register_capabilities(&mut self, registrations: Vec<Registration>) {
        for registration in registrations {
            if let Err(err) = self.register_capability(registration) {
                tracing::error!("{:?}", err);
            }
        }
    }

    fn register_capability(&mut self, registration: Registration) -> Result<()> {
        match registration.method.as_str() {
            DidSaveTextDocument::METHOD => {
                let options = registration
                    .register_options
                    .ok_or_else(|| anyhow!("don't have options"))?;
                let options: TextDocumentSaveRegistrationOptions =
                    serde_json::from_value(options)?;
                self.server_registrations.save = Some(SaveRegistration {
                    include_text: options.include_text.unwrap_or(false),
                    filters: options
                        .text_document_registration_options
                        .document_selector
                        .map(|s| {
                            s.iter()
                                .map(DocumentFilter::from_lsp_filter_loose)
                                .collect()
                        })
                        .unwrap_or_default(),
                });
            }
            _ => {
                eprintln!(
                    "don't handle register capability for {}",
                    registration.method
                );
            }
        }
        Ok(())
    }

    pub fn handle_request(
        &mut self,
        _id: Id,
        method: String,
        params: Params,
        resp: ResponseSender,
    ) {
        if let Err(err) = self.process_request(method, params, resp.clone()) {
            resp.send_err(0, err.to_string());
        }
    }

    pub fn process_request(
        &mut self,
        method: String,
        params: Params,
        resp: ResponseSender,
    ) -> Result<()> {
        match method.as_str() {
            WorkDoneProgressCreate::METHOD => {
                resp.send_null();
            }
            RegisterCapability::METHOD => {
                let params: RegistrationParams =
                    serde_json::from_value(serde_json::to_value(params)?)?;
                self.register_capabilities(params.registrations);
                resp.send_null();
            }
            _ => resp.send_err(-32601, format!("Unsupported LSP request: {method}")),
        }

        Ok(())
    }

    pub fn handle_notification(
        &mut self,
        method: String,
        params: Params,
        from: String,
    ) -> Result<()> {
        match method.as_str() {
            PublishDiagnostics::METHOD => {
                let diagnostics: PublishDiagnosticsParams =
                    serde_json::from_value(serde_json::to_value(params)?)?;
                self.catalog_rpc.core_rpc.publish_diagnostics(diagnostics);
            }
            Progress::METHOD => {
                let progress: ProgressParams =
                    serde_json::from_value(serde_json::to_value(params)?)?;
                self.catalog_rpc.core_rpc.work_done_progress(progress);
            }
            ShowMessage::METHOD => {
                let message: ShowMessageParams =
                    serde_json::from_value(serde_json::to_value(params)?)?;
                let title = format!("Language server: {}", self.server_name);
                self.catalog_rpc.core_rpc.show_message(title, message);
            }
            LogMessage::METHOD => {
                let message: LogMessageParams =
                    serde_json::from_value(serde_json::to_value(params)?)?;
                self.catalog_rpc.core_rpc.log_message(
                    message,
                    format!(
                        "lapce_proxy::plugin::rpc::{}::LogMessage",
                        self.server_name
                    ),
                );
            }
            Cancel::METHOD => {
                let params: CancelParams =
                    serde_json::from_value(serde_json::to_value(params)?)?;
                self.catalog_rpc.core_rpc.cancel(params);
            }
            "experimental/serverStatus" => {
                let param: ServerStatusParams =
                    serde_json::from_value(serde_json::to_value(params)?)?;
                if !param.is_ok() {
                    if let Some(msg) = &param.message {
                        self.core_rpc.show_message(
                            from,
                            ShowMessageParams {
                                typ: MessageType::ERROR,
                                message: msg.clone(),
                            },
                        );
                    }
                }
                if param.is_ok() {
                    self.catalog_rpc.proxy_rpc.notification(lapce_rpc::proxy::ProxyNotification::RefreshSemanticHighlights {});
                }
                self.catalog_rpc.core_rpc.server_status(param);
            }
            _ => {
                self.core_rpc.log(
                    lapce_rpc::core::LogLevel::Warn,
                    format!("host notification {method} not handled"),
                    Some(format!(
                        "lapce_proxy::plugin::rpc::{}::{method}",
                        self.server_name
                    )),
                );
            }
        }
        Ok(())
    }

    pub fn handle_did_save_text_document(
        &self,
        language_id: String,
        path: PathBuf,
        text_document: TextDocumentIdentifier,
        text: Rope,
    ) {
        let (should_send, include_text) =
            self.check_save_capability(language_id.as_str(), &path);
        if !should_send {
            return;
        }
        let params = DidSaveTextDocumentParams {
            text_document,
            text: if include_text {
                Some(text.to_string())
            } else {
                None
            },
        };
        self.server_rpc.server_notification(
            DidSaveTextDocument::METHOD,
            params,
            Some(language_id),
            Some(path),
            false,
        );
    }

    pub fn handle_did_change_text_document(
        &mut self,
        language_id: String,
        document: VersionedTextDocumentIdentifier,
        delta: RopeDelta,
        text: Rope,
        new_text: Rope,
        change: Arc<
            Mutex<(
                Option<TextDocumentContentChangeEvent>,
                Option<TextDocumentContentChangeEvent>,
            )>,
        >,
    ) {
        let kind = match &self.server_capabilities.text_document_sync {
            Some(TextDocumentSyncCapability::Kind(kind)) => *kind,
            Some(TextDocumentSyncCapability::Options(options)) => {
                options.change.unwrap_or(TextDocumentSyncKind::NONE)
            }
            None => TextDocumentSyncKind::NONE,
        };

        let mut existing = change.lock();
        let change = match kind {
            TextDocumentSyncKind::FULL => {
                if let Some(c) = existing.0.as_ref() {
                    c.clone()
                } else {
                    let change = TextDocumentContentChangeEvent {
                        range: None,
                        range_length: None,
                        text: new_text.to_string(),
                    };
                    existing.0 = Some(change.clone());
                    change
                }
            }
            TextDocumentSyncKind::INCREMENTAL => {
                if let Some(c) = existing.1.as_ref() {
                    c.clone()
                } else {
                    let change = get_document_content_change(&text, &delta)
                        .unwrap_or_else(|| TextDocumentContentChangeEvent {
                            range: None,
                            range_length: None,
                            text: new_text.to_string(),
                        });
                    existing.1 = Some(change.clone());
                    change
                }
            }
            TextDocumentSyncKind::NONE => return,
            _ => return,
        };

        let path = document.uri.to_file_path().ok();

        let params = DidChangeTextDocumentParams {
            text_document: document,
            content_changes: vec![change],
        };

        self.server_rpc.server_notification(
            DidChangeTextDocument::METHOD,
            params,
            Some(language_id),
            path,
            false,
        );
    }

    pub fn format_semantic_tokens(
        &self,
        tokens: SemanticTokens,
        text: Rope,
        f: Box<dyn RpcCallback<Vec<LineStyle>, RpcError>>,
    ) {
        let result = format_semantic_styles(
            &text,
            self.server_capabilities.semantic_tokens_provider.as_ref(),
            &tokens,
        )
        .ok_or_else(|| RpcError {
            code: 0,
            message: "can't get styles".to_string(),
        });
        f.call(result);
    }
}

fn get_document_content_change(
    text: &Rope,
    delta: &RopeDelta,
) -> Option<TextDocumentContentChangeEvent> {
    let (interval, _) = delta.summary();
    let (start, end) = interval.start_end();

    let text = RopeTextRef::new(text);

    // TODO: Handle more trivial cases like typing when there's a selection or transpose
    if let Some(node) = delta.as_simple_insert() {
        let (start, end) = interval.start_end();
        let start = text.offset_to_position(start);

        let end = text.offset_to_position(end);

        let text = String::from(node);
        let text_document_content_change_event = TextDocumentContentChangeEvent {
            range: Some(Range { start, end }),
            range_length: None,
            text,
        };

        return Some(text_document_content_change_event);
    }
    // Or a simple delete
    else if delta.is_simple_delete() {
        let end_position = text.offset_to_position(end);

        let start = text.offset_to_position(start);

        let text_document_content_change_event = TextDocumentContentChangeEvent {
            range: Some(Range {
                start,
                end: end_position,
            }),
            range_length: None,
            text: String::new(),
        };

        return Some(text_document_content_change_event);
    }

    None
}

fn format_semantic_styles(
    text: &Rope,
    semantic_tokens_provider: Option<&SemanticTokensServerCapabilities>,
    tokens: &SemanticTokens,
) -> Option<Vec<LineStyle>> {
    let semantic_tokens_provider = semantic_tokens_provider?;
    let semantic_legends = semantic_tokens_legend(semantic_tokens_provider);

    let text = RopeTextRef::new(text);
    let mut highlights = Vec::new();
    let mut line = 0u32;
    let mut column = 0u32;
    let mut previous_end = 0;
    for token in &tokens.data {
        line = line.checked_add(token.delta_line)?;
        column = if token.delta_line > 0 {
            token.delta_start
        } else {
            column.checked_add(token.delta_start)?
        };
        let end_column = column.checked_add(token.length)?;
        if line as usize >= text.num_lines() {
            continue;
        }
        let start_position = lsp_types::Position::new(line, column);
        let end_position = lsp_types::Position::new(line, end_column);
        let start = text.offset_of_position(&start_position);
        let end = text.offset_of_position(&end_position);
        let Some(kind) = semantic_legends.token_types.get(token.token_type as usize)
        else {
            continue;
        };
        if start < previous_end
            || start >= end
            || text.offset_to_position(start) != start_position
            || text.offset_to_position(end) != end_position
        {
            continue;
        }
        previous_end = end;
        highlights.push(LineStyle {
            start,
            end,
            style: Style {
                fg_color: Some(kind.as_str().to_string()),
            },
        });
    }

    Some(highlights)
}

fn semantic_tokens_legend(
    semantic_tokens_provider: &SemanticTokensServerCapabilities,
) -> &SemanticTokensLegend {
    match semantic_tokens_provider {
        SemanticTokensServerCapabilities::SemanticTokensOptions(options) => {
            &options.legend
        }
        SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(
            options,
        ) => &options.semantic_tokens_options.legend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lapce_rpc::proxy::ProxyRpcHandler;

    #[test]
    fn semantic_decoder_rejects_invalid_utf16_ranges_and_unknown_token_types() {
        let provider = SemanticTokensServerCapabilities::SemanticTokensOptions(
            lsp_types::SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: vec![lsp_types::SemanticTokenType::FUNCTION],
                    token_modifiers: vec![],
                },
                ..Default::default()
            },
        );
        let token =
            |delta_line, delta_start, length, token_type| lsp_types::SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type,
                token_modifiers_bitset: 0,
            };
        let tokens = SemanticTokens {
            result_id: None,
            data: vec![
                token(0, 1, 1, 0),  // split the emoji's surrogate pair
                token(0, 2, 1, 0),  // x after é and the emoji
                token(1, 0, 3, 99), // invalid legend index
                token(0, 0, 3, 0),  // abc
                token(99, 0, 1, 0), // nonexistent line
            ],
        };
        let styles = format_semantic_styles(
            &Rope::from("é🌍x\r\nabc"),
            Some(&provider),
            &tokens,
        )
        .unwrap();
        assert_eq!(
            styles
                .iter()
                .map(|style| (style.start, style.end))
                .collect::<Vec<_>>(),
            vec![(6, 7), (9, 12)]
        );
    }

    #[test]
    fn language_server_host_rejects_removed_extension_requests() {
        let core = CoreRpcHandler::new();
        let catalog =
            PluginCatalogRpcHandler::new(core.clone(), ProxyRpcHandler::new());
        let (io, _messages) = crossbeam_channel::unbounded();
        let mut host = PluginHostHandler::new(
            "Test language server".into(),
            vec![],
            core,
            PluginServerRpcHandler::new(io),
            catalog,
        );
        for method in [
            "host/executeProcess",
            "host/startLspServer",
            "host/sendLspRequest",
            "host/sendLspNotification",
            "host/registerDebuggerType",
        ] {
            let (tx, rx) = crossbeam_channel::bounded(1);
            host.handle_request(
                Id::Num(1),
                method.into(),
                Params::None(()),
                ResponseSender::new(tx),
            );
            assert_eq!(rx.recv().unwrap().unwrap_err().code, -32601, "{method}");
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        host.handle_request(
            Id::Num(2),
            WorkDoneProgressCreate::METHOD.into(),
            Params::None(()),
            ResponseSender::new(tx),
        );
        assert_eq!(rx.recv().unwrap().unwrap(), Value::Null);
    }
}
