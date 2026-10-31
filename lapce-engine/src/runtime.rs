//! Headless workspace ownership and blocking backend execution.
//! Async hosts can continue driving the same document synchronization effects.
use crate::{
    document::{DocContent, Document},
    session::{SessionDocument, WorkspaceDocuments},
    sync::SyncOutcome,
};
use lapce_rpc::{
    RpcError,
    proxy::{ProxyRequest, ProxyResponse, ProxyRpcHandler},
};
use lapce_xi_rope::Rope;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub trait RequestExecutor {
    fn request(&self, request: ProxyRequest) -> Result<ProxyResponse, RpcError>;
    fn notification(
        &self,
        _: lapce_rpc::proxy::ProxyNotification,
    ) -> Result<(), RpcError> {
        Err(RpcError {
            code: -32601,
            message: "Notifications are not supported by this executor".into(),
        })
    }
}
pub struct RpcExecutor {
    pub rpc: ProxyRpcHandler,
    pub timeout: Duration,
}
impl RequestExecutor for RpcExecutor {
    fn notification(
        &self,
        notification: lapce_rpc::proxy::ProxyNotification,
    ) -> Result<(), RpcError> {
        self.rpc.notification(notification);
        Ok(())
    }
    fn request(&self, request: ProxyRequest) -> Result<ProxyResponse, RpcError> {
        self.rpc.request_timeout(request, self.timeout)
    }
}

/// Identifies one backend event stream; tokens are unique across sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendGeneration(u64);
impl BackendGeneration {
    fn next() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

pub struct WorkspaceSession<E> {
    backend_generation: BackendGeneration,
    executor: E,
    documents: WorkspaceDocuments<SessionDocument>,
    pub actions: crate::workspace::WorkspaceActions,
    pub status: Option<lapce_rpc::proxy::ProxyStatus>,
    pub diagnostics:
        std::collections::HashMap<PathBuf, crate::diagnostics::DiagnosticState>,
    pub terminals: std::collections::HashMap<
        lapce_rpc::terminal::TermId,
        crate::terminal::TerminalState,
    >,
}

pub enum SessionEvent {
    StaleBackend,
    HighlightsChanged {
        path: PathBuf,
        accepted: bool,
    },
    DocumentChanged {
        path: PathBuf,
        reloaded: bool,
    },
    DocumentDeleted {
        path: PathBuf,
    },
    RepositoryChanged,
    StatusChanged,
    DiagnosticsChanged {
        path: PathBuf,
        accepted: bool,
    },
    TerminalChanged {
        term_id: lapce_rpc::terminal::TermId,
        accepted: bool,
    },
    TerminalOutput {
        term_id: lapce_rpc::terminal::TermId,
        content: Vec<u8>,
    },
    /// The host must route terminal, language-service and other events.
    Unhandled(lapce_rpc::core::CoreNotification),
}
impl<E: RequestExecutor> WorkspaceSession<E> {
    pub fn new(executor: E) -> Self {
        Self {
            backend_generation: BackendGeneration::next(),
            executor,
            documents: WorkspaceDocuments::default(),
            actions: Default::default(),
            status: None,
            diagnostics: Default::default(),
            terminals: Default::default(),
        }
    }
    pub fn list_directory(
        &self,
        path: PathBuf,
    ) -> Result<Vec<lapce_rpc::file::DirectoryEntry>, String> {
        let query = crate::directory::DirectoryQuery::new(path);
        query.finish(self.executor.request(query.request()))
    }
    pub fn document(&self, path: &Path) -> Option<&SessionDocument> {
        self.documents.files.get(path)
    }
    pub fn navigate(
        &mut self,
        path: &Path,
        query: crate::navigation::NavigationQuery,
    ) -> Result<ProxyResponse, String> {
        self.flush(path)?;
        let document = self
            .documents
            .files
            .get(path)
            .ok_or("Document is not open")?;
        if !document.backend_ready() {
            return Err("Document is not connected".into());
        }
        let plan =
            crate::navigation::SessionNavigationPlan::capture(document, query)
                .map_err(|error| error.message)?;
        let response = self.executor.request(plan.request());
        plan.finish(document, response)
            .map_err(|error| error.message)
    }

    pub fn workspace_symbols(
        &self,
        query: String,
    ) -> Result<Vec<lsp_types::SymbolInformation>, String> {
        let plan = crate::workspace::WorkspaceSymbolsQuery::new(query);
        let response = self.executor.request(plan.request());
        plan.finish(response).map_err(|error| error.message)
    }

    pub fn search_workspace(
        &mut self,
        query: crate::global_search::WorkspaceSearchQuery,
    ) -> Result<(), String> {
        let Some(effect) = self.actions.search(query) else {
            return Ok(());
        };
        let response = self.executor.request(effect.request.clone());
        match self.actions.complete(effect, response) {
            crate::workspace::WorkspaceOutcome::SearchFinished(result) => result,
            _ => Err("Search ownership changed".into()),
        }
    }
    pub fn register_terminal(
        &mut self,
        term_id: lapce_rpc::terminal::TermId,
    ) -> bool {
        if self.terminals.contains_key(&term_id) {
            return false;
        }
        self.terminals.insert(term_id, Default::default());
        true
    }
    /// Launch completion and failure are reported through terminal events.
    pub fn launch_terminal(
        &mut self,
        profile: lapce_rpc::terminal::TerminalProfile,
    ) -> Result<lapce_rpc::terminal::TermId, String> {
        let term_id = lapce_rpc::terminal::TermId::next();
        self.register_terminal(term_id);
        if let Err(error) = self.executor.notification(
            lapce_rpc::proxy::ProxyNotification::NewTerminal { term_id, profile },
        ) {
            self.terminals.remove(&term_id);
            return Err(error.message);
        }
        Ok(term_id)
    }
    pub fn terminal_input(
        &self,
        term_id: lapce_rpc::terminal::TermId,
        content: String,
    ) -> Result<(), String> {
        let terminal = self
            .terminals
            .get(&term_id)
            .ok_or("Terminal is not owned by this session")?;
        if let Some(effect) = terminal.input(term_id, content)? {
            self.executor
                .notification(effect)
                .map_err(|error| error.message)?;
        }
        Ok(())
    }
    pub fn terminal_resize(
        &self,
        term_id: lapce_rpc::terminal::TermId,
        columns: usize,
        rows: usize,
    ) -> Result<(), String> {
        let terminal = self
            .terminals
            .get(&term_id)
            .ok_or("Terminal is not owned by this session")?;
        self.executor
            .notification(terminal.resize(term_id, columns, rows)?)
            .map_err(|error| error.message)
    }

    fn synchronize_edits(
        &mut self,
        path: &Path,
        deltas: &[crate::editing::DocumentDelta],
    ) -> Result<(), String> {
        let document = self
            .documents
            .files
            .get_mut(path)
            .ok_or("Document is not open")?;
        let accepted = document.synchronize_edits(deltas);
        if let Some(diagnostics) = self.diagnostics.get_mut(path) {
            for (_, delta, _) in deltas {
                diagnostics.apply_delta(delta);
            }
        }
        if accepted {
            Ok(())
        } else {
            Err("Document is not connected".into())
        }
    }

    pub fn backend_generation(&self) -> BackendGeneration {
        self.backend_generation
    }

    /// Hosts capture the token when attaching a stream, never when receiving an event.
    pub fn handle_backend_event(
        &mut self,
        generation: BackendGeneration,
        event: lapce_rpc::core::CoreNotification,
    ) -> Result<SessionEvent, String> {
        if generation != self.backend_generation {
            return Ok(SessionEvent::StaleBackend);
        }
        self.handle_event(event)
    }

    /// Apply an event already known to belong to the current backend.
    pub fn handle_event(
        &mut self,
        event: lapce_rpc::core::CoreNotification,
    ) -> Result<SessionEvent, String> {
        use lapce_rpc::core::{CoreNotification, FileChanged};
        match event {
            CoreNotification::PublishHighlights { highlights } => {
                let path = highlights.path.clone();
                let accepted =
                    self.documents.files.get_mut(&path).is_some_and(|document| {
                        document.accept_highlights(highlights)
                    });
                Ok(SessionEvent::HighlightsChanged { path, accepted })
            }
            CoreNotification::UpdateTerminal { term_id, content } => {
                if !self.terminals.contains_key(&term_id) {
                    return Ok(SessionEvent::Unhandled(
                        CoreNotification::UpdateTerminal { term_id, content },
                    ));
                }
                Ok(SessionEvent::TerminalOutput { term_id, content })
            }
            CoreNotification::TerminalProcessId {
                term_id,
                process_id,
            } => {
                let Some(terminal) = self.terminals.get_mut(&term_id) else {
                    return Ok(SessionEvent::Unhandled(
                        CoreNotification::TerminalProcessId {
                            term_id,
                            process_id,
                        },
                    ));
                };
                Ok(SessionEvent::TerminalChanged {
                    term_id,
                    accepted: terminal.started(process_id),
                })
            }
            CoreNotification::TerminalProcessStopped { term_id, exit_code } => {
                let Some(terminal) = self.terminals.get_mut(&term_id) else {
                    return Ok(SessionEvent::Unhandled(
                        CoreNotification::TerminalProcessStopped {
                            term_id,
                            exit_code,
                        },
                    ));
                };
                Ok(SessionEvent::TerminalChanged {
                    term_id,
                    accepted: terminal.stopped(exit_code),
                })
            }
            CoreNotification::TerminalLaunchFailed { term_id, error } => {
                let Some(terminal) = self.terminals.get_mut(&term_id) else {
                    return Ok(SessionEvent::Unhandled(
                        CoreNotification::TerminalLaunchFailed { term_id, error },
                    ));
                };
                Ok(SessionEvent::TerminalChanged {
                    term_id,
                    accepted: terminal.failed(error),
                })
            }
            CoreNotification::PublishDiagnostics { diagnostics } => {
                let path = lapce_rpc::file::file_url_path(&diagnostics.uri);
                let Some(path) = path else {
                    return Ok(SessionEvent::Unhandled(
                        CoreNotification::PublishDiagnostics { diagnostics },
                    ));
                };
                let publication = crate::diagnostics::DiagnosticPublication::prepare(
                    diagnostics.diagnostics,
                    diagnostics.version,
                    self.documents.files.get(&path),
                );
                let accepted = publication.is_some_and(|publication| {
                    publication
                        .apply(self.diagnostics.entry(path.clone()).or_default())
                });
                Ok(SessionEvent::DiagnosticsChanged { path, accepted })
            }
            CoreNotification::OpenFileChanged { path, content } => {
                let Some(document) = self.documents.files.get_mut(&path) else {
                    return Ok(SessionEvent::Unhandled(
                        CoreNotification::OpenFileChanged { path, content },
                    ));
                };
                // Late watcher events must not mutate a closing owner's text.
                if document
                    .synchronization
                    .as_ref()
                    .is_none_or(|sync| sync.is_closing())
                {
                    return Ok(SessionEvent::Unhandled(
                        CoreNotification::OpenFileChanged { path, content },
                    ));
                }
                match content {
                    FileChanged::Delete => {
                        document.mark_deleted();
                        Ok(SessionEvent::DocumentDeleted { path })
                    }
                    FileChanged::Change(content) => {
                        let delta =
                            document.reload_from_disk(Rope::from(content)).flatten();
                        let reloaded = delta.is_some();
                        if let Some(delta) = delta {
                            self.synchronize_edits(&path, &[delta])?;
                            self.flush(&path)?;
                            let _ = self.refresh_highlights(&path);
                        }
                        Ok(SessionEvent::DocumentChanged { path, reloaded })
                    }
                }
            }
            CoreNotification::DiffInfo { diff } => {
                self.source_control.apply_diff(&diff);
                Ok(SessionEvent::RepositoryChanged)
            }
            CoreNotification::ProxyStatus { status } => {
                if matches!(status, lapce_rpc::proxy::ProxyStatus::Disconnected) {
                    for (_, document) in self.documents.files.iter_mut() {
                        document.backend_disconnected();
                    }
                }
                self.status = Some(status);
                Ok(SessionEvent::StatusChanged)
            }
            event => Ok(SessionEvent::Unhandled(event)),
        }
    }
    /// Replace the transport executor and independently reattach cached files.
    pub fn reconnect(&mut self, executor: E) -> Vec<(PathBuf, Result<(), String>)> {
        self.executor = executor;
        self.backend_generation = BackendGeneration::next();
        self.status = Some(lapce_rpc::proxy::ProxyStatus::Connecting);
        let mut paths: Vec<_> = self.documents.files.keys().cloned().collect();
        paths.sort();
        for (_, document) in self.documents.files.iter_mut() {
            document.backend_disconnected();
        }
        let results: Vec<_> = paths
            .into_iter()
            .map(|path| {
                let document = self.documents.files.get_mut(&path).unwrap();
                let result = document.begin_reopen().and_then(|plan| {
                    let response = self.executor.request(plan.request());
                    plan.finish(document, response)
                });
                if result.is_ok() {
                    if let Some(diagnostics) = self.diagnostics.get_mut(&path) {
                        use lapce_core::buffer::rope_text::RopeText;
                        diagnostics
                            .bind(document.buffer.text(), document.buffer.rev());
                    }
                }
                (path, result)
            })
            .collect();
        for (path, result) in &results {
            if result.is_ok()
                && self
                    .document(path)
                    .and_then(|document| document.backend_snapshot.as_ref())
                    .is_some_and(|snapshot| !snapshot.instance.is_empty())
            {
                let _ = self.refresh_highlights(path);
            }
        }
        results
    }

    pub fn open(&mut self, path: PathBuf) -> Result<(), String> {
        if self.documents.files.contains_key(&path) {
            return Ok(());
        }
        let mut document = SessionDocument::new(Document::empty(DocContent::File {
            path: path.clone(),
            read_only: true,
        }));
        let opening = crate::open::SessionOpening::capture(&mut document)
            .ok_or("Document is not a file")?;
        let response = self.executor.request(opening.request());
        opening
            .apply(&mut document, response)
            .map_err(|error| error.message)?;
        if let Some(diagnostics) = self.diagnostics.get_mut(&path) {
            use lapce_core::buffer::rope_text::RopeText;
            diagnostics.bind(document.buffer.text(), document.buffer.rev());
        }
        self.documents.files.insert(path.clone(), document);
        if self
            .documents
            .files
            .get(&path)
            .and_then(|document| document.backend_snapshot.as_ref())
            .is_some_and(|snapshot| !snapshot.instance.is_empty())
        {
            let _ = self.refresh_highlights(&path);
        }
        Ok(())
    }
    pub fn refresh_highlights(&mut self, path: &Path) -> Result<bool, String> {
        if self
            .document(path)
            .and_then(|document| document.backend_snapshot.as_ref())
            .is_some_and(|snapshot| snapshot.instance.is_empty())
        {
            return Err(
                "Backend does not advertise highlighting snapshot identity".into()
            );
        }
        let response = self
            .navigate(path, crate::navigation::NavigationQuery::HighlightSnapshot)?;
        match response {
            ProxyResponse::HighlightSnapshot { highlights } => Ok(self
                .documents
                .files
                .get_mut(path)
                .is_some_and(|document| document.accept_highlights(highlights))),
            _ => Err("Unexpected highlight response".into()),
        }
    }

    pub fn flush(&mut self, path: &Path) -> Result<(), String> {
        let document = self
            .documents
            .files
            .get_mut(path)
            .ok_or("Document is not open")?;
        loop {
            let sync = document
                .synchronization
                .as_mut()
                .ok_or("Document is disconnected")?;
            if let Some(error) = &sync.error {
                return Err(error.clone());
            }
            if let Some(notification) = sync.next_notification() {
                if let Err(error) = self.executor.notification(notification) {
                    sync.error = Some(error.message.clone());
                    return Err(error.message);
                }
                continue;
            }
            let Some(effect) = sync.next_effect() else {
                return if sync.is_idle() {
                    Ok(())
                } else {
                    Err("Document has outstanding backend operations".into())
                };
            };
            let response = self.executor.request(effect.request);
            match document.acknowledge(effect.id, response).outcome {
                SyncOutcome::Failed(error) => return Err(error),
                _ => {}
            }
        }
    }
    pub fn close(&mut self, path: &Path) -> Result<(), String> {
        self.close_owned(path)?;
        self.documents.files.remove(path);
        Ok(())
    }
    fn close_owned(&mut self, path: &Path) -> Result<(), String> {
        let document = self
            .documents
            .files
            .get_mut(path)
            .ok_or("Document is not open")?;
        if matches!(
            document.begin_close()?,
            crate::session::DocumentClose::Local
        ) {
            return Ok(());
        }
        self.flush(path)?;
        if !self.documents.files[path]
            .synchronization
            .as_ref()
            .is_some_and(|sync| sync.is_closed())
        {
            return Err("Backend did not acknowledge document closing".into());
        }
        Ok(())
    }
}

impl<E> std::ops::Deref for WorkspaceSession<E> {
    type Target = crate::workspace::WorkspaceActions;
    fn deref(&self) -> &Self::Target {
        &self.actions
    }
}
impl<E> std::ops::DerefMut for WorkspaceSession<E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.actions
    }
}
