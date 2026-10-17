//! Workspace search orchestration and repository browsing shared by hosts.
use crate::{
    global_search::{WorkspaceSearch, WorkspaceSearchQuery, WorkspaceSearchTicket},
    source_control::SourceControlState,
};
use lapce_rpc::{
    RpcError,
    proxy::{ProxyRequest, ProxyResponse},
};

#[derive(Clone, Debug, Default)]
pub struct WorkspaceActions {
    pub search: WorkspaceSearch,
    pub source_control: SourceControlState,
}
#[derive(Clone, Debug)]
pub struct WorkspaceEffect {
    pub request: ProxyRequest,
    ticket: WorkspaceSearchTicket,
}
#[derive(Debug)]
pub enum WorkspaceOutcome {
    Stale,
    SearchFinished(Result<(), String>),
}
impl WorkspaceActions {
    pub fn search(
        &mut self,
        query: WorkspaceSearchQuery,
    ) -> Option<WorkspaceEffect> {
        self.search
            .begin(query)
            .map(|(ticket, request)| WorkspaceEffect { request, ticket })
    }
    pub fn complete(
        &mut self,
        effect: WorkspaceEffect,
        response: Result<ProxyResponse, RpcError>,
    ) -> WorkspaceOutcome {
        if !self.search.finish(effect.ticket, response) {
            return WorkspaceOutcome::Stale;
        }
        WorkspaceOutcome::SearchFinished(
            self.search.error.clone().map_or(Ok(()), Err),
        )
    }
}

/// A workspace-wide language query independent of any document or view.
#[derive(Clone, Debug)]
pub struct WorkspaceSymbolsQuery {
    query: String,
}
impl WorkspaceSymbolsQuery {
    pub fn new(query: String) -> Self {
        Self { query }
    }
    pub fn request(&self) -> ProxyRequest {
        ProxyRequest::GetWorkspaceSymbols {
            query: self.query.clone(),
        }
    }
    pub fn finish(
        self,
        response: Result<ProxyResponse, RpcError>,
    ) -> Result<Vec<lsp_types::SymbolInformation>, RpcError> {
        match response? {
            ProxyResponse::GetWorkspaceSymbols { symbols } => Ok(symbols),
            _ => Err(RpcError {
                code: -32603,
                message: "Unexpected workspace symbols response".into(),
            }),
        }
    }
}
