//! Read-only language navigation anchored to a document snapshot.
use crate::document::Document;
use lapce_core::{buffer::rope_text::RopeText, rope_text_pos::RopeTextPosition};
use lapce_rpc::{
    RpcError,
    buffer::BufferId,
    proxy::{ProxyRequest, ProxyResponse},
};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug)]
pub enum NavigationQuery {
    Definition {
        offset: usize,
    },
    References {
        offset: usize,
    },
    Implementation {
        offset: usize,
    },
    Hover {
        offset: usize,
    },
    Highlights {
        offset: usize,
    },
    Symbols,
    FoldingRanges,
    InlayHints,
    SemanticTokens,
    HighlightSnapshot,
    SyntaxLanguage {
        language: lapce_core::language::LapceLanguage,
    },
}

pub struct NavigationPlan {
    query: NavigationQuery,
    owner: BufferId,
    path: PathBuf,
    revision: u64,
    length: usize,
    request: ProxyRequest,
}

/// Snapshot validation plus ownership of the backend attachment serving it.
pub struct SessionNavigationPlan {
    plan: NavigationPlan,
    generation: crate::session::DocumentBackendGeneration,
}
impl SessionNavigationPlan {
    pub fn capture(
        session: &crate::session::SessionDocument,
        query: NavigationQuery,
    ) -> Result<Self, RpcError> {
        if !session.backend_ready() {
            return Err(error(-32801, "Document is not connected"));
        }
        Ok(Self {
            plan: NavigationPlan::capture(session, query)?,
            generation: session.backend_generation(),
        })
    }
    pub fn request(&self) -> ProxyRequest {
        self.plan.request()
    }
    pub fn finish(
        self,
        session: &crate::session::SessionDocument,
        response: Result<ProxyResponse, RpcError>,
    ) -> Result<ProxyResponse, RpcError> {
        if !session.backend_ready()
            || session.backend_generation() != self.generation
        {
            return Err(error(-32801, "Backend changed during navigation"));
        }
        self.plan.finish(session, response)
    }
}
fn error(code: i64, message: &str) -> RpcError {
    RpcError {
        code,
        message: message.into(),
    }
}
impl NavigationPlan {
    pub fn capture(
        document: &Document,
        query: NavigationQuery,
    ) -> Result<Self, RpcError> {
        if !document.loading.loaded {
            return Err(error(-32801, "Document is not loaded"));
        }
        let path = document
            .content
            .path()
            .ok_or_else(|| error(-32602, "Navigation requires a file document"))?
            .clone();
        let offset = match query {
            NavigationQuery::Definition { offset }
            | NavigationQuery::References { offset }
            | NavigationQuery::Implementation { offset }
            | NavigationQuery::Hover { offset }
            | NavigationQuery::Highlights { offset } => Some(offset),
            NavigationQuery::Symbols
            | NavigationQuery::FoldingRanges
            | NavigationQuery::InlayHints
            | NavigationQuery::SemanticTokens
            | NavigationQuery::HighlightSnapshot
            | NavigationQuery::SyntaxLanguage { .. } => None,
        };
        if offset.is_some_and(|offset| {
            offset > document.buffer.len()
                || !document.buffer.text().is_codepoint_boundary(offset)
        }) {
            return Err(error(-32602, "Navigation offset is not a UTF-8 boundary"));
        }
        let position =
            offset.map(|offset| document.buffer.offset_to_position(offset));
        let request = match query {
            NavigationQuery::Definition { offset } => ProxyRequest::GetDefinition {
                request_id: offset,
                path: path.clone(),
                position: position.unwrap(),
            },
            NavigationQuery::References { .. } => ProxyRequest::GetReferences {
                path: path.clone(),
                position: position.unwrap(),
            },
            NavigationQuery::Implementation { .. } => {
                ProxyRequest::GotoImplementation {
                    path: path.clone(),
                    position: position.unwrap(),
                }
            }
            NavigationQuery::Hover { .. } => ProxyRequest::GetHover {
                request_id: 0,
                path: path.clone(),
                position: position.unwrap(),
            },
            NavigationQuery::Highlights { .. } => {
                ProxyRequest::GetDocumentHighlights {
                    path: path.clone(),
                    position: position.unwrap(),
                }
            }
            NavigationQuery::Symbols => {
                ProxyRequest::GetDocumentSymbols { path: path.clone() }
            }
            NavigationQuery::FoldingRanges => {
                ProxyRequest::LspFoldingRange { path: path.clone() }
            }
            NavigationQuery::InlayHints => {
                ProxyRequest::GetInlayHints { path: path.clone() }
            }
            NavigationQuery::SyntaxLanguage { language } => {
                ProxyRequest::SetSyntaxLanguage {
                    path: path.clone(),
                    language: language.to_string(),
                }
            }
            NavigationQuery::HighlightSnapshot => {
                ProxyRequest::GetHighlightSnapshot { path: path.clone() }
            }
            NavigationQuery::SemanticTokens => {
                ProxyRequest::GetSemanticTokens { path: path.clone() }
            }
        };
        Ok(Self {
            query,
            owner: document.id,
            path,
            revision: document.buffer.rev(),
            length: document.buffer.len(),
            request,
        })
    }
    pub fn request(&self) -> ProxyRequest {
        self.request.clone()
    }
    pub fn finish(
        self,
        document: &Document,
        response: Result<ProxyResponse, RpcError>,
    ) -> Result<ProxyResponse, RpcError> {
        if document.id != self.owner
            || document.content.path() != Some(&self.path)
            || document.buffer.rev() != self.revision
        {
            return Err(error(-32801, "Document changed during navigation"));
        }
        let response = response?;
        let matches = matches!(
            (&self.query, &response),
            (
                NavigationQuery::Definition { .. },
                ProxyResponse::GetDefinitionResponse { .. }
            ) | (
                NavigationQuery::References { .. },
                ProxyResponse::GetReferencesResponse { .. }
            ) | (
                NavigationQuery::Implementation { .. },
                ProxyResponse::GotoImplementationResponse { .. }
            ) | (
                NavigationQuery::Hover { .. },
                ProxyResponse::HoverResponse { .. }
            ) | (
                NavigationQuery::Highlights { .. },
                ProxyResponse::GetDocumentHighlights { .. }
            ) | (
                NavigationQuery::Symbols,
                ProxyResponse::GetDocumentSymbols { .. }
            ) | (
                NavigationQuery::FoldingRanges,
                ProxyResponse::LspFoldingRangeResponse { .. }
            ) | (
                NavigationQuery::InlayHints,
                ProxyResponse::GetInlayHints { .. }
            )
        );
        let matches = matches
            || matches!(
                (&self.query, &response),
                (
                    NavigationQuery::HighlightSnapshot
                        | NavigationQuery::SyntaxLanguage { .. },
                    ProxyResponse::HighlightSnapshot { .. }
                )
            );
        let matches = matches
            || matches!(
                (&self.query, &response),
                (
                    NavigationQuery::SemanticTokens,
                    ProxyResponse::GetSemanticTokens { .. }
                )
            );
        if let ProxyResponse::GetSemanticTokens { styles } = &response {
            if styles.path != self.path || styles.len != self.length {
                return Err(error(
                    -32801,
                    "Semantic tokens belong to another document snapshot",
                ));
            }
        }
        if !matches {
            return Err(error(-32603, "Unexpected navigation response"));
        }
        Ok(response)
    }
}

/// Normalize definition responses for consumers without indexing empty results.
pub fn first_definition(
    definition: lsp_types::GotoDefinitionResponse,
) -> Option<lsp_types::Location> {
    match definition {
        lsp_types::GotoDefinitionResponse::Scalar(location) => Some(location),
        lsp_types::GotoDefinitionResponse::Array(locations) => {
            locations.into_iter().next()
        }
        lsp_types::GotoDefinitionResponse::Link(links) => {
            links.into_iter().next().map(|link| lsp_types::Location {
                uri: link.target_uri,
                range: link.target_selection_range,
            })
        }
    }
}

/// Normalize both LSP symbol response forms into a tree for outline consumers.
/// Flat responses have no reliable hierarchy, so retain each symbol as a root.
#[allow(deprecated)]
pub fn document_symbol_tree(
    response: lsp_types::DocumentSymbolResponse,
) -> Vec<lsp_types::DocumentSymbol> {
    match response {
        lsp_types::DocumentSymbolResponse::Nested(symbols) => symbols,
        lsp_types::DocumentSymbolResponse::Flat(symbols) => symbols
            .into_iter()
            .map(|symbol| lsp_types::DocumentSymbol {
                name: symbol.name,
                detail: symbol.container_name,
                kind: symbol.kind,
                tags: symbol.tags,
                deprecated: symbol.deprecated,
                range: symbol.location.range,
                selection_range: symbol.location.range,
                children: None,
            })
            .collect(),
    }
}
