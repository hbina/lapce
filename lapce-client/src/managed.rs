//! A stable frontend RPC endpoint with replaceable backend ownership.
use crate::{BackendConnection, ConnectionOptions, EventForwarder};
use anyhow::Result;
use lapce_rpc::{
    RpcError,
    core::{CoreNotification, CoreRpcHandler},
    proxy::{ProxyRpc, ProxyRpcHandler, ProxyStatus},
};
use std::{sync::mpsc, thread::JoinHandle, time::Duration};

#[derive(Clone)]
pub struct BackendControl(crossbeam_channel::Sender<()>);
impl BackendControl {
    /// Restart with the same workspace and transport options.
    pub fn restart(&self) -> Result<()> {
        self.0
            .send(())
            .map_err(|_| anyhow::anyhow!("backend manager stopped"))
    }
}

pub struct ManagedBackend {
    pub proxy_rpc: ProxyRpcHandler,
    pub core_rpc: CoreRpcHandler,
    pub events: mpsc::Receiver<CoreNotification>,
    pub control: BackendControl,
    driver: JoinHandle<()>,
    event_driver: JoinHandle<()>,
}
impl ManagedBackend {
    pub fn connect(options: ConnectionOptions) -> Result<Self> {
        let mut connection = Some(BackendConnection::connect(options.clone())?);
        let proxy_rpc = ProxyRpcHandler::new();
        let core_rpc = CoreRpcHandler::new();
        let (tx, events) = mpsc::channel();
        let event_core = core_rpc.clone();
        let event_driver = std::thread::Builder::new()
            .name("Managed backend events".into())
            .spawn(move || event_core.mainloop(&mut EventForwarder(tx)))?;
        let (restart_tx, restart_rx) = crossbeam_channel::unbounded();
        let proxy = proxy_rpc.clone();
        let core = core_rpc.clone();
        let driver = std::thread::Builder::new()
            .name("Managed backend connection".into())
            .spawn(move || {
                loop {
                    // Replacement retires the entire old stream before publishing new events.
                    if restart_rx.try_recv().is_ok() {
                        let mut shutdown = false;
                        for queued in proxy.rx().try_iter() {
                            match queued {
                                ProxyRpc::Request(id, _) => proxy.handle_response(
                                    id,
                                    Err(RpcError {
                                        code: -32800,
                                        message: "Backend connection replaced"
                                            .into(),
                                    }),
                                ),
                                ProxyRpc::Shutdown => shutdown = true,
                                ProxyRpc::Notification(_) => {}
                            }
                        }
                        if shutdown {
                            break;
                        }
                        core.notification(CoreNotification::ProxyStatus {
                            status: ProxyStatus::Disconnected,
                        });
                        if let Some(old) = connection.take() {
                            let _ = old.shutdown_and_wait();
                        }
                        connection =
                            match BackendConnection::connect(options.clone()) {
                                Ok(next) => Some(next),
                                Err(error) => {
                                    core.notification(
                                        CoreNotification::ShowMessage {
                                            title: "Backend restart failed".into(),
                                            message: lsp_types::ShowMessageParams {
                                                typ: lsp_types::MessageType::ERROR,
                                                message: error.to_string(),
                                            },
                                        },
                                    );
                                    None
                                }
                            };
                    }
                    if let Some(active) = &connection {
                        for event in active.events.try_iter() {
                            core.notification(event);
                        }
                    }
                    match proxy.rx().recv_timeout(Duration::from_millis(5)) {
                        Ok(ProxyRpc::Request(id, request)) => {
                            if let Some(active) = &connection {
                                let responder = proxy.clone();
                                active.proxy_rpc.request_async(
                                    request,
                                    move |response| {
                                        responder.handle_response(id, response)
                                    },
                                );
                            } else {
                                proxy.handle_response(
                                    id,
                                    Err(RpcError {
                                        code: -32001,
                                        message: "backend disconnected".into(),
                                    }),
                                );
                            }
                        }
                        Ok(ProxyRpc::Notification(notification)) => {
                            if let Some(active) = &connection {
                                active.proxy_rpc.notification(notification);
                            }
                        }
                        Ok(ProxyRpc::Shutdown)
                        | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            break;
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    }
                }
                if let Some(active) = connection {
                    let _ = active.shutdown_and_wait();
                }
                proxy.disconnect();
                core.shutdown();
            })?;
        Ok(Self {
            proxy_rpc,
            core_rpc,
            events,
            control: BackendControl(restart_tx),
            driver,
            event_driver,
        })
    }
    pub fn shutdown_and_wait(self) -> Result<()> {
        self.proxy_rpc.shutdown();
        self.driver
            .join()
            .map_err(|_| anyhow::anyhow!("backend manager panicked"))?;
        self.event_driver
            .join()
            .map_err(|_| anyhow::anyhow!("backend event worker panicked"))?;
        Ok(())
    }
}
