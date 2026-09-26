use pdxscript::cwt::*;

#[test]
fn preserves_dialect_syntax_and_annotations() {
    let doc = parse(
        "## cardinality = 0..1\n### first\n#### second\nx = <tradition>\nx == { yes no bool alias[effect:x] = scalar }",
        "cwt",
    );

    assert!(doc.diagnostics.is_empty());
    assert_eq!(doc.nodes.len(), 2);
    assert_eq!(doc.nodes[0].annotations.len(), 3);
    assert!(matches!(&doc.nodes[0].value,Value::Scalar(s) if s.text=="<tradition>"));
    assert_eq!(doc.nodes[1].op, Some(Operator::Equal));

    let Value::Block { nodes, .. } = &doc.nodes[1].value else {
        panic!()
    };

    assert_eq!(nodes.len(), 4);
    assert!(matches!(&nodes[0].value,Value::Scalar(s) if s.text=="yes"));
}

#[test]
fn all_failures_remain_visible() {
    for source in ["## orphan", "x =", "x = {", "}", "x = \"unclosed"] {
        assert!(!parse(source, "bad").diagnostics.is_empty(), "{source}");
    }

    let doc = parse("x = { ## orphan\n}\ny = scalar", "cwt");

    assert_eq!(doc.diagnostics[0].kind, "orphan-annotation");
    assert!(doc.nodes[1].annotations.is_empty());
}

#[test]
fn spans_include_unicode_quotes_and_braces() {
    let text = "\u{feff}é = {\n\t\"x\" = scalar\n}";
    let doc = parse(text, "cwt");

    assert!(doc.diagnostics.is_empty());
    assert_eq!(doc.nodes[0].span.start, 3);
    assert_eq!(doc.nodes[0].span.end, text.len());
    assert_eq!(doc.nodes[0].span.end_line, 3);
}

#[test]
fn missing_values_do_not_consume_closing_braces_or_leak_annotations() {
    let doc = parse("outer = { ## missing\nx = }\ny = scalar", "cwt");
    let categories: Vec<_> = doc.diagnostics.iter().map(|d| d.kind.as_str()).collect();

    assert_eq!(categories, ["orphan-annotation", "syntax"]);
    assert_eq!(doc.diagnostics[0].message, "missing");
    assert_eq!(doc.diagnostics[1].message, "Missing assignment value");
    assert_eq!(doc.nodes.len(), 2);
    assert!(matches!(&doc.nodes[0].value, Value::Block { nodes, .. } if nodes.is_empty()));
    assert!(doc.nodes[1].annotations.is_empty());
}

#[test]
fn incomplete_input_keeps_annotations_and_end_locations() {
    let source = "## attached\nx = {\n## pending\n\"é\n";
    let doc = parse(source, "cwt");
    let annotations: Vec<_> = doc
        .diagnostics
        .iter()
        .filter(|d| d.kind == "orphan-annotation")
        .map(|d| (d.message.as_str(), d.span.line))
        .collect();
    let syntax: Vec<_> = doc
        .diagnostics
        .iter()
        .filter(|d| d.kind == "syntax")
        .collect();

    assert!(doc.nodes.is_empty());
    assert_eq!(annotations, [("attached", 1), ("pending", 3)]);
    assert_eq!(syntax.len(), 2);
    for diagnostic in syntax {
        assert_eq!(diagnostic.span.end, source.len());
        assert_eq!(diagnostic.span.end_line, 5);
    }
}
