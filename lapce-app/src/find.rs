use crate::reactive_model::ModelField;
use floem::reactive::{RwSignal, Scope, SignalGet, SignalUpdate, SignalWith};
use lapce_core::selection::Selection;
use lapce_engine::search::SearchState;
use lapce_xi_rope::{Rope, find::CaseMatching};
use serde::{Deserialize, Serialize};

pub type SearchField<T> = ModelField<SearchState, T>;

/// Indicates what changed in the find state.
#[derive(PartialEq, Debug, Clone)]
pub enum FindProgress {
    /// Incremental find is done/not running.
    Ready,

    /// The find process just started.
    Started,

    /// Incremental find is in progress. Keeps tracked of already searched range.
    InProgress(Selection),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FindStatus {
    /// Identifier for the current search query.
    id: usize,

    /// The current search query.
    chars: Option<String>,

    /// Whether the active search is case matching.
    case_sensitive: Option<bool>,

    /// Whether the search query is considered as regular expression.
    is_regex: Option<bool>,

    /// Query only matches whole words.
    whole_words: Option<bool>,

    /// Total number of matches.
    matches: usize,

    /// Line numbers which have find results.
    lines: Vec<usize>,
}

pub use lapce_engine::search::FindSearchString;

#[derive(Clone)]
pub struct Find {
    pub rev: SearchField<u64>,
    /// If the find is shown
    pub visual: RwSignal<bool>,
    /// The currently active search string.
    pub search_string: SearchField<Option<FindSearchString>>,
    /// The case matching setting for the currently active search.
    pub case_matching: SearchField<CaseMatching>,
    /// Query matches only whole words.
    pub whole_words: SearchField<bool>,
    /// The search query should be considered as regular expression.
    pub is_regex: SearchField<bool>,
    pub query_error: SearchField<Option<String>>,
    query_input: SearchField<String>,
    state: RwSignal<SearchState>,
    /// replace editor is shown
    /// replace editor is focused
    /// Triggered by changes in the search string
    pub triggered_by_changes: RwSignal<bool>,
}

impl Find {
    pub fn new(cx: Scope) -> Self {
        let state = cx.create_rw_signal(SearchState::default());
        let changed = cx.create_rw_signal(());
        let find = Self {
            state,
            rev: ModelField::new(
                state,
                changed,
                |s| &s.revision,
                |s| &mut s.revision,
                Some(SearchState::refresh),
            ),
            query_input: ModelField::new(
                state,
                changed,
                |s| &s.input,
                |s| &mut s.input,
                Some(SearchState::refresh),
            ),
            query_error: ModelField::new(
                state,
                changed,
                |s| &s.error,
                |s| &mut s.error,
                Some(SearchState::refresh),
            ),
            visual: cx.create_rw_signal(false),
            search_string: ModelField::new(
                state,
                changed,
                |s| &s.query,
                |s| &mut s.query,
                Some(SearchState::refresh),
            ),
            case_matching: ModelField::new(
                state,
                changed,
                |s| &s.case_matching,
                |s| &mut s.case_matching,
                Some(SearchState::refresh),
            ),
            whole_words: ModelField::new(
                state,
                changed,
                |s| &s.whole_words,
                |s| &mut s.whole_words,
                Some(SearchState::refresh),
            ),
            is_regex: ModelField::new(
                state,
                changed,
                |s| &s.is_regex,
                |s| &mut s.is_regex,
                Some(SearchState::refresh),
            ),
            triggered_by_changes: cx.create_rw_signal(false),
        };

        {
            let find = find.clone();
            cx.create_effect(move |_| {
                find.search_string.track();
                if find.query_input.with_untracked(|input| !input.is_empty()) {
                    find.triggered_by_changes.set(true);
                }
            });
        }

        find
    }

    /// Returns `true` if case sensitive, otherwise `false`
    pub fn case_sensitive(&self, tracked: bool) -> bool {
        match if tracked {
            self.case_matching.get()
        } else {
            self.case_matching.get_untracked()
        } {
            CaseMatching::Exact => true,
            CaseMatching::CaseInsensitive => false,
        }
    }

    /// Sets find case sensitivity.
    pub fn set_case_sensitive(&self, case_sensitive: bool) {
        if self.case_sensitive(false) == case_sensitive {
            return;
        }

        let case_matching = if case_sensitive {
            CaseMatching::Exact
        } else {
            CaseMatching::CaseInsensitive
        };
        self.case_matching.set(case_matching);
    }

    pub fn set_find(&self, search_string: &str) {
        if !search_string.is_empty() && !self.visual.get_untracked() {
            self.visual.set(true);
        }
        if self
            .query_input
            .with_untracked(|input| input == search_string)
        {
            return;
        }
        self.query_input.set(search_string.to_string());
    }

    pub fn search_state(&self) -> SearchState {
        self.state.get_untracked()
    }

    pub fn next(
        &self,
        text: &Rope,
        offset: usize,
        reverse: bool,
        wrap: bool,
    ) -> Option<(usize, usize)> {
        if !self.visual.get_untracked() {
            self.visual.set(true);
        }
        self.state
            .with_untracked(|state| state.next(text, offset, reverse, wrap))
    }

    pub fn is_multiline_regex(&self) -> bool {
        self.state.with_untracked(SearchState::is_multiline_regex)
    }

    pub fn job(
        &self,
        text: Rope,
        document_revision: u64,
    ) -> Option<lapce_engine::search::SearchJob> {
        self.state
            .with_untracked(|state| state.job(text, document_revision))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn find(
        text: &Rope,
        search: &FindSearchString,
        start: usize,
        end: usize,
        case_matching: CaseMatching,
        whole_words: bool,
        include_slop: bool,
        occurrences: &mut Selection,
    ) {
        lapce_engine::search::find(
            text,
            search,
            start,
            end,
            case_matching,
            whole_words,
            include_slop,
            occurrences,
        );
    }

    /// Execute the search on the provided text in the range provided by `start` and `end`.
    pub fn update_find(
        &self,
        text: &Rope,
        start: usize,
        end: usize,
        include_slop: bool,
        occurrences: &mut Selection,
    ) {
        if let Some(search) = self.search_string.get_untracked() {
            Self::find(
                text,
                &search,
                start,
                end,
                self.case_matching.get_untracked(),
                self.whole_words.get_untracked(),
                include_slop,
                occurrences,
            );
        }
    }
}

#[derive(Clone)]
pub struct FindResult {
    pub find_rev: RwSignal<u64>,
    pub progress: RwSignal<FindProgress>,
    pub occurrences: RwSignal<Selection>,
    pub search_string: RwSignal<Option<FindSearchString>>,
    pub case_matching: RwSignal<CaseMatching>,
    pub whole_words: RwSignal<bool>,
    pub is_regex: RwSignal<bool>,
}

impl FindResult {
    pub fn new(cx: Scope) -> Self {
        Self {
            find_rev: cx.create_rw_signal(0),
            progress: cx.create_rw_signal(FindProgress::Started),
            occurrences: cx.create_rw_signal(Selection::new()),
            search_string: cx.create_rw_signal(None),
            case_matching: cx.create_rw_signal(CaseMatching::Exact),
            whole_words: cx.create_rw_signal(false),
            is_regex: cx.create_rw_signal(false),
        }
    }

    pub fn reset(&self) {
        self.progress.set(FindProgress::Started);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_options_recompile_the_owned_engine_query_before_notifying_views() {
        let scope = Scope::new();
        let find = Find::new(scope);
        find.set_find("foo");
        find.is_regex.set(true);
        let text = Rope::from("foo Foo foo");
        assert_eq!(find.next(&text, 0, false, false), Some((4, 7)));
        find.set_case_sensitive(true);
        assert_eq!(find.next(&text, 0, false, false), Some((8, 11)));
        assert!(find.state.with_untracked(|state| state.is_regex));
        assert!(
            find.state
                .with_untracked(|state| state.case_matching == CaseMatching::Exact)
        );
        let revision = find.rev.get_untracked();
        find.set_find("foo");
        assert_eq!(find.rev.get_untracked(), revision);
        scope.dispose();
    }

    #[test]
    fn invalid_gui_regex_keeps_input_and_recovers_without_retyping() {
        let scope = Scope::new();
        let find = Find::new(scope);
        find.is_regex.set(true);
        find.set_find("[");
        assert!(find.query_error.get_untracked().is_some());
        assert!(find.search_string.get_untracked().is_none());
        assert_eq!(find.query_input.get_untracked(), "[");
        find.is_regex.set(false);
        assert!(find.query_error.get_untracked().is_none());
        assert_eq!(find.next(&Rope::from("["), 0, false, true), Some((0, 1)));
        scope.dispose();
    }
}
