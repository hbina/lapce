//! Workspace search ownership; rendering and scheduling belong to the host.
use indexmap::IndexMap;
use lapce_rpc::{
    RpcError,
    proxy::{ProxyRequest, ProxyResponse, SearchMatch},
};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceSearchQuery {
    pub pattern: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub is_regex: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkspaceSearchTicket(u64);

#[derive(Clone, Debug, Default)]
pub struct WorkspaceSearch {
    pub query: WorkspaceSearchQuery,
    pub matches: IndexMap<PathBuf, Vec<SearchMatch>>,
    pub error: Option<String>,
    pending: Option<WorkspaceSearchTicket>,
}
impl WorkspaceSearch {
    /// Every query change invalidates earlier results, including clearing input.
    pub fn begin(
        &mut self,
        query: WorkspaceSearchQuery,
    ) -> Option<(WorkspaceSearchTicket, ProxyRequest)> {
        self.query = query;
        self.matches.clear();
        self.error = None;
        self.pending = None;
        if self.query.pattern.is_empty() {
            return None;
        }
        static NEXT: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        let ticket = WorkspaceSearchTicket(
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        self.pending = Some(ticket);
        Some((
            ticket,
            ProxyRequest::GlobalSearch {
                pattern: self.query.pattern.clone(),
                case_sensitive: self.query.case_sensitive,
                whole_word: self.query.whole_word,
                is_regex: self.query.is_regex,
            },
        ))
    }
    pub fn finish(
        &mut self,
        ticket: WorkspaceSearchTicket,
        result: Result<ProxyResponse, RpcError>,
    ) -> bool {
        if self.pending != Some(ticket) {
            return false;
        }
        self.pending = None;
        match result {
            Ok(ProxyResponse::GlobalSearchResponse { matches }) => {
                self.matches = matches
            }
            Ok(_) => {
                self.error = Some("Unexpected workspace search response".into())
            }
            Err(error) => self.error = Some(error.message),
        }
        true
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
}
