# Startup and logging

A normal launch stays in the process started by the shell or debugger:

```sh
cargo run --bin lapce-floem -- /path/to/workspace
cargo run --bin lapce-iced -- /path/to/workspace
```

Every invocation opens its own instance, including repeated launches of the same
folder. There is no automatic window reuse or detached relaunch, and no `--wait`,
`--new`, or `--reuse-window` option. The application inherits the launch environment.

Each window has exactly one root folder and one backend session. There are no
workspace tabs, workspace switching, or recent-workspace lists.

Both frontends accept multiple paths and open one window. A single folder argument
sets its root. Multiple files or folders use their common parent directory; files
open as editor tabs in Floem or in the open-file row in iced. Directories are
browsable in the explorer. For example:

```sh
cargo run --bin lapce-floem -- Cargo.toml README.md lapce-app lapce-core
cargo run --bin lapce-iced -- Cargo.toml README.md lapce-app lapce-core
```

The repository directory becomes the root, and both files open in that window.
With no paths, the current directory is the root. Old saved workspace lists are
ignored; window geometry and per-root file layouts are retained. Opening another
folder or SSH root from the UI creates a new window. Reload Window rebuilds the
current root's session, and disconnecting an SSH window closes that window.

Default frontend builds run the backend inside the same process. The GUI owns the
main thread, the backend dispatcher runs on `Lapce backend`, and queued logging
runs on `Lapce logs`. Rendering, filesystem watching, transport, terminal handling,
and session adapters still use supporting workers. Configured language servers
and terminal commands remain separate programs.

The frontend and embedded backend exchange framed RPC messages through a connected
Unix socket pair. Backend startup and shutdown do not launch `lapce-proxy`.
Explicit external-process mode uses stdin/stdout pipes, and SSH carries the same
protocol through the remote process's stdin/stdout. Existing named-socket hosts
are still supported; attaching and disconnecting does not stop those hosts.

For Floem, `core.backend-executable` explicitly selects a separate process, and
`core.backend-socket` selects an external socket (taking precedence). Leave both
empty for the embedded default. For iced, use `--proxy /path/to/lapce-proxy` or
`--socket /path/to/backend.sock` to select an external backend.
Process-only frontend library builds remain available with `--no-default-features`;
they require an external backend.

Logs from the frontend and embedded backend use one subscriber and one writer
worker. They appear on stderr and in rotating files under the Lapce logs directory,
with frontend-specific file prefixes. Thread names and IDs appear in records;
startup records also identify the process ID. File logging failure does not
suppress terminal logs. `LAPCE_LOG` controls filtering for both outputs, for example:

```sh
LAPCE_LOG=lapce_app=debug,lapce_client=debug,lapce_proxy=debug,lapce_engine=debug \
  cargo run --bin lapce-floem -- /path/to/workspace
```

Use `RUST_BACKTRACE=1` for panic backtraces. stdout and stderr are inherited;
startup does not redirect them to hidden `stdout.log` or `stderr.log` files.
Standalone proxy logs go to stderr, leaving stdout available for RPC.
