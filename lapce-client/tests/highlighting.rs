#![cfg(feature = "embedded-backend")]
use lapce_client::{
    BackendConnection, ConnectionOptions, LocalBackend,
    shared::{SharedBackend, SharedFrontend},
    workspace::LapceWorkspace,
};
use lapce_rpc::{
    buffer::BufferId,
    core::CoreNotification,
    highlighting::{HighlightSnapshot, HighlightStatus, TextSnapshot},
    proxy::{ProxyRequest, ProxyResponse, ProxyRpcHandler},
};
use lapce_xi_rope::{Delta, Interval, Rope};
use std::{
    path::Path,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

fn options(root: &Path) -> ConnectionOptions {
    ConnectionOptions {
        workspace: LapceWorkspace {
            path: root.into(),
            ..Default::default()
        },
        local_backend: std::env::var_os("LAPCE_TEST_PROXY")
            .map(|path| LocalBackend::Process {
                executable: path.into(),
            })
            .unwrap_or_default(),
        ..Default::default()
    }
}
fn call(rpc: &ProxyRpcHandler, request: ProxyRequest) -> ProxyResponse {
    rpc.request_timeout(request, Duration::from_secs(10))
        .unwrap()
}
fn open(rpc: &ProxyRpcHandler, path: &Path, id: BufferId) -> (u64, TextSnapshot) {
    let ProxyResponse::NewBufferResponse { rev, snapshot, .. } = call(
        rpc,
        ProxyRequest::NewBuffer {
            path: path.into(),
            buffer_id: id,
        },
    ) else {
        panic!("open response")
    };
    assert!(!snapshot.instance.is_empty());
    (rev, snapshot)
}
fn ready(
    rpc: &ProxyRpcHandler,
    events: &Receiver<CoreNotification>,
    path: &Path,
) -> HighlightSnapshot {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let ProxyResponse::HighlightSnapshot { highlights } = call(
            rpc,
            ProxyRequest::GetHighlightSnapshot { path: path.into() },
        ) else {
            panic!("highlight response")
        };
        if highlights.status != HighlightStatus::Pending {
            return highlights;
        }
        assert!(Instant::now() < deadline, "timed out waiting for syntax");
        let _ = events.recv_timeout(Duration::from_millis(50));
    }
}

#[test]
fn backend_publishes_embedded_rust_python_and_plaintext_without_asset_setup() {
    let root = tempfile::tempdir().unwrap();
    let connection = BackendConnection::connect(options(root.path())).unwrap();
    for (name, source, supported) in [
        (
            "a.rs",
            "// café\r\nfn main() { let greeting = \"hello🌍\"; }\r\n",
            true,
        ),
        (
            "a.py",
            "# café\r\ndef greet():\r\n    return \"hello🌍\"\r\n",
            true,
        ),
        ("a.js", "const hello = '🌍';", false),
    ] {
        let path = root.path().join(name);
        std::fs::write(&path, source).unwrap();
        let id = BufferId::next();
        let (_, snapshot) = open(&connection.proxy_rpc, &path, id);
        let result = ready(&connection.proxy_rpc, &connection.events, &path);
        assert_eq!(result.snapshot, snapshot);
        assert!(result.valid_for(&snapshot, source));
        if supported {
            assert_eq!(result.status, HighlightStatus::Ready);
            for scope in ["comment", "keyword", "string", "function"] {
                assert!(
                    result.spans.iter().any(|span| span.scope == scope),
                    "{name} missing {scope}"
                );
            }
            assert!(!result.structure.is_empty());
        } else {
            assert_eq!(result.status, HighlightStatus::Unsupported);
            assert!(result.spans.is_empty());
        }
        call(
            &connection.proxy_rpc,
            ProxyRequest::CloseBuffer {
                path: path.clone(),
                buffer_id: id,
            },
        );
        assert!(
            connection
                .proxy_rpc
                .request_timeout(
                    ProxyRequest::GetHighlightSnapshot { path },
                    Duration::from_secs(10)
                )
                .is_err()
        );
    }
    connection.shutdown_and_wait().unwrap();
}

#[test]
fn shared_frontends_reuse_results_and_keep_authoritative_identity_through_reload() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("shared.rs");
    let original = "fn first() { let café = \"before\"; }\n";
    std::fs::write(&path, original).unwrap();
    let backend = SharedBackend::connect(options(root.path())).unwrap();
    let first = backend.attach().unwrap();
    let id = BufferId(0);
    let (revision, snapshot) = open(&first.proxy_rpc, &path, id);
    let result = ready(&first.proxy_rpc, &first.events, &path);
    let second: SharedFrontend = backend.attach().unwrap();
    let (_, other_snapshot) = open(&second.proxy_rpc, &path, id);
    assert_eq!(snapshot, other_snapshot);
    assert_eq!(result, ready(&second.proxy_rpc, &second.events, &path));

    let changed = "fn later() { let café = \"after!\"; }\n";
    std::fs::write(&path, changed).unwrap();
    let update = |rpc: &ProxyRpcHandler, revision| {
        call(
            rpc,
            ProxyRequest::UpdateBuffer {
                path: path.clone(),
                buffer_id: id,
                rev: revision,
                delta: Delta::simple_edit(
                    Interval::new(0, original.len()),
                    Rope::from(changed),
                    original.len(),
                ),
            },
        )
    };
    let ProxyResponse::BufferUpdated {
        snapshot: updated, ..
    } = update(&first.proxy_rpc, revision + 1)
    else {
        panic!("update response")
    };
    assert_eq!(snapshot.instance, updated.instance);
    assert_ne!(snapshot, updated);
    let changed_result = ready(&first.proxy_rpc, &first.events, &path);
    assert_eq!(changed_result.snapshot, updated);
    assert!(changed_result.valid_for(&updated, changed));
    assert!(!result.valid_for(&updated, changed));
    let ProxyResponse::BufferUpdated {
        snapshot: second_updated,
        ..
    } = update(&second.proxy_rpc, revision + 1)
    else {
        panic!("second update response")
    };
    assert_eq!(updated, second_updated);
    assert_eq!(
        changed_result,
        ready(&second.proxy_rpc, &second.events, &path)
    );

    for frontend in [&first, &second] {
        call(
            &frontend.proxy_rpc,
            ProxyRequest::CloseBuffer {
                path: path.clone(),
                buffer_id: id,
            },
        );
    }
    let (_, reopened) = open(&first.proxy_rpc, &path, id);
    assert_ne!(reopened.instance, snapshot.instance);
    drop(first);
    drop(second);
    backend.shutdown_and_wait().unwrap();
}

#[test]
fn delayed_semantic_results_are_bound_to_their_original_text_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("semantic.rs");
    let original = "fn first() {}\n";
    let changed = "fn later() {}\n";
    std::fs::write(&path, original).unwrap();
    let mut configuration = options(root.path());
    configuration.language_server_configurations = std::collections::HashMap::from(
        [(
            "lapce-native-lsp".into(),
            std::collections::HashMap::from([(
                "servers".into(),
                serde_json::json!([{
                    "language": "rust",
                    "executable": "/usr/bin/python3",
                    "arguments": [format!("{}/tests/fixtures/highlight_lsp.py", env!("CARGO_MANIFEST_DIR")), root.path().display().to_string()]
                }]),
            )]),
        )],
    );
    let connection = BackendConnection::connect(configuration).unwrap();
    let id = BufferId::next();
    let (revision, _) = open(&connection.proxy_rpc, &path, id);
    let initial = ready(&connection.proxy_rpc, &connection.events, &path);
    assert!(
        initial
            .spans
            .iter()
            .any(|span| span.start == 0 && span.scope == "keyword")
    );
    let wait_file = |name: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !root.path().join(name).exists() {
            assert!(Instant::now() < deadline, "fixture did not reach {name}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    wait_file("semantic-1.pending");
    std::fs::write(&path, changed).unwrap();
    let ProxyResponse::BufferUpdated {
        snapshot: current, ..
    } = call(
        &connection.proxy_rpc,
        ProxyRequest::UpdateBuffer {
            path: path.clone(),
            buffer_id: id,
            rev: revision + 1,
            delta: Delta::simple_edit(
                Interval::new(0, original.len()),
                Rope::from(changed),
                original.len(),
            ),
        },
    )
    else {
        panic!("reload response")
    };
    let syntax = ready(&connection.proxy_rpc, &connection.events, &path);
    assert_eq!(syntax.snapshot, current);
    std::fs::write(root.path().join("semantic-1.release"), "").unwrap();
    wait_file("semantic-2.pending");
    let after_stale = ready(&connection.proxy_rpc, &connection.events, &path);
    assert_eq!(
        after_stale, syntax,
        "retired semantic response must not affect current colors"
    );
    std::fs::write(root.path().join("semantic-2.release"), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let enriched = ready(&connection.proxy_rpc, &connection.events, &path);
        if enriched.generation > syntax.generation {
            assert_eq!(enriched.snapshot, current);
            assert!(enriched.spans.iter().any(|span| span.start == 0
                && span.end == 2
                && span.scope == "function"));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "semantic enrichment did not arrive"
        );
        let _ = connection.events.recv_timeout(Duration::from_millis(25));
    }
    // A later semantic request fails; the next syntax generation remains usable.
    std::fs::write(&path, original).unwrap();
    call(
        &connection.proxy_rpc,
        ProxyRequest::UpdateBuffer {
            path: path.clone(),
            buffer_id: id,
            rev: revision + 2,
            delta: Delta::simple_edit(
                Interval::new(0, changed.len()),
                Rope::from(original),
                changed.len(),
            ),
        },
    );
    wait_file("semantic-3.done");
    let fallback = ready(&connection.proxy_rpc, &connection.events, &path);
    assert_eq!(fallback.status, HighlightStatus::Ready);
    assert!(
        fallback
            .spans
            .iter()
            .any(|span| span.start == 0 && span.scope == "keyword")
    );
    connection.shutdown_and_wait().unwrap();
}

#[test]
fn language_switches_and_hover_blocks_are_computed_by_the_backend() {
    use lapce_rpc::highlighting::CodeBlock;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("example.rs");
    std::fs::write(&path, "fn main() { let value = 42; }\n").unwrap();
    let connection = BackendConnection::connect(options(root.path())).unwrap();
    let (_, identity) = open(&connection.proxy_rpc, &path, BufferId::next());
    let rust = ready(&connection.proxy_rpc, &connection.events, &path);
    call(
        &connection.proxy_rpc,
        ProxyRequest::SetSyntaxLanguage {
            path: path.clone(),
            language: "PlainText".into(),
        },
    );
    let plain = ready(&connection.proxy_rpc, &connection.events, &path);
    assert_eq!(plain.snapshot, identity);
    assert_eq!(plain.status, HighlightStatus::Unsupported);
    assert!(plain.spans.is_empty());
    assert!(plain.generation > rust.generation);
    call(
        &connection.proxy_rpc,
        ProxyRequest::SetSyntaxLanguage {
            path: path.clone(),
            language: "Rust".into(),
        },
    );
    let restored = ready(&connection.proxy_rpc, &connection.events, &path);
    assert_eq!(restored.spans, rust.spans);
    assert!(restored.generation > plain.generation);
    let requested = vec![CodeBlock {
        language: "Python".into(),
        text: "def greet():\n    return 42\n".into(),
    }];
    let ProxyResponse::CodeHighlights { blocks } = call(
        &connection.proxy_rpc,
        ProxyRequest::HighlightCode {
            blocks: requested.clone(),
        },
    ) else {
        panic!("code block response")
    };
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].code, requested[0]);
    for scope in ["function", "keyword", "number"] {
        assert!(
            blocks[0].spans.iter().any(|span| span.scope == scope),
            "missing {scope}"
        );
    }
    connection.shutdown_and_wait().unwrap();
}

#[test]
fn large_parse_does_not_block_buffer_requests_and_reload_supersedes_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large.rs");
    let original = format!(
        "fn large() {{\n{}\n}}",
        "let value = call([1, 2, 3]);\n".repeat(10_000)
    );
    std::fs::write(&path, &original).unwrap();
    let connection = BackendConnection::connect(options(root.path())).unwrap();
    let id = BufferId::next();
    let (revision, _) = open(&connection.proxy_rpc, &path, id);
    for _ in 0..5 {
        connection
            .proxy_rpc
            .request_timeout(
                ProxyRequest::GetBuffer { path: path.clone() },
                Duration::from_secs(2),
            )
            .expect("syntax must not block backend buffer service");
    }
    let replacement = "fn current() {}\n";
    std::fs::write(&path, replacement).unwrap();
    let ProxyResponse::BufferUpdated { snapshot, .. } = call(
        &connection.proxy_rpc,
        ProxyRequest::UpdateBuffer {
            buffer_id: id,
            path: path.clone(),
            rev: revision + 1,
            delta: Delta::simple_edit(
                Interval::new(0, original.len()),
                Rope::from(replacement),
                original.len(),
            ),
        },
    ) else {
        panic!("update response")
    };
    let result = ready(&connection.proxy_rpc, &connection.events, &path);
    assert_eq!(result.snapshot, snapshot);
    assert!(result.valid_for(&snapshot, replacement));
    assert!(result.spans.iter().any(|span| span.scope == "function"));
    connection.shutdown_and_wait().unwrap();
}
