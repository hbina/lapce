use lapce_core::{cursor::Cursor, syntax::Syntax};
use lapce_engine::document::{DocContent, Document};
use lapce_xi_rope::Rope;

#[test]
fn headless_syntax_jobs_are_accepted_only_by_their_document() {
    let mut document = Document::new(DocContent::Local, Rope::from("hello"));
    let result = document.syntax_job(None).run().unwrap();
    assert!(document.accept_syntax(result));
    let mut source = Document::new(DocContent::Local, Rope::from("hello"));
    let result = source.syntax_job(None).run().unwrap();
    let mut other = Document::new(DocContent::Local, Rope::from("hello"));
    // Give both documents the same revision and job generation. Identity alone
    // must still prevent one document from accepting the other's result.
    let _ = other.syntax_job(None);
    assert!(!other.accept_syntax(result));
}

#[test]
fn starting_a_new_job_cancels_an_unrun_job_and_rejects_completed_old_results() {
    let mut document = Document::new(DocContent::Local, Rope::from("hello"));
    let old = document.syntax_job(None);
    let current = document.syntax_job(None);
    assert!(old.run().is_none());
    assert!(document.accept_syntax(current.run().unwrap()));
    let result = document.syntax_job(None).run().unwrap();
    let current = document.syntax_job(None);
    assert!(!document.accept_syntax(result));
    assert!(document.accept_syntax(current.run().unwrap()));
}

#[test]
fn language_changes_invalidate_results_even_when_text_revision_is_unchanged() {
    let mut document = Document::new(DocContent::Local, Rope::from("hello"));
    let result = document.syntax_job(None).run().unwrap();
    let revision = document.buffer.rev();
    document.set_syntax(Syntax::from_language(
        lapce_core::language::LapceLanguage::Rust,
    ));
    assert_eq!(document.buffer.rev(), revision);
    assert!(!document.accept_syntax(result));
    let result = document.syntax_job(None).run().unwrap();
    assert!(document.accept_syntax(result));
    assert_eq!(
        document.syntax.language,
        lapce_core::language::LapceLanguage::Rust
    );
}

#[test]
fn edits_cancel_pending_parses_and_reject_completed_results() {
    let mut document = Document::new(DocContent::Local, Rope::from("hello"));
    let result = document.syntax_job(None).run().unwrap();
    let pending = document.syntax_job(None);
    document.insert(&mut Cursor::origin(false), "new", false, false);
    assert!(pending.run().is_none());
    assert!(!document.accept_syntax(result));
    let result = document.syntax_job(None).run().unwrap();
    assert!(document.accept_syntax(result));
}
