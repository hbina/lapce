use lapce_client::{
    ConnectionOptions, LocalBackend, shared::SharedBackend,
    workspace::LapceWorkspace,
};
use lapce_engine::{
    navigation::NavigationQuery,
    runtime::{RpcExecutor, WorkspaceSession},
};
use lapce_rpc::proxy::ProxyResponse;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires LAPCE_TEST_PROXY and LAPCE_TEST_LSP executables"]
fn managed_replacement_restores_live_definition_navigation() {
    use lapce_client::managed::ManagedBackend;
    use lapce_rpc::{core::CoreNotification, proxy::ProxyStatus};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    let text =
        "pub fn answer() -> u32 { 42 }\npub fn caller() -> u32 { answer() }\n";
    std::fs::write(&path, text).unwrap();
    std::fs::write(directory.path().join("Cargo.toml"), "[package]\nname = \"restart_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"lib.rs\"\n").unwrap();
    let backend = ManagedBackend::connect(ConnectionOptions {
        workspace: LapceWorkspace { path: directory.path().into(), ..Default::default() },
        local_backend: LocalBackend::Process { executable: std::env::var_os("LAPCE_TEST_PROXY").expect("set LAPCE_TEST_PROXY").into() },
        language_server_configurations: HashMap::from([("lapce-native-lsp".into(), HashMap::from([("servers".into(), serde_json::json!([{
            "language": "rust", "executable": std::path::PathBuf::from(std::env::var_os("LAPCE_TEST_LSP").expect("set LAPCE_TEST_LSP")),
            "options": { "cargo": { "buildScripts": { "enable": false } }, "procMacro": { "enable": false } }
        }]))]))]),
        ..Default::default()
    }).unwrap();
    let executor = || RpcExecutor {
        rpc: backend.proxy_rpc.clone(),
        timeout: Duration::from_secs(5),
    };
    let mut session = WorkspaceSession::new(executor());
    session.open(path.clone()).unwrap();
    let identity = session.document(&path).unwrap().id;
    let offset = text.find("{ answer()").unwrap() + 2;
    let assert_definition = |session: &mut WorkspaceSession<RpcExecutor>| {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            for event in backend.events.try_iter() {
                session
                    .handle_backend_event(session.backend_generation(), event)
                    .unwrap();
            }
            if let Ok(ProxyResponse::GetDefinitionResponse { definition, .. }) =
                session.navigate(&path, NavigationQuery::Definition { offset })
            {
                if lapce_engine::navigation::first_definition(definition)
                    .is_some_and(|location| location.range.start.line == 0)
                {
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "live definition did not become available"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    assert_definition(&mut session);
    backend.control.restart().unwrap();
    let mut disconnected = false;
    loop {
        match backend
            .events
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
        {
            CoreNotification::ProxyStatus {
                status: ProxyStatus::Disconnected,
            } => disconnected = true,
            CoreNotification::ProxyStatus {
                status: ProxyStatus::Connected,
            } => {
                assert!(
                    disconnected,
                    "replacement must retire the old connection first"
                );
                break;
            }
            _ => {}
        }
    }
    let outcomes = session.reconnect(executor());
    assert!(outcomes.iter().all(|(_, result)| result.is_ok()));
    assert_eq!(session.document(&path).unwrap().id, identity);
    assert_definition(&mut session);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    session.close(&path).unwrap();
    backend.shutdown_and_wait().unwrap();
}

#[test]
#[ignore = "requires LAPCE_TEST_PROXY and LAPCE_TEST_LSP executables"]
fn real_language_server_provides_read_only_navigation_and_diagnostics() {
    let proxy = std::env::var_os("LAPCE_TEST_PROXY").expect("set LAPCE_TEST_PROXY");
    let lsp = std::env::var_os("LAPCE_TEST_LSP").expect("set LAPCE_TEST_LSP");
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(directory.path().join("Cargo.toml"), "[package]\nname = \"navigation_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    let path = directory.path().join("src/lib.rs");
    let text = "pub fn answer() -> u32 { 42 }\npub fn use_answer() -> u32 { answer() }\npub fn broken() { missing(); }\n";
    std::fs::write(&path, text).unwrap();
    let host = SharedBackend::connect(ConnectionOptions {
        workspace: LapceWorkspace { path: directory.path().into(), ..Default::default() },
        local_backend: LocalBackend::Process { executable: proxy.into() },
        language_server_configurations: HashMap::from([("lapce-native-lsp".into(), HashMap::from([("servers".into(), serde_json::json!([{
            "language": "rust", "executable": std::path::PathBuf::from(lsp),
            "options": { "cargo": { "buildScripts": { "enable": false } }, "procMacro": { "enable": false } }
        }]))]))]),
        ..Default::default()
    }).unwrap();
    let connection = host.attach().unwrap();
    let second = host.attach().unwrap();
    let mut second_session = WorkspaceSession::new(RpcExecutor {
        rpc: second.proxy_rpc.clone(),
        timeout: Duration::from_secs(5),
    });
    second_session.open(path.clone()).unwrap();
    let mut session = WorkspaceSession::new(RpcExecutor {
        rpc: connection.proxy_rpc.clone(),
        timeout: Duration::from_secs(5),
    });
    session.open(path.clone()).unwrap();
    let offset = text.find("{ answer()").unwrap() + 2;
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut symbols = false;
    let mut definition = false;
    let mut hover = false;
    let generation = session.backend_generation();
    while Instant::now() < deadline {
        for event in second.events.try_iter() {
            second_session
                .handle_backend_event(second_session.backend_generation(), event)
                .unwrap();
        }
        for event in connection.events.try_iter() {
            if let lapce_rpc::core::CoreNotification::ShowMessage {
                title,
                message,
            } = &event
            {
                eprintln!("{title}: {}", message.message);
            }
            session.handle_backend_event(generation, event).unwrap();
        }
        if let Ok(ProxyResponse::GetDocumentSymbols { resp }) =
            session.navigate(&path, NavigationQuery::Symbols)
        {
            symbols = lapce_engine::navigation::document_symbol_tree(resp)
                .iter()
                .any(|symbol| symbol.name == "answer");
        }
        if let Ok(ProxyResponse::GetDefinitionResponse {
            definition: response,
            ..
        }) = session.navigate(&path, NavigationQuery::Definition { offset })
        {
            definition = lapce_engine::navigation::first_definition(response)
                .is_some_and(|location| location.range.start.line == 0);
        }
        if let Ok(ProxyResponse::HoverResponse {
            hover: response, ..
        }) = session.navigate(&path, NavigationQuery::Hover { offset })
        {
            hover = serde_json::to_string(&response).unwrap().contains("answer");
        }
        let diagnostics = session
            .diagnostics
            .get(&path)
            .is_some_and(|state| !state.items.is_empty());
        if symbols && definition && hover && diagnostics {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(symbols, "live symbols missing");
    assert!(definition, "live definition missing");
    assert!(hover, "live hover missing");
    assert!(
        session
            .diagnostics
            .get(&path)
            .is_some_and(|state| !state.items.is_empty()),
        "live diagnostics missing"
    );
    assert!(session.document(&path).unwrap().content.read_only());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    let corrected = text.replace("missing();", "answer();");
    std::fs::write(&path, &corrected).unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut refreshed = false;
    while Instant::now() < deadline {
        for event in second.events.try_iter() {
            second_session
                .handle_backend_event(second_session.backend_generation(), event)
                .unwrap();
        }
        for event in connection.events.try_iter() {
            session.handle_backend_event(generation, event).unwrap();
        }
        refreshed = session.document(&path).unwrap().buffer.to_string() == corrected
            && session
                .diagnostics
                .get(&path)
                .is_some_and(|state| state.items.is_empty());
        if refreshed {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        refreshed,
        "external file reload did not refresh live diagnostics: content={:?}, diagnostics={:?}",
        session.document(&path).unwrap().buffer.to_string(),
        session.diagnostics.get(&path).map(|state| &state.items)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), corrected);
    let second_generation = second_session.backend_generation();
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        for event in second.events.try_iter() {
            second_session
                .handle_backend_event(second_generation, event)
                .unwrap();
        }
        if second_session.document(&path).unwrap().buffer.to_string() == corrected
            && second_session
                .diagnostics
                .get(&path)
                .is_some_and(|state| state.items.is_empty())
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        second_session.document(&path).unwrap().buffer.to_string(),
        corrected
    );
    assert!(
        second_session
            .diagnostics
            .get(&path)
            .is_some_and(|state| state.items.is_empty())
    );
    assert!(matches!(
        second_session
            .navigate(&path, NavigationQuery::Symbols)
            .unwrap(),
        ProxyResponse::GetDocumentSymbols { .. }
    ));
    second_session.close(&path).unwrap();
    session.close(&path).unwrap();
    host.shutdown_and_wait().unwrap();
}
