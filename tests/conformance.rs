#[path = "../tools/normalized.rs"]
mod normalized;
use pdxscript::script::*;
use serde_json::{Value, json};
#[test]
fn pinned_typescript_grammar_claims() {
    let fixture: Value = serde_json::from_str(include_str!("typescript-fixtures.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() > 90);
    for case in cases {
        let source = case["source"].as_str().unwrap();
        let file = case["file"].as_str().unwrap();
        let actual = match parse(source, file) {
            Ok(doc) => normalized::response(&doc).unwrap(),
            Err(e) => json!({"error":true,"line":e.span.line}),
        };
        assert_eq!(actual, case["expected"], "{file}: {source}");
    }
}
