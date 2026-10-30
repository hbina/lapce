# Native language servers

Lapce runs installed language-server executables using LSP over stdin and stdout.
The proxy owns those processes and routes language results to each frontend.
There is no Volt marketplace, package installation, WASM runtime, or general
executable-extension protocol.

For the Floem desktop, add this to your user settings or workspace
`.lapce/settings.toml`, using the absolute path to an installed executable:

```toml
[[lapce-native-lsp.servers]]
language = "rust"
executable = "/absolute/path/to/rust-analyzer"
arguments = []
options = { cargo = { buildScripts = { enable = false } }, procMacro = { enable = false } }
```

Each entry selects a language, an executable, optional arguments, and optional
LSP initialization options. Add another `[[lapce-native-lsp.servers]]` entry for
another server. Reopen the workspace after changing server configuration.
Executables must already be installed and executable on the backend host; Lapce
does not download them or change their permissions. For SSH workspaces, use paths
on the remote host.

The iced frontend exposes a Rust server through its `--lsp` argument:

```sh
cargo build --bin lapce-iced --bin lapce-proxy
cargo run --bin lapce-iced -- --lsp "$(rustup which rust-analyzer)" /path/to/workspace
```

With a shared backend socket, configure language servers in the host process;
attached frontends use that host's configuration. Use matching frontend and proxy
builds when upgrading, including remote and shared backend hosts.

Old WASM Volts cannot run. Their settings and disabled-package lists are unused.
Retired plugin panels and extension details tabs are discarded when saved layouts
are loaded. Built-in themes and local color themes remain; theme packages from
Volts are no longer discovered.
