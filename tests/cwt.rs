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
