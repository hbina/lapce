//! Backend-owned document and transient-code parsing workers.
use crossbeam_channel::{Sender, bounded};
use lapce_core::{language::LapceLanguage, syntax::Syntax};
use lapce_rpc::{
    core::{CoreNotification, CoreRpcHandler},
    highlighting::{
        HighlightSnapshot, HighlightSpan, HighlightStatus, TextSnapshot,
    },
};
use lapce_xi_rope::Rope;
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

struct Entry {
    text: Rope,
    language: LapceLanguage,
    result: HighlightSnapshot,
    syntax: Option<Syntax>,
    cancel: Arc<AtomicUsize>,
    pending: bool,
    semantic_pending: bool,
    semantic_done: bool,
    syntax_spans: Vec<HighlightSpan>,
}

#[derive(Clone)]
pub struct HighlightService {
    #[cfg(test)]
    parses: Arc<AtomicUsize>,
    snippets: Sender<(
        Vec<lapce_rpc::highlighting::CodeBlock>,
        lapce_rpc::RequestId,
    )>,
    entries: Arc<Mutex<HashMap<PathBuf, Entry>>>,
    wake: Sender<()>,
    core: CoreRpcHandler,
    catalog: crate::plugin::PluginCatalogRpcHandler,
}

impl HighlightService {
    pub fn new(
        core: CoreRpcHandler,
        catalog: crate::plugin::PluginCatalogRpcHandler,
        proxy: lapce_rpc::proxy::ProxyRpcHandler,
    ) -> Self {
        let (snippets, snippet_input) = bounded::<(
            Vec<lapce_rpc::highlighting::CodeBlock>,
            lapce_rpc::RequestId,
        )>(8);
        thread::Builder::new()
            .name("Backend code blocks".into())
            .spawn(move || {
                while let Ok((blocks, id)) = snippet_input.recv() {
                    let blocks = blocks
                        .into_iter()
                        .map(|code| {
                            let language = LapceLanguage::from_name(&code.language)
                                .unwrap_or(LapceLanguage::PlainText);
                            let mut syntax = Syntax::from_language(language);
                            syntax.parse(0, Rope::from(code.text.as_str()), None);
                            let spans = syntax
                                .styles
                                .as_ref()
                                .map(|styles| {
                                    styles
                                        .iter()
                                        .filter_map(|(range, style)| {
                                            Some(HighlightSpan {
                                                start: range.start(),
                                                end: range.end(),
                                                scope: style.fg_color.clone()?,
                                            })
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            lapce_rpc::highlighting::CodeHighlights { code, spans }
                        })
                        .collect();
                    proxy.handle_response(
                        id,
                        Ok(lapce_rpc::proxy::ProxyResponse::CodeHighlights {
                            blocks,
                        }),
                    );
                }
            })
            .expect("start backend code block worker");
        let entries = Arc::new(Mutex::new(HashMap::<PathBuf, Entry>::new()));
        // Work is coalesced in the per-open-buffer map; the wake queue cannot grow.
        let (wake, input) = bounded(1);
        let shared = entries.clone();
        #[cfg(test)]
        let parses = Arc::new(AtomicUsize::new(0));
        #[cfg(test)]
        let worker_parses = parses.clone();
        let worker_core = core.clone();
        let worker_catalog = catalog.clone();
        thread::Builder::new()
            .name("Backend syntax".into())
            .spawn(move || {
                while input.recv().is_ok() {
                    loop {
                        let task = {
                            let mut entries = shared.lock();
                            entries.iter_mut().find(|(_, entry)| entry.pending).map(
                                |(path, entry)| {
                                    entry.pending = false;
                                    (
                                        path.clone(),
                                        entry.result.snapshot.clone(),
                                        entry.text.clone(),
                                        entry.syntax.take(),
                                        entry.cancel.clone(),
                                        entry.language,
                                    )
                                },
                            )
                        };
                        let Some((path, snapshot, text, syntax, cancel, language)) =
                            task
                        else {
                            break;
                        };
                        let mut syntax = syntax
                            .unwrap_or_else(|| Syntax::from_language(language));
                        syntax.cancel_flag = cancel.clone();
                        let supported = matches!(
                            syntax.language,
                            LapceLanguage::Rust | LapceLanguage::Python
                        );
                        #[cfg(test)]
                        worker_parses.fetch_add(1, Ordering::Relaxed);
                        syntax.parse(snapshot.revision, text.clone(), None);
                        if cancel.load(Ordering::Relaxed) != 0 {
                            continue;
                        }
                        let spans = syntax
                            .styles
                            .as_ref()
                            .map(|styles| {
                                styles
                                    .iter()
                                    .filter_map(|(range, style)| {
                                        Some(HighlightSpan {
                                            start: range.start(),
                                            end: range.end(),
                                            scope: style.fg_color.clone()?,
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let structure = if syntax.styles.is_some() {
                            syntax.structural_ranges()
                        } else {
                            Vec::new()
                        };
                        if cancel.load(Ordering::Relaxed) != 0 {
                            continue;
                        }
                        let mut brackets = Vec::new();
                        if supported && syntax.styles.is_some() {
                            let code = text.to_string();
                            let mut parser = lapce_core::syntax::BracketParser::new(
                                String::new(),
                                true,
                                u64::MAX,
                            );
                            let buffer = lapce_core::buffer::Buffer::new(&code);
                            parser.update_code(code, &buffer, Some(&syntax));
                            for (line, styles) in parser.bracket_pos {
                                let base = text.offset_of_line(line);
                                for style in styles {
                                    if let Some(scope) = style.style.fg_color {
                                        brackets.push(HighlightSpan {
                                            start: base + style.start,
                                            end: base + style.end,
                                            scope,
                                        });
                                    }
                                }
                            }
                            brackets.sort_by_key(|span| span.start);
                        }
                        let result = {
                            let mut entries = shared.lock();
                            let Some(entry) = entries.get_mut(&path) else {
                                continue;
                            };
                            if entry.result.snapshot != snapshot
                                || cancel.load(Ordering::Relaxed) != 0
                            {
                                continue;
                            }
                            entry.syntax_spans = spans;
                            entry.result.spans = entry.syntax_spans.clone();
                            entry.result.brackets = brackets;
                            entry.result.structure = structure;
                            entry.result.normal_lines = syntax.normal_lines.clone();
                            entry.result.status = if !supported {
                                HighlightStatus::Unsupported
                            } else if syntax.styles.is_none() {
                                HighlightStatus::Failed(
                                    "Parser produced no highlight result".into(),
                                )
                            } else {
                                HighlightStatus::Ready
                            };
                            entry.result.generation += 1;
                            entry.syntax = Some(syntax);
                            entry.result.clone()
                        };
                        worker_core.notification(
                            CoreNotification::PublishHighlights {
                                highlights: result,
                            },
                        );
                        request_semantics(
                            shared.clone(),
                            worker_core.clone(),
                            worker_catalog.clone(),
                            &path,
                        );
                    }
                }
            })
            .expect("start backend syntax worker");
        Self {
            #[cfg(test)]
            parses,
            snippets,
            entries,
            wake,
            core,
            catalog,
        }
    }

    pub fn highlight_code(
        &self,
        blocks: Vec<lapce_rpc::highlighting::CodeBlock>,
        id: lapce_rpc::RequestId,
    ) -> Result<(), lapce_rpc::RpcError> {
        if blocks.len() > 32
            || blocks.iter().map(|block| block.text.len()).sum::<usize>() > 1_048_576
        {
            return Err(lapce_rpc::RpcError {
                code: -32602,
                message: "Code highlighting request exceeds limits".into(),
            });
        }
        self.snippets
            .try_send((blocks, id))
            .map_err(|_| lapce_rpc::RpcError {
                code: -32800,
                message: "Code highlighting worker is busy".into(),
            })
    }

    pub fn schedule(&self, path: PathBuf, snapshot: TextSnapshot, text: Rope) {
        let mut entries = self.entries.lock();
        if entries
            .get(&path)
            .is_some_and(|entry| entry.result.snapshot == snapshot)
        {
            return;
        }
        let previous = entries.remove(&path);
        let language = previous
            .as_ref()
            .map(|entry| entry.language)
            .unwrap_or_else(|| LapceLanguage::from_path(&path));
        let syntax = previous.and_then(|entry| {
            entry.cancel.store(1, Ordering::Relaxed);
            entry.syntax
        });
        let result = HighlightSnapshot {
            path: path.clone(),
            language: language.to_string(),
            snapshot,
            text_len: text.len(),
            generation: 0,
            status: HighlightStatus::Pending,
            spans: Vec::new(),
            brackets: Vec::new(),
            structure: Vec::new(),
            normal_lines: Vec::new(),
        };
        entries.insert(
            path,
            Entry {
                text,
                language,
                result,
                syntax,
                cancel: Arc::new(AtomicUsize::new(0)),
                pending: true,
                semantic_pending: false,
                semantic_done: false,
                syntax_spans: Vec::new(),
            },
        );
        drop(entries);
        let _ = self.wake.try_send(());
    }

    pub fn set_language(
        &self,
        path: &Path,
        language: LapceLanguage,
    ) -> Option<HighlightSnapshot> {
        let result = {
            let mut entries = self.entries.lock();
            let entry = entries.get_mut(path)?;
            if entry.language == language {
                return Some(entry.result.clone());
            }
            entry.cancel.store(1, Ordering::Relaxed);
            entry.cancel = Arc::new(AtomicUsize::new(0));
            entry.language = language;
            entry.result.language = language.to_string();
            entry.syntax = None;
            entry.pending = true;
            entry.semantic_pending = false;
            entry.semantic_done = false;
            entry.syntax_spans.clear();
            entry.result.spans.clear();
            entry.result.brackets.clear();
            entry.result.structure.clear();
            entry.result.normal_lines.clear();
            entry.result.generation += 1;
            entry.result.status = HighlightStatus::Pending;
            entry.result.clone()
        };
        self.core.notification(CoreNotification::PublishHighlights {
            highlights: result.clone(),
        });
        let _ = self.wake.try_send(());
        Some(result)
    }

    pub fn get(&self, path: &Path) -> Option<HighlightSnapshot> {
        self.entries
            .lock()
            .get(path)
            .map(|entry| entry.result.clone())
    }

    pub fn refresh_semantics(&self, force: bool) {
        let paths: Vec<_> = {
            let mut entries = self.entries.lock();
            if force {
                for entry in entries.values_mut() {
                    entry.semantic_done = false;
                }
            }
            entries.keys().cloned().collect()
        };
        for path in paths {
            request_semantics(
                self.entries.clone(),
                self.core.clone(),
                self.catalog.clone(),
                &path,
            );
        }
    }

    pub fn shutdown(&self) {
        let mut entries = self.entries.lock();
        for entry in entries.values() {
            entry.cancel.store(1, Ordering::Relaxed);
        }
        entries.clear();
    }

    pub fn close(&self, path: &Path) {
        if let Some(entry) = self.entries.lock().remove(path) {
            entry.cancel.store(1, Ordering::Relaxed);
        }
    }
}

fn request_semantics(
    entries: Arc<Mutex<HashMap<PathBuf, Entry>>>,
    core: CoreRpcHandler,
    catalog: crate::plugin::PluginCatalogRpcHandler,
    path: &Path,
) {
    let (snapshot, text, cancel) = {
        let mut state = entries.lock();
        let Some(entry) = state.get_mut(path) else {
            return;
        };
        if entry.semantic_pending
            || entry.semantic_done
            || entry.result.status != HighlightStatus::Ready
        {
            return;
        }
        entry.semantic_pending = true;
        (
            entry.result.snapshot.clone(),
            entry.text.clone(),
            entry.cancel.clone(),
        )
    };
    let path = path.to_path_buf();
    let format_catalog = catalog.clone();
    catalog.get_semantic_tokens(&path.clone(), move |plugin, tokens| {
        let entries = entries.clone();
        let core = core.clone();
        let snapshot = snapshot.clone();
        let path = path.clone();
        let cancel = cancel.clone();
        let apply = move |styles: Result<
            Vec<lapce_rpc::style::LineStyle>,
            lapce_rpc::RpcError,
        >| {
            let result = {
                let mut entries = entries.lock();
                let Some(entry) = entries.get_mut(&path) else {
                    return;
                };
                if entry.result.snapshot != snapshot
                    || cancel.load(Ordering::Relaxed) != 0
                {
                    return;
                }
                entry.semantic_pending = false;
                let Ok(styles) = styles else { return };
                let merged = merge_semantic_spans(
                    &entry.syntax_spans,
                    styles,
                    &entry.text.to_string(),
                );
                entry.semantic_done = true;
                if entry.result.spans == merged {
                    return;
                }
                entry.result.spans = merged;
                entry.result.generation += 1;
                entry.result.clone()
            };
            core.notification(CoreNotification::PublishHighlights {
                highlights: result,
            });
        };
        match tokens {
            Ok(tokens) => format_catalog.format_semantic_tokens(
                plugin,
                tokens,
                text.clone(),
                Box::new(apply),
            ),
            Err(error) => apply(Err(error)),
        }
    });
}

/// Semantic spans override only the covered bytes; syntax survives elsewhere.
fn merge_semantic_spans(
    syntax: &[HighlightSpan],
    mut styles: Vec<lapce_rpc::style::LineStyle>,
    text: &str,
) -> Vec<HighlightSpan> {
    styles.sort_by_key(|span| (span.start, span.end));
    let mut previous_end = 0;
    let semantic: Vec<_> = styles
        .into_iter()
        .filter_map(|style| {
            let scope = style.style.fg_color?;
            if style.start < previous_end
                || style.start >= style.end
                || style.end > text.len()
                || !text.is_char_boundary(style.start)
                || !text.is_char_boundary(style.end)
            {
                return None;
            }
            previous_end = style.end;
            Some(HighlightSpan {
                start: style.start,
                end: style.end,
                scope,
            })
        })
        .collect();
    let mut result = Vec::new();
    for base in syntax {
        let mut cursor = base.start;
        let index = semantic.partition_point(|span| span.end <= cursor);
        for overlay in &semantic[index..] {
            if overlay.start >= base.end {
                break;
            }
            if cursor < overlay.start {
                result.push(HighlightSpan {
                    start: cursor,
                    end: overlay.start.min(base.end),
                    scope: base.scope.clone(),
                });
            }
            cursor = cursor.max(overlay.end).min(base.end);
        }
        if cursor < base.end {
            result.push(HighlightSpan {
                start: cursor,
                end: base.end,
                scope: base.scope.clone(),
            });
        }
    }
    result.extend(semantic);
    result.sort_by_key(|span| span.start);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use lapce_rpc::style::{LineStyle, Style};
    #[test]
    fn repeated_consumers_reuse_one_parse_and_closing_releases_cached_state() {
        let core = CoreRpcHandler::new();
        let proxy = lapce_rpc::proxy::ProxyRpcHandler::new();
        let catalog =
            crate::plugin::PluginCatalogRpcHandler::new(core.clone(), proxy.clone());
        let service = HighlightService::new(core.clone(), catalog, proxy);
        let path = PathBuf::from("/workspace/shared.rs");
        let snapshot = TextSnapshot {
            instance: "one".into(),
            revision: 1,
        };
        for _ in 0..10 {
            service.schedule(
                path.clone(),
                snapshot.clone(),
                Rope::from("fn main() {}"),
            );
        }
        let lapce_rpc::core::CoreRpc::Notification(event) = core
            .rx()
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
        else {
            panic!("publication")
        };
        assert!(
            matches!(*event, CoreNotification::PublishHighlights { highlights } if highlights.status == HighlightStatus::Ready)
        );
        assert_eq!(service.parses.load(Ordering::Relaxed), 1);
        for _ in 0..10 {
            service.schedule(
                path.clone(),
                snapshot.clone(),
                Rope::from("fn main() {}"),
            );
        }
        assert_eq!(service.parses.load(Ordering::Relaxed), 1);
        assert_eq!(service.get(&path).unwrap().snapshot, snapshot);
        service.close(&path);
        assert!(service.get(&path).is_none());
        assert!(service.entries.lock().is_empty());
    }

    #[test]
    fn semantic_overlay_preserves_syntax_and_discards_malformed_ranges() {
        let source = "éabc def";
        let syntax = vec![HighlightSpan {
            start: 0,
            end: source.len(),
            scope: "string".into(),
        }];
        let semantic = |start, end| LineStyle {
            start,
            end,
            style: Style {
                fg_color: Some("function".into()),
            },
        };
        let merged = merge_semantic_spans(
            &syntax,
            vec![
                semantic(1, 3),
                semantic(2, 5),
                semantic(3, 4),
                semantic(7, 99),
            ],
            source,
        );
        assert_eq!(
            merged,
            vec![
                HighlightSpan {
                    start: 0,
                    end: 2,
                    scope: "string".into()
                },
                HighlightSpan {
                    start: 2,
                    end: 5,
                    scope: "function".into()
                },
                HighlightSpan {
                    start: 5,
                    end: source.len(),
                    scope: "string".into()
                },
            ]
        );
    }
}
