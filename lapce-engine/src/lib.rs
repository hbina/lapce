//! Editor behavior shared by graphical, terminal and service frontends.
//! No view, window, signal or renderer types belong in this crate.
pub mod annotations;
pub mod completion;
pub mod diagnostics;
pub mod directory;
pub mod document;
pub mod editing;
pub mod explorer;
pub mod global_search;
pub mod inline_completion;
pub mod navigation;
pub mod open;
pub mod runtime;
pub mod search;
pub mod session;
pub mod snippet;
pub mod source_control;
pub mod sync;
pub mod terminal;
pub mod view_state;
pub mod workspace;
