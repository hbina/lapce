use crate::command::InternalCommand;
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    ops::Range,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{self},
    },
};

use floem::{
    ViewId,
    ext_event::create_ext_action,
    keyboard::Modifiers,
    peniko::Color,
    reactive::{
        ReadSignal, RwSignal, Scope, SignalGet, SignalUpdate, SignalWith, batch,
    },
    text::{Attrs, AttrsList, FamilyOwned, TextLayout},
    views::editor::{
        CursorInfo, Editor, EditorStyle,
        actions::CommonAction,
        command::{Command, CommandExecuted},
        id::EditorId,
        layout::{LineExtraStyle, TextLayoutLine},
        phantom_text::{PhantomText, PhantomTextKind, PhantomTextLine},
        text::{Document, DocumentPhantom, PreeditData, Styling, SystemClipboard},
        view::{ScreenLines, ScreenLinesBase},
    },
};
use itertools::Itertools;
use lapce_core::{
    buffer::{
        Buffer, InvalLines,
        diff::{DiffLines, rope_diff},
        rope_text::RopeText,
    },
    char_buffer::CharBuffer,
    command::EditCommand,
    cursor::{Cursor, CursorAffinity},
    editor::{EditConf, EditType},
    language::LapceLanguage,
    line_ending::LineEnding,
    mode::MotionMode,
    register::Register,
    selection::{InsertDrift, Selection},
    style::line_styles,
    syntax::{BracketParser, Syntax, edit::SyntaxEdit},
    word::{CharClassification, WordCursor, get_char_property},
};
use lapce_rpc::{
    buffer::BufferId,
    proxy::ProxyResponse,
    style::{LineStyle, LineStyles, Style},
};
use lapce_xi_rope::{Rope, RopeDelta, Transformer, spans::Spans};
use lsp_types::{DiagnosticSeverity, InlayHint, InlayHintLabel, TextEdit};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{
    command::{CommandKind, LapceCommand},
    config::{LapceConfig, color::LapceColor},
    editor::{EditorData, compute_screen_lines, gutter::FoldingRanges},
    find::{Find, FindProgress, FindResult},
    history::DocumentHistory,
    keypress::KeyPressFocus,
    main_split::Editors,
    panel::{
        document_symbol::{SymbolData, SymbolInformationItemData},
        kind::PanelKind,
    },
    window_tab::{CommonData, Focus},
    workspace::LapceWorkspace,
};

pub mod diagnostics;
pub use diagnostics::DiagnosticData;
pub use lapce_engine::diagnostics::EditorDiagnostic;

pub use lapce_engine::document::{DocContent, DocHistory};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocInfo {
    pub workspace: LapceWorkspace,
    pub path: PathBuf,
    pub scroll_offset: (f64, f64),
    pub cursor_offset: usize,
}

#[derive(Clone, Debug)]
enum DocumentHighlightsStatus {
    Ready,
    Busy(usize),
}

pub use lapce_engine::annotations::DocumentHighlight;

pub mod model;
use model::{DocumentField, DocumentModel};

#[derive(Clone)]
pub struct Doc {
    pub scope: Scope,
    pub buffer_id: BufferId,
    pub content: DocumentField<DocContent>,
    pub cache_rev: RwSignal<u64>,
    /// Whether the buffer's content has been loaded/initialized into the buffer.
    pub loaded: DocumentField<bool>,
    pub load_error: DocumentField<Option<String>>,
    synchronization: DocumentField<Option<lapce_engine::sync::DocumentSync>>,
    operation_actions:
        Rc<RefCell<HashMap<u64, Box<dyn FnOnce(Result<(), String>)>>>>,
    request_actions: Rc<
        RefCell<
            HashMap<
                u64,
                Box<dyn FnOnce(Result<ProxyResponse, lapce_rpc::RpcError>)>,
            >,
        >,
    >,
    pub buffer: DocumentField<Buffer>,
    pub syntax: DocumentField<Syntax>,
    model: DocumentModel,
    semantic_styles: RwSignal<Option<Spans<Style>>>,
    /// Inlay hints for the document
    pub inlay_hints: DocumentField<Option<Spans<InlayHint>>>,
    /// Current completion lens text, if any.
    /// This will be displayed even on views that are not focused.
    pub completion_lens: RwSignal<Option<String>>,
    /// (line, col)
    pub completion_pos: RwSignal<(usize, usize)>,

    /// Current inline completion text, if any.
    /// This will be displayed even on views that are not focused.
    pub inline_completion: RwSignal<Option<String>>,
    /// (line, col)
    pub inline_completion_pos: RwSignal<(usize, usize)>,

    pub folding_ranges: RwSignal<FoldingRanges>,

    /// Stores information about different versions of the document from source control.
    histories: RwSignal<im::HashMap<String, DocumentHistory>>,
    pub head_changes: RwSignal<im::Vector<DiffLines>>,

    line_styles: Rc<RefCell<LineStyles>>,
    pub parser: Rc<RefCell<BracketParser>>,

    /// A cache for the sticky headers which maps a line to the lines it should show in the header.
    pub sticky_headers: Rc<RefCell<HashMap<usize, Option<Vec<usize>>>>>,

    pub preedit: PreeditData,

    pub find_result: FindResult,

    /// The diagnostics for the document
    pub diagnostics: DiagnosticData,

    editors: Editors,
    pub common: Rc<CommonData>,

    pub document_symbol_data: RwSignal<Option<SymbolData>>,

    get_highlights_status: RwSignal<DocumentHighlightsStatus>,
    pub highlights: RwSignal<Option<Vec<DocumentHighlight>>>,
}

impl Doc {
    pub fn new(
        cx: Scope,
        path: PathBuf,
        diagnostics: DiagnosticData,
        editors: Editors,
        common: Rc<CommonData>,
    ) -> Self {
        let model = DocumentModel::new(
            cx,
            lapce_engine::document::Document::empty(DocContent::File {
                path,
                read_only: true,
            }),
        );
        let config = common.config.get_untracked();
        Doc {
            scope: cx,
            buffer_id: model.id(),
            buffer: model.buffer(),
            syntax: model.syntax(),
            model,
            line_styles: Rc::new(RefCell::new(HashMap::new())),
            parser: Rc::new(RefCell::new(BracketParser::new(
                String::new(),
                config.editor.bracket_pair_colorization,
                config.editor.bracket_colorization_limit,
            ))),
            semantic_styles: cx.create_rw_signal(None),
            inlay_hints: model.inlay_hints(),
            diagnostics,
            completion_lens: cx.create_rw_signal(None),
            completion_pos: cx.create_rw_signal((0, 0)),
            inline_completion: cx.create_rw_signal(None),
            inline_completion_pos: cx.create_rw_signal((0, 0)),
            cache_rev: cx.create_rw_signal(0),
            content: model.content(),
            loaded: model.loaded(),
            load_error: model.load_error(),
            synchronization: model.synchronization(),
            operation_actions: Rc::new(RefCell::new(HashMap::new())),
            request_actions: Rc::new(RefCell::new(HashMap::new())),
            histories: cx.create_rw_signal(im::HashMap::new()),
            head_changes: cx.create_rw_signal(im::Vector::new()),
            sticky_headers: Rc::new(RefCell::new(HashMap::new())),
            find_result: FindResult::new(cx),
            preedit: PreeditData::new(cx),
            editors,
            common,
            document_symbol_data: cx.create_rw_signal(None),
            folding_ranges: cx.create_rw_signal(FoldingRanges::default()),
            get_highlights_status: cx
                .create_rw_signal(DocumentHighlightsStatus::Ready),
            highlights: cx.create_rw_signal(None),
        }
    }

    pub fn new_local(cx: Scope, editors: Editors, common: Rc<CommonData>) -> Doc {
        Self::new_content(cx, DocContent::Local, editors, common)
    }

    pub fn new_content(
        cx: Scope,
        content: DocContent,
        editors: Editors,
        common: Rc<CommonData>,
    ) -> Doc {
        let cx = cx.create_child();
        let model =
            DocumentModel::new(cx, lapce_engine::document::Document::empty(content));
        let config = common.config.get_untracked();
        Self {
            scope: cx,
            buffer_id: model.id(),
            buffer: model.buffer(),
            syntax: model.syntax(),
            model,
            line_styles: Rc::new(RefCell::new(HashMap::new())),
            parser: Rc::new(RefCell::new(BracketParser::new(
                String::new(),
                config.editor.bracket_pair_colorization,
                config.editor.bracket_colorization_limit,
            ))),
            semantic_styles: cx.create_rw_signal(None),
            inlay_hints: model.inlay_hints(),
            diagnostics: DiagnosticData::new(cx),
            completion_lens: cx.create_rw_signal(None),
            completion_pos: cx.create_rw_signal((0, 0)),
            inline_completion: cx.create_rw_signal(None),
            inline_completion_pos: cx.create_rw_signal((0, 0)),
            cache_rev: cx.create_rw_signal(0),
            content: model.content(),
            histories: cx.create_rw_signal(im::HashMap::new()),
            head_changes: cx.create_rw_signal(im::Vector::new()),
            sticky_headers: Rc::new(RefCell::new(HashMap::new())),
            loaded: model.loaded(),
            load_error: model.load_error(),
            synchronization: model.synchronization(),
            operation_actions: Rc::new(RefCell::new(HashMap::new())),
            request_actions: Rc::new(RefCell::new(HashMap::new())),
            find_result: FindResult::new(cx),
            preedit: PreeditData::new(cx),
            editors,
            common,
            document_symbol_data: cx.create_rw_signal(None),
            folding_ranges: cx.create_rw_signal(FoldingRanges::default()),
            get_highlights_status: cx
                .create_rw_signal(DocumentHighlightsStatus::Ready),
            highlights: cx.create_rw_signal(None),
        }
    }

    pub fn new_history(
        cx: Scope,
        content: DocContent,
        editors: Editors,
        common: Rc<CommonData>,
    ) -> Doc {
        let config = common.config.get_untracked();
        let model =
            DocumentModel::new(cx, lapce_engine::document::Document::empty(content));
        Self {
            scope: cx,
            buffer_id: model.id(),
            buffer: model.buffer(),
            syntax: model.syntax(),
            model,
            line_styles: Rc::new(RefCell::new(HashMap::new())),
            parser: Rc::new(RefCell::new(BracketParser::new(
                String::new(),
                config.editor.bracket_pair_colorization,
                config.editor.bracket_colorization_limit,
            ))),
            semantic_styles: cx.create_rw_signal(None),
            inlay_hints: model.inlay_hints(),
            diagnostics: DiagnosticData::new(cx),
            completion_lens: cx.create_rw_signal(None),
            completion_pos: cx.create_rw_signal((0, 0)),
            inline_completion: cx.create_rw_signal(None),
            inline_completion_pos: cx.create_rw_signal((0, 0)),
            cache_rev: cx.create_rw_signal(0),
            content: model.content(),
            sticky_headers: Rc::new(RefCell::new(HashMap::new())),
            loaded: model.loaded(),
            load_error: model.load_error(),
            synchronization: model.synchronization(),
            operation_actions: Rc::new(RefCell::new(HashMap::new())),
            request_actions: Rc::new(RefCell::new(HashMap::new())),
            histories: cx.create_rw_signal(im::HashMap::new()),
            head_changes: cx.create_rw_signal(im::Vector::new()),
            find_result: FindResult::new(cx),
            preedit: PreeditData::new(cx),
            editors,
            common,
            document_symbol_data: cx.create_rw_signal(None),
            folding_ranges: cx.create_rw_signal(FoldingRanges::default()),
            get_highlights_status: cx
                .create_rw_signal(DocumentHighlightsStatus::Ready),
            highlights: cx.create_rw_signal(None),
        }
    }

    /// Create a styling instance for this doc
    pub fn styling(self: &Rc<Doc>) -> Rc<DocStyling> {
        Rc::new(DocStyling {
            config: self.common.config,
            doc: self.clone(),
        })
    }

    /// Create an [`Editor`] instance from this [`Doc`]. Note that this needs to be registered
    /// appropriately to create the [`EditorData`] and such.
    pub fn create_editor(
        self: &Rc<Doc>,
        cx: Scope,
        id: EditorId,
        is_local: bool,
    ) -> Editor {
        let common = &self.common;
        let config = common.config.get_untracked();
        let modal = config.core.modal && !is_local;

        let register = common.register;
        // TODO: we could have these Rcs created once and stored somewhere, maybe on
        // common, to avoid recreating them everytime.
        let cursor_info = CursorInfo {
            blink_interval: Rc::new(move || config.editor.blink_interval()),
            blink_timer: common.window_common.cursor_blink_timer,
            hidden: common.window_common.hide_cursor,
            should_blink: Rc::new(should_blink(common.focus, common.keyboard_focus)),
        };
        let mut editor =
            Editor::new_direct(cx, id, self.clone(), self.styling(), modal);

        editor.register = register;
        editor.cursor_info = cursor_info;
        editor.ime_allowed = common.window_common.ime_allowed;

        editor.recreate_view_effects();

        editor
    }

    fn editor_data(&self, id: EditorId) -> Option<EditorData> {
        self.editors.editor_untracked(id)
    }

    pub fn syntax(&self) -> DocumentField<Syntax> {
        self.syntax
    }

    pub fn set_syntax(&self, syntax: Syntax) {
        batch(|| {
            self.model.set_syntax(syntax);
            if self.semantic_styles.with_untracked(|s| s.is_none()) {
                self.clear_style_cache();
            }
            self.clear_sticky_headers_cache();
        });
    }

    /// Set the syntax highlighting this document should use.
    pub fn set_language(&self, language: LapceLanguage) {
        self.set_syntax(Syntax::from_language(language));
    }

    pub fn find(&self) -> &Find {
        &self.common.find
    }

    /// Capture a headless opening attempt for this document.
    pub fn opening(&self) -> Option<lapce_engine::open::SessionOpening> {
        self.model.opening()
    }

    pub fn fail_loading(&self, message: String) {
        self.model.fail_loading(message);
    }

    pub fn finish_opening(
        &self,
        opening: lapce_engine::open::SessionOpening,
        response: Result<ProxyResponse, lapce_rpc::RpcError>,
    ) -> Option<Result<(), lapce_rpc::RpcError>> {
        batch(|| {
            let result = self.model.finish_opening(opening, response);
            if matches!(&result, Some(Ok(()))) {
                self.on_update(None);
                self.init_parser();
                self.init_diagnostics();
                self.retrieve_head();
            }
            result
        })
    }

    /// Whether the underlying buffer is loaded.
    pub fn loaded(&self) -> bool {
        self.loaded.get_untracked()
    }

    //// Initialize the content with some text, this marks the document as loaded.
    pub fn init_content(&self, content: Rope) {
        batch(|| {
            self.model.update_buffer(|document| document.load(content));
            self.on_update(None);
            self.init_parser();
            self.init_diagnostics();
            self.retrieve_head();
        });
    }

    fn init_parser(&self) {
        let code = self.buffer.get_untracked().to_string();
        self.syntax.with_untracked(|syntax| {
            if syntax.styles.is_some() {
                self.parser.borrow_mut().update_code(
                    code,
                    &self.buffer.get_untracked(),
                    Some(syntax),
                );
            } else {
                self.parser.borrow_mut().update_code(
                    code,
                    &self.buffer.get_untracked(),
                    None,
                );
            }
        });
    }

    /// Reload the document's content, and is what you should typically use when you want to *set*
    /// an existing document's content.
    pub fn reload(&self, content: Rope, set_pristine: bool) {
        // self.inlay_hints = None;
        let delta = self
            .model
            .update_buffer(|document| document.reload(content, set_pristine))
            .unwrap();
        self.apply_deltas(&[delta]);
    }

    pub fn handle_file_changed(&self, content: Rope) {
        if let Some(delta) = self.model.reload_from_disk(content) {
            self.apply_deltas(&[delta]);
        }
    }

    pub fn do_insert(
        &self,
        cursor: &mut Cursor,
        s: &str,
        config: &LapceConfig,
    ) -> Vec<(Rope, RopeDelta, InvalLines)> {
        if self.content.with_untracked(|c| c.read_only()) {
            return Vec::new();
        }

        let deltas = self
            .model
            .update_buffer(|document| {
                document.insert(
                    cursor,
                    s,
                    config.editor.auto_closing_matching_pairs,
                    config.editor.auto_surround,
                )
            })
            .unwrap();
        self.apply_deltas(&deltas);
        deltas
    }

    pub fn do_raw_edit(
        &self,
        edits: &[(impl AsRef<Selection>, &str)],
        edit_type: EditType,
    ) -> Option<(Rope, RopeDelta, InvalLines)> {
        if self.content.with_untracked(|c| c.read_only()) {
            return None;
        }

        let (text, delta, inval_lines) = self
            .model
            .update_buffer(|document| document.raw_edit(edits, edit_type))
            .flatten()?;
        self.apply_deltas(&[(text.clone(), delta.clone(), inval_lines.clone())]);
        Some((text, delta, inval_lines))
    }

    pub fn do_edit(
        &self,
        cursor: &mut Cursor,
        cmd: &EditCommand,
        modal: bool,
        register: &mut Register,
        smart_tab: bool,
    ) -> Vec<(Rope, RopeDelta, InvalLines)> {
        if self.content.with_untracked(|c| c.read_only())
            && !cmd.not_changing_buffer()
        {
            return Vec::new();
        }

        let mut clipboard = SystemClipboard::new();
        let deltas = self
            .model
            .update_buffer(|document| {
                let comment_token = document.syntax.language.comment_token();
                document.edit_command(
                    cursor,
                    cmd,
                    &mut clipboard,
                    register,
                    EditConf {
                        comment_token,
                        modal,
                        smart_tab,
                        keep_indent: true,
                        auto_indent: true,
                    },
                )
            })
            .unwrap();
        if !deltas.is_empty() {
            self.apply_deltas(&deltas);
        }

        deltas
    }

    pub fn apply_deltas(&self, deltas: &[(Rope, RopeDelta, InvalLines)]) {
        batch(|| {
            for (before, delta, _inval) in deltas {
                self.update_styles(delta);
                self.update_diagnostics(delta);
                self.update_completion_lens(before, delta);
                self.update_find_result(delta);
            }
            if !self.model.synchronize_edits(deltas) {
                self.common.internal_command.send(InternalCommand::ShowAlert {
                    title: "Document is not connected".into(),
                    msg: "Local edits are preserved, but the backend document is not available for updates.".into(),
                    buttons: Vec::new(),
                });
            }
        });

        self.dispatch_sync();
        let edits = lapce_engine::syntax::edits_from_deltas(deltas);
        self.on_update(Some(edits));
    }

    pub fn is_pristine(&self) -> bool {
        self.model.is_pristine(false)
    }

    pub fn is_pristine_tracked(&self) -> bool {
        self.model.is_pristine(true)
    }

    /// Get the buffer's current revision. This is used to track whether the buffer has changed.
    pub fn rev(&self) -> u64 {
        self.buffer.with_untracked(|b| b.rev())
    }

    /// Get the buffer's line-ending.
    /// Note: this may not be the same as what the actual line endings in the file are, rather this
    /// is what the line-ending is set to (and what it will be saved as).
    pub fn line_ending(&self) -> LineEnding {
        self.buffer.with_untracked(|b| b.line_ending())
    }

    fn on_update(&self, edits: Option<Vec<SyntaxEdit>>) {
        batch(|| {
            self.trigger_syntax_change(edits);
            self.trigger_head_change();
            self.get_inlay_hints();
            self.find_result.reset();
            self.get_semantic_styles();
            self.do_bracket_colorization();
            self.clear_style_cache();
            self.get_document_symbol();
            self.get_folding_range();
        });
    }

    fn do_bracket_colorization(&self) {
        if self.parser.borrow().active {
            self.syntax.with_untracked(|syntax| {
                if syntax.rev == self.rev() && syntax.styles.is_some() {
                    self.parser.borrow_mut().update_code(
                        self.buffer.get_untracked().to_string(),
                        &self.buffer.get_untracked(),
                        Some(syntax),
                    );
                } else {
                    self.parser.borrow_mut().update_code(
                        self.buffer.get_untracked().to_string(),
                        &self.buffer.get_untracked(),
                        None,
                    );
                }
            })
        }
    }

    pub fn do_text_edit(&self, edits: &[TextEdit]) {
        if let Some(delta) = self
            .model
            .update_buffer(|document| document.apply_text_edits(edits))
            .flatten()
        {
            self.apply_deltas(&[delta]);
        }
    }

    /// Update the styles after an edit, so the highlights are at the correct positions.
    /// This does not do a reparse of the document itself.
    fn update_styles(&self, delta: &RopeDelta) {
        batch(|| {
            self.semantic_styles.update(|styles| {
                if let Some(styles) = styles.as_mut() {
                    styles.apply_shape(delta);
                }
            });
            self.syntax.update(|syntax| {
                if let Some(styles) = syntax.styles.as_mut() {
                    styles.apply_shape(delta);
                }
                syntax.lens.apply_delta(delta);
            });
        });
    }

    /// Schedule a syntax parse of the current document revision.
    pub fn trigger_syntax_change(&self, edits: Option<Vec<SyntaxEdit>>) {
        let Some(job) = self.model.syntax_job(edits) else {
            return;
        };
        let doc = self.clone();
        let send = create_ext_action(self.scope, move |result| {
            if doc.model.accept_syntax(result) {
                doc.do_bracket_colorization();
                doc.clear_style_cache();
                doc.clear_sticky_headers_cache();
            }
        });
        rayon::spawn(move || {
            if let Some(result) = job.run() {
                send(result);
            }
        });
    }

    fn clear_style_cache(&self) {
        self.line_styles.borrow_mut().clear();
        self.clear_text_cache();
    }

    /// Inform any dependents on this document that they should clear any cached text.
    pub fn clear_text_cache(&self) {
        self.cache_rev.try_update(|cache_rev| {
            *cache_rev += 1;

            // TODO: ???
            // Update the text layouts within the callback so that those alerted to cache rev
            // will see the now empty layouts.
            // self.text_layouts.borrow_mut().clear(*cache_rev, None);
        });
    }

    fn clear_sticky_headers_cache(&self) {
        self.sticky_headers.borrow_mut().clear();
    }

    /// Get the active style information, either the semantic styles or the
    /// tree-sitter syntax styles.
    fn styles(&self) -> Option<Spans<Style>> {
        if let Some(semantic_styles) = self.semantic_styles.get_untracked() {
            Some(semantic_styles)
        } else {
            self.syntax.with_untracked(|syntax| syntax.styles.clone())
        }
    }

    /// Get the style information for the particular line from semantic/syntax highlighting.
    /// This caches the result if possible.
    pub fn line_style(&self, line: usize) -> Arc<Vec<LineStyle>> {
        if self.line_styles.borrow().get(&line).is_none() {
            let styles = self.styles();

            let line_styles = styles
                .map(|styles| {
                    let text =
                        self.buffer.with_untracked(|buffer| buffer.text().clone());
                    line_styles(&text, line, &styles)
                })
                .unwrap_or_default();
            self.line_styles
                .borrow_mut()
                .insert(line, Arc::new(line_styles));
        }
        self.line_styles.borrow().get(&line).cloned().unwrap()
    }

    /// Request semantic styles for the buffer from the LSP through the proxy.
    pub fn get_semantic_styles(&self) {
        if !self.backend_ready() {
            return;
        }
        if !self.loaded() {
            return;
        }

        let (atomic_rev, rev, text) = self
            .buffer
            .with_untracked(|b| (b.atomic_rev(), b.rev(), b.text().clone()));

        let doc = self.clone();
        let send = create_ext_action(self.scope, move |styles| {
            if let Some(styles) = styles {
                if doc.buffer.with_untracked(|b| b.rev()) == rev {
                    doc.semantic_styles.set(Some(styles));
                    doc.clear_style_cache();
                }
            }
        });

        self.request_navigation(
            lapce_engine::navigation::NavigationQuery::SemanticTokens,
            move |result| {
                if let Ok(ProxyResponse::GetSemanticTokens { styles }) = result {
                    if atomic_rev.load(atomic::Ordering::Acquire) != rev {
                        send(None);
                        return;
                    }
                    std::thread::spawn(move || {
                        send(lapce_engine::annotations::semantic_spans(
                            &text,
                            styles.styles,
                            || atomic_rev.load(atomic::Ordering::Acquire) != rev,
                        ));
                    });
                } else {
                    send(None);
                }
            },
        );
    }

    pub fn get_document_symbol(&self) {
        if !self.backend_ready() {
            return;
        }
        let cx = self.scope;
        let doc = self.clone();
        let rev = self.rev();
        if let DocContent::File { path, .. } = doc.content.get_untracked() {
            let send = create_ext_action(cx, {
                let path = path.clone();
                move |result| {
                    if rev != doc.rev() {
                        return;
                    }
                    if let Ok(ProxyResponse::GetDocumentSymbols { resp }) = result {
                        let items: Vec<RwSignal<SymbolInformationItemData>> =
                            lapce_engine::navigation::document_symbol_tree(resp)
                                .into_iter()
                                .map(|symbol| {
                                    cx.create_rw_signal(
                                        SymbolInformationItemData::from((
                                            symbol, cx,
                                        )),
                                    )
                                })
                                .collect();
                        let symbol_new = Some(SymbolData::new(items, path, cx));
                        doc.document_symbol_data.update(|symbol| {
                            *symbol = symbol_new;
                        });
                    }
                }
            });

            self.request_navigation(
                lapce_engine::navigation::NavigationQuery::Symbols,
                move |result| {
                    send(result);
                },
            );
        }
    }

    /// Request inlay hints for the buffer from the LSP through the proxy.
    pub fn get_inlay_hints(&self) {
        if !self.backend_ready() {
            return;
        }
        if !self.loaded() {
            return;
        }

        let (buffer, rev) = self.buffer.with_untracked(|b| (b.clone(), b.rev()));

        let doc = self.clone();
        let send = create_ext_action(self.scope, move |hints| {
            if doc.buffer.with_untracked(|b| b.rev()) == rev {
                doc.inlay_hints.set(Some(hints));
                doc.clear_text_cache();
            }
        });

        self.request_navigation(
            lapce_engine::navigation::NavigationQuery::InlayHints,
            move |result| {
                if let Ok(ProxyResponse::GetInlayHints { hints }) = result {
                    send(lapce_engine::annotations::inlay_spans(&buffer, hints));
                }
            },
        );
    }

    pub fn diagnostics(&self) -> &DiagnosticData {
        &self.diagnostics
    }

    /// Update the diagnostics' positions after an edit so that they appear in the correct place.
    fn update_diagnostics(&self, delta: &RopeDelta) {
        self.diagnostics.apply_delta(delta);
    }

    /// init diagnostics offset ranges from lsp positions
    pub fn init_diagnostics(&self) {
        let (text, revision) = self
            .buffer
            .with_untracked(|buffer| (buffer.text().clone(), buffer.rev()));
        self.diagnostics.bind(&text, revision);
        self.on_diagnostics_changed();
    }

    pub(crate) fn on_diagnostics_changed(&self) {
        self.clear_text_cache();
    }

    pub fn get_folding_range(&self) {
        if !self.backend_ready() {
            return;
        }
        let doc = self.clone();
        let send = create_ext_action(self.scope, move |result| {
            if let Ok(ProxyResponse::LspFoldingRangeResponse { resp, .. }) = result {
                let mut folding: Vec<_> = resp
                    .unwrap_or_default()
                    .into_iter()
                    .map(crate::editor::gutter::FoldingRange::from_lsp)
                    .collect();
                folding.sort_by_key(|range| range.start.line);
                doc.folding_ranges.update(|ranges| ranges.0 = folding);
            }
        });
        self.request_navigation(
            lapce_engine::navigation::NavigationQuery::FoldingRanges,
            move |result| send(result),
        );
    }

    /// Get the current completion lens text
    pub fn completion_lens(&self) -> Option<String> {
        self.completion_lens.get_untracked()
    }

    pub fn set_completion_lens(
        &self,
        completion_lens: String,
        line: usize,
        col: usize,
    ) {
        // TODO: more granular invalidation
        self.completion_lens.set(Some(completion_lens));
        self.completion_pos.set((line, col));
        self.clear_text_cache();
    }

    pub fn clear_completion_lens(&self) {
        // TODO: more granular invalidation
        if self.completion_lens.get_untracked().is_some() {
            self.completion_lens.set(None);
            self.clear_text_cache();
        }
    }

    /// Update the completion lens position after an edit so that it appears in the correct place.
    pub fn update_completion_lens(&self, before: &Rope, delta: &RopeDelta) {
        let Some(completion) = self.completion_lens.get_untracked() else {
            return;
        };
        let transformed = lapce_engine::annotations::transform_completion_lens(
            before,
            delta,
            self.completion_pos.get_untracked(),
            &completion,
        );
        if let Some((text, position)) = transformed {
            self.completion_lens.set(Some(text));
            self.completion_pos.set(position);
        } else {
            self.clear_completion_lens();
        }
    }

    fn update_find_result(&self, delta: &RopeDelta) {
        self.find_result.occurrences.update(|s| {
            *s = s.apply_delta(delta, true, InsertDrift::Default);
        })
    }

    pub fn update_find(&self) {
        let find_rev = self.common.find.rev.get_untracked();
        if self.find_result.find_rev.get_untracked() != find_rev {
            if self
                .common
                .find
                .search_string
                .with_untracked(|search_string| {
                    search_string
                        .as_ref()
                        .map(|s| s.content.is_empty())
                        .unwrap_or(true)
                })
            {
                self.find_result.occurrences.set(Selection::new());
            }
            self.find_result.reset();
            self.find_result.find_rev.set(find_rev);
        }

        if self.find_result.progress.get_untracked() != FindProgress::Started {
            return;
        }

        let (revision, text) = self
            .buffer
            .with_untracked(|buffer| (buffer.rev(), buffer.text().clone()));
        let Some(job) = self.common.find.job(text, revision) else {
            self.find_result.progress.set(FindProgress::Ready);
            return;
        };
        self.find_result
            .progress
            .set(FindProgress::InProgress(Selection::new()));
        let doc = self.clone();
        let send = create_ext_action(
            self.scope,
            move |result: lapce_engine::search::SearchResult| {
                if result.is_current(doc.rev(), doc.common.find.rev.get_untracked())
                {
                    doc.find_result.occurrences.set(result.occurrences);
                    doc.find_result.progress.set(FindProgress::Ready);
                }
            },
        );
        rayon::spawn(move || send(job.run()));
    }

    /// Get the sticky headers for a particular line, creating them if necessary.
    pub fn sticky_headers(&self, line: usize) -> Option<Vec<usize>> {
        if let Some(lines) = self.sticky_headers.borrow().get(&line) {
            return lines.clone();
        }
        let lines = self.buffer.with_untracked(|buffer| {
            let offset = buffer.offset_of_line(line + 1);
            self.syntax.with_untracked(|syntax| {
                syntax.sticky_headers(offset).map(|offsets| {
                    offsets
                        .iter()
                        .filter_map(|offset| {
                            let l = buffer.line_of_offset(*offset);
                            if l <= line { Some(l) } else { None }
                        })
                        .dedup()
                        .sorted()
                        .collect()
                })
            })
        });
        self.sticky_headers.borrow_mut().insert(line, lines.clone());
        lines
    }

    pub fn head_changes(&self) -> RwSignal<im::Vector<DiffLines>> {
        self.head_changes
    }

    /// Retrieve the `head` version of the buffer
    pub fn retrieve_head(&self) {
        if let DocContent::File { path, .. } = self.content.get_untracked() {
            let histories = self.histories;

            let send = {
                let path = path.clone();
                let doc = self.clone();
                create_ext_action(self.scope, move |result| {
                    if let Ok(ProxyResponse::BufferHeadResponse {
                        content, ..
                    }) = result
                    {
                        let history = DocumentHistory::new(
                            path.clone(),
                            "head".to_string(),
                            &content,
                        );
                        histories.update(|histories| {
                            histories.insert("head".to_string(), history);
                        });

                        doc.trigger_head_change();
                    }
                })
            };

            let path = path.clone();
            let proxy = self.common.proxy.clone();
            std::thread::spawn(move || {
                proxy.get_buffer_head(path, move |result| {
                    send(result);
                });
            });
        }
    }

    pub fn trigger_head_change(&self) {
        let history = if let Some(text) =
            self.histories.with_untracked(|histories| {
                histories
                    .get("head")
                    .map(|history| history.buffer.text().clone())
            }) {
            text
        } else {
            return;
        };

        let rev = self.rev();
        let left_rope = history;
        let (atomic_rev, right_rope) = self
            .buffer
            .with_untracked(|b| (b.atomic_rev(), b.text().clone()));

        let send = {
            let atomic_rev = atomic_rev.clone();
            let head_changes = self.head_changes;
            create_ext_action(self.scope, move |changes| {
                let changes = if let Some(changes) = changes {
                    changes
                } else {
                    return;
                };

                if atomic_rev.load(atomic::Ordering::Acquire) != rev {
                    return;
                }

                head_changes.set(changes);
            })
        };

        rayon::spawn(move || {
            let changes =
                rope_diff(left_rope, right_rope, rev, atomic_rev.clone(), None);
            send(changes.map(im::Vector::from));
        });
    }

    pub fn local_backend_version(&self, backend_version: i32) -> Option<i32> {
        self.synchronization.with_untracked(|sync| {
            sync.as_ref().map_or(Some(backend_version), |sync| {
                sync.local_version(backend_version)
            })
        })
    }
    pub fn diagnostic_publication(
        &self,
        diagnostics: Vec<lsp_types::Diagnostic>,
        version: Option<i32>,
    ) -> Option<lapce_engine::diagnostics::DiagnosticPublication> {
        self.model.diagnostic_publication(diagnostics, version)
    }

    pub fn initialize_sync(&self, remote_rev: u64) {
        if self.model.initialize_sync(remote_rev) {
            self.refresh_backend_annotations();
        }
    }

    fn recover_sync(&self) {
        let Some(path) = self
            .content
            .with_untracked(|content| content.path().cloned())
        else {
            return;
        };
        let doc = self.clone();
        let send = create_ext_action(
            self.scope,
            move |result: Result<ProxyResponse, lapce_rpc::RpcError>| {
                let result = result
                    .map_err(|error: lapce_rpc::RpcError| error.message)
                    .and_then(|snapshot| doc.model.recover_sync(snapshot));
                if let Err(error) = result {
                    doc.common
                        .internal_command
                        .send(InternalCommand::ShowAlert {
                            title: "Document recovery failed".into(),
                            msg: error,
                            buttons: Vec::new(),
                        });
                } else {
                    doc.cancel_operation_actions(
                        "Document synchronization was reset".into(),
                    );
                    doc.cancel_document_requests();
                    doc.common.internal_command.send(InternalCommand::HideAlert);
                    doc.dispatch_sync();
                }
            },
        );
        self.common.proxy.request_async(
            lapce_rpc::proxy::ProxyRequest::GetBuffer { path },
            move |result| send(result),
        );
    }

    fn backend_ready(&self) -> bool {
        self.model.backend_ready()
    }

    pub fn backend_disconnected(&self) {
        self.model.backend_disconnected();
        self.cancel_operation_actions("Backend disconnected".into());
        self.cancel_document_requests();
    }

    fn refresh_backend_annotations(&self) {
        self.get_inlay_hints();
        self.get_semantic_styles();
        self.get_document_symbol();
        self.get_folding_range();
    }

    pub fn request_navigation(
        &self,
        query: lapce_engine::navigation::NavigationQuery,
        action: impl FnOnce(Result<ProxyResponse, lapce_rpc::RpcError>) + 'static,
    ) {
        let plan = match self.model.navigation_plan(query) {
            Ok(plan) => plan,
            Err(error) => {
                action(Err(error));
                return;
            }
        };
        let model = self.model;
        self.request_document(plan.request(), move |response| {
            action(model.finish_navigation(plan, response));
        });
    }

    pub fn request_document(
        &self,
        request: lapce_rpc::proxy::ProxyRequest,
        action: impl FnOnce(Result<ProxyResponse, lapce_rpc::RpcError>) + 'static,
    ) {
        if self.synchronization.with_untracked(|sync| {
            sync.as_ref().is_some_and(|sync| sync.error.is_some())
        }) {
            action(Err(lapce_rpc::RpcError {
                code: -32800,
                message: "Document synchronization is paused".into(),
            }));
            return;
        }
        let rev = self.rev();
        let id = self
            .synchronization
            .try_update(|sync| {
                sync.as_mut().and_then(|sync| sync.request(request, rev))
            })
            .flatten();
        if let Some(id) = id {
            self.request_actions
                .borrow_mut()
                .insert(id, Box::new(action));
            self.dispatch_sync();
        } else {
            action(Err(lapce_rpc::RpcError {
                code: -32801,
                message: "Document is not connected".into(),
            }));
        }
    }

    pub fn notification_document(
        &self,
        notification: lapce_rpc::proxy::ProxyNotification,
    ) {
        self.synchronization.update(|sync| {
            if let Some(sync) = sync {
                sync.notification(notification);
            }
        });
        self.dispatch_sync();
    }

    fn cancel_operation_actions(&self, message: String) {
        let actions = std::mem::take(&mut *self.operation_actions.borrow_mut());
        for (_, action) in actions {
            action(Err(message.clone()));
        }
    }

    fn cancel_document_requests(&self) {
        let actions = std::mem::take(&mut *self.request_actions.borrow_mut());
        for (_, action) in actions {
            action(Err(lapce_rpc::RpcError {
                code: -32800,
                message: "Document synchronization changed".into(),
            }));
        }
    }

    fn dispatch_sync(&self) {
        loop {
            let notification = self
                .synchronization
                .try_update(|sync| {
                    sync.as_mut().and_then(|sync| sync.next_notification())
                })
                .flatten();
            let Some(notification) = notification else {
                break;
            };
            self.common.proxy.notification(notification);
        }
        let effect = self
            .synchronization
            .try_update(|sync| sync.as_mut().and_then(|sync| sync.next_effect()))
            .flatten();
        let Some(effect) = effect else {
            return;
        };
        let detached = effect.detached;
        let generation = self.model.backend_generation();
        let doc = self.clone();
        let send = create_ext_action(
            self.scope,
            move |response: Result<ProxyResponse, lapce_rpc::RpcError>| {
                let Some(completion) = doc
                    .model
                    .acknowledge_backend(generation, effect.id, response)
                else {
                    return;
                };
                let refresh = completion.refresh_annotations;
                // A close callback may reopen this document and register new reads.
                // Cancel the previous owner's callbacks before invoking it.
                if matches!(
                    completion.outcome,
                    lapce_engine::sync::SyncOutcome::Closed { .. }
                ) {
                    doc.cancel_document_requests();
                }
                if let Some((id, result)) = completion.request_result {
                    let action = doc.request_actions.borrow_mut().remove(&id);
                    if let Some(action) = action {
                        action(result);
                    }
                }
                if let Some((id, result)) = completion.operation_result {
                    let action = doc.operation_actions.borrow_mut().remove(&id);
                    if let Some(action) = action {
                        action(result);
                    }
                }
                match Some(completion.outcome) {
                    Some(lapce_engine::sync::SyncOutcome::Failed(error)) => {
                        doc.cancel_operation_actions(error.clone());
                        doc.cancel_document_requests();
                        doc.common.internal_command.send(InternalCommand::ShowAlert {
                        title: "Document synchronization failed".into(),
                        msg: format!("{error}\nYour local edits are preserved. Resynchronizing will replace the backend's in-memory buffer with your local text; it will not save the file."),
                        buttons: vec![crate::alert::AlertButton {
                            text: "Resynchronize with local text".into(),
                            action: Rc::new({ let doc = doc.clone(); move || doc.recover_sync() }),
                        }],
                    });
                    }
                    _ => {}
                }
                doc.dispatch_sync();
                if refresh && doc.backend_ready() {
                    doc.refresh_backend_annotations();
                }
            },
        );
        self.common
            .proxy
            .request_async(effect.request, move |response| send(response));
        if detached {
            self.dispatch_sync();
        }
    }

    pub fn retarget_closed(&self, path: std::path::PathBuf) -> Result<(), String> {
        self.model.retarget_closed(path)?;
        self.clear_style_cache();
        self.clear_sticky_headers_cache();
        Ok(())
    }

    pub fn mark_deleted(&self) {
        self.model.update_buffer(|document| document.mark_deleted());
    }
    pub fn mark_disk_present(&self) {
        self.model
            .update_buffer(|document| document.mark_disk_present());
    }

    pub fn can_reopen_backend(&self) -> bool {
        self.model.can_reopen_backend()
    }

    pub fn reopen_backend(
        &self,
        after_open: impl FnOnce(Result<(), String>) + 'static,
    ) {
        let plan = match self.model.reopen_plan() {
            Ok(plan) => plan,
            Err(error) => {
                after_open(Err(error));
                return;
            }
        };
        let request = plan.request();
        let doc = self.clone();
        let send = create_ext_action(self.scope, move |response| {
            let result = doc.model.finish_reopen(plan, response);
            if result.is_ok() {
                doc.init_parser();
                doc.init_diagnostics();
                doc.trigger_syntax_change(None);
                doc.clear_style_cache();
                doc.clear_sticky_headers_cache();
                doc.dispatch_sync();
                if doc.backend_ready() {
                    doc.refresh_backend_annotations();
                }
            }
            after_open(result);
        });
        self.common
            .proxy
            .request_async(request, move |response| send(response));
    }

    /// Close a document after earlier mutations have been acknowledged.
    pub fn close_backend(
        &self,
        after_close: impl FnOnce(Result<(), String>) + 'static,
    ) -> bool {
        let id = match self.model.begin_close() {
            Ok(lapce_engine::session::DocumentClose::Local) => {
                self.cancel_document_requests();
                self.cancel_operation_actions("Document closed".into());
                after_close(Ok(()));
                return true;
            }
            Ok(lapce_engine::session::DocumentClose::Backend { operation_id }) => {
                operation_id
            }
            Err(error) => {
                after_close(Err(error));
                return false;
            }
        };
        self.operation_actions
            .borrow_mut()
            .insert(id, Box::new(after_close));
        self.cancel_document_requests();
        self.dispatch_sync();
        true
    }

    pub fn set_inline_completion(
        &self,
        inline_completion: String,
        line: usize,
        col: usize,
    ) {
        // TODO: more granular invalidation
        batch(|| {
            self.inline_completion.set(Some(inline_completion));
            self.inline_completion_pos.set((line, col));
            self.clear_text_cache();
        });
    }

    pub fn clear_inline_completion(&self) {
        if self.inline_completion.with_untracked(Option::is_some) {
            self.inline_completion.set(None);
            self.clear_text_cache();
        }
    }

    pub fn update_inline_completion(&self, delta: &RopeDelta) {
        let Some(completion) = self.inline_completion.get_untracked() else {
            return;
        };

        let (line, col) = self.completion_pos.get_untracked();
        let offset = self
            .buffer
            .with_untracked(|b| b.offset_of_line_col(line, col));

        // If the edit is easily checkable + updateable from, then we alter the text.
        // In normal typing, if we didn't do this, then the text would jitter forward and then
        // backwards as the completion is updated.
        // TODO: this could also handle simple deletion, but we don't currently keep track of
        // the past completion string content in the field.
        if delta.as_simple_insert().is_some() {
            let (iv, new_len) = delta.summary();
            if iv.start() == iv.end()
                && iv.start() == offset
                && new_len <= completion.len()
            {
                // Remove the # of newly inserted characters
                // These aren't necessarily the same as the characters literally in the
                // text, but the completion will be updated when the completion widget
                // receives the update event, and it will fix this if needed.
                self.inline_completion
                    .set(Some(completion[new_len..].to_string()));
            }
        }

        // Shift the position by the rope delta
        let mut transformer = Transformer::new(delta);

        let new_offset = transformer.transform(offset, true);
        let new_pos = self
            .buffer
            .with_untracked(|b| b.offset_to_line_col(new_offset));

        self.inline_completion_pos.set(new_pos);
    }

    /// Returns the offsets of the brackets enclosing the given offset.
    /// Uses a language aware algorithm if syntax support is available for the current language,
    /// else falls back to a language unaware algorithm.
    pub fn find_enclosing_brackets(&self, offset: usize) -> Option<(usize, usize)> {
        let rev = self.rev();
        self.syntax
            .with_untracked(|syntax| {
                (!syntax.text.is_empty() && syntax.rev == rev)
                    .then(|| syntax.find_enclosing_pair(offset))
            })
            // If syntax.text is empty, either the buffer is empty or we don't have syntax support
            // for the current language.
            // Try a language unaware search for enclosing brackets in case it is the latter.
            .unwrap_or_else(|| {
                self.buffer.with_untracked(|buffer| {
                    WordCursor::new(buffer.text(), offset).find_enclosing_pair()
                })
            })
    }

    pub fn update_highlights(&self, offset: usize) {
        if !self.loaded() {
            return;
        }

        if !self.content.with_untracked(|content| content.is_file()) {
            return;
        }

        let cursor_in_highlight = self.highlights.with_untracked(|highlights| {
            if let Some(highlights) = highlights {
                DocumentHighlight::find_for_offset(highlights, offset).is_some()
            } else {
                false
            }
        });

        if cursor_in_highlight {
            // Cursor is in the region of same highlight, skip querying LSP.
            return;
        }

        let need_start = match self.get_highlights_status.get_untracked() {
            DocumentHighlightsStatus::Ready => true,
            DocumentHighlightsStatus::Busy(_) => false,
        };

        self.get_highlights_status
            .update(|status| *status = DocumentHighlightsStatus::Busy(offset));

        if !need_start {
            return;
        }

        let rev = self.rev();

        let doc = self.clone();

        let handle_response = create_ext_action(self.scope, move |response| {
            let mut new_highlights: Option<Vec<DocumentHighlight>> = None;

            if doc.buffer.with_untracked(|b| b.rev()) == rev {
                if let DocumentHighlightsStatus::Busy(offset) =
                    doc.get_highlights_status.get_untracked()
                {
                    if let Ok(ProxyResponse::GetDocumentHighlights {
                        highlights: Some(highlights),
                    }) = response
                    {
                        new_highlights = doc.buffer.with_untracked(move |buffer| {
                            let result =
                                lapce_engine::annotations::document_highlights(
                                    buffer, highlights,
                                );

                            // This is a kind of debouncing - if cursor moves before we got response from LSP,
                            // check if cursor is still in the region.
                            // In case cursor moved too far we may want to issue LSP request again, for new offset
                            // but it is easily recoverable by the user - just move cursor a bit to update highlights.
                            DocumentHighlight::find_for_offset(&result, offset)
                                .map(|_| result)
                        });
                    }
                }
            }

            doc.highlights.update(|value| *value = new_highlights);
            doc.get_highlights_status
                .update(|status| *status = DocumentHighlightsStatus::Ready);
        });

        self.request_navigation(
            lapce_engine::navigation::NavigationQuery::Highlights { offset },
            move |response| handle_response(response),
        );
    }
}

impl Document for Doc {
    fn text(&self) -> Rope {
        self.buffer.with_untracked(|buffer| buffer.text().clone())
    }

    fn cache_rev(&self) -> RwSignal<u64> {
        self.cache_rev
    }

    fn find_unmatched(&self, offset: usize, previous: bool, ch: char) -> usize {
        self.syntax().with_untracked(|syntax| {
            if syntax.layers.is_some() {
                syntax
                    .find_tag(offset, previous, &CharBuffer::from(ch))
                    .unwrap_or(offset)
            } else {
                let text = self.text();
                let mut cursor = WordCursor::new(&text, offset);
                let new_offset = if previous {
                    cursor.previous_unmatched(ch)
                } else {
                    cursor.next_unmatched(ch)
                };

                new_offset.unwrap_or(offset)
            }
        })
    }

    fn find_matching_pair(&self, offset: usize) -> usize {
        self.syntax().with_untracked(|syntax| {
            if syntax.layers.is_some() {
                syntax.find_matching_pair(offset).unwrap_or(offset)
            } else {
                let text = self.text();
                WordCursor::new(&text, offset)
                    .match_pairs()
                    .unwrap_or(offset)
            }
        })
    }

    fn preedit(&self) -> PreeditData {
        self.preedit.clone()
    }

    fn compute_screen_lines(
        &self,
        editor: &Editor,
        base: RwSignal<ScreenLinesBase>,
    ) -> ScreenLines {
        let Some(editor_data) = self.editor_data(editor.id()) else {
            return ScreenLines {
                lines: Default::default(),
                info: Default::default(),
                diff_sections: Default::default(),
                base,
            };
        };

        compute_screen_lines(
            self.common.config,
            base,
            editor_data.kind.read_only(),
            &editor_data.doc_signal().get(),
            editor.lines(),
            editor.text_prov(),
            editor.config_id(),
        )
    }

    fn run_command(
        &self,
        ed: &Editor,
        cmd: &Command,
        count: Option<usize>,
        modifiers: Modifiers,
    ) -> CommandExecuted {
        let Some(editor_data) = self.editor_data(ed.id()) else {
            return CommandExecuted::No;
        };

        let cmd = CommandKind::from(cmd.clone());
        let cmd = LapceCommand {
            kind: cmd,
            data: None,
        };
        editor_data.run_command(&cmd, count, modifiers)
    }

    fn receive_char(&self, ed: &Editor, c: &str) {
        let Some(editor_data) = self.editor_data(ed.id()) else {
            return;
        };

        editor_data.receive_char(c);
    }

    fn edit(
        &self,
        iter: &mut dyn Iterator<Item = (Selection, &str)>,
        edit_type: EditType,
    ) {
        let delta = self
            .buffer
            .try_update(|buffer| buffer.edit(iter, edit_type))
            .unwrap();
        self.apply_deltas(&[delta]);
    }
}

impl DocumentPhantom for Doc {
    fn phantom_text(
        &self,
        _: EditorId,
        _: &EditorStyle,
        line: usize,
    ) -> PhantomTextLine {
        let config = &self.common.config.get_untracked();

        let (start_offset, end_offset) = self.buffer.with_untracked(|buffer| {
            (buffer.offset_of_line(line), buffer.offset_of_line(line + 1))
        });

        let inlay_hints = self.inlay_hints.get_untracked();
        // If hints are enabled, and the hints field is filled, then get the hints for this line
        // and convert them into PhantomText instances
        let hints = config
            .editor
            .enable_inlay_hints
            .then_some(())
            .and(inlay_hints.as_ref())
            .map(|hints| hints.iter_chunks(start_offset..end_offset))
            .into_iter()
            .flatten()
            .filter(|(interval, _)| {
                interval.start >= start_offset && interval.start < end_offset
            })
            .map(|(interval, inlay_hint)| {
                let (col, affinity) = self.buffer.with_untracked(|b| {
                    let mut cursor =
                        lapce_xi_rope::Cursor::new(b.text(), interval.start);

                    let next_char = cursor.peek_next_codepoint();
                    let prev_char = cursor.prev_codepoint();

                    let mut affinity = None;
                    if let Some(prev_char) = prev_char {
                        let c = get_char_property(prev_char);
                        if c == CharClassification::Other {
                            affinity = Some(CursorAffinity::Backward)
                        } else if matches!(
                            c,
                            CharClassification::Lf
                                | CharClassification::Cr
                                | CharClassification::Space
                        ) {
                            affinity = Some(CursorAffinity::Forward)
                        }
                    };
                    if affinity.is_none() {
                        if let Some(next_char) = next_char {
                            let c = get_char_property(next_char);
                            if c == CharClassification::Other {
                                affinity = Some(CursorAffinity::Forward)
                            } else if matches!(
                                c,
                                CharClassification::Lf
                                    | CharClassification::Cr
                                    | CharClassification::Space
                            ) {
                                affinity = Some(CursorAffinity::Backward)
                            }
                        }
                    }

                    let (_, col) = b.offset_to_line_col(interval.start);
                    (col, affinity)
                });
                let text = match &inlay_hint.label {
                    InlayHintLabel::String(label) => label.to_string(),
                    InlayHintLabel::LabelParts(parts) => {
                        parts.iter().map(|p| &p.value).join("")
                    }
                };
                PhantomText {
                    kind: PhantomTextKind::InlayHint,
                    col,
                    text,
                    affinity,
                    fg: Some(config.color(LapceColor::INLAY_HINT_FOREGROUND)),
                    // font_family: Some(config.editor.inlay_hint_font_family()),
                    font_size: Some(config.editor.inlay_hint_font_size()),
                    bg: Some(config.color(LapceColor::INLAY_HINT_BACKGROUND)),
                    under_line: None,
                }
            });
        // You're quite unlikely to have more than six hints on a single line
        // this later has the diagnostics added onto it, but that's still likely to be below six
        // overall.
        let mut text: SmallVec<[PhantomText; 6]> = hints.collect();

        // If error lens is enabled, and the diagnostics field is filled, then get the diagnostics
        // that end on this line which have a severity worse than HINT and convert them into
        // PhantomText instances

        let mut diag_text: SmallVec<[PhantomText; 6]> =
            self.buffer.with_untracked(|buffer| {
                config
                    .editor
                    .enable_error_lens
                    .then_some(())
                    .map(|_| self.diagnostics.diagnostics_span.get_untracked())
                    .map(|diags| {
                        diags
                            .iter_chunks(start_offset..end_offset)
                            .filter_map(|(iv, diag)| {
                                let end = iv.end();
                                let end_line = buffer.line_of_offset(end);
                                if end_line == line
                                    && diag.severity < Some(DiagnosticSeverity::HINT)
                                {
                                    let fg = {
                                        let severity = diag
                                            .severity
                                            .unwrap_or(DiagnosticSeverity::WARNING);
                                        let theme_prop = if severity
                                            == DiagnosticSeverity::ERROR
                                        {
                                            LapceColor::ERROR_LENS_ERROR_FOREGROUND
                                        } else if severity
                                            == DiagnosticSeverity::WARNING
                                        {
                                            LapceColor::ERROR_LENS_WARNING_FOREGROUND
                                        } else {
                                            // information + hint (if we keep that) + things without a severity
                                            LapceColor::ERROR_LENS_OTHER_FOREGROUND
                                        };

                                        config.color(theme_prop)
                                    };

                                    let text =
                                        if config.editor.only_render_error_styling {
                                            "".to_string()
                                        } else if config.editor.error_lens_multiline
                                        {
                                            format!("    {}", diag.message)
                                        } else {
                                            format!(
                                                "    {}",
                                                diag.message.lines().join(" ")
                                            )
                                        };
                                    Some(PhantomText {
                                        kind: PhantomTextKind::Diagnostic,
                                        col: end_offset - start_offset,
                                        affinity: Some(CursorAffinity::Backward),
                                        text,
                                        fg: Some(fg),
                                        font_size: Some(
                                            config.editor.error_lens_font_size(),
                                        ),
                                        bg: None,
                                        under_line: None,
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect::<SmallVec<[PhantomText; 6]>>()
                    })
                    .unwrap_or_default()
            });

        text.append(&mut diag_text);

        let (completion_line, completion_col) = self.completion_pos.get_untracked();
        let completion_text = config
            .editor
            .enable_completion_lens
            .then_some(())
            .and(self.completion_lens.get_untracked())
            // TODO: We're probably missing on various useful completion things to include here!
            .filter(|_| line == completion_line)
            .map(|completion| PhantomText {
                kind: PhantomTextKind::Completion,
                col: completion_col,
                text: completion.clone(),
                fg: Some(config.color(LapceColor::COMPLETION_LENS_FOREGROUND)),
                font_size: Some(config.editor.completion_lens_font_size()),
                affinity: Some(CursorAffinity::Backward),
                // font_family: Some(config.editor.completion_lens_font_family()),
                bg: None,
                under_line: None,
                // TODO: italics?
            });
        if let Some(completion_text) = completion_text {
            text.push(completion_text);
        }

        // TODO: don't display completion lens and inline completion at the same time
        // and/or merge them so that they can be shifted between like multiple inline completions
        // can
        let (inline_completion_line, inline_completion_col) =
            self.inline_completion_pos.get_untracked();
        let inline_completion_text = config
            .editor
            .enable_inline_completion
            .then_some(())
            .and(self.inline_completion.get_untracked())
            .filter(|_| line == inline_completion_line)
            .map(|completion| PhantomText {
                kind: PhantomTextKind::Completion,
                col: inline_completion_col,
                text: completion.clone(),
                affinity: Some(CursorAffinity::Backward),
                fg: Some(config.color(LapceColor::COMPLETION_LENS_FOREGROUND)),
                font_size: Some(config.editor.completion_lens_font_size()),
                // font_family: Some(config.editor.completion_lens_font_family()),
                bg: None,
                under_line: None,
                // TODO: italics?
            });
        if let Some(inline_completion_text) = inline_completion_text {
            text.push(inline_completion_text);
        }

        if let Some(preedit) = self
            .preedit_phantom(Some(config.color(LapceColor::EDITOR_FOREGROUND)), line)
        {
            text.push(preedit)
        }

        text.sort_by(|a, b| {
            if a.col == b.col {
                a.kind.cmp(&b.kind)
            } else {
                a.col.cmp(&b.col)
            }
        });

        PhantomTextLine { text }
    }

    fn has_multiline_phantom(&self, _: EditorId, _: &EditorStyle) -> bool {
        // TODO: actually check
        true
    }
}
impl CommonAction for Doc {
    fn exec_motion_mode(
        &self,
        _ed: &Editor,
        cursor: &mut Cursor,
        motion_mode: MotionMode,
        range: Range<usize>,
        is_vertical: bool,
        register: &mut Register,
    ) {
        let deltas = self
            .model
            .update_buffer(move |document| {
                document.execute_motion_mode(
                    cursor,
                    motion_mode,
                    range,
                    is_vertical,
                    register,
                )
            })
            .unwrap();
        self.apply_deltas(&deltas);
    }

    fn do_edit(
        &self,
        _ed: &Editor,
        cursor: &mut Cursor,
        cmd: &EditCommand,
        modal: bool,
        register: &mut Register,
        smart_tab: bool,
    ) -> bool {
        let deltas = Doc::do_edit(self, cursor, cmd, modal, register, smart_tab);
        !deltas.is_empty()
    }
}

#[derive(Clone)]
pub struct DocStyling {
    config: ReadSignal<Arc<LapceConfig>>,
    doc: Rc<Doc>,
}
impl DocStyling {
    fn apply_colorization(
        &self,
        edid: EditorId,
        style: &EditorStyle,
        line: usize,
        attrs: &Attrs,
        attrs_list: &mut AttrsList,
    ) {
        let config = self.config.get_untracked();
        let phantom_text = self.doc.phantom_text(edid, style, line);
        if let Some(bracket_styles) = self.doc.parser.borrow().bracket_pos.get(&line)
        {
            for bracket_style in bracket_styles.iter() {
                if let Some(fg_color) = bracket_style.style.fg_color.as_ref() {
                    if let Some(fg_color) = config.style_color(fg_color) {
                        let start = phantom_text.col_at(bracket_style.start);
                        let end = phantom_text.col_at(bracket_style.end);
                        attrs_list
                            .add_span(start..end, attrs.clone().color(fg_color));
                    }
                }
            }
        }
    }
}
impl Styling for DocStyling {
    fn id(&self) -> u64 {
        self.config.with_untracked(|config| config.id)
    }

    fn font_size(&self, _: EditorId, _line: usize) -> usize {
        self.config
            .with_untracked(|config| config.editor.font_size())
    }

    fn line_height(&self, _: EditorId, _line: usize) -> f32 {
        self.config
            .with_untracked(|config| config.editor.line_height()) as f32
    }

    fn font_family(
        &self,
        _: EditorId,
        _line: usize,
    ) -> std::borrow::Cow<'_, [floem::text::FamilyOwned]> {
        // TODO: cache this
        Cow::Owned(self.config.with_untracked(|config| {
            FamilyOwned::parse_list(&config.editor.font_family).collect()
        }))
    }

    fn weight(&self, _: EditorId, _line: usize) -> floem::text::Weight {
        floem::text::Weight::NORMAL
    }

    fn italic_style(&self, _: EditorId, _line: usize) -> floem::text::Style {
        floem::text::Style::Normal
    }

    fn stretch(&self, _: EditorId, _line: usize) -> floem::text::Stretch {
        floem::text::Stretch::Normal
    }

    fn indent_line(&self, _: EditorId, line: usize, line_content: &str) -> usize {
        if line_content.trim().is_empty() {
            let text = self.doc.rope_text();
            let offset = text.offset_of_line(line);
            if let Some(offset) =
                self.doc.syntax.with_untracked(|s| s.parent_offset(offset))
            {
                return text.line_of_offset(offset);
            }
        }

        line
    }

    fn tab_width(&self, _: EditorId, _line: usize) -> usize {
        self.config.with_untracked(|config| config.editor.tab_width)
    }

    fn atomic_soft_tabs(&self, _: EditorId, _line: usize) -> bool {
        self.config
            .with_untracked(|config| config.editor.atomic_soft_tabs)
    }

    fn apply_attr_styles(
        &self,
        edid: EditorId,
        style: &EditorStyle,
        line: usize,
        default: Attrs,
        attrs_list: &mut AttrsList,
    ) {
        let config = self.doc.common.config.get_untracked();

        self.apply_colorization(edid, style, line, &default, attrs_list);

        let phantom_text = self.doc.phantom_text(edid, style, line);
        for line_style in self.doc.line_style(line).iter() {
            if let Some(fg_color) = line_style.style.fg_color.as_ref() {
                if let Some(fg_color) = config.style_color(fg_color) {
                    let start = phantom_text.col_at(line_style.start);
                    let end = phantom_text.col_at(line_style.end);
                    attrs_list.add_span(start..end, default.clone().color(fg_color));
                }
            }
        }
    }

    fn apply_layout_styles(
        &self,
        edid: EditorId,
        style: &EditorStyle,
        line: usize,
        layout_line: &mut TextLayoutLine,
    ) {
        let doc = &self.doc;
        let config = doc.common.config.get_untracked();

        layout_line.extra_style.clear();
        let layout = &layout_line.text;

        let phantom_text = doc.phantom_text(edid, style, line);

        let phantom_styles = phantom_text
            .offset_size_iter()
            .filter(move |(_, _, _, p)| p.bg.is_some() || p.under_line.is_some())
            .flat_map(move |(col_shift, size, col, phantom)| {
                let start = col + col_shift;
                let end = start + size;

                extra_styles_for_range(
                    layout,
                    start,
                    end,
                    phantom.bg,
                    phantom.under_line,
                    None,
                )
            });
        layout_line.extra_style.extend(phantom_styles);

        let (start_offset, end_offset) = doc.buffer.with_untracked(|buffer| {
            (buffer.offset_of_line(line), buffer.offset_of_line(line + 1))
        });

        let mut max_severity: Option<DiagnosticSeverity> = None;
        doc.diagnostics.diagnostics_span.with_untracked(|diags| {
            diags
                .iter_chunks(start_offset..end_offset)
                .for_each(|(iv, diag)| {
                    let start = iv.start();
                    let end = iv.end();

                    if start <= end_offset
                        && end >= start_offset
                        && diag.severity < Some(DiagnosticSeverity::HINT)
                    {
                        let start = start.saturating_sub(start_offset);
                        let end = end - start_offset;
                        let start = phantom_text.col_after(start, true);
                        let end = phantom_text.col_after(end, false);

                        match (diag.severity, max_severity) {
                            (Some(severity), Some(max)) => {
                                if severity < max {
                                    max_severity = Some(severity);
                                }
                            }
                            (Some(severity), None) => {
                                max_severity = Some(severity);
                            }
                            _ => {}
                        }

                        let color_name = match diag.severity {
                            Some(DiagnosticSeverity::ERROR) => {
                                LapceColor::LAPCE_ERROR
                            }
                            _ => LapceColor::LAPCE_WARN,
                        };
                        let color = config.color(color_name);
                        let styles = extra_styles_for_range(
                            layout,
                            start,
                            end,
                            None,
                            None,
                            Some(color),
                        );
                        layout_line.extra_style.extend(styles);
                    }
                });
        });

        // Add the styling for the diagnostic severity, if applicable
        if let Some(max_severity) = max_severity {
            let theme_prop = if max_severity == DiagnosticSeverity::ERROR {
                LapceColor::ERROR_LENS_ERROR_BACKGROUND
            } else if max_severity == DiagnosticSeverity::WARNING {
                LapceColor::ERROR_LENS_WARNING_BACKGROUND
            } else {
                LapceColor::ERROR_LENS_OTHER_BACKGROUND
            };

            let size = layout.size();
            let x1 = if !config.editor.error_lens_end_of_line {
                let error_end_x = size.width;
                Some(error_end_x.max(size.width))
            } else {
                None
            };

            // TODO(minor): Should we show the background only on wrapped lines that have the
            // diagnostic actually on that line?
            // That would make it more obvious where it is from and matches other editors.
            layout_line.extra_style.push(LineExtraStyle {
                x: 0.0,
                y: 0.0,
                width: x1,
                height: size.height,
                bg_color: Some(config.color(theme_prop)),
                under_line: None,
                wave_line: None,
            });
        }
    }

    fn paint_caret(&self, edid: EditorId, _line: usize) -> bool {
        let Some(e_data) = self.doc.editor_data(edid) else {
            return true;
        };

        // If the find is active, then we don't want to paint the caret
        !e_data.find_focus.get_untracked()
    }
}

impl std::fmt::Debug for Doc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("Document {:?}", self.buffer_id))
    }
}

fn should_blink(
    focus: RwSignal<Focus>,
    keyboard_focus: RwSignal<Option<ViewId>>,
) -> impl Fn() -> bool {
    move || {
        let Some(focus) = focus.try_get_untracked() else {
            return false;
        };
        if matches!(
            focus,
            Focus::Workbench
                | Focus::Palette
                | Focus::Panel(PanelKind::Plugin)
                | Focus::Panel(PanelKind::Search)
                | Focus::Panel(PanelKind::SourceControl)
        ) {
            return true;
        }

        if keyboard_focus.get_untracked().is_some() {
            return true;
        }
        false
    }
}

fn extra_styles_for_range(
    text_layout: &TextLayout,
    start: usize,
    end: usize,
    bg_color: Option<Color>,
    under_line: Option<Color>,
    wave_line: Option<Color>,
) -> impl Iterator<Item = LineExtraStyle> + '_ {
    let start_hit = text_layout.hit_position(start);
    let end_hit = text_layout.hit_position(end);

    text_layout
        .layout_runs()
        .enumerate()
        .filter_map(move |(current_line, run)| {
            if current_line < start_hit.line || current_line > end_hit.line {
                return None;
            }

            let x = if current_line == start_hit.line {
                start_hit.point.x
            } else {
                run.glyphs.first().map(|g| g.x).unwrap_or(0.0) as f64
            };
            let end_x = if current_line == end_hit.line {
                end_hit.point.x
            } else {
                run.glyphs.last().map(|g| g.x + g.w).unwrap_or(0.0) as f64
            };
            let width = end_x - x;

            if width == 0.0 {
                return None;
            }

            let height = (run.max_ascent + run.max_descent) as f64;
            let y = run.line_y as f64 - run.max_ascent as f64;

            Some(LineExtraStyle {
                x,
                y,
                width: Some(width),
                height,
                bg_color,
                under_line,
                wave_line,
            })
        })
}
