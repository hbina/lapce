use lapce_engine::{
    document::{DocContent, Document},
    open::OpenDocument,
};
use lapce_rpc::proxy::ProxyResponse;
use lapce_xi_rope::Rope;

#[test]
fn session_open_atomically_loads_readonly_content_and_attaches_backend() {
    use lapce_engine::{open::SessionOpening, session::SessionDocument};
    let mut session = SessionDocument::new(document());
    let retired = SessionOpening::capture(&mut session).unwrap();
    assert!(SessionOpening::capture(&mut session).is_none());
    assert!(!session.can_reopen_backend());
    session.backend_disconnected();
    let opening = SessionOpening::capture(&mut session).unwrap();
    opening.apply(&mut session, response(false)).unwrap();
    assert!(session.loading.loaded);
    assert!(session.content.read_only());
    assert!(session.backend_ready());
    assert_eq!(session.buffer.to_string(), "backend");
    assert!(
        matches!(retired.apply(&mut session, Err(lapce_rpc::RpcError {
        code: -1, message: "late failure".into(),
    })), Err(error) if error.code == -32801)
    );
    assert!(session.loading.error.is_none());
    assert!(session.backend_ready());
}

#[test]
fn session_open_failure_retains_unloaded_state_without_backend_ownership() {
    use lapce_engine::{open::SessionOpening, session::SessionDocument};
    let mut session = SessionDocument::new(document());
    let opening = SessionOpening::capture(&mut session).unwrap();
    assert!(
        opening
            .apply(
                &mut session,
                Err(lapce_rpc::RpcError {
                    code: -1,
                    message: "cannot read file".into(),
                })
            )
            .is_err()
    );
    assert!(!session.loading.loaded);
    assert_eq!(session.loading.error.as_deref(), Some("cannot read file"));
    assert!(!session.backend_ready());
}

#[test]
fn session_open_rejects_retired_backend_even_when_text_revision_is_unchanged() {
    let mut session = lapce_engine::session::SessionDocument::new(document());
    let opening = lapce_engine::open::SessionOpening::capture(&mut session).unwrap();
    session.backend_disconnected();
    assert!(
        matches!(opening.finish(&session, response(true)), Err(error) if error.code == -32801)
    );
    let opening = lapce_engine::open::SessionOpening::capture(&mut session).unwrap();
    assert_eq!(
        opening.finish(&session, response(true)).unwrap().content,
        "backend"
    );
}

fn document() -> Document {
    Document::empty(DocContent::File {
        path: "example.txt".into(),
        read_only: false,
    })
}
fn response(read_only: bool) -> Result<ProxyResponse, lapce_rpc::RpcError> {
    Ok(ProxyResponse::NewBufferResponse {
        snapshot: Default::default(),
        content: "backend".into(),
        read_only,
        rev: 42,
    })
}
#[test]
fn opening_always_enforces_read_only_content() {
    let doc = document();
    for read_only in [false, true] {
        let opened = OpenDocument::capture(&doc)
            .unwrap()
            .finish(&doc, response(read_only))
            .unwrap();
        assert_eq!(opened.content, "backend");
        assert_eq!(opened.remote_revision, 42);
        assert!(opened.read_only);
    }
}
#[test]
fn delayed_open_cannot_replace_edited_moved_or_replaced_documents() {
    let mut doc = document();
    let plan = OpenDocument::capture(&doc).unwrap();
    doc.load(Rope::from("newer"));
    assert_eq!(
        plan.finish(&doc, response(false)).err().unwrap().code,
        -32801
    );
    let plan = OpenDocument::capture(&doc).unwrap();
    doc.content = DocContent::File {
        path: "moved.txt".into(),
        read_only: false,
    };
    assert!(plan.finish(&doc, response(false)).is_err());
    let plan = OpenDocument::capture(&doc).unwrap();
    assert!(plan.finish(&document(), response(false)).is_err());
}
