//! Workspace document ownership, independent of views and reactive runtimes.
use crate::document::Document;
use std::path::PathBuf;

/// Hosts may wrap engine documents in their own observable adapter. Each path
/// retains one shared document regardless of how many views display it.
#[derive(Clone)]
pub struct WorkspaceDocuments<D: Clone = Document> {
    pub files: im::HashMap<PathBuf, D>,
    pub scratch: im::HashMap<String, D>,
}
impl<D: Clone> Default for WorkspaceDocuments<D> {
    fn default() -> Self {
        Self {
            files: im::HashMap::new(),
            scratch: im::HashMap::new(),
        }
    }
}
impl<D: Clone> WorkspaceDocuments<D> {
    pub fn next_scratch_name(&self) -> String {
        let next = self
            .scratch
            .keys()
            .filter_map(|name| name.strip_prefix("Untitled-")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        format!("Untitled-{next}")
    }

    /// Reuse a document without invoking its factory or issuing another open.
    pub fn file(&mut self, path: PathBuf, create: impl FnOnce() -> D) -> (D, bool) {
        if let Some(document) = self.files.get(&path) {
            return (document.clone(), false);
        }
        let document = create();
        self.files.insert(path, document.clone());
        (document, true)
    }
}

/// Document state and its backend synchronization share one session owner.
#[derive(Clone)]
pub struct SessionDocument {
    pub document: Document,
    pub synchronization: Option<crate::sync::DocumentSync>,
    backend_generation: u64,
    pub(crate) opening: bool,
}

/// Identifies the document and backend attachment owning an asynchronous operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentBackendGeneration {
    owner: lapce_rpc::buffer::BufferId,
    generation: u64,
}

/// Semantic results of one acknowledgement, independent of host callbacks.
pub struct DocumentCompletion {
    pub outcome: crate::sync::SyncOutcome,
    pub request_result: Option<(
        u64,
        Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError>,
    )>,
    pub operation_result: Option<(u64, Result<(), String>)>,
    pub refresh_annotations: bool,
}
/// The host may release immediately or wait for the queued backend close.
pub enum DocumentClose {
    Local,
    Backend { operation_id: u64 },
}
impl SessionDocument {
    pub fn begin_reopen(&mut self) -> Result<ReopenDocument, String> {
        let plan = ReopenDocument::capture(self)?;
        self.opening = true;
        Ok(plan)
    }
    pub fn begin_close(&mut self) -> Result<DocumentClose, String> {
        if self.can_close_locally()
            || self
                .synchronization
                .as_ref()
                .is_some_and(|sync| sync.is_closed())
        {
            return Ok(DocumentClose::Local);
        }
        let operation_id = self.synchronization.as_mut()
            .and_then(|sync| sync.close())
            .ok_or("Document cannot close while disconnected, paused or already closing")?;
        Ok(DocumentClose::Backend { operation_id })
    }
    /// Apply a watcher snapshot only while this document owns a live buffer.
    /// The host schedules synchronization and refreshes its views for the delta.
    pub fn reload_from_disk(
        &mut self,
        content: lapce_xi_rope::Rope,
    ) -> Option<Option<crate::editing::DocumentDelta>> {
        if self
            .synchronization
            .as_ref()
            .is_none_or(|sync| sync.is_closing())
        {
            return None;
        }
        if self.buffer.to_string() == content.to_string() {
            self.document.mark_disk_present();
            return Some(None);
        }
        Some(self.document.disk_changed(content))
    }
    pub fn backend_generation(&self) -> DocumentBackendGeneration {
        DocumentBackendGeneration {
            owner: self.id,
            generation: self.backend_generation,
        }
    }
    pub fn acknowledge_backend(
        &mut self,
        generation: DocumentBackendGeneration,
        id: u64,
        response: Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError>,
    ) -> DocumentCompletion {
        if generation != self.backend_generation() {
            return DocumentCompletion {
                outcome: crate::sync::SyncOutcome::Stale,
                request_result: None,
                operation_result: None,
                refresh_annotations: false,
            };
        }
        self.acknowledge(id, response)
    }

    /// A read-only file with no backend owner can be released locally.
    pub fn can_close_locally(&self) -> bool {
        self.synchronization.is_none()
            && self.content.is_file()
            && self.content.read_only()
    }
    pub fn can_reopen_backend(&self) -> bool {
        !self.opening && self.can_close_locally()
    }

    /// Invalidate transport ownership while preserving the loaded file snapshot.
    pub fn backend_disconnected(&mut self) {
        self.opening = false;
        self.document.invalidate_highlights();
        self.synchronization = None;
        self.backend_generation += 1;
    }
    pub fn backend_ready(&self) -> bool {
        self.synchronization
            .as_ref()
            .is_some_and(|sync| sync.is_idle() && !sync.is_closed())
    }
    pub fn accept_highlights(
        &mut self,
        highlights: lapce_rpc::highlighting::HighlightSnapshot,
    ) -> bool {
        self.backend_ready() && self.document.accept_highlights(highlights)
    }

    pub fn acknowledge(
        &mut self,
        id: u64,
        response: Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError>,
    ) -> DocumentCompletion {
        use crate::sync::SyncOutcome;
        let outcome = self
            .synchronization
            .as_mut()
            .map_or(SyncOutcome::Stale, |sync| {
                sync.acknowledge(id, response.clone())
            });
        if matches!(outcome, SyncOutcome::Edited) && self.backend_ready() {
            if let Ok(lapce_rpc::proxy::ProxyResponse::BufferUpdated {
                snapshot,
                ..
            }) = &response
            {
                self.document.backend_snapshot = Some(snapshot.clone());
            }
        }
        let request_result = match &outcome {
            SyncOutcome::RequestCompleted { id, current } => Some((
                *id,
                if *current {
                    response
                } else {
                    Err(lapce_rpc::RpcError {
                        code: -32801,
                        message: "Document changed while the request was pending"
                            .into(),
                    })
                },
            )),
            _ => None,
        };
        let operation_result = match &outcome {
            SyncOutcome::Closed { id } => Some((*id, Ok(()))),
            _ => None,
        };
        let refresh_annotations = matches!(outcome, SyncOutcome::Edited);
        DocumentCompletion {
            outcome,
            request_result,
            operation_result,
            refresh_annotations,
        }
    }
    pub fn new(document: Document) -> Self {
        Self {
            document,
            synchronization: None,
            backend_generation: 0,
            opening: false,
        }
    }
    pub fn initialize_sync(&mut self, remote_revision: u64) -> bool {
        self.opening = false;
        let Some(path) = self.document.content.path() else {
            return false;
        };
        self.synchronization = Some(crate::sync::DocumentSync::new(
            path.clone(),
            self.document.id,
            self.document.buffer.rev(),
            remote_revision,
        ));
        self.backend_generation += 1;
        true
    }
    /// Retarget only after owned closing has completed. Reopening then restores
    /// backend synchronization while retaining this document's edit history.
    pub fn retarget_closed(&mut self, path: PathBuf) -> Result<(), String> {
        if !self.document.content.is_file()
            || !self
                .synchronization
                .as_ref()
                .is_some_and(|sync| sync.is_closed())
        {
            return Err("Document must be a closed file before retargeting".into());
        }
        self.document
            .set_syntax(lapce_core::syntax::Syntax::init(&path));
        self.document.inlay_hints = None;
        self.document.content = crate::document::DocContent::File {
            path,
            read_only: false,
        };
        self.backend_generation += 1;
        Ok(())
    }

    /// Queue exactly the local revisions produced by a document edit batch.
    pub fn synchronize_edits(
        &mut self,
        deltas: &[crate::editing::DocumentDelta],
    ) -> bool {
        if deltas.is_empty() || !self.document.content.is_file() {
            return true;
        }
        let Some(base) = self.document.buffer.rev().checked_sub(deltas.len() as u64)
        else {
            return false;
        };
        let Some(sync) = self.synchronization.as_mut() else {
            return false;
        };
        deltas.iter().enumerate().all(|(index, (_, delta, _))| {
            sync.edit(delta.clone(), base + index as u64 + 1)
        })
    }

    pub fn recover_sync(
        &mut self,
        snapshot: lapce_rpc::proxy::ProxyResponse,
    ) -> Result<(), String> {
        use lapce_core::buffer::rope_text::RopeText;
        let synchronization = self
            .synchronization
            .as_mut()
            .ok_or("Document is no longer synchronized")?;
        synchronization.recover_keep_local(
            snapshot,
            self.document.buffer.text().clone(),
            self.document.buffer.rev(),
        )
    }
}
impl std::ops::Deref for SessionDocument {
    type Target = Document;
    fn deref(&self) -> &Document {
        &self.document
    }
}
impl std::ops::DerefMut for SessionDocument {
    fn deref_mut(&mut self) -> &mut Document {
        &mut self.document
    }
}

/// Reattach a disconnected file, including an interrupted read-only initial load.
pub struct ReopenDocument {
    id: lapce_rpc::buffer::BufferId,
    path: PathBuf,
    generation: u64,
}
impl ReopenDocument {
    pub fn capture(session: &SessionDocument) -> Result<Self, String> {
        if session.opening
            || (!session.document.loading.loaded
                && !session.document.content.read_only())
            || session
                .synchronization
                .as_ref()
                .is_some_and(|sync| !sync.is_closed())
        {
            return Err(
                "Document must be disconnected and loaded or read-only before reopening".into()
            );
        }
        Ok(Self {
            id: session.document.id,
            path: session
                .document
                .content
                .path()
                .ok_or("Document is not a file")?
                .clone(),
            generation: session.backend_generation,
        })
    }
    pub fn request(&self) -> lapce_rpc::proxy::ProxyRequest {
        lapce_rpc::proxy::ProxyRequest::NewBuffer {
            buffer_id: self.id,
            path: self.path.clone(),
        }
    }
    pub fn finish(
        self,
        session: &mut SessionDocument,
        response: Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError>,
    ) -> Result<(), String> {
        use lapce_core::buffer::rope_text::RopeText;
        use lapce_rpc::proxy::ProxyResponse;
        if session.backend_generation == self.generation {
            session.opening = false;
        }
        if session.document.id != self.id
            || session.document.content.path() != Some(&self.path)
            || session.backend_generation != self.generation
            || session
                .synchronization
                .as_ref()
                .is_some_and(|sync| !sync.is_closed())
        {
            return Err("Document ownership changed while reopening".into());
        }
        let ProxyResponse::NewBufferResponse {
            content,
            read_only,
            rev,
            snapshot,
        } = response.map_err(|error| error.message)?
        else {
            return Err("Unexpected document-reopen response".into());
        };
        let equal = session
            .document
            .buffer
            .text()
            .iter_chunks(..)
            .flat_map(str::bytes)
            .eq(content.bytes());
        session.document.invalidate_highlights();
        if session.document.content.read_only() {
            if !session.document.loading.loaded {
                session.document.load(lapce_xi_rope::Rope::from(content));
            } else if !equal {
                session
                    .document
                    .reload(lapce_xi_rope::Rope::from(content), true);
            }
            session.synchronization = Some(crate::sync::DocumentSync::new(
                self.path,
                self.id,
                session.document.buffer.rev(),
                rev,
            ));
            session.document.backend_snapshot = Some(snapshot);
            session.backend_generation += 1;
            return Ok(());
        }
        if read_only && !equal {
            return Err(
                "Reopened file is read-only; local edits are preserved".into()
            );
        }
        let baseline = (!equal).then(|| lapce_xi_rope::Rope::from(content.as_str()));
        let mut synchronization = crate::sync::DocumentSync::new(
            self.path.clone(),
            self.id,
            session.document.buffer.rev(),
            rev,
        );
        if !equal {
            synchronization.recover_keep_local(
                ProxyResponse::BufferSnapshot {
                    buffer_id: self.id,
                    path: self.path,
                    content,
                    read_only,
                    rev,
                    snapshot,
                },
                session.document.buffer.text().clone(),
                session.document.buffer.rev(),
            )?;
        }
        if let crate::document::DocContent::File {
            read_only: current, ..
        } = &mut session.document.content
        {
            *current = read_only;
        }
        if let Some(baseline) = baseline {
            session.document.set_disk_baseline(baseline);
        }
        session.synchronization = Some(synchronization);
        session.backend_generation += 1;
        Ok(())
    }
}
