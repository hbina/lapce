use lapce_core::selection::Selection;
use lapce_engine::search::{FindSearchString, find};
use lapce_xi_rope::{Rope, find::CaseMatching};

#[test]
fn headless_search_filters_whole_words() {
    let text = Rope::from("hello shell hello\n");
    let search = FindSearchString {
        content: "hello".into(),
        regex: None,
    };
    let mut occurrences = Selection::new();
    find(
        &text,
        &search,
        0,
        text.len(),
        CaseMatching::Exact,
        true,
        false,
        &mut occurrences,
    );
    let ranges: Vec<_> = occurrences
        .regions()
        .iter()
        .map(|r| (r.start, r.end))
        .collect();
    assert_eq!(ranges, vec![(0, 5), (12, 17)]);
}

#[test]
fn zero_width_regex_search_advances_over_unicode() {
    let text = Rope::from("🌍a\n");
    let search = FindSearchString {
        content: "x*".into(),
        regex: Some(regex::Regex::new("x*").unwrap()),
    };
    for whole_words in [false, true] {
        let mut occurrences = Selection::new();
        find(
            &text,
            &search,
            0,
            text.len(),
            CaseMatching::Exact,
            whole_words,
            false,
            &mut occurrences,
        );
        assert!(
            occurrences
                .regions()
                .iter()
                .all(|r| text.is_codepoint_boundary(r.start)
                    && text.is_codepoint_boundary(r.end))
        );
        if whole_words {
            assert!(occurrences.regions().is_empty());
        }
    }
}

#[test]
fn shared_query_navigation_handles_direction_wrapping_and_case_changes() {
    let text = Rope::from("foo Foo foo");
    let mut state = lapce_engine::search::SearchState::default();
    state.set_input("foo");
    assert_eq!(state.next(&text, 0, false, false), Some((4, 7)));
    assert_eq!(state.next(&text, 8, true, false), Some((4, 7)));
    assert_eq!(state.next(&text, 8, false, false), None);
    assert_eq!(state.next(&text, 8, false, true), Some((0, 3)));
    assert_eq!(state.next(&text, 0, true, true), Some((8, 11)));
    state.set_options(CaseMatching::Exact, true, false);
    assert_eq!(state.next(&text, 0, false, false), Some((8, 11)));
    state.set_options(CaseMatching::CaseInsensitive, true, false);
    assert_eq!(state.next(&text, 0, false, false), Some((4, 7)));
}

#[test]
fn invalid_regex_keeps_input_and_recovers_when_regex_mode_changes() {
    let mut state = lapce_engine::search::SearchState::default();
    state.set_options(CaseMatching::Exact, true, false);
    state.set_input("[");
    assert!(state.error.is_some());
    assert!(state.query.is_none());
    assert_eq!(state.next(&Rope::from("["), 0, false, true), None);
    assert_eq!(state.input, "[");
    state.set_options(CaseMatching::Exact, false, false);
    assert!(state.error.is_none());
    assert_eq!(state.next(&Rope::from("["), 0, false, true), Some((0, 1)));
    let revision = state.revision;
    state.set_input("[");
    assert_eq!(state.revision, revision);
    state.set_input("");
    assert!(state.query.is_none());
    assert!(state.error.is_none());
}

#[test]
fn next_zero_width_match_always_advances_at_unicode_boundaries() {
    let text = Rope::from("🌍a");
    let mut state = lapce_engine::search::SearchState::default();
    state.set_options(CaseMatching::Exact, true, false);
    state.set_input("x*");
    assert_eq!(state.next(&text, 0, false, false), Some((4, 4)));
    assert_eq!(state.next(&text, 4, false, false), Some((5, 5)));
    assert_eq!(state.next(&text, 5, false, true), Some((0, 0)));
    assert_eq!(state.next(&text, 5, true, false), Some((4, 4)));
    assert_eq!(state.next(&text, 0, true, true), Some((5, 5)));
    state.set_options(CaseMatching::Exact, true, true);
    assert_eq!(state.next(&text, 0, false, true), None);
    assert_eq!(state.next(&text, 5, true, true), None);
}

#[test]
fn whole_word_navigation_handles_non_ascii_text_without_partial_codepoints() {
    let text = Rope::from("héllo hélloworld");
    let mut state = lapce_engine::search::SearchState::default();
    state.set_options(CaseMatching::Exact, false, true);
    state.set_input("héllo");
    assert_eq!(state.next(&text, 0, false, true), Some((0, 6)));
    assert_eq!(state.next(&text, text.len(), true, false), Some((0, 6)));
}

#[test]
fn incremental_search_rounds_partial_unicode_range_edges() {
    let text = Rope::from("🌍a");
    let search = FindSearchString {
        content: "🌍".into(),
        regex: None,
    };
    let mut occurrences = Selection::new();
    find(
        &text,
        &search,
        1,
        2,
        CaseMatching::Exact,
        false,
        false,
        &mut occurrences,
    );
    assert_eq!(occurrences.regions().len(), 1);
    assert_eq!(occurrences.regions()[0].start, 0);
    assert_eq!(occurrences.regions()[0].end, 4);
}

#[test]
fn background_search_snapshots_queries_and_checks_both_revisions() {
    let mut state = lapce_engine::search::SearchState::default();
    state.set_input("a");
    let query_revision = state.revision;
    let job = state.job(Rope::from("a b a"), 10).unwrap();
    state.set_input("b");
    let result = job.run();
    let ranges: Vec<_> = result
        .occurrences
        .regions()
        .iter()
        .map(|r| (r.start, r.end))
        .collect();
    assert_eq!(ranges, [(0, 1), (4, 5)]);
    assert!(result.is_current(10, query_revision));
    assert!(!result.is_current(11, query_revision));
    assert!(!result.is_current(10, state.revision));
    state.set_options(CaseMatching::Exact, true, false);
    state.set_input("[");
    assert!(state.job(Rope::from("a"), 10).is_none());
}

#[test]
fn search_jobs_include_zero_width_eof_matches_and_preserve_anchor_context() {
    let mut state = lapce_engine::search::SearchState::default();
    state.set_options(CaseMatching::Exact, true, false);
    state.set_input("x*");
    let result = state.job(Rope::from("🌍a"), 1).unwrap().run();
    let ranges: Vec<_> = result
        .occurrences
        .regions()
        .iter()
        .map(|r| (r.start, r.end))
        .collect();
    assert_eq!(ranges, [(0, 0), (4, 4), (5, 5)]);
    state.set_input("^");
    assert_eq!(state.next(&Rope::from("a"), 0, false, false), None);
    state.set_input("$");
    assert_eq!(state.next(&Rope::from("a"), 0, false, false), Some((1, 1)));
}
