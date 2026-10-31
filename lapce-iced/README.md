# iced frontend

A Linux read-only code browser built with iced 0.14. It uses `lapce-client`
and `lapce-engine::runtime::WorkspaceSession`, with no dependency on the Floem
desktop. The backend is embedded by default through `lapce-client`. Blocking session operations run on a worker
thread; bounded channels carry UI commands and presentation snapshots.

Build the frontend and backend:

```sh
cargo build --profile ci --bin lapce-iced --bin lapce-proxy
target/ci/lapce-iced /absolute/path/to/workspace
```

An initial file can be supplied instead of a directory. The default backend runs on a thread inside the frontend process and communicates
over a connected Unix socket pair. Use `--proxy /path/to/lapce-proxy` to explicitly
launch a separate backend process. Logs are visible on stderr; see
[foreground startup and logging](../docs/startup-and-logging.md).

Expand or collapse folders in the left explorer tree; click a file to open it.
Folders load on expansion, and multiple branches can stay open at once. The
highlighted file is the active document. You can also enter a file path and press Open.
Relative file paths resolve against the workspace root. The workspace
search root remains the initial directory (or the initial file's parent).
Select and navigate text in the code pane; text mutations are discarded.
Search and path inputs remain editable. Search, definition and reference results
open the destination file at its line; symbol results provide the same navigation.
Hover and language responses appear in the bottom pane along with diagnostics.
External file changes refresh the document through the shared engine session.

Language support uses [native LSP servers](../docs/native-language-servers.md);
WASM Volts are not supported. For Rust navigation, supply an actual rust-analyzer executable:

```sh
target/ci/lapce-iced --lsp "$(rustup which rust-analyzer)" /path/to/rust/workspace
```

The standalone frontend currently exposes native Rust LSP configuration only.
Other languages require additional server configuration support. Language actions
may report an error until the language server finishes initializing.

To attach to an existing shared backend, use:

```sh
target/ci/lapce-iced --socket /absolute/path/to/workspace.sock /path/to/workspace
```

Start that host with `lapce-client/examples/shared_host.rs`; the terminal frontend
can attach to the same socket. The host controls the workspace and language-server
configuration in socket mode; `--lsp` configures the embedded backend or an explicitly launched proxy.

This first frontend has a single file pane, basic directory browsing and plain
monospace text. It does not yet provide syntax highlighting, navigation history,
desktop settings/persistence, SSH selection, terminal UI or backend restart.
It demonstrates reuse of the shared engine; desktop orchestration consolidation
and complete feature parity remain separate work.

Validation:

```sh
cargo test --profile ci -p lapce-iced
LAPCE_TEST_PROXY="$PWD/target/ci/lapce-proxy" \
LAPCE_TEST_LSP="$(rustup which rust-analyzer)" \
cargo test --profile ci -p lapce-iced -- --include-ignored
python3 scripts/check-architecture.py --resolved
```

The integration test drives the same worker used by iced against a real proxy:
Unicode paths/content, directory listing, open, workspace search, result-line
navigation, optional live definition lookup, watcher reload, close and shutdown.
Unit tests cover Unicode cursor offsets and rejection of code-edit actions.
