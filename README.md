<h1 align="center">
  <a href="https://lapce.dev" target="_blank">
  <img src="extra/images/logo.png" width=200 height=200/><br>
  Lapce
  </a>
</h1>

This fork is a **Linux-only, read-only code browser**. Windows, macOS, and WSL are not supported. SSH connections require a Linux host.

<h4 align="center">Read-only code navigation for Linux</h4>

<div align="center">
  <a href="https://github.com/lapce/lapce/actions/workflows/ci.yml" target="_blank">
    <img src="https://github.com/lapce/lapce/actions/workflows/ci.yml/badge.svg" />
  </a>
  <a href="https://discord.gg/n8tGJ6Rn6D" target="_blank">
    <img src="https://img.shields.io/discord/946858761413328946?logo=discord" />
  </a>
  <a href="https://docs.lapce.dev" target="_blank">
      <img src="https://img.shields.io/static/v1?label=Docs&message=docs.lapce.dev&color=blue" alt="Lapce Docs">
  </a>
</div>
<br/>


Lapce (IPA: /læps/) is written in pure Rust, with a UI in [Floem](https://github.com/lapce/floem). It is designed with [Rope Science](https://xi-editor.io/docs/rope_science_00.html) from the [Xi-Editor](https://github.com/xi-editor/xi-editor), enabling lightning-fast computation, and leverages [wgpu](https://github.com/gfx-rs/wgpu) for rendering. More information about the features of Lapce can be found on the [main website](https://lapce.dev) and user documentation can be found on [GitBook](https://docs.lapce.dev/).

![](https://github.com/lapce/lapce/blob/master/extra/images/screenshot.png?raw=true)

## Features

* Always-read-only source browsing with navigation, selection, copying and search.
* Language-server definitions, symbols, references, hover and diagnostics.
* Read-only repository diffs and automatic refresh after external file changes.
* One root folder per window, Linux SSH connections, and independent frontends sharing a backend.
* Vim-style navigation and an optional terminal for commands in the root folder.

Editing, saving, filesystem mutations and debugger execution are removed from
the code browser. Search and command inputs remain editable.

## Installation

This fork targets Linux. Build it from this checkout; upstream release binaries do not contain these changes.
If you'd like to compile from source, you can find the [guide](docs/building-from-source.md).

The root `lapce` package provides three executables:

- `lapce-floem`: the main Floem desktop frontend (the default for `cargo run`).
- `lapce-iced`: the additional [iced frontend](lapce-iced/README.md).
- `lapce-proxy`: the optional standalone backend executable.

Build all three, then run either frontend:

```sh
cargo build --profile ci
cargo run --profile ci --bin lapce-floem -- /path/to/workspace
cargo run --profile ci --bin lapce-iced -- /path/to/workspace
```

Frontend implementations remain separate workspace library crates; select the
executable from the root package with `--bin`, without a `-p` option.

Both frontends run in the foreground and use an embedded backend by default.
Every invocation starts its own instance. Logs go to stderr and a rotating file;
see [startup and logging](docs/startup-and-logging.md).

Language support uses [native LSP server executables](docs/native-language-servers.md).
Volt/WASM extensions and the extension marketplace have been removed.

## Contributing

Guidelines for contributing to Lapce can be found in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Feedback & Contact

The most popular place for Lapce developers and users is on the [Discord server](https://discord.gg/n8tGJ6Rn6D).

Or, join the discussion on [Reddit](https://www.reddit.com/r/lapce/) where we are just getting started.

There is also a [Matrix Space](https://matrix.to/#/#lapce-editor:matrix.org), which is linked to the content from the Discord server.

## License

Lapce is released under the Apache License Version 2, which is an open source license. You may contribute to this project, or use the code as you please as long as you adhere to its conditions. You can find a copy of the license text here: [`LICENSE`](LICENSE).
