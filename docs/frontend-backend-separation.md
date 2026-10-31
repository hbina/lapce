# Frontend/backend separation

## Current product direction

The user has refocused Lapce as an always-read-only, navigable code browser with
multiple UI frontends. Editing and save actions must be removed, not offered
behind a toggle. Debugging must be removed. Preserve file navigation, search,
symbols, references, hover, diagnostics and read-only diffs. The editor/debug
extraction notes below describe the earlier implementation and are historical
where they conflict with this direction.

The Volt/WASM extension system has been removed. The proxy now launches only
configured native LSP servers; it has no Wasmtime/WASI dependencies, extension
marketplace, package loader, or plugin process-execution protocol. Desktop
extension UI and persistence have been removed, with saved-layout migration for
retired panels and tabs. See [native language-server configuration](native-language-servers.md).
Historical notes below about Volts, WASI, PSP, and extension management no longer
describe supported functionality.

Both GUI frontends now stay in the launching process and default to a backend
thread connected over a Unix socket pair. CLI detaching, `--wait`, `--new`, window
reuse, and the old desktop-routing socket have been removed. Logging is initialized
once per process, with one queued worker writing stderr and rotating files.
Explicit process, shared-socket, and SSH backend transports remain supported.
See [startup and logging](startup-and-logging.md). Historical startup notes below
that describe detached launches or iced's process default no longer apply.

Each desktop window now owns one root folder and one backend session. Workspace
tabs, switching, recent-workspace palettes, timestamps, and saved workspace lists
have been removed. CLI paths resolve to a single common root; a launch with no
paths uses the current directory. Other folders and SSH roots open independent
windows. Internal workspace types now describe only a root and transport; LSP
workspace operations and per-root layout caching remain. Historical notes below
about multiple workspace tabs or their persistence no longer apply.

The architectural goal is independence from the frontend UI library, not a
requirement to run separate processes. Embedding the backend remains a valid
deployment option if communication stays behind the queue/protocol boundary.
This is an initial architecture checkpoint, not a complete frontend/backend
split: desktop and headless adapters still coordinate some application behavior
separately. Completion requires a different UI library to use the same application
and session API without importing Floem or copying orchestration from the desktop.
Shared-host notification routing, terminal ownership, cancellation and backend
restart also need consolidation before claiming complete parity across frontends.

`lapce-iced` now provides an additional Linux GUI adapter using iced rather than
Floem. It calls `WorkspaceSession` for directory browsing, document loading,
workspace search, navigation and backend events. A worker thread schedules these
operations and sends presentation snapshots to iced; document ownership and
synchronization remain in the engine. It supports a separate proxy process or
an existing shared-host socket. Architecture checks reject Floem/desktop imports
and backend embedding in its resolved dependencies. See its
[run instructions and limitations](../lapce-iced/README.md). This is concrete
evidence of another GUI library consuming shared behavior, while the desktop's
separate orchestration and full frontend parity remain unfinished.

The remaining desktop and engine debugger controllers have now been removed:
Run/Debug palette and workbench commands, titlebar launcher, task configuration
loading, terminal prelaunch/DAP state, breakpoint persistence and editor break-line
highlighting. Ordinary terminals use the shared plain terminal lifecycle. Older
workspace files still deserialize their layout while discarding saved breakpoints;
legacy debugger wire messages are decoded and ignored or rejected. The backend
cannot launch a debugger. Tests for deleted debugger controllers were removed
with the implementation; the command rejection and workspace migration regressions
cover the current product behavior.

The backend now denies editor, save, filesystem mutation, repository mutation
and debugger requests. Open/snapshot responses advertise read-only content.
Buffer synchronization accepts only text matching the current disk file so
external changes can still propagate to language services. The desktop command
palette excludes mutating edit commands and common write/debug workbench
actions; direct invocation of write/debug workbench and save/format/rename focus
actions is also blocked. Local search inputs retain their editing key bindings. Full
removal of their UI surfaces, obsolete controllers/protocols, tests and examples
remains underway. The earlier debug extraction is superseded by removal.

The source-control panel now only lists changed files and opens read-only diffs.
Commit-message editing, commit buttons, selection checkboxes and discard menus
have been removed, along with the engine commit/repository-action controllers.
`WorkspaceActions` now coordinates search and read-only repository metadata.
File-explorer context menus retain reveal, copy-path, compare and refresh actions;
creation, rename, duplication and trash actions have been removed. The native
File menu no longer offers New File, Save or Save All.

The headless `WorkspaceSession` no longer exposes edit, save, resource mutation
or mutable document access. It enforces read-only file content independently of
the backend's advertised flag. Disk changes still synchronize to language
services, including restoration after deletion: read-only documents always load
the latest disk snapshot. Legacy lower-level editor modules remain to be removed
as their desktop adapters are retired. The full engine test suite passes after
this session API cleanup.

Directory browsing now uses `lapce-engine::directory::DirectoryQuery` in the
desktop explorer and the headless `WorkspaceSession::list_directory` API. It
validates response ownership, rejects duplicate/non-child entries, and sorts
directories first with natural filename ordering. The terminal example exposes
this API through `ls`. Two dedicated engine tests pass and both frontends compile.
Explorer tree expansion, exclusions and row projection still live in the GUI.
Directory child reconciliation now lives in the engine as well, through a small
`DirectoryNode` adapter. The desktop retains its row state while shared logic
preserves unchanged nodes, removes missing paths, resets file/directory type
changes and returns loaded subdirectories to refresh. Three directory tests pass;
the desktop compiles with this adapter.
The desktop ignores directory results for missing/non-directory nodes and starts
child refreshes after releasing the tree mutation. A GUI adapter test verifies
that a directory-to-file change drops expanded descendants and recalculates row
counts. A process-backed terminal smoke test verified `ls`, directory-first
natural ordering, Unicode file display, unchanged disk content and clean shutdown.
The shared `OpenDocument` policy now forces read-only content even when a backend
advertises a writable file. It no longer carries unsaved editing content through
the opening API. Desktop file documents are read-only from creation, and opening
no longer restores historical unsaved edits over disk content. Dedicated opening
tests cover writable/read-only backend flags and stale response rejection.
The blocking workspace session now executes this same opening plan rather than
duplicating its request/response policy. Desktop file-open callers no longer pass
unsaved content. Runtime coverage verifies malformed opening responses leave no
owned document and allow a successful retry. Opening, navigation and runtime
tests pass; desktop and terminal frontend compile checks pass.
Serializable document location state now lives in
`lapce-engine::view_state::DocumentViewState`; desktop restoration is a GUI
extension adapter. Capturing a file location forces read-only content and drops
unsaved text while preserving cursor, scroll and confirmation state. The old
serialization schema remains readable. Legacy scratch-content compatibility is
still present and must be retired with scratch editing. Two persistence tests
cover navigation roundtrips and normalization of old file state.
Normalization is now enforced during serialization and deserialization, as well
as capture, so legacy writable flags and unsaved file text cannot bypass shared
policy through direct construction or an old session file. Three persistence
tests cover all three boundaries; the desktop compiles with the unchanged schema.
The terminal frontend now drains backend events through `WorkspaceSession` while
waiting for input, rather than leaving filesystem/status/diagnostic publications
unread. Input runs on a separate thread; session ownership stays on the frontend
loop. `status` and `diagnostics` expose shared event-driven state. A real-proxy
smoke test changed a file externally while the frontend was idle and verified
that `show` displayed its new Unicode content with no reopen or disk mutation,
followed by clean shutdown. Live language-server diagnostics remain unverified.
Desktop editor save/format methods and the Save All implementation are removed,
along with the file-tab Save button. Shared view-state policy now excludes file
tabs from unsaved-edit close confirmations, including externally deleted files.
Legacy scratch save dialogs and document-level save machinery still remain and
must be retired. Four view-state tests cover persistence and close policy.
Scratch save dialogs, Save As dispatch and their desktop document adapters are
now removed. The unused engine Save As module and its mutation-oriented tests
are removed as well. Existing dirty scratch tabs still require confirmation
before closing and dropping their stored content. Document-level save machinery
and legacy internal command variants remain for subsequent cleanup.

Validation for this cleanup: all 30 desktop library tests and 13 targeted engine
tests passed; the terminal example compiles and `git diff --check` passes. Live
GUI interaction and macOS-specific menu compilation remain unverified.

This refactor is in progress. The GUI remains usable while application behavior
is extracted into APIs that other frontends can call. The complete target is a
headless editor/workspace application, not just a reusable filesystem proxy.

## Implemented boundaries

- `lapce-core`: text-editing primitives and syntax support. Its
  `floem-editor-core` dependency is a headless text library, not the Floem GUI.
- `lapce-engine`: document identity/content, a plain document model, insertion
  and edit-command behavior, snippet parsing, LSP edit conversion, document
  search, and revision-aware pristine-state handling. Completion candidate
  filtering, snippet previews and inline-completion request/revision tracking
  also live here. The Floem adapter uses these shared controllers and keeps
  selection signals, view geometry and rendering in the GUI.
  The GUI document owns one engine `Document`; buffer, syntax and content signals
  are borrowed reactive projections rather than separate application models.
  Loading, host reloads, raw edits and LSP edits use engine operations too.
  Workspace edits retain ordered document/resource operations, versions and
  annotations in an engine plan. A host interface executes the plan without
  views. Document changes validate versions, UTF-16 ranges and overlapping edits
  before mutation. The GUI applies text changes without navigating editor tabs,
  waits for document loads in operation order, and reports load/apply failures.
  The GUI uses engine `WorkspaceEditExecution` for asynchronous operation order,
  completion identity and failure indices. Loading a document holds the current
  effect; duplicate, stale and foreign completions cannot advance execution.
  Engine preflight validates capabilities and file URI schemes before any effect.
  Hosts supply explicit approved annotation IDs. The GUI displays required labels
  and descriptions and starts execution only after a one-shot Apply action;
  Cancel dispatches no edit. The GUI dispatches resource effects through the same
  ordered executor and validates backend responses. Operations affecting open
  documents still fail at the backend ownership guard; multi-document close,
  retarget/reopen and restoration coordination remain unfinished.
  Search input/options, compiled queries, validation errors, revisions and
  next/previous navigation now belong to an engine `SearchState`. GUI fields
  project that model through the same generic reactive adapter used by documents.
  Immutable engine search jobs carry document/query revisions; stale background
  results cannot overwrite a newer query or document. Invalid regexes retain
  their input and report an error rather than falling back to literal matches.
  Inlay hints are engine-document state and transform during engine edits.
  Diagnostic/inlay span construction and completion-preview transformations are
  shared annotation APIs. Completion positions use each edit's before-text and
  consume matching Unicode prefixes without slicing through a character.
  Modal range editing also uses the engine, including read-only enforcement and
  cursor history for undo.
  Syntax job creation, cancellation, delta-to-edit conversion, parsing and
  result acceptance now live in the engine. Results are checked against document
  identity, text revision and job generation, so same-revision language changes
  cannot be overwritten by an older parse. The GUI supplies an executor and
  refreshes rendering caches only after engine acceptance.
  Diagnostic payloads, version validation, UTF-16 mapping, edit transformations,
  severity filtering and counts live in engine `DiagnosticState` models shared
  by document/workspace consumers. GUI adapters retain panel expansion and
  reactive notifications. Unloaded documents defer offset mapping; versioned
  publications for an open document must match its revision.
  Source-control branch/tag metadata, changed-file selection, diff refreshes and
  commit validation, pending-operation ownership and response handling live in
  engine `SourceControlState`. Commit uses acknowledged RPC; failures retain the
  message, duplicate actions are suppressed, and success clears only unchanged
  message text.
  The GUI projects that model and supplies the message from its editor widget.
  Search, commit and repository-action completion routing now share an engine
  `WorkspaceActions` controller with the headless session. The GUI supplies
  asynchronous scheduling and presentation; document coordination remains open.
  Run/debug configuration and breakpoint persistence, active-entry request
  construction and edit transformations live in engine `BreakpointStore`.
  The GUI shares that model through a reactive adapter. Each delta maps against
  its own before/after text, including multi-edit batches and Unicode text.
  Engine `DebugSession` owns paused/thread/frame state and source-location
  selection. Stop generations invalidate variable responses after another stop
  or resume, even when the adapter reuses variable references. The GUI retains
  stack/variable tree presentation and navigation effects. Scope payloads and
  variable caches now belong to the engine; transport-independent request effects
  deduplicate pending references and carry session/stop/request identities.
  Failed loads can retry, and stale/duplicate replies cannot replace cache data.
  Expanded GUI aliases receive the same cached result. Continue, pause, step,
  stop and restart use one engine command API that validates session state,
  selects the thread and builds RPC effects. Commands preserve confirmed state
  until backend events arrive. Run/debug launch planning, configuration expansion,
  terminal reuse and prelaunch-exit transitions also use engine APIs. Expansion
  takes workspace/executable paths from the host rather than reading GUI state.
  Prelaunch must exit with an explicit zero status before the main program or
  debugger starts. GUI adapters still create/focus terminal views and dispatch
  effects; session transport and process ownership remain to be extracted.
- `lapce-client`: workspace descriptors, connection setup, ordered backend
  events, local embedded/process execution, SSH/WSL provisioning, and process
  transport. The `embedded-backend` feature is optional. With it disabled, a
  client can use an external backend without linking the proxy implementation.
- `lapce-services`: shared host HTTP, CLI, environment and plugin-package
  services. This separates those utilities from the Wasmtime/LSP/DAP runtime.
- `lapce-rpc`: protocol DTOs, request tracking and newline-delimited JSON.
  Directory listing responses contain only paths and entry kinds. Explorer
  expansion, naming and visible-row state belong to the frontend model.
- `lapce-proxy`: machine-side filesystem, Git, PTY, plugin, LSP and DAP services.
- `lapce-app`: Floem views and adapters. It has no direct proxy dependency or
  implementation imports. Workspace startup and transport live in the client.

Connections expose an ordered `Receiver<CoreNotification>` including terminal
bytes. The GUI alone converts these events to Floem signals and terminal paint
notifications. Callers must shut down connections; short-lived headless programs
can use `shutdown_and_wait` to reap backend processes before exiting.

Engine `SessionDocument` owns both the document and its backend synchronization
queue. GUI buffer/content/annotation/loading and synchronization projections now
share this session owner. Initialization, edit-batch revision assignment and
conflict recovery use its APIs; recovery reads the owned current text in the same
mutation rather than borrowing separate frontend signals. Transport callbacks
and multi-document resource coordination still need session orchestration.

Document loaded/error state belongs to the engine document. Loading file text
marks it ready and clears errors; late failures cannot downgrade ready documents.
Local and scratch documents start ready. GUI loading observers project this state
and are notified after engine mutations, with no duplicate GUI loading fields.

Engine `OpenDocument` captures opening identity, path and text revision, builds
the backend request and validates its response. The GUI delegates to this policy;
delayed responses cannot overwrite an edited, moved or replaced document, and
unsaved session text is restored only for writable files.
Engine `WorkspaceDocuments` owns file/scratch collections and document reuse per
path. The GUI projects both collections through reactive adapters and delegates
file lookup/construction and scratch naming to the shared registry. The registry
accepts plain engine documents or host-observable document wrappers. Loading
notifications, backend dispatch and close orchestration remain in the GUI.

Opening an already-open path is idempotent for the same buffer ID: it returns
the current backend text and revision without reloading disk or publishing a
second language-server open. Another buffer ID cannot replace that document.
Acknowledged edit, save and close requests require that buffer ID and reject
another document's ID before mutation, disk writes or closing. This changes
those request payloads; external callers must include `buffer_id`. The legacy
unacknowledged update notification has been removed; frontends must use
acknowledged edits through the engine queue.

The protocol adds acknowledged `update_buffer`, `get_buffer` snapshots and
`close_buffer` operations. Opening returns the server revision. Invalid revisions,
unknown documents and malformed delta copy ranges produce errors rather than
panicking or notifying language servers about rejected changes. Document closing
also unregisters file watching and emits LSP `didClose`. Unacknowledged document update notifications are no longer supported.
Opened GUI file documents and the terminal example now share engine `DocumentSync`:
edits, saves and owned closing are ordered. Queuing a close rejects later
operations, and a document becomes closed only after backend acknowledgement.
Failed closes retain their state for explicit recovery. GUI close callbacks report
success, failure and cancellation; paused/disconnected close attempts fail immediately. The terminal frontend and GUI cache removal use this shared close queue.
Filesystem deletion no longer destroys backend buffers: open/dirty GUI documents
remain available, and unused pristine cache entries are removed only after owned
close acknowledgement. Views opened while closing keep the same session document and use engine
`ReopenDocument` after acknowledgement. Reopening preserves current local text,
revision and undo history, and reconciles backend memory through the edit queue.
Ownership generations reject stale reopen responses; read-only conflicts preserve
local state and fail explicitly. A reopened disk baseline drives shared pristine
and save policy, including undo back to disk text, without resetting the buffer. General session shutdown and
view-independent close policy still need consolidation.
Updates and saves are ordered, only acknowledged revisions advance, and saves
cannot overtake earlier edits. Failed updates pause the queue and preserve local
text. The GUI offers explicit recovery using a validated backend snapshot; choosing
local text replaces the backend's in-memory buffer without saving to disk and
cancels queued saves. Background document annotations wait for the queue to drain.
Scratch Save As uses an engine snapshot plan, preserves edits made while saving,
then joins the same acknowledged queue. Only the submitted revision becomes
pristine. The workspace replaces its scratch registration with the file identity.
Legacy GUI update/save fallbacks have been removed. Backend Save As registers
watching and LSP state only on success and rejects another open document's path.
Backend/local revision mapping covers diagnostics and versioned workspace edits,
including publications preceding their update acknowledgement.
The queue also schedules document language requests and completion notifications
behind prior edit acknowledgements. Queries detach once sent, so a slow language
server does not hold up later edits; replies for superseded local revisions are
rejected. Completion, hover, navigation, code actions, formatting and autosave
formatting use this API. Background annotations share the same request path.
Rename plans validate source identity/revision and UTF-16 positions from preparation
through confirmation. Save eligibility, scratch Save As decisions, normalization
policy, autosave deadline/revision tickets and formatting-response/timeout gates
live in the engine. A response or timeout can finish an operation only once;
manual saves can save current text without stale formatting, while autosave skips
superseded revisions. GUI adapters provide timers and preserve per-view cursor
behavior when applying edits to the original document.
Workspace-wide and follow-up metadata requests, request
timeouts/cancellation and session-owned executors still need extraction.
Window/tab IDs no longer identify backend sessions or appear in the client
initialization API. Ignored, optional wire placeholders remain so the GUI can
still initialize older proxies.

Client initialization is queued before `connect()` exposes its request handler,
so an immediate request cannot overtake workspace setup.
Synchronous requests have an explicit timeout option. Disconnects fail outstanding
requests. An RPC EOF terminates the reader. These are still native transports;
a browser requires a server gateway rather than running the PTY/plugin backend
inside browser WebAssembly.

The engine converts LSP resource operations into neutral `ResourceOperation`
requests. The backend delegates filesystem execution to `lapce-services`, which
preserves overwrite/ignore precedence, recursive deletion and missing-resource
options. Symlink deletion leaves its target intact. Affected open backend buffers
are protected, including directory descendants and existing canonical path
aliases. The GUI dispatches these resource requests through the shared execution
queue. Create operations now close affected document owners and reopen them
against the new disk baseline while preserving local text and history. Delete
operations also close and reopen affected owners, retaining local text and undo.
The engine marks successfully deleted files as missing: scheduled and pending
autosave cannot recreate them, while explicit save can recreate even an empty
file. Failed or ignored operations do not mark documents deleted. Successful
create or rename clears the missing-file state.
Engine rename planning captures affected document identities and descendant path
mappings, rejects open destination collisions, and retains original reopen paths
for ignored or failed operations. The engine resource coordinator sequences closes,
the filesystem effect and reopening. A close failure restores previously closed
documents; failed or ignored filesystem effects reopen original paths. A reopening
failure does not prevent the remaining documents from reopening. Tickets reject
duplicate, foreign and wrong-stage completions. Workspace edits and explorer rename drive these effects using
the same document handles, preserving history and unsaved text. The unused
legacy RenamePath RPC has been removed; it previously retargeted backend buffers
without updating their watcher and language-server lifecycle. This adapter is
compile checked; full GUI runtime rename coverage remains outstanding.

## A second frontend

The terminal example is now a read-only consumer of the workspace engine. It
supports `show`, `open <path>`, `search <pattern>`, `symbols`,
`definition/references/implementation/hover <byte-offset>` and `quit`. It uses an external
proxy process and contains no Floem imports or edit/save/resource commands.

```sh
cargo build --bin lapce-proxy --profile ci
cargo run -p lapce-engine --profile ci --example terminal -- \
  "$PWD/target/ci/lapce-proxy" /absolute/path/to/file
```

This is a browsing integration example, not a full terminal UI. Independently
launched local proxy processes use `--no-cli-socket` to avoid claiming the desktop
frontend's open-path routing socket.

Use a backend built from the same source revision. Protocol version/capability
negotiation and interoperability with older released proxies remain unfinished.

## Remaining work toward the read-only frontend/backend split

### Current completion audit (Linux-only scope)

Windows, macOS and WSL support were explicitly removed from product scope by
user request. They are no longer validation blockers. This fork and its remote
backends support Linux only; non-Linux builds fail with a clear compile error.
CI and release workflows target Linux. Native Windows/macOS branches, dependencies,
packaging, keymaps, updater paths and WSL connections were removed.

| Original gap | Current implementation and evidence |
| --- | --- |
| Desktop orchestration | Desktop and headless runtime share opening, close, reopen, disk reload and navigation transitions; rendering/scheduling are frontend adapters. Engine and desktop adapter regressions cover ownership and stale callbacks. |
| Desktop replacement | The desktop uses `ManagedBackend`; real-process replacement and Linux GUI restart passed. |
| Live language server | rust-analyzer navigation, diagnostics, external reload, shared sessions and definition navigation after managed replacement passed through queues. |
| GUI process mode | Linux rendering, read-only input, external reload, backend replacement and definition navigation were observed. |
| SSH | The real SSH queue test passed on `pn50`, including save rejection and unchanged Unicode fixture content. Only Linux remote hosts are accepted. |
| Multiple frontends | Independent terminal processes share a backend over a Unix socket; queue tests cover independent ownership, reload, diagnostic revision mapping and close. |

The history below includes removed platform work and former blockers. It does
not define current support or outstanding requirements.

Validation after removing platform support passes: the full workspace library
and integration suite, Linux desktop/backend builds with no default features,
all seven real-process queue tests, both reconnect tests, both live rust-analyzer
tests, independent shared frontend processes, and the live SSH queue test using
the rebuilt backend on `pn50`. The remote fixture was unchanged and removed
afterward. Rustfmt, architecture boundaries, workflow YAML/job references and
diff whitespace checks pass. Clippy completes with existing lint warnings.

After the debugger-controller removal, all 32 desktop tests, the engine library
and integration suite, workspace compilation and architecture checks pass. The
file context menu no longer includes task/run configuration actions and uses
shared document identity for remote files instead of testing remote paths on the
local filesystem.

The real PTY smoke test prints `plain-terminal-ok`. Both live rust-analyzer and
both real-process reconnect tests also pass after removing the engine debugger
module. The rebuilt process-only GUI passed the reusable smoke harness inside
an isolated Xephyr display. Captures in `/tmp/lapce-gui-debug-removal` were visually
inspected: the debugger launcher is absent, a normal fish terminal is running,
source remains read-only, external reload displays the changed value, and F12
after backend replacement selects `answer` at line 1, column 8. This closes the
previous isolated-display navigation-after-restart validation gap.

### Validation history and implementation details

The entries below record successive implementation stages. Statements about
remaining work apply to their stage; the completion audit above is current.

2026-10-17: live SSH validation now passes against `pn50` (Linux x86_64).
The current backend executable was copied to a temporary directory and selected
with `ConnectionOptions::remote_backend_executable` through
`LAPCE_TEST_SSH_PROXY`. This option never installs or downloads a replacement
when its version check fails. The real `BackendConnection` SSH test opened,
queried and closed a read-only Unicode fixture with a space in its path. Client
unit and lease tests also pass. WSL, Windows and macOS runtime checks still
require available environments or successful CI results. Earlier statements
below about the absence of an SSH host describe the previous validation state.

The live managed-replacement regression now passes on Linux: definition
navigation succeeds before and after replacement through the stable frontend
queues, shared-session reconnect preserves document identity, and disk content
is unchanged. This uses the same managed client and session APIs as desktop.
The opt-in `lapce-client/tests/remote.rs` SSH runtime test compiles but has not
run. It requires `LAPCE_TEST_SSH_HOST` and `LAPCE_TEST_SSH_FILE` naming an
existing absolute remote fixture. The current environment is Linux, has no WSL
or macOS runtime, no configured SSH fixture, and no `/run/sshd` runtime directory
or usable local SSH host keys. Completing the original cross-platform/remote
validation therefore requires accessible runtime environments or actual CI
results. Do not mark the overall goal complete on the existing Linux evidence.

Latest consolidated Linux verification passes: resolved architecture boundaries,
the engine library/integration suite, all 33 desktop unit tests, eight client unit
tests, seven real-process client tests, both process reconnect tests, and the live
rust-analyzer test. The remote launcher process test uses a local transport
adapter and does not claim an actual SSH connection. Recent WSL corrections
expand home paths before direct execution, preserve the distribution UNC root
for uploads, and execute installer pipelines through an explicit shell. The
installer regression executes the shell arguments locally and checks that spaces
and shell-like path text are preserved literally. Actual SSH, WSL, macOS and
Windows runtime verification still needs accessible host environments; configured
CI jobs are not evidence of a successful run.

Queue contracts are the main integration-test boundary. The client process suite
now drives a real backend through requests and event receivers without a renderer
or session model: both shared frontends receive external disk-change payloads,
each acknowledges its own revision, duplicate acknowledgements are rejected, and
a forged edit cannot masquerade as a disk reload. All six process tests pass on
Linux, including managed replacement and independent socket disconnection. This
complements the existing shared-session and live language-server queue tests;
it does not establish rendering or SSH/WSL/non-Linux runtime behavior.

Current validation update: shared hosting is established for two independent
terminal processes over a Unix socket. Desktop backend replacement is wired
through the managed client. A live rust-analyzer integration test now verifies
symbols, definition, hover, diagnostics, external disk reload and diagnostic
clearing. GUI interaction, SSH/WSL and non-Linux runtime validation remain open.

Basic Linux GUI process mode is now observed: the process-only desktop displayed
the temporary Rust file and explorer, and its separate `lapce-proxy` child was
verified. Direct window captures initially exposed a startup ownership alert:
the first Connected event competed with a still-pending initial open. Shared
session ownership now tracks pending open/reopen attempts, skips duplicate
reattachment and permits retry after failure/disconnection. A second capture
showed the file rendered without the alert. Opening/session and all 33 desktop
tests pass. Linux GUI typing/read-only enforcement, external reload and backend
restart interaction were subsequently validated: typing, Backspace, Delete and
Save left file text unchanged; external disk changes appeared in the view; the
restart command replaced the proxy PID and preserved the refreshed file without
an ownership alert. Captures were visually inspected. The reusable manual
`scripts/test-gui-process.py` harness passed again with fresh temporary config,
workspace and data directories. It requires Linux X11, Pillow, xwininfo, xprop,
xwd and libXtst, and retains captures for visual review. Live GUI language-server
definition navigation was subsequently observed with rust-analyzer: F12 moved
from the call on line 2 to its definition on line 1, with unchanged source text.
The harness supports `--lsp` for this check. A subsequent run intended to verify
navigation after restart was interrupted by shared-display capture interference;
it is not counted as a pass. The harness now checks window ownership and keyboard
focus before capture/input. Isolated-display validation of navigation after
restart, SSH/WSL and non-Linux GUI behavior remain open.

Diagnostic URI conversion now uses `lapce_rpc::file::file_url_path` across the
desktop, headless session and shared host. Native file paths retain native URL
handling; foreign POSIX paths fall back to decoded file URI paths without
accepting non-file schemes or dropping unsupported authorities. This supports
remote Linux workspace diagnostics on Windows consistently across frontends.
The shared host finds cached publications by the decoded workspace path rather
than constructing a URI using the frontend OS. Space/Unicode path regressions
run locally and are now part of the platform CI gate.

A process-only desktop binary was built and launched with isolated temporary
config/data directories on the available display. It exited cleanly before
window inspection; this does not verify file rendering, navigation or restart
interaction. Live GUI validation therefore remains open.

CI now schedules process-only desktop compilation, real backend process tests,
shared-session reconnect/reload tests and live rust-analyzer validation on Linux,
macOS and Windows. The separate Unix-socket frontend smoke test also runs on
macOS. These are configured gates, not evidence of completed remote/platform
runs. SSH upload now reports failed `scp` status and stderr instead of returning
success; a local subprocess regression verifies that failure propagation.
Shared-host attachment checks replay size before sending any initial events,
making insufficient-capacity rejection independent of reader timing.

Remote home lookup now validates subprocess success and rejects empty or literal
`$HOME` results while preserving spaces in directory names. WSL invokes `sh -c`
to expand `$HOME`; its previous direct `echo` invocation passed it literally.
SSH transport exit 255 during platform detection retains the remote error rather
than falling through Windows-shell probes and reporting an unknown platform.
Local regressions cover home lookup and transport-failure reporting. Live SSH/WSL
validation remains required.

Desktop and headless disk-snapshot reloads now share
`SessionDocument::reload_from_disk`, including the buffer ownership guard.
Disconnected documents retain their cached content until a new backend snapshot
reopens them; late watcher snapshots cannot overwrite it. A regression covers
reload before attachment, successful attached reload, and stale reload after
disconnection. Desktop rendering and asynchronous synchronization remain host
adapters; this is another consolidation step, not completion of orchestration.

Interrupted read-only initial loads now recover through the same reopen plan as
loaded files. The desktop reconnect loop no longer excludes unloaded documents.
Successful reopen completes loading, clears the prior load error, initializes
parser/diagnostic adapters and notifies loading observers, including for empty
files. Desktop asynchronous openings use `SessionOpening`, which captures the
document/backend generation and rejects retired-backend responses even when text
revision and file identity are unchanged. Engine regressions and a GUI adapter
observer regression pass; this still does not establish live GUI interaction.

Initial document opening now completes through `SessionOpening::apply` in both
the desktop and `WorkspaceSession`. This shared transition validates ownership,
loads content, forces read-only policy, records valid load failures and initializes
backend synchronization before observers see a loaded file. The desktop retains
parser setup, diagnostics binding and view notification as adapters; its old
separate load/attach/read-only callback sequence is removed. Engine tests cover
success, failure and a retired response after successful attachment. A desktop
observer test verifies loaded content is already read-only and attached when
loading becomes visible.

Close initiation also uses a shared session transition:
`SessionDocument::begin_close` returns immediate local release or a queued
backend operation ID. Desktop callback routing and headless blocking execution
adapt this same policy; duplicate/paused closes fail without releasing ownership,
and acknowledged buffers can be released locally. A regression verifies ownership
is retained until acknowledgement. All 33 desktop tests, 11 session tests,
runtime close-failure coverage and real-process reconnect/shared-close tests pass.

Desktop and headless document navigation now use `SessionNavigationPlan`, adding
backend attachment generation to the existing text/identity snapshot checks.
Results from a retired backend cannot populate annotations or navigation state
after reconnecting to identical text, where buffer revision alone would not
change. Disconnected documents cannot capture session navigation plans. A
regression verifies unchanged text revision across reopen, rejection of the old
result and acceptance of the replacement backend's result. All 16 navigation
tests and 33 desktop tests pass.

Shared-host external reload validation now covers two live engine sessions over
one backend process. It exposed competing backend revisions during reload.
The host now retains each frontend's text snapshot/revision, validates incoming
deltas against that owner, and translates changed text to the backend's current
revision. Identical text already acknowledged by another frontend needs no second
backend update. Shared document reload policy also ignores unchanged watcher
snapshots. Both sessions reload Unicode content and close independently in the
real-process regression. Versioned diagnostics now translate matching backend
versions to each frontend's revision and discard stale versions for owned files.
The latest publication is replayed after a matching reload acknowledgement, so
a peer does not lose diagnostics that arrived before it synchronized. Navigation
checks frontend text against the backend snapshot and returns a retryable stale
content error while they differ. A regression covers divergent diagnostic
revisions, and the live rust-analyzer test now verifies reload, diagnostic clears,
navigation and independent closes with two sessions sharing one backend.

- Consolidate desktop workspace/document orchestration with the headless session.
  The desktop still owns asynchronous loading, annotation scheduling, and parts
  of lifecycle recovery; shared models and request plans do not by themselves
  prove that two frontends execute identical behavior.
- Separate semantic navigation commands and neutral input from Floem events.
  Layout-dependent cursor movement needs a frontend layout interface.
- Separate application/plugin settings and document session persistence from
  colors, icons, fonts, window layouts and rendering caches.
- Complete buffer ownership, close/reconnect behavior and generation-aware recovery
  across the desktop and headless runtime. Independent backend connections are
  tested, as are concurrent terminal frontends sharing one backend; desktop
  interaction and terminal event ownership still need validation.
- Define and verify version/capability negotiation, cancellation, bounded event
  queues and the cross-platform path/URI contract.
- Verify desktop process-mode interaction and live language-server navigation,
  annotations and reload behavior. Compile checks and fake language responses
  do not establish these runtime gates.
- Verify SSH/WSL and non-Linux builds. Local Linux checks do not establish them.
- Remove remaining inactive editing/debug controllers and settings that no longer
  serve the always-read-only product. Their extraction is not required: editing,
  saves, renaming, replacement, workspace edits and debugging were removed from
  product scope by the user.

Browser transport/gateway support was explored in the earlier plan; it is an
additional frontend option, not evidence that the shared application API is
finished. Historical validation below describes earlier editing implementations
and is retained as history; current read-only checks are recorded in later entries.

## Validation

```sh
cargo check -p lapce-app --offline
cargo check -p lapce-client --no-default-features --offline
cargo test -p lapce-app --lib --offline --profile ci
cargo test -p lapce-client -p lapce-engine -p lapce-rpc -p lapce-services \
  --offline --profile ci
```

Tests exercise headless open/edit/save/close, snapshot-based conflict inspection,
invalid Unicode delta ranges, semantic directory listings, disconnect and timeout
behavior, wire roundtrips, shared undo/redo and UTF-16 edits, snippets and search.
Completion tests cover Unicode highlights, snippet previews and stale response
rejection. GUI adapter tests verify shared document ownership, independent field
notifications, notification after mutation, and scope disposal. Document tests
also distinguish read-only user edits from host reloads. The terminal example can
also be driven with piped commands against the built proxy to validate the actual
process boundary. None of these establishes completion of the remaining list.
Workspace-edit tests execute ordered edits and resource changes in a headless
memory host and check failure indices, stale versions, invalid UTF-16 ranges,
overlaps, annotation preservation and confirmation gates. These do not establish
filesystem resource execution or GUI navigation behavior at runtime.
Search tests cover case toggles, invalid-regex recovery, directional navigation,
wrapping, Unicode whole words/range edges, zero-width matches including EOF, and
stale background results. GUI adapter tests verify that search options update the
same engine model before views are notified.
Annotation tests cover UTF-16 validation, inlay ordering and stale responses,
engine-owned inlay transformations, and Unicode preview movement/consumption.
Modal editing tests cover read-only documents and shared undo history.
Syntax-job tests cover ownership, cancellation, completed stale results, edits,
and same-revision syntax replacement. Adapter tests verify notifications occur
only after acceptance. These validate lifecycle rules; they do not assert
language-specific grammar output or every parser runtime.
Diagnostic tests cover deferred mapping, Unicode offsets, edit transformations,
severity filtering, clearing and stale version rejection. Adapter tests verify
shared consumer state and separation of panel expansion from semantic updates.
Source-control tests cover metadata refreshes, selected-path persistence, commit
validation, message trimming, duplicate suppression, stale/foreign replies,
retry after failure and preservation of changed commit messages without a GUI.
The client integration test also commits a temporary repository through backend
RPC, checks the resulting Git history and verifies an acknowledged non-repository
error. Checkout, discard and initialization also use acknowledged requests and an
engine-owned operation queue. They are mutually exclusive with commits, and a
failed rename restore prevents dispatching the subsequent trash step. Runtime
checks cover file discard, detached revision checkout and invalid references.
Coordination with unsaved open documents and broader GUI runtime coverage remain
outstanding.

Save tests cover policy eligibility, scratch/file transitions, normalization flags,
autosave identity/revision deadlines, single-winner formatting completion,
stale/late replies and document identity changes. Timer and apply-edit adapters
still need broader runtime coverage across frontends.


Workspace search query options, current request ownership, result maps and errors
now live in engine `WorkspaceSearch`. Query changes and empty input invalidate
pending tickets; duplicate, foreign and stale results cannot refill the panel.
The GUI projects accepted result rows and keeps expansion/geometry as view state;
it displays current-query errors. Backend search cancellation generations belong
to each dispatcher rather than one static counter across all sessions. Engine
checks cover stale/cleared queries, errors and recovery; a client integration test
runs searches concurrently in two independent workspaces. Search transport dispatch
and replacement orchestration still need a shared session controller.


Document replace-next and replace-all planning and application now use engine
`ReplacementPlan`. Plans capture document identity/revision and compute ranges
from the compiled engine query and current text, rather than GUI highlight caches.
They reject foreign/stale documents, read-only content, unloaded documents and
invalid queries. Replacement stays literal as before; no capture interpolation
was introduced. The GUI retains cursor, snippet and rendering updates after an
accepted batch. Tests cover Unicode edits, wrapping, empty queries, one revision
per batch, zero-width EOF matches, stale identity/revision and read-only rejection.
Workspace-wide replacement and application-session dispatch remain outstanding.


Engine `WorkspaceSession` now owns a headless document registry and blocking
open/flush/save/close execution over a neutral `RequestExecutor`. `RpcExecutor`
adapts the existing RPC handler with an explicit timeout. Duplicate opens reuse
one owned document; failed close retains its text and identity, and paused sync
is reported rather than silently treated as flushed. The terminal example uses
this controller instead of implementing its own RPC and acknowledgement loop.
Async GUI session dispatch, event routing, resource/annotation controllers and
browser transport are still outstanding; this controller is not the completion
of the application-session extraction.


`WorkspaceSession::resource` now drives the shared resource coordinator through
its request executor. It closes affected owners without removing their engine
documents, applies the filesystem effect, retargets registry keys, reopens the
same identities and flushes reconciliation against disk. Failures restore the
previously closed owners at their original paths; successful deletes retain the
missing-file/autosave policy. The terminal example exposes rename and delete.
Runtime tests preserve local text across successful rename and failed rename,
and separate-process checks cover Unicode edits, undo/redo after rename, explicit
save after delete and deletion remaining on disk after quit. Async GUI adapters
still have their own transport dispatcher; that migration and event routing
remain unfinished.


The engine owns `Document::disk_changed`: pristine documents reload, dirty ones
retain text/history and compare against the latest disk baseline. Both the GUI
adapter and headless session use this policy. GUI deletion notifications mark
retained documents missing so autosave cannot recreate externally deleted files.
`WorkspaceSession::handle_event` routes file changes, deletion, repository metadata
and connection status. Other notifications are returned as `SessionEvent::Unhandled`
for the host rather than discarded. Closing owners reject late watcher reloads.
Injected-event tests cover pristine/dirty changes, deletion/reappearance, closing,
repository/status updates and forwarding. Actual watcher event causality and async
GUI event-dispatch migration still require verification and extraction; terminal,
language-service and debug event controllers are not yet fully session-owned.


Engine `DiagnosticPublication` now prepares backend-to-local revision mapping and
immutable text snapshots for both GUI and headless publication. The headless
workspace routes diagnostics events, retains unopened-file diagnostics, binds
UTF-16 offsets when opening, rejects stale backend versions and moves entries
when retargeting a renamed document. Session `synchronize_edits` updates both the
backend queue and live diagnostic spans; the terminal example uses this API.
Tests cover unopened binding, Unicode insertion, pending/acknowledged backend
revision mapping and stale publications. Full language-server runtime validation
and the async GUI session registry migration remain outstanding.


Engine `TerminalState` now owns launching/running/stopped/failed lifecycle,
process ID, exit status and launch errors. The GUI launch-error field projects
this model and terminal event handlers use its transitions; duplicate exit and
late start/failure events cannot overwrite a terminal outcome before restart.
The headless session registers terminal ownership, routes lifecycle notifications
and forwards output as unparsed bytes without retaining a renderer buffer. Unknown
terminal notifications remain available to the host. Tests cover lifecycle,
restart, duplicate exit, binary output and unknown ownership. Terminal parsing,
layout, selection and rendering remain frontend work. Run/debug routing, broader PTY
runtime coverage, and protocol generations for late events across reused-ID
launches still require work; terminal separation is not complete.


The headless session can now submit terminal launch notifications via its neutral
executor and owns the terminal ID before events arrive. Executors without
notification support return an error and leave no phantom terminal entry.
`examples/pty.rs` demonstrates a second terminal consumer with no Floem imports:

```sh
cargo run -p lapce-engine --example pty -- /path/to/lapce-proxy /bin/sh -c 'printf hello'
```

Separate-process Linux checks exercise a real PTY, start-before-exit ordering,
byte-exact invalid-UTF-8/NUL/escape output, successful completion, nonzero exit,
spawn failure and backend shutdown. The example reports disconnect and uses a
30-second completion deadline. Interactive input/resize controllers, alternate
platforms and generation-aware restarts remain unverified/unfinished.


`WorkspaceSession` now exposes terminal input and resize over the same neutral
notification executor. Owned-terminal and lifecycle checks reject unknown,
stopped or failed terminals; empty input is a no-op. GUI character input uses
the shared lifecycle planner. RPC `TerminalSize` clamps dimensions to nonzero
16-bit cell counts and is used by the headless planner, GUI cell layout, RPC
helper and backend construction/resize, avoiding truncation or zero dimensions.
The PTY example's `--input-stdin` option reads a text payload and sends it after
the process-start event, following an 80x24 resize. A separate-process real-PTY
check confirms Unicode input reaches the child and `stty size` observes 24x80.
Tests cover ownership, empty input, terminal outcomes and dimension extremes.
Continuous interactive input, generation-aware close/restart, async GUI session
ownership and alternate-platform validation remain unfinished.


The blocking workspace session now owns search execution, commit execution and
ordered repository-action execution over `RequestExecutor`. Empty search queries
clear state without a backend request; failures record model errors and retries
reuse the same shared policy. Failed repository restoration prevents a subsequent
trash effect. The terminal example exposes `search <pattern>` through this API;
a separate-process check returns another file's match and Unicode preview without
changing the opened file. Three session-controller tests cover search recovery,
commit failure/retry and repository failure ordering. Coordinating repository
changes with unsaved/open documents and migrating async GUI dispatch still remain
unfinished; these execution methods do not establish those gates.


The blocking workspace session exposes `edit_document` to coordinate local edits,
diagnostic span transformations, synchronization and backend acknowledgement.
The terminal frontend uses this entry point for insertion and undo/redo instead
of reproducing that sequence. Unknown, disconnected, closing and failed owners
are rejected before the edit callback runs. Transport errors after mutation
retain local text for recovery. Cursor and clipboard policy remain frontend inputs.
This does not replace the asynchronous GUI dispatcher or finish its migration.


`WorkspaceActions` now owns the shared search, commit and repository-action
controller. Requests carry opaque operation ownership through `WorkspaceEffect`;
completion yields a typed outcome or the next ordered effect. The blocking
`WorkspaceSession` and the async Floem adapter both drive this protocol. GUI
search and source-control panels project the same workspace owner rather than
creating independent panel-owned models. Stale, foreign and duplicate replies
cannot consume pending ownership, and a failed repository step emits no followup.
Commit completion compares the current host message before requesting its reset.
Engine controller tests exercise delayed/foreign search results, clearing input,
repository followup ordering, failure and commit-message changes. GUI compile
checks cover both adapters; live GUI search/commit behavior remains unverified.
This extraction does not yet consolidate documents, debug controllers, backend
event routing, settings or application persistence into this workspace owner.

Validation for the workspace-controller migration: `cargo check -p lapce-app
--offline` and engine/client example checks pass. Fifteen targeted engine tests
cover the runtime, blocking workspace operations and shared controller; the Floem
adapter test passes and verifies notifications after mutation, stale replies and
scope disposal. These checks do not establish live GUI or cross-platform behavior.


The backend DAP transport/process launcher, debugger registration storage and
plugin-catalog debug request/notification orchestration have been deleted.
Legacy plugin debugger-registration requests are acknowledged without storing
or launching anything, allowing plugins that also provide language navigation
to keep working. Legacy frontend DAP wire messages remain rejected/ignored until
protocol cleanup finishes. The debug panel view and breakpoint gutter views are
removed, and restored panel layouts filter the old Debug entry. Remaining GUI
terminal/run-debug adapters and engine debug models still need removal.
Client integration tests now verify read-only open/close ownership, invalid
Unicode deltas, rejected repository/resource/save operations, directory listings,
concurrent workspace searches and transport shutdown under this product policy.

Validation after debugger-runtime/UI removal: all seven client integration tests
and all 29 GUI library tests pass. The app compiles, the client compiles without
its embedded backend, and the proxy plus terminal-browser example build. A real
separate-process check verified Unicode display, workspace search, switching
files, rejection of legacy edit/save commands, unchanged disk contents and clean
shutdown. This does not establish complete GUI removal, live language-server
navigation or a fully consolidated frontend-neutral session API.


Read-only language navigation now uses engine `NavigationQuery` and
`NavigationPlan`. The plan validates loaded file ownership and UTF-8 boundaries,
converts offsets to UTF-16 LSP positions and validates response kinds. Replies
from changed, retargeted or replaced documents fail before frontend effects.
GUI definition/reference/implementation/hover and document-symbol retrieval use
this API via the existing acknowledged queue. The blocking workspace session
exposes `navigate` over the same plan after draining pending synchronization;
the terminal frontend exposes the same queries and reports language-service
failures without ending the browsing session. Engine tests cover Unicode,
invalid/unloaded/local documents, stale/foreign snapshots, wrong responses and
headless read-only dispatch. Live language-server coverage remains outstanding.

Definition-to-reference fallback also uses the shared document navigation queue.
Definition location normalization is engine-owned and handles empty LSP location
link arrays without indexing/panicking. The GUI adapter test verifies late
responses after a document reload and after scope disposal are rejected.

### Read-only document save cleanup

Desktop document save requests, autosave scheduling and formatting-before-save
orchestration have been removed. The unused engine save-policy module and its
mutation-oriented tests have been removed too. Disk reloads and language-service
synchronization remain. All 30 desktop library tests and the full engine suite
pass after this cleanup. Low-level synchronization save variants and legacy
scratch internal command variants still need removal.

Unused scratch-save internal commands have now been removed. Autosave,
format-on-save, normalize-line-endings and format-on-autosave configuration fields
and defaults are removed, so generated settings UI no longer offers them. A
compatibility test verifies old keys are accepted and ignored during loading,
then omitted from serialization and settings descriptors. The 30 existing desktop
tests and the new compatibility test pass.

The shared document synchronization queue no longer defines save operations or
save acknowledgements. Its generic request path rejects requests outside the
code-browser allowlist before enqueueing them. Nine queue tests and six runtime
tests pass, covering disk-update ordering, query ownership, close/recovery and
mutation-request rejection. The desktop compiles after this API removal.

Document synchronization notifications now accept only owned-file reload signals
and language-request cancellation. Completion/signature-help/debugger traffic,
foreign-file reloads and connection lifecycle notifications are rejected before
queueing. Terminal/workspace notifications remain on their separate connection
APIs. Ten queue tests cover notification filtering and ordered reload/query flow.

Symbol rename UI and dispatch have now been removed: the overlay, rename focus
state, internal StartRename command, editor prepare-rename path and document
adapters are gone. The unused engine rename planner and its mutation-oriented
tests are removed. Browsing navigation remains in the shared navigation API.
Legacy rename wire variants remain rejected by the backend allowlist.

Document replacement planning and dispatch are removed from the engine and
 desktop. Find UI no longer renders Toggle Replace, replacement input, Replace
Next or Replace All controls. Search input, match counts and navigation remain.
Legacy replace focus/state and internal input-routing variants still remain for
subsequent cleanup. The full engine suite passes after planner removal.

Replacement input state is now removed from the desktop: no hidden replacement
editor, replacement focus/activation signals or replacement input commands are
created. Find input routes directly to the search editor. The old replacement
focus condition is always false; its command is excluded from the palette. All
31 desktop tests pass after state and routing removal.

Workspace-edit execution, change-confirmation UI, code-action resolution and
workspace-document-edit adapters are removed from the desktop. The unused engine
workspace-edit planner/executor and its mutation-oriented tests are removed.
Legacy internal code-action/workspace-edit commands are ignored; backend mutation
requests remain rejected. All 31 desktop tests and the full engine suite pass.
Resource coordination and code-action presentation still need cleanup.

Desktop resource coordination and its engine module are removed. Legacy internal
create/rename/duplicate/path-creation commands are ignored before any backend
request. Directory reads, refreshes and navigation remain. File-explorer naming
state and the compatibility command variants still need removal.

File-explorer mutation controllers are removed: rename/trash dispatch, typed-name
validation, naming completion and the unused create/rename/duplicate internal
commands are gone. Explorer keyboard focus now uses Normal mode and ignores text
input. Legacy naming row models and their hidden input view still remain to be
removed before extracting the complete browsing tree.

Explorer naming input views and their editor allocation are removed. The explorer
constructor no longer depends on desktop `Editors`; it takes workspace context
and a reactive scope. Its virtual list currently projects browsing rows with no
naming state. Unused naming model types remain to be removed when extracting the
complete tree/row model. Desktop compilation and library tests pass.

The explorer tree and visible-row projection now live in `lapce-engine::explorer`.
Desktop model names are compatibility aliases and Floem's virtual-list adapter
only delegates slicing. All naming/renaming/duplicating model types are removed.
Shared tree coverage verifies expanded natural ordering, collapsed descendants,
row counts and every subrange of a sample tree. Slice ranges now use an exclusive
end, correcting the former extra-row behavior. The engine tree tests pass and
 desktop compilation passes.

Directory expansion decisions and snapshot application now belong to shared
`FileNodeItem::toggle_directory` and `refresh_directory` methods. The desktop
only filters excluded entries, executes reads and publishes model changes.
Collapsing an unread folder no longer triggers an unnecessary read. Two tree
integration tests cover row slices, collapse/read races, loaded-child refreshes
and ignored responses for removed nodes; the desktop compiles.

Explorer reveal decisions now use shared `RevealStep`: visible row, directory
read or missing path. The desktop delegates ancestor expansion and chooses only
scrolling/read execution. Failed reads no longer recursively retry the same
folder. Three tree tests cover nested reveal, outside/missing paths, expansion
and row projection; the desktop compiles.

Directory snapshot validation is shared between transport queries and direct
explorer refreshes. Invalid/duplicate/non-child entries are rejected before tree
mutation, preserving the previous expansion, row counts and child snapshot. Seven
focused directory/explorer tests pass, including atomic rejection coverage.

Code-action presentation is removed: overlay, gutter lightbulb, focus/controller
state and internal Show/RunCodeAction/ApplyWorkspaceEdit variants are gone. The
palette excludes Show Code Actions. Legacy code-lens presentation adapters now
ignore invocation and still need removal. All 30 desktop tests pass.

Desktop code-lens UI, command runner, per-document cache and requests are removed.
The unused code-lens module and overlay adapters are removed; folding gutter
controls remain. Legacy code-lens wire variants are still rejected by the backend.

Folding-range retrieval now uses the shared snapshot-validated navigation API. The desktop requests folding ranges through this API, and the terminal example exposes `folds`. Regression coverage checks response type and stale document rejection; live language-server rendering remains to be verified.

Inlay-hint requests now use the shared navigation API as well. The desktop keeps only hint-span projection and rendering invalidation; shared request capture and completion reject mismatched responses and stale snapshots. Headless session coverage verifies another frontend can retrieve hints through the same API.

Workspace symbol search now has a shared typed request/response adapter and a headless session method, exposed by the terminal example as `workspace-symbols <query>`. The desktop palette uses the same adapter and ignores responses from superseded palette runs or queries.

Removed the remaining desktop code-action cache and automatic keypress-triggered requests. Code-action UI had already been removed and requests were rejected by the read-only boundary; the desktop now avoids allocating and scheduling this obsolete editing feature. Legacy command identifiers remain filtered/no-op for compatibility.

Document-highlight requests now use shared navigation capture/completion instead of the desktop calling the proxy directly. Shared validation covers Unicode cursor offsets, response type, document identity, path and revision; desktop cursor debounce and rendering remain host responsibilities.

The shared annotations module now owns document-highlight offset ranges, selection conversion, sorting and cursor lookup. Desktop highlight projection delegates to it. Regression coverage verifies UTF-16 conversion, rejection of malformed ranges and overlapping-range lookup (which the previous binary search could miss).

Semantic-token requests now use the shared navigation API. Shared completion validates document identity, path/revision ownership, response type and semantic payload path/length. Desktop still owns background render-span construction. Full engine suite passed before this extraction; the expanded navigation suite and desktop compile check passed afterward.

Semantic span construction moved into shared annotations. It sorts tokens, rejects invalid/overlapping/out-of-bounds or non-Unicode-boundary spans, and supports host cancellation during processing. Desktop background scheduling remains local and now accepts empty semantic results to clear obsolete styles. Annotation regression coverage and desktop compilation passed.

The desktop document-symbol palette now uses `Doc::request_navigation(Symbols)` rather than calling the proxy directly, sharing synchronization and snapshot validation with the outline and headless session. Document-symbol completions check palette run/kind/status; workspace-symbol completions also reject closed palettes. Desktop compilation and library tests passed.

Shared navigation now normalizes document-symbol responses for tree consumers. The desktop outline delegates to this converter, preserving flat response symbols as roots (previously discarded), nested children, ranges and metadata. Regression coverage checks both response forms.

Backend embedding is now opt-in for `lapce-client`; the desktop explicitly requests `embedded-backend`, while process/remote frontends use the default lightweight client. `python3 scripts/check-architecture.py` checks shared-crate dependency boundaries and optional backend ownership; CI runs this gate before building. The client builds without default features, and embedded client integration tests pass with the feature enabled explicitly.

The desktop now has an optional `embedded-backend` feature (enabled in its default features for existing distribution builds). `cargo check -p lapce-app --no-default-features` verifies the UI without proxy/Wasmtime dependencies. In settings, `core.backend-executable = "/path/to/lapce-proxy"` selects a separate local backend process; an empty string uses the compiled client default (embedded when enabled, `lapce-proxy` from PATH otherwise). Reopen the workspace to apply. Remote connections continue using their existing transport. The schema includes this setting. Headless/process backend integration was previously verified; desktop GUI process-mode interaction still requires a runtime smoke test.

Connection startup failures now emit a shared error message event, including the failed executable path for local processes, before reporting disconnection. Terminal event handling displays these messages. A process-mode integration test without the embedded backend verifies missing-executable failure, pending/future request release, disconnected status and clean worker shutdown.

Real process-mode integration now has a repeatable test and an Ubuntu CI gate. Build `cargo build --profile ci --bin lapce-proxy`, then run `LAPCE_TEST_PROXY=/absolute/path/to/target/ci/lapce-proxy cargo test --profile ci -p lapce-client --no-default-features --test process -- --include-ignored`. Both startup-failure and success-path tests passed locally. Success covers initialization, Unicode file opening, read-only metadata, directory listing, workspace search, rejected saving, unchanged disk content and worker shutdown. The success test is explicitly ignored for ordinary runs that have no separately built proxy; CI supplies the executable and includes it.

The architecture gate also supports `python3 scripts/check-architecture.py --resolved`, which inspects Cargo's normal/build feature graph for process-only client and desktop builds and rejects proxy/Wasmtime dependencies. It passed locally and is included in Ubuntu CI after dependency fetching/building. The current remaining-work list was updated to reflect the user's always-read-only scope rather than obsolete editing/debug extraction tasks.

Headless disconnect events now revoke each document's backend synchronization ownership and increment its backend generation, while preserving loaded read-only text. A regression test verifies cached browsing survives but language navigation issues no backend request after disconnect. The full engine suite passes. Desktop disconnect/callback coordination still requires consolidation; this change does not claim full reconnect support.

Desktop disconnection now delegates each open document to the same shared backend-ownership transition used by the headless runtime. The Floem adapter notifies synchronization observers, then cancels pending request/operation callbacks through existing cancellation paths. Loaded text remains available. A GUI model regression verifies ownership revocation, cached text preservation and safe handling of disposed scopes; all 31 desktop library tests pass. Full transport replacement/reconnect remains outstanding.

Read-only files without backend synchronization ownership can now close locally in both desktop and headless hosts. The shared session predicate prevents disconnected tabs from requiring an impossible backend acknowledgement. Headless coverage confirms local removal without sending requests; the GUI projection checks the same predicate before/after disconnect. Navigation and desktop library tests passed.

Shared reopen now refreshes changed cached text from the replacement backend for read-only files, preserves buffer identity and read-only status even if the backend advertises write access, and emits no cached-text upload. `WorkspaceSession::reconnect` replaces its executor and independently reattaches all cached paths with per-file outcomes. Hosts must retire the old connection/event stream before replacing it. Tests verify identity, restored navigation and disk-text refresh; the full engine suite passes. Desktop connection replacement and old-stream fencing remain to be wired.

Shared sessions now issue a unique backend event-owner token, renewed on executor replacement. `handle_backend_event` rejects old and foreign stream tokens before any state mutation; hosts capture the token on stream attachment. The terminal routes events through this API. Regression coverage verifies old disconnect notifications cannot revoke a new connection. A real-process reconnect integration test stops the first backend, changes a Unicode file on disk, starts a replacement, and verifies refreshed read-only content with preserved buffer identity, acknowledged close and clean shutdown. It passed locally and is included in the Ubuntu process-test CI step. The full engine suite and both frontend compile checks passed; desktop stream replacement is still outstanding.

Desktop backend replacement is now wired through `lapce-client::managed::ManagedBackend`, which preserves the frontend RPC endpoint while retiring the prior backend/event stream and starting another with the same options. `Restart Workspace Backend` triggers it; disconnection cancels document callbacks, and connection completion reopens loaded read-only documents through shared reopen plans. Syntax/render caches refresh after reattachment. Queued old-generation requests fail during replacement and old notifications are discarded. A real-process test verifies that a retained frontend handler works before and after restart, with disconnect preceding new connect. Three process integration tests and all 31 desktop library tests passed. GUI interaction remains unverified: this environment has no Xvfb and rust-analyzer is only a rustup shim without the component installed. Only a Linux compilation target is installed; SSH workspace details were requested for live remote validation. Shared multi-frontend backend hosting and remaining orchestration consolidation are still outstanding.

Asynchronous desktop synchronization acknowledgements now include a shared document/backend-generation token. Operation IDs restart after reattachment, so an old callback with a reused ID could previously acknowledge a new request. Shared completion now rejects stale attachment/owner tokens before touching synchronization; regression coverage reproduces identical old/new operation IDs and confirms only the current callback completes the operation. Eight session tests and 31 desktop library tests passed.

Shared-backend hosting now has a frontend-namespaced buffer lease model in `lapce-client::leases`. It allocates canonical backend owners independently of frontend-local buffer IDs, reference-counts path ownership, rejects identity/path misuse, and releases backend buffers only after the final frontend closes or disconnects. Two tests cover colliding local IDs, independent closes, disconnect cleanup and failed-open reservation release. This is the ownership foundation; the multiplexing host and frontend transports still need to be connected to it.

`lapce-client::shared::SharedBackend` now multiplexes attached frontend RPC endpoints over one backend connection. It maps frontend-local buffer IDs through leases, rewrites snapshot ownership, preserves backend buffers until final close/disconnect, rejects writes and broadcasts backend events. Requests are serialized to isolate backend workspace-wide search cancellation; per-frontend cancellation notifications are not forwarded with ambiguous IDs. A real-process test opens the same Unicode file with colliding local IDs, verifies independent close/disconnect, concurrent searches, snapshot routing, rejected save and reopening after final close. Four process tests and two lease tests pass, and resolved architecture checks pass. This is a callable shared host inside a program; separately launched desktop/terminal transports, event replay/backpressure, terminal ownership and shared-host reconnect remain to be integrated/validated.

Shared-host frontend event queues are now bounded (default 128, configurable on attachment). Overflow or receiver loss disconnects only that peer, fails its requests and releases its buffer leases; broadcasts use non-blocking delivery so a slow peer cannot block others. A host-level regression injects a burst, verifies the slow peer disconnects, and verifies the other receives every event and continues requests. Four real-process tests passed with bounded queues. The host/lease tests were added to the Ubuntu CI validation step. Late-attachment diagnostic/state replay and external frontend transport remain outstanding.

Late shared-host attachment now replays the most recent repository snapshot and diagnostic publication per URI before emitting connection status. Diagnostic clears supersede older errors; transient notifications are not replayed. If the initial snapshot exceeds a peer's event capacity, it disconnects before a ready status can be emitted. Host-level tests verify replay ordering, cleared diagnostics, and insufficient-capacity failure; shared host tests and all four real-process tests passed. External frontend transport and live validation remain outstanding.

A Unix-domain socket adapter now serves attached shared frontends and connects external `BackendConnection`s using `LocalBackend::SharedSocket`. EOF releases only that frontend's leases. A real-process test validates two socket clients with colliding local buffer IDs and independent disconnect. All five process tests passed outside the socket-restricting sandbox. The `lapce-client` example `shared_host <proxy executable> <workspace> <socket path>` creates an owner-only socket and accepts frontends; `quit` shuts down the backend and peer workers. Desktop/terminal socket selection and Windows transport remain to be wired.

Desktop shared-host selection is now wired with `core.backend-socket = "/absolute/path/to/backend.sock"`, which takes precedence over `core.backend-executable` for local workspaces. Reopen the workspace to apply; desktop restart reconnects its attachment, not the shared host. The terminal accepts `terminal --socket <socket> <file>`. Both use `LocalBackend::SharedSocket` through the same client API. Non-Unix builds return an explicit unsupported-transport error instead of silently falling back. `scripts/test-shared-frontends.py` passed outside the socket-restricting sandbox: two independently launched terminal processes share the host/proxy, read Unicode content, close independently, and shut down without disk changes or a leftover socket. The process-only desktop compile check passes. Ubuntu CI now builds both examples and runs this smoke test; GUI interaction and Windows transport remain outstanding.
