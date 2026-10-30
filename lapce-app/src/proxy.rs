//! Floem adapter for the headless backend connection.

use std::{
    collections::HashMap,
    sync::{Arc, mpsc::Sender},
};

use floem::{ext_event::create_signal_from_channel, reactive::ReadSignal};
use lapce_client::{
    ConnectionOptions, LocalBackend,
    managed::{BackendControl, ManagedBackend},
};
use lapce_rpc::{
    core::{CoreNotification, CoreRpcHandler},
    proxy::ProxyRpcHandler,
    terminal::TermId,
};

use crate::{terminal::event::TermEvent, workspace::LapceWorkspace};

#[derive(Clone)]
pub struct ProxyData {
    pub control: BackendControl,
    pub proxy_rpc: ProxyRpcHandler,
    pub core_rpc: CoreRpcHandler,
    pub notification: ReadSignal<Option<CoreNotification>>,
}

impl ProxyData {
    pub fn shutdown(&self) {
        self.proxy_rpc.shutdown();
    }
}

pub fn new_proxy(
    workspace: Arc<LapceWorkspace>,
    backend_executable: &str,
    backend_socket: &str,
    language_server_configurations: HashMap<
        String,
        HashMap<String, serde_json::Value>,
    >,
    term_tx: Sender<(TermId, TermEvent)>,
) -> ProxyData {
    let connection = ManagedBackend::connect(ConnectionOptions {
        workspace: workspace.as_ref().clone(),
        local_backend: LocalBackend::from_settings(
            backend_executable,
            backend_socket,
        ),
        language_server_configurations,
        ..Default::default()
    })
    .expect("start workspace backend");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("Floem backend events".into())
        .spawn(move || {
            for event in connection.events {
                let result = match event {
                    CoreNotification::UpdateTerminal { term_id, content } => term_tx
                        .send((term_id, TermEvent::UpdateContent(content)))
                        .map_err(|e| e.to_string()),
                    event => tx.send(event).map_err(|e| e.to_string()),
                };
                if let Err(error) = result {
                    tracing::error!("Frontend event delivery failed: {error}");
                }
            }
        })
        .expect("start frontend event adapter");
    ProxyData {
        control: connection.control,
        proxy_rpc: connection.proxy_rpc,
        core_rpc: connection.core_rpc,
        notification: create_signal_from_channel(rx),
    }
}
