//! Document opening policy shared by frontend loading adapters.
use std::path::PathBuf;

use lapce_rpc::{
    RpcError,
    buffer::BufferId,
    proxy::{ProxyRequest, ProxyResponse},
};

use crate::document::Document;

pub struct OpenDocument {
    id: BufferId,
    path: PathBuf,
    revision: u64,
}

pub struct OpenedDocument {
    pub content: String,
    pub read_only: bool,
    pub remote_revision: u64,
    pub snapshot: lapce_rpc::highlighting::TextSnapshot,
}

/// An asynchronous opening belongs to one backend attachment as well as a file.
pub struct SessionOpening {
    opening: OpenDocument,
    generation: crate::session::DocumentBackendGeneration,
}
impl SessionOpening {
    /// Complete the shared load/ownership transition before hosts notify views.
    pub fn apply(
        self,
        session: &mut crate::session::SessionDocument,
        response: Result<ProxyResponse, RpcError>,
    ) -> Result<(), RpcError> {
        if self.generation == session.backend_generation() {
            session.opening = false;
        }
        let opened = match self.finish(session, response) {
            Ok(opened) => opened,
            Err(error) => {
                if error.code != -32801 {
                    session.document.fail_loading(error.message.clone());
                }
                return Err(error);
            }
        };
        if let crate::document::DocContent::File { read_only, .. } =
            &mut session.document.content
        {
            *read_only = true;
        }
        session
            .document
            .load(lapce_xi_rope::Rope::from(opened.content));
        session.initialize_sync(opened.remote_revision);
        session.document.backend_snapshot = Some(opened.snapshot);
        Ok(())
    }
    pub fn capture(session: &mut crate::session::SessionDocument) -> Option<Self> {
        if session.opening || session.backend_ready() {
            return None;
        }
        let opening = OpenDocument::capture(session)?;
        session.opening = true;
        Some(Self {
            opening,
            generation: session.backend_generation(),
        })
    }
    pub fn request(&self) -> ProxyRequest {
        self.opening.request()
    }
    pub fn finish(
        self,
        session: &crate::session::SessionDocument,
        response: Result<ProxyResponse, RpcError>,
    ) -> Result<OpenedDocument, RpcError> {
        if session.backend_generation() != self.generation {
            return Err(RpcError {
                code: -32801,
                message: "Backend changed while opening document".into(),
            });
        }
        self.opening.finish(session, response)
    }
}

impl OpenDocument {
    pub fn capture(document: &Document) -> Option<Self> {
        Some(Self {
            id: document.id,
            path: document.content.path()?.clone(),
            revision: document.buffer.rev(),
        })
    }

    pub fn request(&self) -> ProxyRequest {
        ProxyRequest::NewBuffer {
            buffer_id: self.id,
            path: self.path.clone(),
        }
    }

    /// Consume an opening attempt once. A response cannot overwrite edits made
    /// while loading, a replacement document, or a document moved to another path.
    pub fn finish(
        self,
        document: &Document,
        response: Result<ProxyResponse, RpcError>,
    ) -> Result<OpenedDocument, RpcError> {
        if document.id != self.id
            || document.content.path() != Some(&self.path)
            || document.buffer.rev() != self.revision
        {
            return Err(RpcError {
                code: -32801,
                message: "Document changed while opening".into(),
            });
        }
        match response? {
            ProxyResponse::NewBufferResponse {
                content,
                read_only: _,
                rev,
                snapshot,
            } => Ok(OpenedDocument {
                content,
                read_only: true,
                remote_revision: rev,
                snapshot,
            }),
            _ => Err(RpcError {
                code: -32603,
                message: "Unexpected document-open response".into(),
            }),
        }
    }
}
