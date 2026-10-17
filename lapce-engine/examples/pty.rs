//! Headless PTY frontend: pty <lapce-proxy> [--input-stdin] <program> [arguments...]
use anyhow::{Result, anyhow};
use lapce_client::{BackendConnection, ConnectionOptions, LocalBackend};
use lapce_engine::{
    runtime::{RpcExecutor, SessionEvent, WorkspaceSession},
    terminal::TerminalPhase,
};
use lapce_rpc::terminal::TerminalProfile;
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let executable = PathBuf::from(
        arguments
            .next()
            .ok_or_else(|| anyhow!("expected proxy executable"))?,
    );
    let mut command = arguments
        .next()
        .ok_or_else(|| anyhow!("expected terminal program"))?;
    let input = if command == "--input-stdin" {
        command = arguments
            .next()
            .ok_or_else(|| anyhow!("expected terminal program"))?;
        let mut input = String::new();
        io::stdin().read_to_string(&mut input)?;
        Some(input)
    } else {
        None
    };
    let connection = BackendConnection::connect(ConnectionOptions {
        local_backend: LocalBackend::Process { executable },
        ..Default::default()
    })?;
    let result = (|| -> Result<()> {
        let mut session = WorkspaceSession::new(RpcExecutor {
            rpc: connection.proxy_rpc.clone(),
            timeout: Duration::from_secs(10),
        });
        let id = session
            .launch_terminal(TerminalProfile {
                command: Some(command),
                arguments: Some(arguments.collect()),
                ..Default::default()
            })
            .map_err(|error| anyhow!(error))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut started = false;
        loop {
            let event = connection
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
            match session
                .handle_event(event)
                .map_err(|error| anyhow!(error))?
            {
                SessionEvent::TerminalOutput { term_id, content }
                    if term_id == id =>
                {
                    io::stdout().write_all(&content)?;
                    io::stdout().flush()?;
                }
                SessionEvent::TerminalChanged { term_id, .. } if term_id == id => {
                    let terminal = &session.terminals[&id];
                    match terminal.phase {
                        TerminalPhase::Running => {
                            if !started {
                                session
                                    .terminal_resize(id, 80, 24)
                                    .map_err(|error| anyhow!(error))?;
                                if let Some(input) = &input {
                                    session
                                        .terminal_input(id, input.clone())
                                        .map_err(|error| anyhow!(error))?;
                                }
                            }
                            started = true;
                        }
                        TerminalPhase::Failed => {
                            return Err(anyhow!(
                                terminal.launch_error.clone().unwrap_or_default()
                            ));
                        }
                        TerminalPhase::Stopped => {
                            if !started {
                                return Err(anyhow!(
                                    "process exited before its start event"
                                ));
                            }
                            if terminal.exit_code.is_some_and(|code| code != 0) {
                                return Err(anyhow!(
                                    "process exited with {:?}",
                                    terminal.exit_code
                                ));
                            }
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                SessionEvent::StatusChanged
                    if matches!(
                        session.status,
                        Some(lapce_rpc::proxy::ProxyStatus::Disconnected)
                    ) =>
                {
                    return Err(anyhow!(
                        "backend disconnected before terminal completion"
                    ));
                }
                _ => {}
            }
        }
    })();
    connection.shutdown_and_wait()?;
    result
}
