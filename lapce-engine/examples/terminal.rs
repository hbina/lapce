//! A minimal read-only terminal frontend using the same workspace engine as Floem.
//! Usage: terminal <lapce-proxy executable> <file>, or terminal --socket <socket> <file>
use anyhow::{Result, anyhow};
use lapce_client::{
    BackendConnection, ConnectionOptions, LocalBackend, workspace::LapceWorkspace,
};
use lapce_core::buffer::rope_text::RopeText;
use lapce_engine::{
    navigation::NavigationQuery,
    runtime::{RpcExecutor, WorkspaceSession},
};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

fn run(connection: &BackendConnection, mut path: PathBuf) -> Result<()> {
    let mut session = WorkspaceSession::new(RpcExecutor {
        rpc: connection.proxy_rpc.clone(),
        timeout: Duration::from_secs(10),
    });
    let backend_generation = session.backend_generation();
    session.open(path.clone()).map_err(|error| anyhow!(error))?;
    println!(
        "Commands: show, ls, status, diagnostics, open <path>, search <pattern>, workspace-symbols <query>, symbols, folds, definition/references/implementation/hover <byte-offset>, quit"
    );
    // Read input separately so backend events continue while the user is idle.
    let (input_tx, input_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if input_tx.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        for event in connection.events.try_iter() {
            if let lapce_rpc::core::CoreNotification::ShowMessage {
                title,
                message,
            } = &event
            {
                eprintln!("{title}: {}", message.message);
            }
            if let Err(error) =
                session.handle_backend_event(backend_generation, event)
            {
                eprintln!("Backend event failed: {error}");
            }
        }
        let line = match input_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => line?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match line.as_str() {
            "status" => println!("{:?}", session.status),
            "diagnostics" => {
                if let Some(state) = session.diagnostics.get(&path) {
                    for severity in [
                        lsp_types::DiagnosticSeverity::ERROR,
                        lsp_types::DiagnosticSeverity::WARNING,
                        lsp_types::DiagnosticSeverity::INFORMATION,
                        lsp_types::DiagnosticSeverity::HINT,
                    ] {
                        for entry in state.entries(severity) {
                            println!(
                                "{}:{}: {}",
                                entry.diagnostic.range.start.line + 1,
                                entry.diagnostic.range.start.character + 1,
                                entry.diagnostic.message
                            );
                        }
                    }
                }
            }
            "show" => {
                print!("{}", session.document(&path).unwrap().buffer.text());
                io::stdout().flush()?;
            }
            "ls" => {
                match session.list_directory(path.parent().unwrap().to_path_buf()) {
                    Ok(entries) => {
                        for entry in entries {
                            println!(
                                "{}{}",
                                entry.path.display(),
                                if entry.is_dir { "/" } else { "" }
                            );
                        }
                    }
                    Err(error) => println!("Directory listing failed: {error}"),
                }
            }
            command if command.starts_with("open ") => {
                let target = PathBuf::from(&command[5..]);
                let target = if target.is_absolute() {
                    target
                } else {
                    path.parent().unwrap().join(target)
                };
                if target != path {
                    session
                        .open(target.clone())
                        .map_err(|error| anyhow!(error))?;
                    session.close(&path).map_err(|error| anyhow!(error))?;
                    path = target;
                }
                println!("Opened {}", path.display());
            }
            command if command.starts_with("search ") => {
                session
                    .search_workspace(
                        lapce_engine::global_search::WorkspaceSearchQuery {
                            pattern: command[7..].into(),
                            ..Default::default()
                        },
                    )
                    .map_err(|error| anyhow!(error))?;
                for (path, matches) in &session.search.matches {
                    for item in matches {
                        println!(
                            "{}:{}: {}",
                            path.display(),
                            item.line,
                            item.line_content
                        );
                    }
                }
            }
            command if command.starts_with("workspace-symbols ") => {
                match session.workspace_symbols(command[18..].to_owned()) {
                    Ok(symbols) => {
                        println!("{}", serde_json::to_string_pretty(&symbols)?)
                    }
                    Err(error) => println!("Workspace symbols failed: {error}"),
                }
            }
            command @ ("symbols" | "folds") => match session.navigate(
                &path,
                if command == "folds" {
                    NavigationQuery::FoldingRanges
                } else {
                    NavigationQuery::Symbols
                },
            ) {
                Ok(response) => {
                    println!("{}", serde_json::to_string_pretty(&response)?)
                }
                Err(error) => println!("Navigation failed: {error}"),
            },
            command
                if ["definition ", "references ", "implementation ", "hover "]
                    .iter()
                    .any(|prefix| command.starts_with(prefix)) =>
            {
                let (kind, offset) = command.split_once(' ').unwrap();
                match offset.parse::<usize>() {
                    Ok(offset) => {
                        let query = match kind {
                            "definition" => NavigationQuery::Definition { offset },
                            "references" => NavigationQuery::References { offset },
                            "implementation" => {
                                NavigationQuery::Implementation { offset }
                            }
                            _ => NavigationQuery::Hover { offset },
                        };
                        match session.navigate(&path, query) {
                            Ok(response) => println!(
                                "{}",
                                serde_json::to_string_pretty(&response)?
                            ),
                            Err(error) => println!("Navigation failed: {error}"),
                        }
                    }
                    Err(_) => println!("Expected a UTF-8 byte offset"),
                }
            }
            "quit" => break,
            _ => {
                println!("Unknown command");
            }
        };
    }
    session.close(&path).map_err(|error| anyhow!(error))?;
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let first = args
        .next()
        .ok_or_else(|| anyhow!("expected proxy executable or --socket"))?;
    let local_backend = if first == "--socket" {
        LocalBackend::SharedSocket {
            path: args
                .next()
                .ok_or_else(|| anyhow!("expected socket path"))?
                .into(),
        }
    } else {
        LocalBackend::Process {
            executable: first.into(),
        }
    };
    let path =
        PathBuf::from(args.next().ok_or_else(|| anyhow!("expected file path"))?);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    let connection = BackendConnection::connect(ConnectionOptions {
        workspace: LapceWorkspace {
            path: path.parent().map(PathBuf::from),
            ..Default::default()
        },
        local_backend,
        ..Default::default()
    })?;
    let result = run(&connection, path);
    connection.shutdown_and_wait()?;
    result
}
