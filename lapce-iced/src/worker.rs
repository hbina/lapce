use lapce_client::{
    BackendConnection, ConnectionOptions, LocalBackend, workspace::LapceWorkspace,
};
use lapce_engine::{
    navigation::NavigationQuery,
    runtime::{RpcExecutor, WorkspaceSession},
};
use lapce_rpc::{file::DirectoryEntry, proxy::ProxyResponse};
use std::{ffi::OsString, path::PathBuf, sync::mpsc, time::Duration};

#[derive(Clone)]
pub struct Options {
    pub path: PathBuf,
    pub backend: LocalBackend,
    pub lsp: Option<PathBuf>,
}
impl Options {
    pub fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let mut backend = LocalBackend::Process {
            executable: std::env::current_exe()
                .map_err(|e| e.to_string())?
                .with_file_name("lapce-proxy"),
        };
        let mut path = None;
        let mut lsp = None;
        while let Some(arg) = args.next() {
            if arg == "--lsp" {
                lsp = Some(PathBuf::from(
                    args.next().ok_or("Expected language server executable")?,
                ));
            } else if arg == "--proxy" || arg == "--socket" {
                let value =
                    PathBuf::from(args.next().ok_or("Expected backend path")?);
                backend = if arg == "--socket" {
                    LocalBackend::SharedSocket { path: value }
                } else {
                    LocalBackend::Process { executable: value }
                };
            } else if arg == "--help" || path.is_some() {
                return Err("Usage: lapce-iced [--proxy EXECUTABLE | --socket SOCKET] [--lsp RUST_ANALYZER] [FILE_OR_DIRECTORY]".into());
            } else {
                path = Some(PathBuf::from(arg));
            }
        }
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let path = path.unwrap_or_else(|| cwd.clone());
        Ok(Self {
            path: if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            },
            backend,
            lsp,
        })
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Navigation {
    Definition,
    References,
    Hover,
    Symbols,
}
#[derive(Debug, Clone)]
pub enum Command {
    Open(PathBuf),
    Jump(PathBuf, u32),
    List(PathBuf),
    Search(String),
    Navigate {
        kind: Navigation,
        offset: usize,
        path: PathBuf,
        revision: u64,
    },
}
#[derive(Default, Debug, Clone)]
pub struct Snapshot {
    pub path: PathBuf,
    pub directory: PathBuf,
    pub content: String,
    pub revision: u64,
    pub entries: Vec<DirectoryEntry>,
    pub status: String,
    pub detail: String,
    pub jump: Option<(u64, usize)>,
    pub diagnostics: String,
    pub results: Vec<(PathBuf, u32, String)>,
}
pub fn spawn(
    options: Options,
) -> (
    mpsc::SyncSender<Command>,
    mpsc::Receiver<Result<Snapshot, String>>,
    std::thread::JoinHandle<()>,
) {
    let (commands, input) = mpsc::sync_channel(16);
    let (output, events) = mpsc::sync_channel(4);
    let thread = std::thread::spawn(move || {
        if let Err(error) = run(options, input, &output) {
            let _ = output.send(Err(error));
        }
    });
    (commands, events, thread)
}
fn run(
    options: Options,
    input: mpsc::Receiver<Command>,
    output: &mpsc::SyncSender<Result<Snapshot, String>>,
) -> Result<(), String> {
    let directory = if options.path.is_dir() {
        options.path.clone()
    } else {
        options
            .path
            .parent()
            .ok_or("File has no parent")?
            .to_path_buf()
    };
    let connection = BackendConnection::connect(ConnectionOptions {
        workspace: LapceWorkspace {
            path: Some(directory.clone()),
            ..Default::default()
        },
        local_backend: options.backend,
        language_server_configurations: options.lsp.map(|executable| std::collections::HashMap::from([("lapce-native-lsp".into(), std::collections::HashMap::from([("servers".into(), serde_json::json!([{ "language": "rust", "executable": executable }]))]))])).unwrap_or_default(),
        ..Default::default()
    })
    .map_err(|e| e.to_string())?;
    let result = serve(&connection, options.path, directory, input, output);
    let shutdown = connection.shutdown_and_wait().map_err(|e| e.to_string());
    result.and(shutdown)
}
fn serve(
    connection: &BackendConnection,
    initial: PathBuf,
    directory: PathBuf,
    input: mpsc::Receiver<Command>,
    output: &mpsc::SyncSender<Result<Snapshot, String>>,
) -> Result<(), String> {
    let mut session = WorkspaceSession::new(RpcExecutor {
        rpc: connection.proxy_rpc.clone(),
        timeout: Duration::from_secs(10),
    });
    let generation = session.backend_generation();
    let mut snapshot = Snapshot {
        directory: directory.clone(),
        ..Default::default()
    };
    snapshot.entries = session.list_directory(directory)?;
    if !initial.is_dir() {
        open(&mut session, &mut snapshot, initial)?;
    }
    output
        .send(Ok(snapshot.clone()))
        .map_err(|e| e.to_string())?;
    loop {
        let mut changed = false;
        for event in connection.events.try_iter() {
            session.handle_backend_event(generation, event)?;
            changed = true;
        }
        let command = match input.recv_timeout(Duration::from_millis(50)) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Some(command) = command {
            let result = execute(&mut session, &mut snapshot, command);
            if let Err(error) = result {
                output.send(Err(error)).map_err(|e| e.to_string())?;
            }
            changed = true;
        }
        if changed {
            if let Some(document) = session.document(&snapshot.path) {
                snapshot.content = document.buffer.to_string();
                snapshot.revision = document.buffer.rev();
            }
            snapshot.diagnostics = session
                .diagnostics
                .get(&snapshot.path)
                .map(|state| {
                    state
                        .items
                        .iter()
                        .map(|item| {
                            format!(
                                "{}:{} {}",
                                item.range.start.line + 1,
                                item.range.start.character + 1,
                                item.message
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            snapshot.status = format!(
                "{:?}{}",
                session.status,
                if !snapshot.diagnostics.is_empty() {
                    " · diagnostics available"
                } else {
                    ""
                }
            );
            output
                .send(Ok(snapshot.clone()))
                .map_err(|e| e.to_string())?;
        }
    }
    if session.document(&snapshot.path).is_some() {
        session.close(&snapshot.path)?;
    }
    Ok(())
}
fn open(
    session: &mut WorkspaceSession<RpcExecutor>,
    snapshot: &mut Snapshot,
    path: PathBuf,
) -> Result<(), String> {
    let path = if path.is_absolute() {
        path
    } else {
        snapshot.directory.join(path)
    };
    if snapshot.path != path {
        session.open(path.clone())?;
        if session.document(&snapshot.path).is_some() {
            session.close(&snapshot.path)?;
        }
        snapshot.path = path;
    }
    snapshot.content = session
        .document(&snapshot.path)
        .ok_or("File did not open")?
        .buffer
        .to_string();
    snapshot.revision = session
        .document(&snapshot.path)
        .ok_or("File did not open")?
        .buffer
        .rev();
    Ok(())
}
fn execute(
    session: &mut WorkspaceSession<RpcExecutor>,
    snapshot: &mut Snapshot,
    command: Command,
) -> Result<(), String> {
    match command {
        Command::Open(path) => open(session, snapshot, path)?,
        Command::Jump(path, line) => {
            open(session, snapshot, path)?;
            snapshot.jump =
                Some((snapshot.jump.map_or(1, |(id, _)| id + 1), line as usize));
        }
        Command::List(path) => {
            snapshot.entries = session.list_directory(path.clone())?;
            snapshot.directory = path;
        }
        Command::Search(pattern) => {
            session.search_workspace(
                lapce_engine::global_search::WorkspaceSearchQuery {
                    pattern,
                    ..Default::default()
                },
            )?;
            snapshot.results = session
                .actions
                .search
                .matches
                .iter()
                .flat_map(|(path, matches)| {
                    matches.iter().map(move |item| {
                        (
                            path.clone(),
                            item.line.saturating_sub(1) as u32,
                            item.line_content.clone(),
                        )
                    })
                })
                .collect();
            snapshot.detail = format!("{} search results", snapshot.results.len());
        }
        Command::Navigate {
            kind,
            offset,
            path,
            revision,
        } => {
            if snapshot.path != path
                || session
                    .document(&path)
                    .is_none_or(|document| document.buffer.rev() != revision)
            {
                return Err(
                    "Document changed; retry navigation on the current snapshot"
                        .into(),
                );
            }
            let query = match kind {
                Navigation::Definition => NavigationQuery::Definition { offset },
                Navigation::References => NavigationQuery::References { offset },
                Navigation::Hover => NavigationQuery::Hover { offset },
                Navigation::Symbols => NavigationQuery::Symbols,
            };
            let response = session.navigate(&snapshot.path, query)?;
            snapshot.detail = serde_json::to_string_pretty(&response)
                .map_err(|e| e.to_string())?;
            snapshot.results.clear();
            match response {
                ProxyResponse::GetDefinitionResponse { definition, .. } => {
                    let locations = match definition {
                        lsp_types::GotoDefinitionResponse::Scalar(location) => {
                            vec![location]
                        }
                        lsp_types::GotoDefinitionResponse::Array(locations) => {
                            locations
                        }
                        lsp_types::GotoDefinitionResponse::Link(links) => links
                            .into_iter()
                            .map(|link| lsp_types::Location {
                                uri: link.target_uri,
                                range: link.target_selection_range,
                            })
                            .collect(),
                    };
                    for location in locations {
                        if let Some(path) =
                            lapce_rpc::file::file_url_path(&location.uri)
                        {
                            snapshot.results.push((
                                path,
                                location.range.start.line,
                                "Definition".into(),
                            ));
                        }
                    }
                }
                ProxyResponse::GetReferencesResponse { references } => {
                    for location in references {
                        if let Some(path) =
                            lapce_rpc::file::file_url_path(&location.uri)
                        {
                            snapshot.results.push((
                                path,
                                location.range.start.line,
                                "Reference".into(),
                            ));
                        }
                    }
                }
                ProxyResponse::GetDocumentSymbols { resp } => {
                    fn collect(
                        path: &PathBuf,
                        symbols: Vec<lsp_types::DocumentSymbol>,
                        results: &mut Vec<(PathBuf, u32, String)>,
                    ) {
                        for symbol in symbols {
                            results.push((
                                path.clone(),
                                symbol.selection_range.start.line,
                                symbol.name,
                            ));
                            if let Some(children) = symbol.children {
                                collect(path, children, results);
                            }
                        }
                    }
                    collect(
                        &snapshot.path,
                        lapce_engine::navigation::document_symbol_tree(resp),
                        &mut snapshot.results,
                    );
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn receive_until(
        events: &mpsc::Receiver<Result<Snapshot, String>>,
        mut predicate: impl FnMut(&Snapshot) -> bool,
    ) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let snapshot = events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("worker response timed out")
                .expect("worker failed");
            if predicate(&snapshot) {
                return snapshot;
            }
        }
    }

    #[test]
    #[ignore = "requires LAPCE_TEST_PROXY; optionally LAPCE_TEST_LSP for live navigation"]
    fn real_backend_open_search_reload_and_close() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source 🌍.rs");
        let original = "pub fn answer() -> u32 { 42 }\npub fn caller() -> u32 { answer() }\n// 🌍\n";
        std::fs::write(&path, original).unwrap();
        std::fs::write(directory.path().join("Cargo.toml"), "[package]\nname = \"iced_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"source 🌍.rs\"\n").unwrap();
        let lsp = std::env::var_os("LAPCE_TEST_LSP").map(PathBuf::from);
        let (commands, events, thread) = spawn(Options {
            path: path.clone(),
            backend: LocalBackend::Process {
                executable: std::env::var_os("LAPCE_TEST_PROXY")
                    .expect("set LAPCE_TEST_PROXY")
                    .into(),
            },
            lsp: lsp.clone(),
        });
        let loaded = receive_until(&events, |snapshot| snapshot.content == original);
        commands.send(Command::Search("answer".into())).unwrap();
        receive_until(&events, |snapshot| {
            snapshot.results.iter().any(|(file, _, _)| file == &path)
        });
        commands.send(Command::Jump(path.clone(), 1)).unwrap();
        receive_until(&events, |snapshot| snapshot.jump == Some((1, 1)));
        if lsp.is_some() {
            let deadline = Instant::now() + Duration::from_secs(45);
            loop {
                commands
                    .send(Command::Navigate {
                        kind: Navigation::Definition,
                        offset: original.find("{ answer()").unwrap() + 2,
                        path: path.clone(),
                        revision: loaded.revision,
                    })
                    .unwrap();
                let mut found = false;
                while let Ok(event) = events.recv_timeout(Duration::from_millis(500))
                {
                    if let Ok(snapshot) = event {
                        found |=
                            snapshot.results.iter().any(|(file, line, label)| {
                                file == &path && *line == 0 && label == "Definition"
                            });
                    }
                    if found {
                        break;
                    }
                }
                if found {
                    break;
                }
                assert!(Instant::now() < deadline, "live definition did not arrive");
            }
        }
        let changed = original.replace("42", "43");
        std::fs::write(&path, &changed).unwrap();
        receive_until(&events, |snapshot| snapshot.content == changed);
        drop(commands);
        while events.recv_timeout(Duration::from_secs(15)).is_ok() {}
        thread.join().unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), changed);
    }
}
