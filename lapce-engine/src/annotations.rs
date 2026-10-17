//! Semantic annotation construction and edit transformations, without rendering.
use lapce_core::{buffer::rope_text::RopeText, rope_text_pos::RopeTextPosition};
use lapce_xi_rope::{
    Interval, Rope, RopeDelta, Transformer,
    spans::{Spans, SpansBuilder},
};
use lsp_types::{Diagnostic, InlayHint};

pub fn diagnostic_spans(
    text: &impl RopeTextPosition,
    diagnostics: impl IntoIterator<Item = Diagnostic>,
) -> Spans<Diagnostic> {
    let mut diagnostics: Vec<_> = diagnostics.into_iter().collect();
    diagnostics.sort_by_key(|diagnostic| diagnostic.range.start);
    let mut spans = SpansBuilder::new(text.len());
    for diagnostic in diagnostics {
        let start = text.offset_of_position(&diagnostic.range.start);
        let end = text.offset_of_position(&diagnostic.range.end);
        if start <= end
            && text.offset_to_position(start) == diagnostic.range.start
            && text.offset_to_position(end) == diagnostic.range.end
        {
            spans.add_span(Interval::new(start, end), diagnostic);
        }
    }
    spans.build()
}

pub fn inlay_spans(
    text: &impl RopeTextPosition,
    mut hints: Vec<InlayHint>,
) -> Spans<InlayHint> {
    hints.sort_by_key(|hint| hint.position);
    let mut spans = SpansBuilder::new(text.len());
    for hint in hints {
        let offset = text.offset_of_position(&hint.position);
        if text.offset_to_position(offset) == hint.position {
            spans
                .add_span(Interval::new(offset, (offset + 1).min(text.len())), hint);
        }
    }
    spans.build()
}

/// Transform a completion preview using the text before this particular edit.
/// Only an actual matching insertion consumes the preview's prefix.
pub fn transform_completion_lens(
    before: &Rope,
    delta: &RopeDelta,
    position: (usize, usize),
    completion: &str,
) -> Option<(String, (usize, usize))> {
    let offset = before.offset_of_line_col(position.0, position.1);
    let mut completion = completion;
    if let Some(inserted) = delta.as_simple_insert() {
        let (range, _) = delta.summary();
        if range.start() == offset {
            let inserted = inserted.to_string();
            completion = completion.strip_prefix(&inserted)?;
        }
    }
    let after = delta.apply(before);
    let offset = Transformer::new(delta).transform(offset, true);
    Some((completion.to_string(), after.offset_to_line_col(offset)))
}

/// A language-service highlight projected onto UTF-8 buffer offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentHighlight {
    pub start: usize,
    pub end: usize,
}
impl From<&DocumentHighlight> for lapce_core::selection::SelRegion {
    fn from(value: &DocumentHighlight) -> Self {
        Self {
            start: value.start,
            end: value.end,
            horiz: None,
        }
    }
}
impl DocumentHighlight {
    pub fn find_for_offset(list: &[Self], offset: usize) -> Option<usize> {
        // Ranges may overlap: sorting by start does not order their ends.
        list.iter()
            .position(|range| range.start <= offset && offset <= range.end)
    }
    pub fn sort(list: &mut [Self]) {
        list.sort_by_key(|range| (range.start, range.end));
    }
}

/// Ignore malformed language-server ranges rather than clamping them to unrelated text.
pub fn document_highlights(
    text: &impl RopeTextPosition,
    highlights: impl IntoIterator<Item = lsp_types::DocumentHighlight>,
) -> Vec<DocumentHighlight> {
    let mut result: Vec<_> = highlights
        .into_iter()
        .filter_map(|highlight| {
            let start = text.offset_of_position(&highlight.range.start);
            let end = text.offset_of_position(&highlight.range.end);
            (start <= end
                && text.offset_to_position(start) == highlight.range.start
                && text.offset_to_position(end) == highlight.range.end)
                .then_some(DocumentHighlight { start, end })
        })
        .collect();
    DocumentHighlight::sort(&mut result);
    result
}

/// Build semantic spans for a text snapshot, allowing the host to cancel stale work.
pub fn semantic_spans(
    text: &Rope,
    styles: impl IntoIterator<Item = lapce_rpc::style::LineStyle>,
    mut cancelled: impl FnMut() -> bool,
) -> Option<Spans<lapce_rpc::style::Style>> {
    if cancelled() {
        return None;
    }
    let mut styles: Vec<_> = styles.into_iter().collect();
    styles.sort_by_key(|style| (style.start, style.end));
    let mut spans = SpansBuilder::new(text.len());
    let mut previous_end = 0;
    for style in styles {
        if cancelled() {
            return None;
        }
        if style.start < previous_end
            || style.start >= style.end
            || style.end > text.len()
            || !text.is_codepoint_boundary(style.start)
            || !text.is_codepoint_boundary(style.end)
        {
            continue;
        }
        spans.add_span(Interval::new(style.start, style.end), style.style);
        previous_end = style.end;
    }
    if cancelled() {
        return None;
    }
    Some(spans.build())
}
