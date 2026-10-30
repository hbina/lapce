//! Frontend-independent connections to Lapce's workspace services.
//!
//! Each connection owns one workspace session. Events are delivered in order on
//! a plain channel; scheduling them on a UI loop belongs to the frontend.

use std::{collections::HashMap, path::PathBuf, process::Command, sync::mpsc};

use anyhow::{Context, Result};
use lapce_rpc::{
    core::{CoreHandler, CoreNotification, CoreRpcHandler},
    proxy::{ProxyRpcHandler, ProxyStatus},
};

pub mod leases;
pub mod managed;
pub mod process;
pub mod remote;
pub mod shared;

pub mod socket;
pub mod ssh;
pub mod workspace;

use workspace::{LapceWorkspace, LapceWorkspaceType};

#[derive(Clone, Debug)]
pub enum LocalBackend {
    Embedded,
    Process { executable: PathBuf },
    SharedSocket { path: PathBuf },
}

impl LocalBackend {
    /// A shared socket takes precedence over an executable; empty settings use the build default.
    pub fn from_settings(executable: &str, shared_socket: &str) -> Self {
        if !shared_socket.is_empty() {
            Self::SharedSocket {
                path: shared_socket.into(),
            }
        } else if !executable.is_empty() {
            Self::Process {
                executable: executable.into(),
            }
        } else {
            Self::default()
        }
    }
}

impl Default for LocalBackend {
    fn default() -> Self {
        #[cfg(feature = "embedded-backend")]
        {
            Self::Embedded
        }
        #[cfg(not(feature = "embedded-backend"))]
        {
            Self::Process {
                executable: "lapce-proxy".into(),
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConnectionOptions {
    pub workspace: LapceWorkspace,
    pub local_backend: LocalBackend,
    /// An already installed remote executable; never downloaded or replaced.
    pub remote_backend_executable: Option<String>,
    pub language_server_configurations:
        HashMap<String, HashMap<String, serde_json::Value>>,
}

pub struct BackendConnection {
    pub proxy_rpc: ProxyRpcHandler,
    pub core_rpc: CoreRpcHandler,
    /// Includes terminal bytes as well as filesystem and language-service events.
    pub events: mpsc::Receiver<CoreNotification>,
    driver: std::thread::JoinHandle<()>,
    event_driver: std::thread::JoinHandle<()>,
}

impl BackendConnection {
    pub fn connect(options: ConnectionOptions) -> Result<Self> {
        let proxy_rpc = ProxyRpcHandler::new();
        let core_rpc = CoreRpcHandler::new();
        let (tx, events) = mpsc::channel();

        let event_rpc = core_rpc.clone();
        let event_driver = std::thread::Builder::new()
            .name("Lapce backend events".into())
            .spawn(move || event_rpc.mainloop(&mut EventForwarder(tx)))?;

        let driver_core = core_rpc.clone();
        let driver_proxy = proxy_rpc.clone();
        // Queue initialization before exposing the connection. Requests sent
        // immediately after connect must not overtake workspace setup.
        driver_proxy.initialize(
            options.workspace.path.clone(),
            options.language_server_configurations.clone(),
        );
        let driver = match std::thread::Builder::new()
            .name("Lapce backend connection".into())
            .spawn(move || {
                driver_core.notification(CoreNotification::ProxyStatus {
                    status: ProxyStatus::Connecting,
                });
                let result = match options.workspace.kind {
                    LapceWorkspaceType::Local => match options.local_backend {
                        LocalBackend::Embedded => {
                            #[cfg(feature = "embedded-backend")]
                            {
                                run_embedded(
                                    driver_core.clone(),
                                    driver_proxy.clone(),
                                )
                            }
                            #[cfg(not(feature = "embedded-backend"))]
                            {
                                Err(anyhow::anyhow!(
                                    "embedded backend support is disabled"
                                ))
                            }
                        }
                        LocalBackend::SharedSocket { path } => socket::run_client(
                            &path,
                            driver_core.clone(),
                            driver_proxy.clone(),
                        ),
                        LocalBackend::Process { executable } => {
                            let child = new_command(&executable)
                                .arg("--proxy")
                                .arg("--no-cli-socket")
                                .stdin(std::process::Stdio::piped())
                                .stdout(std::process::Stdio::piped())
                                .spawn();
                            child
                                .with_context(|| {
                                    format!("Start backend {}", executable.display())
                                })
                                .and_then(|child| {
                                    process::run_process(
                                        child,
                                        driver_core.clone(),
                                        driver_proxy.clone(),
                                    )
                                })
                        }
                    },
                    LapceWorkspaceType::RemoteSSH(ssh) => {
                        remote::start_remote_with_executable(
                            ssh::SshRemote { ssh },
                            driver_core.clone(),
                            driver_proxy.clone(),
                            options.remote_backend_executable.as_deref(),
                        )
                    }
                };
                if let Err(error) = result {
                    tracing::error!("Backend connection failed: {error:#}");
                    driver_core.notification(CoreNotification::ShowMessage {
                        title: "Backend connection failed".into(),
                        message: lsp_types::ShowMessageParams {
                            typ: lsp_types::MessageType::ERROR,
                            message: format!("{error:#}"),
                        },
                    });
                }
                driver_proxy.disconnect();
                driver_proxy.shutdown();
                driver_core.notification(CoreNotification::ProxyStatus {
                    status: ProxyStatus::Disconnected,
                });
                driver_core.shutdown();
            }) {
            Ok(driver) => driver,
            Err(error) => {
                proxy_rpc.shutdown();
                core_rpc.shutdown();
                let _ = event_driver.join();
                return Err(error.into());
            }
        };

        Ok(Self {
            proxy_rpc,
            core_rpc,
            events,
            driver,
            event_driver,
        })
    }

    pub fn shutdown(&self) {
        self.proxy_rpc.shutdown();
    }

    /// Wait for the backend process and event workers to stop before exiting a
    /// headless program. GUI event loops should use the non-blocking shutdown.
    pub fn shutdown_and_wait(self) -> Result<()> {
        self.shutdown();
        self.driver
            .join()
            .map_err(|_| anyhow::anyhow!("backend worker panicked"))?;
        self.event_driver
            .join()
            .map_err(|_| anyhow::anyhow!("event worker panicked"))?;
        Ok(())
    }
}

#[cfg(feature = "embedded-backend")]
fn run_embedded(
    driver_core: CoreRpcHandler,
    driver_proxy: ProxyRpcHandler,
) -> Result<()> {
    let (frontend, backend) = std::os::unix::net::UnixStream::pair()?;
    let worker = std::thread::Builder::new()
        .name("Lapce backend".into())
        .spawn(move || lapce_proxy::transport::serve_stream(backend))?;
    let result =
        socket::run_stream(frontend, driver_core.clone(), driver_proxy.clone());
    let backend_result = worker
        .join()
        .map_err(|_| anyhow::anyhow!("embedded backend panicked"))?;
    result.and(backend_result)
}

struct EventForwarder(mpsc::Sender<CoreNotification>);

impl CoreHandler for EventForwarder {
    fn handle_notification(&mut self, event: CoreNotification) {
        let _ = self.0.send(event);
    }

    fn handle_request(
        &mut self,
        _id: lapce_rpc::RequestId,
        request: lapce_rpc::core::CoreRequest,
    ) {
        match request {}
    }
}

pub fn new_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);

    cmd
}
