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

#[test]
fn backend_structural_answers_match_parser_queries_at_every_cursor_boundary() {
    for (path, source) in [
        (
            "sample.rs",
            "fn main() {\n    let café = call([1, 2]); // (ignored)\n    println!(\"{}\", café);\n}\n",
        ),
        (
            "sample.py",
            "def greet(name):\r\n    # (ignored)\r\n    return call([name, 'é'])\r\n",
        ),
    ] {
        let mut parsed = Syntax::init(Path::new(path));
        parsed.parse(1, Rope::from(source), None);
        let mut presentation = Syntax::without_parser(parsed.language);
        presentation.text = parsed.text.clone();
        presentation.remote_structure =
            Some(std::sync::Arc::new(parsed.structural_ranges()));
        for offset in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
            assert_eq!(
                presentation.find_matching_pair(offset),
                parsed.find_matching_pair(offset),
                "matching {path}:{offset}"
            );
            assert_eq!(
                presentation.parent_offset(offset),
                parsed.parent_offset(offset),
                "parent {path}:{offset}"
            );
            assert_eq!(
                presentation.find_enclosing_pair(offset),
                parsed.find_enclosing_pair(offset),
                "enclosing {path}:{offset}"
            );
            assert_eq!(
                presentation.find_enclosing_parentheses(offset),
                parsed.find_enclosing_parentheses(offset),
                "parentheses {path}:{offset}"
            );
            assert_eq!(
                presentation.sticky_headers(offset),
                parsed.sticky_headers(offset),
                "headers {path}:{offset}"
            );
            for tag in ["(", ")", "{", "}", "[", "]"] {
                for previous in [true, false] {
                    assert_eq!(
                        presentation.find_tag(offset, previous, tag),
                        parsed.find_tag(offset, previous, tag),
                        "tag {path}:{offset} {tag} {previous}"
                    );
                }
            }
        }
    }
}

#[test]
fn bracket_coloring_ignores_missing_tokens_in_incomplete_files() {
    for (path, source) in [
        ("broken.rs", "fn broken(\n"),
        ("missing-brace.rs", "fn main() {\n"),
        ("missing-brace.rs", "fn main() {\n    let x = call(42);\n"),
        ("broken.py", "def broken(\n"),
    ] {
        let mut syntax = Syntax::init(Path::new(path));
        syntax.parse(1, Rope::from(source), None);
        let mut brackets =
            lapce_core::syntax::BracketParser::new(String::new(), true, u64::MAX);
        let buffer = lapce_core::buffer::Buffer::new(source);
        brackets.update_code(source.into(), &buffer, Some(&syntax));
        let rope = Rope::from(source);
        for (line, spans) in brackets.bracket_pos {
            for span in spans {
                let start = rope.offset_of_line(line) + span.start;
                let end = rope.offset_of_line(line) + span.end;
                assert!(matches!(
                    source.get(start..end),
                    Some("(" | ")" | "{" | "}" | "[" | "]")
                ));
            }
        }
    }
}

#[test]
fn cancelled_parse_does_not_retain_styles_from_an_older_text() {
    let mut syntax = Syntax::init(Path::new("a.rs"));
    syntax.parse(1, Rope::from("fn previous() {}"), None);
    assert!(syntax.styles.is_some());
    syntax
        .cancel_flag
        .store(1, std::sync::atomic::Ordering::Relaxed);
    syntax.parse(2, Rope::from("fn x() {}"), None);
    assert!(syntax.styles.is_none());
}
