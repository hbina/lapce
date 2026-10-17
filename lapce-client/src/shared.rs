//! Multiple frontend RPC endpoints sharing one read-only workspace backend.
use crate::{
    BackendConnection, ConnectionOptions,
    leases::{BufferLeases, FrontendId},
};
use lapce_rpc::{
    RpcError,
    core::CoreNotification,
    proxy::{ProxyRequest, ProxyResponse, ProxyRpc, ProxyRpcHandler, ProxyStatus},
};
use std::{sync::mpsc, thread::JoinHandle, time::Duration};

#[derive(Default)]
struct SharedState {
    diagnostics: std::collections::BTreeMap<String, CoreNotification>,
    repository: Option<CoreNotification>,
}
impl SharedState {
    fn observe(&mut self, event: &CoreNotification) {
        match event {
            CoreNotification::PublishDiagnostics { diagnostics } => {
                self.diagnostics
                    .insert(diagnostics.uri.to_string(), event.clone());
            }
            CoreNotification::DiffInfo { .. } => {
                self.repository = Some(event.clone())
            }
            _ => {}
        }
    }
    fn replay(&self) -> impl Iterator<Item = CoreNotification> + '_ {
        self.repository
            .iter()
            .chain(self.diagnostics.values())
            .cloned()
    }
}

struct Peer {
    id: FrontendId,
    rpc: ProxyRpcHandler,
    events: mpsc::SyncSender<CoreNotification>,
    event_capacity: usize,
}
struct FrontendSnapshot {
    text: lapce_xi_rope::Rope,
    local_rev: u64,
    backend_rev: u64,
}
type FrontendSnapshots =
    std::collections::HashMap<(FrontendId, std::path::PathBuf), FrontendSnapshot>;

fn frontend_event(
    peer: FrontendId,
    snapshots: &FrontendSnapshots,
    event: &CoreNotification,
) -> Option<CoreNotification> {
    let mut event = event.clone();
    if let CoreNotification::PublishDiagnostics { diagnostics } = &mut event {
        if let (Some(version), Some(path)) = (
            diagnostics.version,
            lapce_rpc::file::file_url_path(&diagnostics.uri),
        ) {
            if let Some(snapshot) = snapshots.get(&(peer, path)) {
                if u64::try_from(version).ok() != Some(snapshot.backend_rev) {
                    return None;
                }
                diagnostics.version = Some(i32::try_from(snapshot.local_rev).ok()?);
            }
        }
    }
    Some(event)
}
enum Control {
    Attach(Peer),
    Shutdown,
}
pub struct SharedFrontend {
    pub proxy_rpc: ProxyRpcHandler,
    pub events: mpsc::Receiver<CoreNotification>,
}
impl Drop for SharedFrontend {
    fn drop(&mut self) {
        self.proxy_rpc.shutdown();
    }
}
pub struct SharedBackend {
    control: crossbeam_channel::Sender<Control>,
    worker: JoinHandle<()>,
}
fn error(message: impl Into<String>) -> RpcError {
    RpcError {
        code: -32602,
        message: message.into(),
    }
}
fn request(
    backend: &BackendConnection,
    leases: &mut BufferLeases,
    snapshots: &mut FrontendSnapshots,
    peer: FrontendId,
    request: ProxyRequest,
) -> Result<ProxyResponse, RpcError> {
    if !request.allowed_in_code_browser() {
        return Err(error(
            "Request is unavailable in the read-only shared backend",
        ));
    }
    let call = |request| {
        backend
            .proxy_rpc
            .request_timeout(request, Duration::from_secs(10))
    };
    match request {
        ProxyRequest::NewBuffer { buffer_id, path } => {
            let already_owned = leases.local_id(peer, &path).is_some();
            let lease = leases
                .acquire(peer, path.clone(), buffer_id)
                .map_err(error)?;
            let response = call(ProxyRequest::NewBuffer {
                buffer_id: lease.backend_id,
                path: path.clone(),
            });
            if response.is_err() && !already_owned {
                let _ = leases.release(peer, &path, buffer_id);
            }
            if let Ok(ProxyResponse::NewBufferResponse { content, rev, .. }) =
                &response
            {
                snapshots.insert(
                    (peer, path),
                    FrontendSnapshot {
                        text: lapce_xi_rope::Rope::from(content.as_str()),
                        local_rev: *rev,
                        backend_rev: *rev,
                    },
                );
            }
            response
        }
        ProxyRequest::GetBuffer { path } => {
            let local = leases
                .local_id(peer, &path)
                .ok_or_else(|| error("Frontend does not own file"))?;
            match call(ProxyRequest::GetBuffer { path })? {
                ProxyResponse::BufferSnapshot {
                    path,
                    content,
                    read_only,
                    rev,
                    ..
                } => {
                    snapshots.insert(
                        (peer, path.clone()),
                        FrontendSnapshot {
                            text: lapce_xi_rope::Rope::from(content.as_str()),
                            local_rev: rev,
                            backend_rev: rev,
                        },
                    );
                    Ok(ProxyResponse::BufferSnapshot {
                        buffer_id: local,
                        path,
                        content,
                        read_only,
                        rev,
                    })
                }
                _ => Err(error("Unexpected snapshot response")),
            }
        }
        ProxyRequest::UpdateBuffer {
            buffer_id,
            path,
            delta,
            rev,
        } => {
            let canonical =
                leases.backend_id(peer, &path, buffer_id).map_err(error)?;
            let snapshot = snapshots
                .get(&(peer, path.clone()))
                .ok_or_else(|| error("Frontend has no buffer snapshot"))?;
            let text = &snapshot.text;
            let local_rev = snapshot.local_rev;
            if local_rev.checked_add(1) != Some(rev) || delta.base_len != text.len()
            {
                return Err(error(
                    "Frontend buffer revision or delta base mismatch",
                ));
            }
            let mut previous_end = 0;
            for element in &delta.els {
                if let lapce_xi_rope::DeltaElement::Copy(start, end) = element {
                    if *start < previous_end
                        || start > end
                        || *end > text.len()
                        || !text.is_codepoint_boundary(*start)
                        || !text.is_codepoint_boundary(*end)
                    {
                        return Err(error("Invalid frontend delta range"));
                    }
                    previous_end = *end;
                }
            }
            let candidate = delta.apply(text);
            let ProxyResponse::BufferSnapshot {
                content,
                rev: mut backend_rev,
                ..
            } = call(ProxyRequest::GetBuffer { path: path.clone() })?
            else {
                return Err(error("Unexpected backend snapshot response"));
            };
            if candidate.to_string() != content {
                let backend_text = lapce_xi_rope::Rope::from(content);
                let delta = lapce_xi_rope::RopeDelta::simple_edit(
                    0..backend_text.len(),
                    candidate.clone(),
                    backend_text.len(),
                );
                let next = backend_rev
                    .checked_add(1)
                    .ok_or_else(|| error("Backend revision exhausted"))?;
                match call(ProxyRequest::UpdateBuffer {
                    buffer_id: canonical,
                    path: path.clone(),
                    delta,
                    rev: next,
                })? {
                    ProxyResponse::BufferUpdated { rev: accepted }
                        if accepted == next =>
                    {
                        backend_rev = next;
                    }
                    _ => return Err(error("Unexpected backend update response")),
                }
            }
            snapshots.insert(
                (peer, path),
                FrontendSnapshot {
                    text: candidate,
                    local_rev: rev,
                    backend_rev,
                },
            );
            Ok(ProxyResponse::BufferUpdated { rev })
        }
        ProxyRequest::CloseBuffer { buffer_id, path } => {
            let canonical =
                leases.backend_id(peer, &path, buffer_id).map_err(error)?;
            // Do not release the final lease until backend close succeeds.
            let last = leases
                .is_last_owner(peer, &path, buffer_id)
                .map_err(error)?;
            if last {
                call(ProxyRequest::CloseBuffer {
                    buffer_id: canonical,
                    path: path.clone(),
                })?;
            }
            leases.release(peer, &path, buffer_id).map_err(error)?;
            snapshots.remove(&(peer, path));
            Ok(ProxyResponse::Success {})
        }
        request => {
            let navigation_path = match &request {
                ProxyRequest::GetHover { path, .. }
                | ProxyRequest::GetSelectionRange { path, .. }
                | ProxyRequest::GetReferences { path, .. }
                | ProxyRequest::GotoImplementation { path, .. }
                | ProxyRequest::GetDefinition { path, .. }
                | ProxyRequest::GetTypeDefinition { path, .. }
                | ProxyRequest::ShowCallHierarchy { path, .. }
                | ProxyRequest::CallHierarchyIncoming { path, .. }
                | ProxyRequest::GetDocumentHighlights { path, .. }
                | ProxyRequest::GetDocumentSymbols { path }
                | ProxyRequest::GetInlayHints { path }
                | ProxyRequest::GetSemanticTokens { path }
                | ProxyRequest::LspFoldingRange { path } => Some(path),
                _ => None,
            };
            if let Some(path) = navigation_path {
                let snapshot =
                    snapshots.get(&(peer, path.clone())).ok_or_else(|| {
                        error("Frontend does not own navigation document")
                    })?;
                let ProxyResponse::BufferSnapshot { content, .. } =
                    call(ProxyRequest::GetBuffer { path: path.clone() })?
                else {
                    return Err(error("Unexpected backend snapshot response"));
                };
                if snapshot.text.to_string() != content {
                    return Err(RpcError { code: -32801, message: "Navigation document is waiting for a shared backend reload".into() });
                }
            }
            call(request)
        }
    }
}
impl SharedBackend {
    pub fn connect(options: ConnectionOptions) -> anyhow::Result<Self> {
        Self::from_connection(BackendConnection::connect(options)?)
    }
    fn from_connection(backend: BackendConnection) -> anyhow::Result<Self> {
        let (control, commands) = crossbeam_channel::unbounded();
        let worker = std::thread::Builder::new()
            .name("Shared workspace backend".into())
            .spawn(move || {
                let mut peers: Vec<Peer> = Vec::new();
                let mut leases = BufferLeases::default();
                let mut snapshots = std::collections::HashMap::new();
                let mut status = ProxyStatus::Connecting;
                let mut state = SharedState::default();
                loop {
                    let mut stop = false;
                    for command in commands.try_iter() {
                        match command {
                            Control::Attach(peer) => {
                                if state.replay().count().saturating_add(1)
                                    > peer.event_capacity
                                {
                                    peer.rpc.disconnect();
                                    continue;
                                }
                                let replay = state.replay().chain(std::iter::once(
                                    CoreNotification::ProxyStatus {
                                        status: status.clone(),
                                    },
                                ));
                                if replay.into_iter().any(|event| {
                                    peer.events.try_send(event).is_err()
                                }) {
                                    peer.rpc.disconnect();
                                    continue;
                                }
                                peers.push(peer);
                            }
                            Control::Shutdown => stop = true,
                        }
                    }
                    if stop {
                        break;
                    }
                    let mut departed = Vec::new();
                    for event in backend.events.try_iter() {
                        state.observe(&event);
                        if let CoreNotification::ProxyStatus { status: current } =
                            &event
                        {
                            status = current.clone();
                        }
                        for peer in &peers {
                            if let Some(event) =
                                frontend_event(peer.id, &snapshots, &event)
                            {
                                if peer.events.try_send(event).is_err() {
                                    departed.push(peer.id);
                                }
                            }
                        }
                    }
                    // Serialize backend requests: shared search cancellation is workspace-wide.
                    for peer in &peers {
                        if departed.contains(&peer.id) {
                            continue;
                        }
                        match peer.rpc.rx().try_recv() {
                            Ok(ProxyRpc::Request(id, query)) => {
                                let updated_path = match &query {
                                    ProxyRequest::UpdateBuffer { path, .. } => {
                                        Some(path.clone())
                                    }
                                    _ => None,
                                };
                                let response = request(
                                    &backend,
                                    &mut leases,
                                    &mut snapshots,
                                    peer.id,
                                    query,
                                );
                                let updated = response.is_ok();
                                peer.rpc.handle_response(id, response);
                                // A publication may have arrived before this peer
                                // acknowledged the matching disk snapshot.
                                if updated {
                                    if let Some(path) = updated_path {
                                        {
                                            if let Some(event) = state
                                                .diagnostics.values().find(|event| {
                                                    matches!(event, CoreNotification::PublishDiagnostics { diagnostics }
                                                        if lapce_rpc::file::file_url_path(&diagnostics.uri).as_ref() == Some(&path))
                                                })
                                                .and_then(|event| {
                                                    frontend_event(
                                                        peer.id, &snapshots, event,
                                                    )
                                                })
                                            {
                                                if peer
                                                    .events
                                                    .try_send(event)
                                                    .is_err()
                                                {
                                                    departed.push(peer.id);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(ProxyRpc::Notification(_)) => {
                                // Workspace initialization and file reloads are backend-owned.
                                // Frontend-local notification IDs must not cancel another peer's requests.
                            }
                            Ok(ProxyRpc::Shutdown) => departed.push(peer.id),
                            Err(_) => {}
                        }
                    }
                    for id in departed {
                        if let Some(index) =
                            peers.iter().position(|peer| peer.id == id)
                        {
                            let peer = peers.remove(index);
                            peer.rpc.disconnect();
                            snapshots.retain(|(owner, _), _| *owner != id);
                            for (path, buffer_id) in leases.disconnect(id) {
                                let _ = backend.proxy_rpc.request_timeout(
                                    ProxyRequest::CloseBuffer { buffer_id, path },
                                    Duration::from_secs(10),
                                );
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                for peer in peers {
                    peer.rpc.disconnect();
                }
                let _ = backend.shutdown_and_wait();
            })?;
        Ok(Self { control, worker })
    }
    pub fn attach(&self) -> anyhow::Result<SharedFrontend> {
        self.attach_with_event_capacity(128)
    }
    /// Overflow disconnects this frontend; events are never silently dropped.
    pub fn attach_with_event_capacity(
        &self,
        capacity: usize,
    ) -> anyhow::Result<SharedFrontend> {
        anyhow::ensure!(capacity > 0, "Event capacity must be positive");
        let rpc = ProxyRpcHandler::new();
        let (events, receiver) = mpsc::sync_channel(capacity);
        self.control.send(Control::Attach(Peer {
            id: FrontendId::next(),
            rpc: rpc.clone(),
            events,
            event_capacity: capacity,
        }))?;
        Ok(SharedFrontend {
            proxy_rpc: rpc,
            events: receiver,
        })
    }
    pub fn shutdown_and_wait(self) -> anyhow::Result<()> {
        self.control.send(Control::Shutdown)?;
        self.worker
            .join()
            .map_err(|_| anyhow::anyhow!("shared backend worker panicked"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_use_each_frontends_revision_and_reject_stale_backend_versions() {
        let peer = FrontendId::next();
        let path = std::env::temp_dir().join("shared-revision.rs");
        let snapshots = FrontendSnapshots::from([(
            (peer, path.clone()),
            FrontendSnapshot {
                text: lapce_xi_rope::Rope::from("text"),
                local_rev: 3,
                backend_rev: 8,
            },
        )]);
        let event = |version| CoreNotification::PublishDiagnostics {
            diagnostics: lsp_types::PublishDiagnosticsParams {
                uri: lsp_types::Url::from_file_path(&path).unwrap(),
                diagnostics: Vec::new(),
                version,
            },
        };
        assert!(matches!(frontend_event(peer, &snapshots, &event(Some(8))),
            Some(CoreNotification::PublishDiagnostics { diagnostics }) if diagnostics.version == Some(3)));
        assert!(frontend_event(peer, &snapshots, &event(Some(7))).is_none());
        assert!(
            matches!(frontend_event(FrontendId::next(), &snapshots, &event(Some(8))),
            Some(CoreNotification::PublishDiagnostics { diagnostics }) if diagnostics.version == Some(8))
        );
        assert!(matches!(frontend_event(peer, &snapshots, &event(None)),
            Some(CoreNotification::PublishDiagnostics { diagnostics }) if diagnostics.version.is_none()));
    }
    fn backend_fixture() -> (BackendConnection, mpsc::Sender<CoreNotification>) {
        let proxy = ProxyRpcHandler::new();
        let worker_proxy = proxy.clone();
        let driver = std::thread::spawn(move || {
            loop {
                match worker_proxy.rx().recv().unwrap() {
                    ProxyRpc::Request(id, _) => worker_proxy.handle_response(
                        id,
                        Ok(ProxyResponse::GlobalSearchResponse {
                            matches: Default::default(),
                        }),
                    ),
                    ProxyRpc::Shutdown => break,
                    _ => {}
                }
            }
        });
        let (tx, events) = mpsc::channel();
        let backend = BackendConnection {
            proxy_rpc: proxy,
            core_rpc: lapce_rpc::core::CoreRpcHandler::new(),
            events,
            driver,
            event_driver: std::thread::spawn(|| {}),
        };
        (backend, tx)
    }
    #[test]
    fn event_overflow_disconnects_only_the_slow_frontend() {
        let (backend, tx) = backend_fixture();
        let host = SharedBackend::from_connection(backend).unwrap();
        let slow = host.attach_with_event_capacity(2).unwrap();
        let fast = host.attach().unwrap();
        let query = || ProxyRequest::GlobalSearch {
            pattern: "x".into(),
            case_sensitive: false,
            whole_word: false,
            is_regex: false,
        };
        fast.proxy_rpc
            .request_timeout(query(), Duration::from_secs(2))
            .unwrap();
        for _ in 0..20 {
            tx.send(CoreNotification::WorkspaceFileChange).unwrap();
        }
        loop {
            match slow.events.recv_timeout(Duration::from_secs(2)) {
                Ok(_) => {
                    // Let the broadcaster finish its burst before consuming more.
                    if slow
                        .proxy_rpc
                        .request_timeout(query(), Duration::from_secs(2))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(error) => panic!("slow frontend did not disconnect: {error}"),
            }
        }
        assert!(
            slow.proxy_rpc
                .request_timeout(query(), Duration::from_secs(2))
                .is_err()
        );
        assert!(
            fast.proxy_rpc
                .request_timeout(query(), Duration::from_secs(2))
                .is_ok()
        );
        let changes = (0..21)
            .filter(|_| {
                matches!(
                    fast.events.recv_timeout(Duration::from_secs(2)).unwrap(),
                    CoreNotification::WorkspaceFileChange
                )
            })
            .count();
        assert_eq!(changes, 20);
        host.shutdown_and_wait().unwrap();
    }
    #[test]
    fn late_attachment_replays_latest_diagnostics_and_clears_before_status() {
        let (backend, tx) = backend_fixture();
        let host = SharedBackend::from_connection(backend).unwrap();
        let existing = host.attach().unwrap();
        existing
            .events
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        let uri = lsp_types::Url::parse("file:///file.rs").unwrap();
        let publish = |diagnostics| CoreNotification::PublishDiagnostics {
            diagnostics: lsp_types::PublishDiagnosticsParams {
                uri: uri.clone(),
                diagnostics,
                version: Some(1),
            },
        };
        tx.send(publish(vec![lsp_types::Diagnostic::new_simple(
            Default::default(),
            "old error".into(),
        )]))
        .unwrap();
        tx.send(publish(vec![])).unwrap();
        tx.send(CoreNotification::DiffInfo {
            diff: lapce_rpc::source_control::DiffInfo {
                head: "main".into(),
                branches: vec![],
                tags: vec![],
                diffs: vec![],
            },
        })
        .unwrap();
        for _ in 0..3 {
            existing
                .events
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        }
        let late = host.attach().unwrap();
        assert!(
            matches!(late.events.recv_timeout(Duration::from_secs(2)).unwrap(), CoreNotification::DiffInfo { diff } if diff.head == "main")
        );
        assert!(
            matches!(late.events.recv_timeout(Duration::from_secs(2)).unwrap(), CoreNotification::PublishDiagnostics { diagnostics } if diagnostics.diagnostics.is_empty() && diagnostics.uri == uri)
        );
        assert!(matches!(
            late.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            CoreNotification::ProxyStatus { .. }
        ));
        let too_small = host.attach_with_event_capacity(1).unwrap();
        assert!(matches!(
            too_small.events.recv_timeout(Duration::from_secs(2)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        host.shutdown_and_wait().unwrap();
    }
}
