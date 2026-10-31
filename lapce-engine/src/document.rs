use lapce_core::buffer::rope_text::RopeText;
use lapce_rpc::buffer::BufferId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DocHistory {
    pub path: PathBuf,
    pub version: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum DocContent {
    /// A file at some location. This can be a remote path.
    File { path: PathBuf, read_only: bool },
    /// A local document, which doesn't need to be sync to the disk.
    Local,
    /// A document of an old version in the source control
    History(DocHistory),
    /// A new file which doesn't exist in the file system
    Scratch { id: BufferId, name: String },
}

impl DocContent {
    pub fn is_local(&self) -> bool {
        matches!(self, DocContent::Local)
    }

    pub fn is_file(&self) -> bool {
        matches!(self, DocContent::File { .. })
    }

    pub fn read_only(&self) -> bool {
        match self {
            DocContent::File { read_only, .. } => *read_only,
            DocContent::Local => false,
            DocContent::History(_) => true,
            DocContent::Scratch { .. } => false,
        }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            DocContent::File { path, .. } => Some(path),
            DocContent::Local => None,
            DocContent::History(_) => None,
            DocContent::Scratch { .. } => None,
        }
    }
}

/// Loading status shared by all views of a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentLoading {
    pub loaded: bool,
    pub error: Option<String>,
}

/// A document model with no view or event-loop ownership. Frontends may wrap
/// this model in their own observable state and keep per-view cursors separately.
#[derive(Clone)]
pub struct Document {
    pub id: BufferId,
    pub content: DocContent,
    pub loading: DocumentLoading,
    pub buffer: lapce_core::buffer::Buffer,
    pub syntax: lapce_core::syntax::Syntax,
    pub inlay_hints: Option<lapce_xi_rope::spans::Spans<lsp_types::InlayHint>>,
    pub backend_snapshot: Option<lapce_rpc::highlighting::TextSnapshot>,
    pub highlights:
        Option<std::sync::Arc<lapce_rpc::highlighting::HighlightSnapshot>>,
    disk_baseline: Option<lapce_xi_rope::Rope>,
    missing_on_disk: bool,
}

impl Document {
    pub fn new(content: DocContent, text: lapce_xi_rope::Rope) -> Self {
        let mut document = Self::empty(content);
        document.load(text);
        document
    }

    /// Construct a document before its text has arrived from the host.
    pub fn empty(content: DocContent) -> Self {
        let syntax = match &content {
            DocContent::File { path, .. } => {
                lapce_core::syntax::Syntax::without_parser(
                    lapce_core::language::LapceLanguage::from_path(path),
                )
            }
            DocContent::History(history) => {
                lapce_core::syntax::Syntax::without_parser(
                    lapce_core::language::LapceLanguage::from_path(&history.path),
                )
            }
            _ => lapce_core::syntax::Syntax::without_parser(
                lapce_core::language::LapceLanguage::PlainText,
            ),
        };
        Self {
            id: match &content {
                DocContent::Scratch { id, .. } => *id,
                _ => BufferId::next(),
            },
            loading: DocumentLoading {
                loaded: !content.is_file(),
                error: None,
            },
            content,
            buffer: lapce_core::buffer::Buffer::new(""),
            syntax,
            inlay_hints: None,
            backend_snapshot: None,
            highlights: None,
            disk_baseline: None,
            missing_on_disk: false,
        }
    }

    /// Rendering state accepts only spans for the exact authoritative text.
    pub fn accept_highlights(
        &mut self,
        highlights: lapce_rpc::highlighting::HighlightSnapshot,
    ) -> bool {
        let Some(snapshot) = &self.backend_snapshot else {
            return false;
        };
        if self.content.path() != Some(&highlights.path)
            || snapshot.instance.is_empty()
            || !highlights.valid_for(snapshot, &self.buffer.to_string())
            || self.highlights.as_ref().is_some_and(|old| {
                old.snapshot == highlights.snapshot
                    && old.generation >= highlights.generation
            })
        {
            return false;
        }
        let mut styles = lapce_xi_rope::spans::SpansBuilder::new(self.buffer.len());
        for span in &highlights.spans {
            styles.add_span(
                lapce_xi_rope::Interval::new(span.start, span.end),
                lapce_rpc::style::Style {
                    fg_color: Some(span.scope.clone()),
                },
            );
        }
        self.syntax.language =
            lapce_core::language::LapceLanguage::from_name(&highlights.language)
                .unwrap_or(self.syntax.language);
        self.syntax.styles = Some(styles.build());
        self.syntax.remote_structure = (!highlights.structure.is_empty())
            .then(|| std::sync::Arc::new(highlights.structure.clone()));
        self.syntax.normal_lines = highlights.normal_lines.clone();
        self.syntax.rev = self.buffer.rev();
        self.syntax.text = self.buffer.text().clone();
        self.syntax
            .update_lens_height(self.syntax.line_height, self.syntax.lens_height);
        self.highlights = Some(std::sync::Arc::new(highlights));
        true
    }

    pub fn invalidate_highlights(&mut self) {
        self.backend_snapshot = None;
        self.highlights = None;
        self.syntax.styles = None;
        self.syntax.remote_structure = None;
    }

    pub fn load(&mut self, text: lapce_xi_rope::Rope) {
        self.invalidate_highlights();
        self.inlay_hints = None;
        self.buffer.init_content(text);
        self.disk_baseline = None;
        self.missing_on_disk = false;
        self.loading.loaded = true;
        self.loading.error = None;
        self.buffer.detect_indent(|| {
            lapce_core::indent::IndentStyle::from_str(
                self.syntax.language.indent_unit(),
            )
        });
    }

    pub fn fail_loading(&mut self, message: String) {
        if !self.loading.loaded {
            self.loading.error = Some(message);
        }
    }

    pub fn set_syntax(&mut self, syntax: lapce_core::syntax::Syntax) {
        self.syntax = syntax;
    }

    pub fn insert(
        &mut self,
        cursor: &mut lapce_core::cursor::Cursor,
        text: &str,
        auto_closing_matching_pairs: bool,
        auto_surround: bool,
    ) -> Vec<crate::editing::DocumentDelta> {
        if self.content.read_only() {
            return Vec::new();
        }
        let deltas = crate::editing::insert(
            &mut self.buffer,
            &self.syntax,
            cursor,
            text,
            auto_closing_matching_pairs,
            auto_surround,
        );
        self.apply_annotation_deltas(&deltas);
        deltas
    }

    pub fn edit_command(
        &mut self,
        cursor: &mut lapce_core::cursor::Cursor,
        command: &lapce_core::command::EditCommand,
        clipboard: &mut impl lapce_core::register::Clipboard,
        register: &mut lapce_core::register::Register,
        config: lapce_core::editor::EditConf<'_>,
    ) -> Vec<crate::editing::DocumentDelta> {
        if self.content.read_only() && !command.not_changing_buffer() {
            return Vec::new();
        }
        let deltas = crate::editing::edit_command(
            &mut self.buffer,
            cursor,
            command,
            clipboard,
            register,
            config,
        );
        self.apply_annotation_deltas(&deltas);
        deltas
    }

    pub fn apply_text_edits(
        &mut self,
        edits: &[lsp_types::TextEdit],
    ) -> Option<crate::editing::DocumentDelta> {
        if self.content.read_only() {
            return None;
        }
        let edits = crate::editing::text_edits(&self.buffer, edits);
        self.raw_edit(&edits, lapce_core::editor::EditType::Completion)
    }

    pub fn raw_edit(
        &mut self,
        edits: &[(impl AsRef<lapce_core::selection::Selection>, &str)],
        edit_type: lapce_core::editor::EditType,
    ) -> Option<crate::editing::DocumentDelta> {
        if self.content.read_only() {
            return None;
        }
        let delta = self.buffer.edit(edits, edit_type);
        self.apply_annotation_deltas(std::slice::from_ref(&delta));
        Some(delta)
    }

    /// Host reloads also apply to read-only documents. This is distinct from
    /// a user edit, which must respect the document's access mode.
    pub fn reload(
        &mut self,
        text: lapce_xi_rope::Rope,
        pristine: bool,
    ) -> crate::editing::DocumentDelta {
        let delta = self.buffer.reload(text, pristine);
        if pristine {
            self.disk_baseline = None;
        }
        self.apply_annotation_deltas(std::slice::from_ref(&delta));
        delta
    }

    fn apply_annotation_deltas(&mut self, deltas: &[crate::editing::DocumentDelta]) {
        if !deltas.is_empty() {
            self.invalidate_highlights();
        }
        if let Some(hints) = self.inlay_hints.as_mut() {
            for (_, delta, _) in deltas {
                hints.apply_shape(delta);
            }
        }
    }

    pub fn set_inlay_hints(
        &mut self,
        revision: u64,
        hints: Vec<lsp_types::InlayHint>,
    ) -> bool {
        if self.buffer.rev() != revision {
            return false;
        }
        self.inlay_hints =
            Some(crate::annotations::inlay_spans(&self.buffer, hints));
        true
    }

    pub fn execute_motion_mode(
        &mut self,
        cursor: &mut lapce_core::cursor::Cursor,
        motion_mode: lapce_core::mode::MotionMode,
        range: std::ops::Range<usize>,
        is_vertical: bool,
        register: &mut lapce_core::register::Register,
    ) -> Vec<crate::editing::DocumentDelta> {
        if self.content.read_only()
            && !matches!(motion_mode, lapce_core::mode::MotionMode::Yank { .. })
        {
            return Vec::new();
        }
        let before = cursor.mode.clone();
        let deltas = lapce_core::editor::Action::execute_motion_mode(
            cursor,
            &mut self.buffer,
            motion_mode,
            range,
            is_vertical,
            register,
        );
        if !deltas.is_empty() {
            self.buffer.set_cursor_before(before);
            self.buffer.set_cursor_after(cursor.mode.clone());
            self.apply_annotation_deltas(&deltas);
        }
        deltas
    }

    /// Deleted files retain text and history, but only an explicit save may
    /// recreate them. Even an empty deleted document needs a manual save.
    pub fn mark_deleted(&mut self) {
        self.missing_on_disk = true;
    }
    pub fn missing_on_disk(&self) -> bool {
        self.missing_on_disk
    }
    pub fn mark_disk_present(&mut self) {
        self.missing_on_disk = false;
    }

    /// A disk event may reload pristine text, but must retain local edits and
    /// their history. The latest disk contents become the comparison baseline.
    pub fn disk_changed(
        &mut self,
        text: lapce_xi_rope::Rope,
    ) -> Option<crate::editing::DocumentDelta> {
        let pristine = self.content.read_only() || self.is_pristine();
        self.mark_disk_present();
        if pristine {
            Some(self.reload(text, true))
        } else {
            self.set_disk_baseline(text);
            None
        }
    }

    pub fn is_pristine(&self) -> bool {
        if self.missing_on_disk {
            return false;
        }

        match &self.disk_baseline {
            Some(disk) => disk.iter_chunks(..).flat_map(str::bytes).eq(self
                .buffer
                .text()
                .iter_chunks(..)
                .flat_map(str::bytes)),
            None => self.buffer.is_pristine(),
        }
    }
    pub fn set_disk_baseline(&mut self, disk: lapce_xi_rope::Rope) {
        self.disk_baseline = Some(disk);
    }

    pub fn mark_saved(&mut self, revision: u64) -> bool {
        let saved = crate::editing::mark_saved(&mut self.buffer, revision);
        if saved {
            self.missing_on_disk = false;
            self.disk_baseline = None;
        }
        saved
    }
}
