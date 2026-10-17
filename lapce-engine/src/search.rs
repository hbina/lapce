//! Search algorithms shared by all frontends.
use lapce_core::{
    selection::{SelRegion, Selection},
    word::WordCursor,
};
use lapce_xi_rope::{
    Cursor, Interval, Rope,
    find::{CaseMatching, find as rope_find},
};
use regex::{Regex, RegexBuilder};
use std::cmp::{max, min};

#[derive(Clone)]
pub struct FindSearchString {
    pub content: String,
    pub regex: Option<Regex>,
}

/// Query state, compilation and navigation shared by all editor frontends.
#[derive(Clone)]
pub struct SearchState {
    pub input: String,
    pub query: Option<FindSearchString>,
    pub error: Option<String>,
    pub case_matching: CaseMatching,
    pub is_regex: bool,
    pub whole_words: bool,
    pub revision: u64,
    compiled: (String, CaseMatching, bool, bool),
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            input: String::new(),
            query: None,
            error: None,
            case_matching: CaseMatching::CaseInsensitive,
            is_regex: false,
            whole_words: false,
            revision: 0,
            compiled: (String::new(), CaseMatching::CaseInsensitive, false, false),
        }
    }
}

impl SearchState {
    pub fn job(&self, text: Rope, document_revision: u64) -> Option<SearchJob> {
        Some(SearchJob {
            text,
            query: self.query.clone()?,
            case_matching: self.case_matching,
            whole_words: self.whole_words,
            query_revision: self.revision,
            document_revision,
        })
    }
    pub fn set_input(&mut self, input: &str) {
        self.input = input.into();
        self.refresh();
    }

    pub fn set_options(
        &mut self,
        case_matching: CaseMatching,
        is_regex: bool,
        whole_words: bool,
    ) {
        self.case_matching = case_matching;
        self.is_regex = is_regex;
        self.whole_words = whole_words;
        self.refresh();
    }

    /// Rebuild after changing options. An invalid regex remains an error with
    /// its original input preserved; it never falls back to literal matching.
    pub fn refresh(&mut self) {
        let key = (
            self.input.clone(),
            self.case_matching,
            self.is_regex,
            self.whole_words,
        );
        if self.compiled == key {
            return;
        }
        self.compiled = key;
        self.revision += 1;
        self.error = None;
        self.query = if self.input.is_empty() {
            None
        } else if self.is_regex {
            match RegexBuilder::new(&self.input)
                .size_limit(1_000_000)
                .case_insensitive(
                    self.case_matching == CaseMatching::CaseInsensitive,
                )
                .build()
            {
                Ok(regex) => Some(FindSearchString {
                    content: self.input.clone(),
                    regex: Some(regex),
                }),
                Err(error) => {
                    self.error = Some(error.to_string());
                    None
                }
            }
        } else {
            Some(FindSearchString {
                content: self.input.clone(),
                regex: None,
            })
        };
    }

    pub fn is_multiline_regex(&self) -> bool {
        self.query.as_ref().is_some_and(|query| {
            query.regex.is_some()
                && lapce_xi_rope::find::is_multiline_regex(&query.content)
        })
    }

    pub fn next(
        &self,
        text: &Rope,
        offset: usize,
        reverse: bool,
        wrap: bool,
    ) -> Option<(usize, usize)> {
        let query = self.query.as_ref()?;
        let offset = text
            .at_or_prev_codepoint_boundary(offset.min(text.len()))
            .unwrap_or(0);
        if !reverse {
            let next = scan_matches(
                text,
                query,
                offset,
                self.case_matching,
                self.whole_words,
                |start, _| start > offset,
            );
            if next.is_some() || !wrap {
                return next;
            }
            scan_matches(
                text,
                query,
                0,
                self.case_matching,
                self.whole_words,
                |_, _| true,
            )
        } else {
            let mut before = None;
            let mut last = None;
            scan_matches(
                text,
                query,
                0,
                self.case_matching,
                self.whole_words,
                |start, end| {
                    last = Some((start, end));
                    if start < offset {
                        before = last;
                    }
                    false
                },
            );
            before.or(if wrap { last } else { None })
        }
    }
}

/// An immutable snapshot that any host executor can run off the frontend thread.
pub struct SearchJob {
    text: Rope,
    query: FindSearchString,
    case_matching: CaseMatching,
    whole_words: bool,
    query_revision: u64,
    document_revision: u64,
}

pub struct SearchResult {
    pub occurrences: Selection,
    pub query_revision: u64,
    pub document_revision: u64,
}

impl SearchJob {
    pub fn run(self) -> SearchResult {
        let mut occurrences = Selection::new();
        find(
            &self.text,
            &self.query,
            0,
            self.text.len(),
            self.case_matching,
            self.whole_words,
            false,
            &mut occurrences,
        );
        SearchResult {
            occurrences,
            query_revision: self.query_revision,
            document_revision: self.document_revision,
        }
    }
}

impl SearchResult {
    pub fn is_current(&self, document_revision: u64, query_revision: u64) -> bool {
        self.document_revision == document_revision
            && self.query_revision == query_revision
    }
}

fn scan_matches(
    text: &Rope,
    query: &FindSearchString,
    from: usize,
    case_matching: CaseMatching,
    whole_words: bool,
    mut accept: impl FnMut(usize, usize) -> bool,
) -> Option<(usize, usize)> {
    let mut cursor = Cursor::new(text, from);
    let mut lines = text.lines_raw(from..text.len());
    while let Some(start) = rope_find(
        &mut cursor,
        &mut lines,
        case_matching,
        &query.content,
        query.regex.as_ref(),
    ) {
        let end = cursor.pos();
        if (!whole_words || is_matching_whole_words(text, start, end))
            && accept(start, end)
        {
            return Some((start, end));
        }
        if start == end {
            let Some(next) = text.next_codepoint_offset(end) else {
                break;
            };
            cursor.set(next);
        }
        lines = text.lines_raw(cursor.pos()..text.len());
    }
    // lines_raw has no item at EOF, but a regex can have an empty match there.
    // Keep the last line's context so anchors/boundaries are interpreted correctly.
    if !whole_words
        && empty_match_at_eof(text, query)
        && accept(text.len(), text.len())
    {
        return Some((text.len(), text.len()));
    }
    None
}

fn empty_match_at_eof(text: &Rope, query: &FindSearchString) -> bool {
    let Some(regex) = &query.regex else {
        return false;
    };
    let from = if lapce_xi_rope::find::is_multiline_regex(&query.content) {
        0
    } else {
        text.offset_of_line(text.line_of_offset(text.len()))
    };
    let context = text.subseq(Interval::new(from, text.len())).to_string();
    regex
        .find_at(&context, context.len())
        .is_some_and(|m| m.is_empty())
}

/// Checks if the start and end of a match is matching whole words.
pub fn is_matching_whole_words(text: &Rope, start: usize, end: usize) -> bool {
    if start == end {
        return false;
    }
    let mut word_end_cursor =
        WordCursor::new(text, text.prev_codepoint_offset(end).unwrap_or(start));
    let mut word_start_cursor =
        WordCursor::new(text, text.next_codepoint_offset(start).unwrap_or(end));

    if word_start_cursor.prev_code_boundary() != start {
        return false;
    }

    if word_end_cursor.next_code_boundary() != end {
        return false;
    }

    true
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
    let search_string = &search.content;

    let slop = if include_slop {
        search.content.len() * 2
    } else {
        0
    };

    // expand region to be able to find occurrences around the region's edges
    let expanded_start = (max(start, slop) - slop).min(text.len());
    let expanded_end = min(end.saturating_add(slop), text.len());
    let from = text
        .at_or_prev_codepoint_boundary(expanded_start)
        .unwrap_or(0);
    let to = if text.is_codepoint_boundary(expanded_end) {
        expanded_end
    } else {
        // xi-rope's at_or_next_codepoint_boundary does not advance an offset
        // inside a multibyte character. Advance from its valid start instead.
        text.at_or_prev_codepoint_boundary(expanded_end)
            .and_then(|start| text.next_codepoint_offset(start))
            .unwrap_or(text.len())
    };
    if from > to {
        return;
    }
    let mut to_cursor = Cursor::new(text, to);
    let _ = to_cursor.next_leaf();

    let sub_text = text.subseq(Interval::new(0, to_cursor.pos()));
    let mut find_cursor = Cursor::new(&sub_text, from);

    let mut raw_lines = text.lines_raw(from..to);

    while let Some(start) = rope_find(
        &mut find_cursor,
        &mut raw_lines,
        case_matching,
        search_string,
        search.regex.as_ref(),
    ) {
        let end = find_cursor.pos();

        if whole_words && !is_matching_whole_words(text, start, end) {
            if start == end {
                let Some(next) = text.next_codepoint_offset(end) else {
                    break;
                };
                if next > to {
                    break;
                }
                find_cursor.set(next);
            }
            raw_lines = text.lines_raw(find_cursor.pos()..to);
            continue;
        }

        let region = SelRegion::new(start, end, None);
        let (_, e) = occurrences.add_range_distinct(region);
        // in case of ambiguous search results (e.g. search "aba" in "ababa"),
        // the search result closer to the beginning of the file wins
        if e != end {
            // Skip the search result and keep the occurrence that is closer to
            // the beginning of the file. Re-align the cursor to the kept
            // occurrence
            find_cursor.set(e);
            raw_lines = text.lines_raw(find_cursor.pos()..to);
            continue;
        }

        // in case current cursor matches search result (for example query a* matches)
        // all cursor positions, then cursor needs to be increased so that search
        // continues at next position. Otherwise, search will result in overflow since
        // search will always repeat at current cursor position.
        if start == end {
            // determine whether end of text is reached and stop search or increase
            // cursor manually
            let Some(next) = text.next_codepoint_offset(end) else {
                break;
            };
            if next > to {
                break;
            }
            find_cursor.set(next);
        }

        // update line iterator so that line starts at current cursor position
        raw_lines = text.lines_raw(find_cursor.pos()..to);
    }
    if to == text.len()
        && !whole_words
        && empty_match_at_eof(text, search)
        && !occurrences
            .regions()
            .iter()
            .any(|region| region.start == to && region.end == to)
    {
        occurrences.add_range_distinct(SelRegion::new(to, to, None));
    }
}
