//! Diagnostics shared by document and workspace consumers, without panel state.
use lapce_xi_rope::{
    Rope, RopeDelta,
    spans::{Spans, SpansBuilder},
};
use lsp_types::{Diagnostic, DiagnosticSeverity};

/// Publication snapshot shared by GUI and headless workspace controllers.
pub struct DiagnosticPublication {
    items: Vec<Diagnostic>,
    version: Option<i32>,
    snapshot: Option<(Rope, u64)>,
}
impl DiagnosticPublication {
    pub fn prepare(
        items: Vec<Diagnostic>,
        version: Option<i32>,
        document: Option<&crate::session::SessionDocument>,
    ) -> Option<Self> {
        let document = document.filter(|document| document.loading.loaded);
        let version = match (version, document) {
            (Some(version), Some(document)) => Some(
                document
                    .synchronization
                    .as_ref()
                    .map_or(Some(version), |sync| sync.local_version(version))?,
            ),
            (version, _) => version,
        };
        use lapce_core::buffer::rope_text::RopeText;
        let snapshot = document
            .map(|document| (document.buffer.text().clone(), document.buffer.rev()));
        Some(Self {
            items,
            version,
            snapshot,
        })
    }
    pub fn apply(self, state: &mut DiagnosticState) -> bool {
        state.publish(
            self.items,
            self.version,
            self.snapshot.as_ref().map(|(text, rev)| (text, *rev)),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorDiagnostic {
    pub range: Option<(usize, usize)>,
    pub diagnostic: Diagnostic,
}

#[derive(Clone)]
pub struct DiagnosticState {
    pub items: im::Vector<Diagnostic>,
    pub spans: Spans<Diagnostic>,
    pub version: Option<i32>,
    mapped: bool,
}

impl Default for DiagnosticState {
    fn default() -> Self {
        Self {
            items: im::Vector::new(),
            spans: SpansBuilder::new(0).build(),
            version: None,
            mapped: false,
        }
    }
}

impl DiagnosticState {
    pub fn counts(&self) -> (usize, usize) {
        let errors = self
            .items
            .iter()
            .filter(|item| item.severity == Some(DiagnosticSeverity::ERROR))
            .count();
        let warnings = self
            .items
            .iter()
            .filter(|item| item.severity == Some(DiagnosticSeverity::WARNING))
            .count();
        (errors, warnings)
    }
    pub fn publish(
        &mut self,
        mut diagnostics: Vec<Diagnostic>,
        version: Option<i32>,
        document: Option<(&Rope, u64)>,
    ) -> bool {
        if let (Some(version), Some((_, revision))) = (version, document) {
            if version != revision as i32 {
                return false;
            }
        }
        diagnostics.sort_by_key(|diagnostic| diagnostic.range.start);
        self.items = diagnostics.into_iter().collect();
        self.version = version;
        self.mapped = false;
        self.spans = SpansBuilder::new(0).build();
        if let Some((text, revision)) = document {
            self.bind(text, revision);
        }
        true
    }

    /// Map unopened-file diagnostics when its document becomes available.
    /// Versioned data from a different snapshot must not acquire live offsets.
    pub fn bind(&mut self, text: &Rope, revision: u64) -> bool {
        if self
            .version
            .is_some_and(|version| version != revision as i32)
        {
            self.items.clear();
            self.spans = SpansBuilder::new(text.len()).build();
            self.mapped = true;
            return false;
        }
        self.spans =
            crate::annotations::diagnostic_spans(text, self.items.iter().cloned());
        self.mapped = true;
        true
    }

    pub fn apply_delta(&mut self, delta: &RopeDelta) {
        if self.mapped {
            self.spans.apply_shape(delta);
        }
    }

    pub fn entries(
        &self,
        severity: DiagnosticSeverity,
    ) -> im::Vector<EditorDiagnostic> {
        if self.mapped {
            self.spans
                .iter()
                .filter(|(_, diagnostic)| diagnostic.severity == Some(severity))
                .map(|(range, diagnostic)| EditorDiagnostic {
                    range: Some((range.start, range.end)),
                    diagnostic: diagnostic.clone(),
                })
                .collect()
        } else {
            self.items
                .iter()
                .filter(|diagnostic| diagnostic.severity == Some(severity))
                .map(|diagnostic| EditorDiagnostic {
                    range: None,
                    diagnostic: diagnostic.clone(),
                })
                .collect()
        }
    }
}
