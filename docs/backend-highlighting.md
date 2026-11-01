# Backend-owned syntax highlighting

Rust and Python grammars and highlight queries ship in the executable. No runtime
grammar download, dynamic grammar library or user-installed query file is used.
Other languages display plain text. Syntax coloring works without a language
server; configured native servers can supply semantic enrichment.

## Ownership

- `lapce-core` embeds the pinned grammar crates and implements parsing algorithms.
  The Python query moves its generic identifier capture last because Lapce gives
  earlier patterns priority. Parser and query versions remain pinned together.
- `lapce-proxy::highlighting` owns parser state, cancellation, cached results and
  background execution. One document parsing worker coalesces pending work by
  open buffer; a second worker handles transient code blocks with a bounded queue.
- `lapce-rpc::highlighting` defines serializable snapshot identities, scopes and
  derived structural navigation answers.
- `lapce-client` transports results and filters shared publications by document
  ownership and exact backend snapshot identity. Its existing overflow policy
  disconnects a slow frontend rather than silently dropping state. Reopening
  obtains current text and cached highlighting.
- `lapce-engine` validates received results and stores rendering state. It does
  not initialize parsers or schedule syntax jobs.
- Floem and iced resolve scopes through their own color palettes. Highlight-only
  updates invalidate rendering without replacing text, selection or scroll state.

Embedding a backend thread does not change these boundaries. Process, shared
socket and SSH connections use the same messages.

## Protocol

`TextSnapshot` contains an opaque buffer-instance identifier and the accepted
backend revision. Opening responses, buffer snapshots, update acknowledgements
and highlight publications carry it. Shared connections preserve it while
translating frontend-local IDs and revision counters. Reload text has no accepted
highlight identity until synchronization acknowledges it.

`PublishHighlights` delivers a `HighlightSnapshot`; `GetHighlightSnapshot` retrieves
the latest cached result, including pending, unsupported and failed statuses.
Retrieval after opening/acknowledgement handles early-publication races and late
attachment. Each result includes a generation counter for style changes without
text changes. Frontends reject stale instances, snapshots and generations.

Spans use half-open UTF-8 byte offsets into the whole text. Scopes are semantic
names, not colors. Frontend acceptance checks ordered, nonoverlapping spans,
Unicode boundaries, text length and structural metadata bounds. The iced adapter
splits spans at logical lines, excluding CRLF terminators.

The backend publishes syntax first. Valid semantic spans override only their
covered bytes; syntax remains elsewhere. Semantic responses retain the captured
text identity and cancellation token, so responses from a previous reload or
language selection cannot recolor the current text. Server initialization and
status changes trigger backend refreshes. Semantic failures preserve syntax.

Bracket scopes are supplied separately so Floem can retain its colorization
preference and line limit. Structural data contains precomputed answers for
cursor intervals: matching/enclosing brackets, parent offsets, unmatched-tag
navigation and sticky headers. Clients do not receive Tree-sitter trees.

`SetSyntaxLanguage` changes backend highlighting for the shared document, including
PlainText. The choice is retained across reloads while the buffer is open.
`HighlightCode` handles transient hover code blocks on the backend. Its queue is
bounded to eight requests; each request permits at most 32 blocks and 1 MiB of
text. The hover remains readable with plain code if highlighting is unavailable.

## Validation

```sh
cargo test --offline --profile ci -p lapce-core --test embedded_highlighting
cargo test --offline --profile ci -p lapce-engine --test syntax
cargo test --offline --profile ci -p lapce-proxy --lib
cargo test --offline --profile ci -p lapce-client --features embedded-backend --test highlighting
cargo test --offline --profile ci -p lapce-app -p lapce-iced
cargo build --offline --profile ci --bin lapce-floem --bin lapce-iced --bin lapce-proxy
python3 scripts/check-architecture.py --resolved
```

The embedded grammar tests exercise Rust/Python scope output, Unicode/CRLF,
unsupported languages, and backend structural answers against direct parser
queries at every cursor boundary. Engine and Floem model tests cover stale
snapshot rejection, style invalidation and notifications after acceptance. Iced
tests cover line splitting and style-only invalidation.

Client integration tests use real embedded or process backends, two shared
frontends, late attachment, reloads, close/reopen identity, language changes,
hover code blocks and responsiveness during a large parse. A deterministic LSP
fixture gates semantic replies to verify stale rejection, enrichment and failure
fallback. Backend unit tests verify one parse is reused by repeated consumers,
cache release, semantic precedence and malformed UTF-16 token rejection.

Set `LAPCE_TEST_PROXY` to an absolute built `lapce-proxy` path when running the
client highlighting suite to repeat it through a separate backend process.
The architecture check rejects local syntax parsing in frontend/engine sources
and checks that embedded grammar dependencies and dependency boundaries remain.

## Verified implementation

Validation on 2026-10-31 completed with 224 passing workspace tests. The separate
process suite passed all 11 tests, including shared-socket highlight retrieval;
the highlighting integration suite passed all five tests against embedded and
separate process backends. All ten iced tests also passed with the optional
process-backed worker check enabled. Both GUI binaries and the proxy built, and
formatting, whitespace and resolved architecture checks passed.

The highlighting integration suite also passed inside a network namespace with
networking disabled, and with fresh empty XDG configuration/data directories.
The latter run created no grammar or query directories. Optional live SSH and
real language-server tests were not run; semantic lifecycle tests use the
controlled LSP fixture described above. GUI validation here is through renderer
and worker tests rather than manual window interaction.

### Iced layout-cache correction

The custom highlighter leaves an uninitialized editor alone until line data is
available, and the initial code pane explicitly contains one empty line. Removing
highlight data still clears existing colors without replacing text.

A local `iced_graphics` 0.14.0 patch also shapes the caret line after layout and
highlight updates. Upstream only shapes visible lines after replacing attributes;
when the caret remains above the viewport, its invalidated layout causes the
`Line layout should be cached` panic. The patch preserves scrolling and clears
cached selection geometry after highlighting. See
[patch provenance](../patches/iced_graphics/README.lapce.md).

Renderer regression tests exercise empty startup, delayed colors, color removal,
and highlighting/resizing after scrolling the caret out of view. These tests call
the real iced graphics editor rather than testing only the span adapter.
