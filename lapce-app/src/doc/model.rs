//! Floem observation of the shared document, without a second copy of its state.
use floem::reactive::{
    RwSignal, Scope, SignalTrack, SignalUpdate, SignalWith, batch,
};
use lapce_engine::{document::Document, session::SessionDocument};

#[derive(Clone, Copy)]
pub struct DocumentModel {
    document: RwSignal<SessionDocument>,
    buffer_changed: RwSignal<()>,
    syntax_changed: RwSignal<()>,
    content_changed: RwSignal<()>,
    inlay_changed: RwSignal<()>,
    loading_changed: RwSignal<()>,
    sync_changed: RwSignal<()>,
}

impl DocumentModel {
    pub fn begin_close(
        self,
    ) -> Result<lapce_engine::session::DocumentClose, String> {
        let result = self
            .document
            .try_update(SessionDocument::begin_close)
            .ok_or("Document has closed")?;
        if result.is_ok() {
            self.sync_changed.set(());
        }
        result
    }
    pub fn opening(self) -> Option<lapce_engine::open::SessionOpening> {
        self.document
            .try_update(lapce_engine::open::SessionOpening::capture)
            .flatten()
    }
    pub fn can_reopen_backend(self) -> bool {
        self.document.try_with_untracked(|session| {
            session.is_some_and(SessionDocument::can_reopen_backend)
        })
    }
    pub fn finish_opening(
        self,
        opening: lapce_engine::open::SessionOpening,
        response: Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError>,
    ) -> Option<Result<(), lapce_rpc::RpcError>> {
        let result = self
            .document
            .try_update(|session| opening.apply(session, response))?;
        batch(|| {
            self.loading_changed.set(());
            if result.is_ok() {
                self.buffer_changed.set(());
                self.content_changed.set(());
                self.inlay_changed.set(());
                self.sync_changed.set(());
            }
        });
        Some(result)
    }
    pub fn reload_from_disk(
        self,
        content: lapce_xi_rope::Rope,
    ) -> Option<lapce_engine::editing::DocumentDelta> {
        let delta = self
            .document
            .try_update(|session| session.reload_from_disk(content))??;
        batch(|| {
            self.buffer_changed.set(());
            self.inlay_changed.set(());
            self.loading_changed.set(());
        });
        delta
    }
    pub fn can_close_locally(self) -> bool {
        self.document.try_with_untracked(|session| {
            session.is_some_and(SessionDocument::can_close_locally)
        })
    }

    pub fn backend_disconnected(self) {
        if self
            .document
            .try_update(|session| session.backend_disconnected())
            .is_some()
        {
            self.sync_changed.set(());
        }
    }
    pub fn backend_ready(self) -> bool {
        self.document.try_with_untracked(|session| {
            session.is_some_and(SessionDocument::backend_ready)
        })
    }
    pub fn backend_generation(
        self,
    ) -> lapce_engine::session::DocumentBackendGeneration {
        self.document
            .with_untracked(SessionDocument::backend_generation)
    }
    pub fn acknowledge_backend(
        self,
        generation: lapce_engine::session::DocumentBackendGeneration,
        id: u64,
        response: Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError>,
    ) -> Option<lapce_engine::session::DocumentCompletion> {
        let completion = self.document.try_update(|session| {
            session.acknowledge_backend(generation, id, response)
        })?;
        batch(|| {
            self.sync_changed.set(());
            if matches!(completion.operation_result, Some((_, Ok(())))) {
                self.buffer_changed.set(());
            }
        });
        Some(completion)
    }
    pub fn new(scope: Scope, document: Document) -> Self {
        Self {
            document: scope.create_rw_signal(SessionDocument::new(document)),
            buffer_changed: scope.create_rw_signal(()),
            syntax_changed: scope.create_rw_signal(()),
            content_changed: scope.create_rw_signal(()),
            inlay_changed: scope.create_rw_signal(()),
            loading_changed: scope.create_rw_signal(()),
            sync_changed: scope.create_rw_signal(()),
        }
    }

    pub fn navigation_plan(
        self,
        query: lapce_engine::navigation::NavigationQuery,
    ) -> Result<lapce_engine::navigation::SessionNavigationPlan, lapce_rpc::RpcError>
    {
        self.document.with_untracked(|document| {
            lapce_engine::navigation::SessionNavigationPlan::capture(document, query)
        })
    }
    pub fn finish_navigation(
        self,
        plan: lapce_engine::navigation::SessionNavigationPlan,
        response: Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError>,
    ) -> Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError> {
        self.document.try_with_untracked(|document| match document {
            Some(document) => plan.finish(document, response),
            None => Err(lapce_rpc::RpcError {
                code: -32801,
                message: "Document has closed".into(),
            }),
        })
    }

    pub fn id(self) -> lapce_rpc::buffer::BufferId {
        self.document.with_untracked(|document| document.id)
    }

    pub fn synchronization(
        self,
    ) -> DocumentField<Option<lapce_engine::sync::DocumentSync>> {
        self.field(
            self.sync_changed,
            |session| &session.synchronization,
            |session| &mut session.synchronization,
        )
    }

    pub fn retarget_closed(self, path: std::path::PathBuf) -> Result<(), String> {
        let result = self
            .document
            .try_update(|session| session.retarget_closed(path))
            .ok_or("Document has closed")?;
        if result.is_ok() {
            batch(|| {
                self.content_changed.set(());
                self.syntax_changed.set(());
                self.inlay_changed.set(());
            });
        }
        result
    }

    pub fn initialize_sync(self, revision: u64) -> bool {
        let initialized = self
            .document
            .try_update(|session| session.initialize_sync(revision))
            .unwrap_or(false);
        if initialized {
            self.sync_changed.set(());
        }
        initialized
    }

    pub fn synchronize_edits(
        self,
        deltas: &[lapce_engine::editing::DocumentDelta],
    ) -> bool {
        let queued = self
            .document
            .try_update(|session| session.synchronize_edits(deltas))
            .unwrap_or(false);
        if queued {
            self.sync_changed.set(());
        }
        queued
    }

    pub fn recover_sync(
        self,
        snapshot: lapce_rpc::proxy::ProxyResponse,
    ) -> Result<(), String> {
        let result = self
            .document
            .try_update(|session| session.recover_sync(snapshot))
            .ok_or("Document has closed")?;
        if result.is_ok() {
            self.sync_changed.set(());
        }
        result
    }

    pub fn reopen_plan(
        self,
    ) -> Result<lapce_engine::session::ReopenDocument, String> {
        self.document
            .try_update(SessionDocument::begin_reopen)
            .ok_or("Document has closed")?
    }
    pub fn finish_reopen(
        self,
        plan: lapce_engine::session::ReopenDocument,
        response: Result<lapce_rpc::proxy::ProxyResponse, lapce_rpc::RpcError>,
    ) -> Result<(), String> {
        let result = self
            .document
            .try_update(|session| plan.finish(session, response))
            .ok_or("Document has closed")?;
        if result.is_ok() {
            batch(|| {
                self.content_changed.set(());
                self.buffer_changed.set(());
                self.sync_changed.set(());
                self.loading_changed.set(());
            });
        }
        result
    }

    pub fn is_pristine(self, tracked: bool) -> bool {
        if tracked {
            self.buffer_changed.track();
        }
        self.document
            .with_untracked(|session| session.document.is_pristine())
    }
    pub fn mark_saved(self, revision: u64) -> bool {
        let saved = self
            .document
            .try_update(|session| session.document.mark_saved(revision))
            .unwrap_or(false);
        if saved {
            self.buffer_changed.set(());
        }
        saved
    }

    pub fn buffer(self) -> DocumentField<lapce_core::buffer::Buffer> {
        self.field(self.buffer_changed, |d| &d.buffer, |d| &mut d.buffer)
    }

    pub fn loaded(self) -> DocumentField<bool> {
        self.field(
            self.loading_changed,
            |d| &d.loading.loaded,
            |d| &mut d.loading.loaded,
        )
    }

    pub fn load_error(self) -> DocumentField<Option<String>> {
        self.field(
            self.loading_changed,
            |d| &d.loading.error,
            |d| &mut d.loading.error,
        )
    }

    pub fn fail_loading(self, message: String) {
        let changed = self
            .document
            .try_update(|document| {
                let before = document.loading.clone();
                document.fail_loading(message);
                before != document.loading
            })
            .unwrap_or(false);
        if changed {
            self.loading_changed.set(());
        }
    }

    pub fn syntax(self) -> DocumentField<lapce_core::syntax::Syntax> {
        self.field(self.syntax_changed, |d| &d.syntax, |d| &mut d.syntax)
    }

    pub fn content(self) -> DocumentField<lapce_engine::document::DocContent> {
        self.field(self.content_changed, |d| &d.content, |d| &mut d.content)
    }

    pub fn inlay_hints(
        self,
    ) -> DocumentField<Option<lapce_xi_rope::spans::Spans<lsp_types::InlayHint>>>
    {
        self.field(
            self.inlay_changed,
            |d| &d.inlay_hints,
            |d| &mut d.inlay_hints,
        )
    }

    pub fn with_document<O>(self, f: impl FnOnce(&Document) -> O) -> Option<O> {
        self.document.try_with_untracked(|document| {
            document.map(|session| f(&session.document))
        })
    }
    pub fn diagnostic_publication(
        self,
        diagnostics: Vec<lsp_types::Diagnostic>,
        version: Option<i32>,
    ) -> Option<lapce_engine::diagnostics::DiagnosticPublication> {
        self.document.try_with_untracked(|session| {
            session.and_then(|session| {
                lapce_engine::diagnostics::DiagnosticPublication::prepare(
                    diagnostics,
                    version,
                    Some(session),
                )
            })
        })
    }

    pub fn set_syntax(self, syntax: lapce_core::syntax::Syntax) {
        if self
            .document
            .try_update(|document| document.set_syntax(syntax))
            .is_some()
        {
            self.syntax_changed.set(());
        }
    }

    pub fn syntax_job(
        self,
        edits: Option<Vec<lapce_core::syntax::edit::SyntaxEdit>>,
    ) -> Option<lapce_engine::syntax::SyntaxJob> {
        self.document
            .try_update(|document| document.syntax_job(edits))
    }

    pub fn accept_syntax(self, result: lapce_engine::syntax::SyntaxResult) -> bool {
        let accepted = self
            .document
            .try_update(|document| document.accept_syntax(result))
            .unwrap_or(false);
        if accepted {
            self.syntax_changed.set(());
        }
        accepted
    }

    fn field<T: 'static>(
        self,
        changed: RwSignal<()>,
        get: fn(&SessionDocument) -> &T,
        get_mut: fn(&mut SessionDocument) -> &mut T,
    ) -> DocumentField<T> {
        DocumentField::new(self.document, changed, get, get_mut, None)
    }

    /// Run a buffer operation against the engine model, then notify views after
    /// the mutable borrow ends. Syntax/content observers remain independent.
    pub fn update_buffer<O>(self, f: impl FnOnce(&mut Document) -> O) -> Option<O> {
        let (result, loading_changed) = self.document.try_update(|document| {
            let before = document.loading.clone();
            let result = f(document);
            (result, before != document.loading)
        })?;
        batch(|| {
            self.buffer_changed.set(());
            self.inlay_changed.set(());
            if loading_changed {
                self.loading_changed.set(());
            }
        });
        Some(result)
    }
}

pub type DocumentField<T> = crate::reactive_model::ModelField<SessionDocument, T>;

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use lapce_core::buffer::rope_text::RopeText;
    use lapce_engine::document::DocContent;
    use lapce_xi_rope::Rope;

    use super::*;

    #[test]
    fn projections_share_the_engine_model_and_notify_after_mutation() {
        let scope = Scope::new();
        let model = DocumentModel::new(scope, Document::empty(DocContent::Local));
        let buffer = model.buffer();
        let content = model.content();
        let buffer_reads = Rc::new(Cell::new(0));
        let content_reads = Rc::new(Cell::new(0));
        scope.create_effect({
            let reads = buffer_reads.clone();
            move |_| {
                // Reading here also verifies that notification does not run
                // while the engine document is still mutably borrowed.
                buffer.with(|buffer| buffer.len());
                reads.set(reads.get() + 1);
            }
        });
        scope.create_effect({
            let reads = content_reads.clone();
            move |_| {
                content.track();
                reads.set(reads.get() + 1);
            }
        });

        model
            .update_buffer(|document| document.load(Rope::from("hello")))
            .unwrap();
        assert_eq!(buffer.with_untracked(|b| b.to_string()), "hello");
        assert_eq!(buffer_reads.get(), 2);
        assert_eq!(content_reads.get(), 1);

        content.set(DocContent::File {
            path: "example.rs".into(),
            read_only: true,
        });
        assert!(model.document.with_untracked(|d| d.content.read_only()));
        assert_eq!(buffer_reads.get(), 2);
        assert_eq!(content_reads.get(), 2);
        buffer.update(|buffer| buffer.set_pristine());
        assert!(model.document.with_untracked(|d| d.buffer.is_pristine()));
        assert_eq!(buffer_reads.get(), 3);
        scope.dispose();
    }

    #[test]
    fn disposing_the_scope_disposes_all_projections() {
        let scope = Scope::new();
        let model = DocumentModel::new(scope, Document::empty(DocContent::Local));
        let buffer = model.buffer();
        scope.dispose();
        assert!(buffer.try_get_untracked().is_none());
        assert!(buffer.try_get().is_none());
        assert!(buffer.try_update(|_| ()).is_none());
    }

    #[test]
    fn scratch_identity_comes_from_the_engine() {
        let scope = Scope::new();
        let id = lapce_rpc::buffer::BufferId::next();
        let model = DocumentModel::new(
            scope,
            Document::empty(DocContent::Scratch {
                id,
                name: "scratch".into(),
            }),
        );
        assert_eq!(model.id(), id);
        scope.dispose();
    }

    #[test]
    fn syntax_observers_are_notified_only_after_engine_acceptance() {
        let scope = Scope::new();
        let model = DocumentModel::new(scope, Document::empty(DocContent::Local));
        let reads = Rc::new(Cell::new(0));
        let syntax = model.syntax();
        scope.create_effect({
            let reads = reads.clone();
            move |_| {
                syntax.with(|syntax| syntax.styles.is_some());
                reads.set(reads.get() + 1);
            }
        });
        let result = model.syntax_job(None).unwrap().run().unwrap();
        assert_eq!(reads.get(), 1);
        model.set_syntax(lapce_core::syntax::Syntax::plaintext());
        assert_eq!(reads.get(), 2);
        assert!(!model.accept_syntax(result));
        assert_eq!(reads.get(), 2);
        let result = model.syntax_job(None).unwrap().run().unwrap();
        assert!(model.accept_syntax(result));
        assert_eq!(reads.get(), 3);
        scope.dispose();
    }
}

#[cfg(test)]
mod loading_tests {
    use super::*;
    use lapce_engine::document::DocContent;
    use lapce_xi_rope::Rope;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn loading_observers_read_engine_state_after_failure_and_success() {
        let scope = Scope::new();
        let model = DocumentModel::new(
            scope,
            Document::empty(DocContent::File {
                path: "example.txt".into(),
                read_only: false,
            }),
        );
        let states = Rc::new(RefCell::new(Vec::new()));
        scope.create_effect({
            let states = states.clone();
            move |_| {
                states
                    .borrow_mut()
                    .push((model.loaded().get(), model.load_error().get()));
                model.with_document(|document| {
                    assert_eq!(
                        document.loading.loaded,
                        model.loaded().get_untracked()
                    )
                });
            }
        });
        model.fail_loading("unavailable".into());
        assert_eq!(
            states.borrow().last(),
            Some(&(false, Some("unavailable".into())))
        );
        model.update_buffer(|document| document.load(Rope::from("loaded")));
        assert_eq!(states.borrow().last(), Some(&(true, None)));
        let count = states.borrow().len();
        model.fail_loading("late failure".into());
        assert_eq!(states.borrow().len(), count);
        scope.dispose();
    }
}

#[cfg(test)]
mod synchronization_tests {
    use super::*;
    use lapce_engine::document::DocContent;
    use lapce_rpc::proxy::ProxyResponse;
    use lapce_xi_rope::Rope;

    #[test]
    fn synchronization_and_text_share_one_engine_session_owner() {
        let scope = Scope::new();
        let model = DocumentModel::new(
            scope,
            Document::new(
                DocContent::File {
                    path: "example.txt".into(),
                    read_only: false,
                },
                Rope::from("local"),
            ),
        );
        assert!(model.initialize_sync(7));
        let owner = model.id();
        model
            .recover_sync(ProxyResponse::BufferSnapshot {
                snapshot: Default::default(),
                buffer_id: owner,
                path: "example.txt".into(),
                content: "remote".into(),
                read_only: false,
                rev: 12,
            })
            .unwrap();
        let effect = model
            .synchronization()
            .try_update(|sync| sync.as_mut().unwrap().next_effect().unwrap())
            .unwrap();
        let lapce_rpc::proxy::ProxyRequest::UpdateBuffer { delta, rev: 13, .. } =
            effect.request
        else {
            panic!("expected recovery edit");
        };
        assert_eq!(delta.apply(&Rope::from("remote")).to_string(), "local");
        assert_eq!(model.buffer().get_untracked().to_string(), "local");
        scope.dispose();
    }
}

#[cfg(test)]
mod reopening_tests {
    use super::*;
    use lapce_engine::document::DocContent;
    use lapce_rpc::proxy::ProxyResponse;
    use lapce_xi_rope::Rope;
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn reopen_notifies_dirty_observers_without_replacing_local_buffer() {
        let scope = Scope::new();
        let model = DocumentModel::new(
            scope,
            Document::new(
                DocContent::File {
                    path: "example.txt".into(),
                    read_only: false,
                },
                Rope::from("local"),
            ),
        );
        let observed = Rc::new(Cell::new(false));
        scope.create_effect({
            let observed = observed.clone();
            move |_| {
                observed.set(model.is_pristine(true));
            }
        });
        assert!(observed.get());
        let revision = model.buffer().get_untracked().rev();
        let plan = model.reopen_plan().unwrap();
        model
            .finish_reopen(
                plan,
                Ok(ProxyResponse::NewBufferResponse {
                    snapshot: Default::default(),
                    content: "disk".into(),
                    read_only: false,
                    rev: 7,
                }),
            )
            .unwrap();
        assert!(!observed.get());
        assert_eq!(model.buffer().get_untracked().rev(), revision);
        assert_eq!(model.buffer().get_untracked().to_string(), "local");
        assert!(model.mark_saved(revision));
        assert!(observed.get());
        scope.dispose();
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use lapce_engine::{document::DocContent, navigation::NavigationQuery};
    use lapce_rpc::proxy::ProxyResponse;
    use lapce_xi_rope::Rope;

    #[test]
    fn gui_navigation_uses_shared_snapshot_validation_and_handles_disposal() {
        let scope = Scope::new();
        let model = DocumentModel::new(
            scope,
            Document::new(
                DocContent::File {
                    path: "file.txt".into(),
                    read_only: true,
                },
                Rope::from("🌍text"),
            ),
        );
        assert!(model.navigation_plan(NavigationQuery::Symbols).is_err());
        model.document.update(|session| {
            session.initialize_sync(0);
        });
        let plan = model
            .navigation_plan(NavigationQuery::References { offset: 4 })
            .unwrap();
        assert!(
            model
                .finish_navigation(
                    plan,
                    Ok(ProxyResponse::GetReferencesResponse { references: vec![] })
                )
                .is_ok()
        );
        let plan = model
            .navigation_plan(NavigationQuery::References { offset: 4 })
            .unwrap();
        model
            .update_buffer(|document| document.reload(Rope::from("external"), true));
        assert!(
            model
                .finish_navigation(
                    plan,
                    Ok(ProxyResponse::GetReferencesResponse { references: vec![] })
                )
                .is_err()
        );
        let plan = model
            .navigation_plan(NavigationQuery::References { offset: 0 })
            .unwrap();
        scope.dispose();
        assert!(
            model
                .finish_navigation(
                    plan,
                    Ok(ProxyResponse::GetReferencesResponse { references: vec![] })
                )
                .is_err()
        );
    }
}

#[cfg(test)]
mod disconnect_tests {
    use super::*;
    use lapce_engine::document::DocContent;
    use lapce_xi_rope::Rope;

    #[test]
    fn initial_loading_observer_sees_attached_readonly_session() {
        let scope = Scope::new();
        let model = DocumentModel::new(
            scope,
            Document::empty(DocContent::File {
                path: "file.rs".into(),
                read_only: false,
            }),
        );
        let loaded = model.loaded();
        let observed = std::rc::Rc::new(std::cell::Cell::new(false));
        scope.create_effect({
            let observed = observed.clone();
            move |_| {
                if loaded.get() {
                    assert!(model.backend_ready());
                    assert!(model.content().get_untracked().read_only());
                    assert_eq!(model.buffer().get_untracked().to_string(), "disk🌍");
                    observed.set(true);
                }
            }
        });
        let opening = model.opening().unwrap();
        model
            .finish_opening(
                opening,
                Ok(lapce_rpc::proxy::ProxyResponse::NewBufferResponse {
                    snapshot: Default::default(),
                    content: "disk🌍".into(),
                    read_only: false,
                    rev: 7,
                }),
            )
            .unwrap()
            .unwrap();
        assert!(observed.get());
        scope.dispose();
    }

    #[test]
    fn reconnect_completes_interrupted_loading_and_notifies_views() {
        let scope = Scope::new();
        let model = DocumentModel::new(
            scope,
            Document::empty(DocContent::File {
                path: "empty.rs".into(),
                read_only: true,
            }),
        );
        model.fail_loading("backend disconnected".into());
        let loaded = model.loaded();
        let observed = std::rc::Rc::new(std::cell::Cell::new(false));
        scope.create_effect({
            let observed = observed.clone();
            move |_| {
                observed.set(loaded.get());
            }
        });
        assert!(!observed.get());
        let plan = model.reopen_plan().unwrap();
        model
            .finish_reopen(
                plan,
                Ok(lapce_rpc::proxy::ProxyResponse::NewBufferResponse {
                    snapshot: Default::default(),
                    content: String::new(),
                    read_only: true,
                    rev: 0,
                }),
            )
            .unwrap();
        assert!(observed.get());
        assert!(model.backend_ready());
        assert!(model.load_error().get_untracked().is_none());
        scope.dispose();
    }

    #[test]
    fn gui_disconnect_revokes_engine_ownership_without_replacing_text() {
        let scope = Scope::new();
        let model = DocumentModel::new(
            scope,
            Document::new(
                DocContent::File {
                    path: "file.rs".into(),
                    read_only: true,
                },
                Rope::from("cached🌍"),
            ),
        );
        model.document.update(|session| {
            assert!(session.initialize_sync(1));
        });
        assert!(model.backend_ready());
        assert!(!model.can_close_locally());
        model.backend_disconnected();
        assert!(!model.backend_ready());
        assert!(model.can_close_locally());
        model.document.with_untracked(|session| {
            assert!(session.loading.loaded);
            assert_eq!(session.buffer.to_string(), "cached🌍");
            assert!(session.synchronization.is_none());
        });
        scope.dispose();
        model.backend_disconnected();
    }
}
