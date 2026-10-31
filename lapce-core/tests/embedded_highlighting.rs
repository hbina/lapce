use lapce_core::syntax::Syntax;
use lapce_xi_rope::Rope;
use std::path::Path;

#[test]
fn bundled_rust_and_python_produce_scopes_without_installed_assets() {
    for (path, source) in [
        (
            "example.rs",
            "// café\r\nfn main() { let message = \"hello\"; }\r\n",
        ),
        (
            "example.py",
            "# café\r\ndef greet():\r\n    return \"hello\"\r\n",
        ),
    ] {
        let mut syntax = Syntax::init(Path::new(path));
        assert!(syntax.layers.is_some(), "missing embedded grammar: {path}");
        syntax.parse(1, Rope::from(source), None);
        let styles = syntax.styles.as_ref().expect("parsed highlight spans");
        let scopes: Vec<_> = styles
            .iter()
            .map(|(_, style)| style.fg_color.as_deref().unwrap())
            .collect();
        for scope in ["comment", "keyword", "string", "function"] {
            assert!(
                scopes.contains(&scope),
                "{path} missing {scope}: {scopes:?}"
            );
        }
        for (range, _) in styles.iter() {
            assert!(source.is_char_boundary(range.start()));
            assert!(source.is_char_boundary(range.end()));
            assert!(range.start() < range.end());
        }
    }
}

#[test]
fn unbundled_languages_are_plain_text() {
    let mut syntax = Syntax::init(Path::new("example.js"));
    assert!(syntax.layers.is_none());
    syntax.parse(1, Rope::from("const x = 'hello';"), None);
    assert!(syntax.styles.is_none());
}
