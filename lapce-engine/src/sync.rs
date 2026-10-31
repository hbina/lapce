//! Ordered disk synchronization and language queries, independent of frontend event loops.
use lapce_rpc::{
    RpcError,
    proxy::{ProxyRequest, ProxyResponse},
};
use lapce_xi_rope::RopeDelta;
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
};

#[derive(Clone, Debug)]
enum Operation {
    Edit {
        delta: RopeDelta,
        local_rev: u64,
    },
    Request {
        request: ProxyRequest,
        local_rev: u64,
    },
    Close,
    Notification(lapce_rpc::proxy::ProxyNotification),
}

#[derive(Clone, Debug)]
pub struct DocumentSync {
    path: PathBuf,
    buffer_id: lapce_rpc::buffer::BufferId,
    remote_rev: u64,
    local_rev: u64,
    next_id: u64,
    latest_local_rev: u64,
    requests: HashMap<u64, u64>,
    pending: VecDeque<(u64, Operation)>,
    inflight: Option<(u64, Operation)>,
    pub error: Option<String>,
    closing: Option<u64>,
    closed: bool,
}

pub struct SyncEffect {
    pub id: u64,
    pub request: ProxyRequest,
    pub detached: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum SyncOutcome {
    Edited,
    Closed { id: u64 },
    RequestCompleted { id: u64, current: bool },
    Failed(String),
    Stale,
}

impl DocumentSync {
    pub fn new(
        path: PathBuf,
        buffer_id: lapce_rpc::buffer::BufferId,
        local_rev: u64,
        remote_rev: u64,
    ) -> Self {
        Self {
            path,
            buffer_id,
            local_rev,
            remote_rev,
            next_id: 0,
            latest_local_rev: local_rev,
            requests: HashMap::new(),
            pending: VecDeque::new(),
            inflight: None,
            error: None,
            closing: None,
            closed: false,
        }
    }
    fn enqueue(&mut self, operation: Operation) -> Option<u64> {
        if self.closing.is_some() || self.closed {
            return None;
        }
        self.next_id += 1;
        self.pending.push_back((self.next_id, operation));
        Some(self.next_id)
    }
    pub fn edit(&mut self, delta: RopeDelta, local_rev: u64) -> bool {
        if self.closing.is_some() || self.closed {
            return false;
        }
        self.latest_local_rev = local_rev;
        self.enqueue(Operation::Edit { delta, local_rev }).is_some()
    }
    pub fn request(&mut self, request: ProxyRequest, local_rev: u64) -> Option<u64> {
        if !request.allowed_in_code_browser() {
            return None;
        }
        self.enqueue(Operation::Request { request, local_rev })
    }
    pub fn notification(
        &mut self,
        notification: lapce_rpc::proxy::ProxyNotification,
    ) -> bool {
        // Workspace/terminal lifecycle belongs to the connection, and editing
        // or debugger traffic has no place in a document browsing queue.
        match &notification {
            lapce_rpc::proxy::ProxyNotification::OpenFileChanged { path }
                if path == &self.path => {}
            lapce_rpc::proxy::ProxyNotification::LspCancel { .. } => {}
            _ => return false,
        }
        self.enqueue(Operation::Notification(notification))
            .is_some()
    }
    /// Queue an owned close after earlier mutations. Later operations are rejected.
    pub fn close(&mut self) -> Option<u64> {
        if self.error.is_some() {
            return None;
        }
        let id = self.enqueue(Operation::Close)?;
        self.closing = Some(id);
        Some(id)
    }
    pub fn is_closed(&self) -> bool {
        self.closed
    }
    pub fn is_closing(&self) -> bool {
        self.closing.is_some()
    }
    pub fn next_notification(
        &mut self,
    ) -> Option<lapce_rpc::proxy::ProxyNotification> {
        if self.error.is_some() || self.inflight.is_some() {
            return None;
        }
        if !matches!(self.pending.front(), Some((_, Operation::Notification(_)))) {
            return None;
        }
        let (_, Operation::Notification(notification)) = self.pending.pop_front()?
        else {
            unreachable!()
        };
        Some(notification)
    }
    pub fn next_effect(&mut self) -> Option<SyncEffect> {
        if self.error.is_some() || self.inflight.is_some() {
            return None;
        }
        if matches!(self.pending.front(), Some((_, Operation::Notification(_)))) {
            return None;
        }
        let (id, operation) = self.pending.pop_front()?;
        if let Operation::Request { request, local_rev } = operation {
            self.requests.insert(id, local_rev);
            return Some(SyncEffect {
                id,
                request,
                detached: true,
            });
        }
        let request = match &operation {
            Operation::Edit { delta, .. } => ProxyRequest::UpdateBuffer {
                buffer_id: self.buffer_id,
                path: self.path.clone(),
                delta: delta.clone(),
                rev: self.remote_rev + 1,
            },
            Operation::Close => ProxyRequest::CloseBuffer {
                buffer_id: self.buffer_id,
                path: self.path.clone(),
            },
            Operation::Request { .. } | Operation::Notification(_) => unreachable!(),
        };
        self.inflight = Some((id, operation));
        Some(SyncEffect {
            id,
            request,
            detached: false,
        })
    }
    pub fn acknowledge(
        &mut self,
        id: u64,
        response: Result<ProxyResponse, RpcError>,
    ) -> SyncOutcome {
        if let Some(local_rev) = self.requests.remove(&id) {
            return SyncOutcome::RequestCompleted {
                id,
                current: !self.closed
                    && self.closing.is_none()
                    && self.error.is_none()
                    && local_rev == self.latest_local_rev,
            };
        }
        let Some((current, operation)) = self.inflight.as_ref() else {
            return SyncOutcome::Stale;
        };
        if *current != id || self.error.is_some() {
            return SyncOutcome::Stale;
        }
        let result = match (operation, response) {
            (
                Operation::Edit { local_rev, .. },
                Ok(ProxyResponse::BufferUpdated { rev, .. }),
            ) if rev == self.remote_rev + 1 => {
                self.remote_rev = rev;
                self.local_rev = *local_rev;
                SyncOutcome::Edited
            }
            (Operation::Close, Ok(ProxyResponse::Success {})) => {
                self.closed = true;
                SyncOutcome::Closed { id }
            }
            (_, Err(error)) => SyncOutcome::Failed(error.message),
            _ => SyncOutcome::Failed(
                "Unexpected document synchronization response or revision".into(),
            ),
        };
        if let SyncOutcome::Failed(error) = &result {
            self.error = Some(error.clone());
            // Retain the rejected operation and later edits. Recovery must be
            // explicit; a backend snapshot must never overwrite unsaved text.
        } else {
            self.inflight = None;
        }
        result
    }
    /// Explicit conflict resolution: retain local text and replace only the
    /// backend's in-memory buffer. Previously queued saves are canceled.
    pub fn recover_keep_local(
        &mut self,
        snapshot: ProxyResponse,
        local_text: lapce_xi_rope::Rope,
        local_rev: u64,
    ) -> Result<(), String> {
        if self.closed {
            return Err("Document is closed".into());
        }
        let ProxyResponse::BufferSnapshot {
            buffer_id,
            path,
            content,
            read_only,
            rev,
            ..
        } = snapshot
        else {
            return Err("Expected a backend document snapshot".into());
        };
        if path != self.path || buffer_id != self.buffer_id || read_only {
            return Err(
                "Snapshot identity or write access does not match the document"
                    .into(),
            );
        }
        let remote_text = lapce_xi_rope::Rope::from(content);
        self.closing = None;
        self.pending.clear();
        self.requests.clear();
        self.latest_local_rev = local_rev;
        self.inflight = None;
        self.error = None;
        self.remote_rev = rev;
        self.local_rev = local_rev;
        if !remote_text
            .iter_chunks(..)
            .flat_map(str::bytes)
            .eq(local_text.iter_chunks(..).flat_map(str::bytes))
        {
            self.edit(
                lapce_xi_rope::Delta::simple_edit(
                    lapce_xi_rope::Interval::new(0, remote_text.len()),
                    local_text,
                    remote_text.len(),
                ),
                local_rev,
            );
        }
        Ok(())
    }

    pub fn is_idle(&self) -> bool {
        self.error.is_none() && self.inflight.is_none() && self.pending.is_empty()
    }

    /// Translate backend LSP versions to local editor revisions. A publication
    /// may arrive before its update acknowledgement on a separate event stream.
    pub fn local_version(&self, backend_version: i32) -> Option<i32> {
        if self.closed {
            return None;
        }
        if backend_version == self.remote_rev as i32 {
            return Some(self.local_rev as i32);
        }
        if let Some((_, Operation::Edit { local_rev, .. })) = &self.inflight {
            if backend_version == (self.remote_rev + 1) as i32 {
                return Some(*local_rev as i32);
            }
        }
        None
    }

    pub fn remote_revision(&self) -> u64 {
        self.remote_rev
    }
}
