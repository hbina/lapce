# Local iced_graphics patch

This directory contains the published `iced_graphics` 0.14.0 sources, normalized
Cargo manifest and bundled fonts. Upstream: https://github.com/iced-rs/iced/tree/0.14.0/graphics
License: MIT; the upstream license is included in `LICENSE`.

The source changes are in `src/text/editor.rs`: shape the caret line with
`Buffer::line_layout` after layout updates and highlighting, and clear the cached
selection after replacing text attributes. Highlighting can invalidate
lines above the viewport, including a caret left behind when scrolling; visible
line shaping alone then leaves `Editor::selection` with no cached caret layout.
Shaping just that line preserves the current scroll position.

The root Cargo manifest patches crates.io to this local copy. It is excluded from
the workspace so upstream development targets are not added to Lapce's test suite.
Regression coverage is in `lapce-iced/src/highlighting.rs`:
`style_update_after_scrolling_keeps_the_offscreen_caret_layout` reproduces the
original panic in unmodified iced_graphics 0.14.0.
