use pdxscript::{ErrorKind, MAX_NESTING_DEPTH, script::*};
use proptest::prelude::*;

fn roundtrip(text: &str) {
    let doc = parse(text, "test").unwrap();
    let written = serialize(&doc.items).unwrap();
    let again = parse(&written, "roundtrip").unwrap();
    assert_eq!(without_spans(&doc.items), without_spans(&again.items));
    assert!(again.diagnostics.is_empty());
    assert_eq!(written, serialize(&again.items).unwrap());
}
#[test]
fn grammar_and_roundtrips() {
    for source in [
        "",
        "a = yes a = no",
        "{ a OR = { x > 3 } {} }",
        "color = hsv { .1 +0.10 3 }",
        "x = rgb\n{}",
        "@x = 9007199254740993\ny = @x",
        "v = @[ 1 + 2 ]",
        "v = @\\[ $A$ + 2 ]",
        "[[P] x = yes ]",
        "[[!P] x = { ]",
        "[[P] \" ] \" # ]\n @[ ] ]",
        "\"a b\" != \"yes\"",
        "\u{feff}a = 1",
        "v = \"multi\nline\"",
        "[[P] [[Q] } ] ]",
    ] {
        roundtrip(source);
    }
}
#[test]
fn documented_repairs_and_errors() {
    for (source, kind) in [
        ("}", Repair::StrayClosingBrace),
        ("x = {", Repair::UnclosedAtEof),
        ("x { y = 1 }", Repair::OperatorLessEntry),
    ] {
        let doc = parse(source, "bad").unwrap();
        assert_eq!(doc.diagnostics[0].kind, kind);
        roundtrip(source);
    }
    for source in [
        "a == b",
        "a ?= b",
        "\"oops",
        "@[oops",
        "[[P]oops",
        "]",
        "= 1",
        "a =",
        "x = [[P] a ]",
    ] {
        assert!(parse(source, "bad").is_err(), "{source}");
    }
}
#[test]
fn exact_numerals_and_checked_construction() {
    assert_eq!(canonical_numeral("-000.000").unwrap(), "0");
    assert_eq!(canonical_numeral("+000.1200").unwrap(), "0.12");
    assert_eq!(
        number_value("1000000000000000128").unwrap(),
        1000000000000000128.0
    );
    assert!(try_number_value("1000000000000000100").is_none());
    assert!(try_number_value("9007199254740993").is_none());
    assert_eq!(try_number_value("0.1"), Some(0.1));
    assert!(numeral("1e3").is_err());
    assert!(quoted("x\\").is_err());
    assert!(inline_math("@[x]y").is_err());
    assert!(var_ref("x").is_err());
    assert!(param_text("P", " x = 1 ", false).is_err());
    assert!(param_text("P", " ] ", false).is_err());
    assert!(param_text("P", " x = { ", false).is_ok());
    let items = vec![
        kv("key", Value::Scalar(scalar("yes").unwrap())).unwrap(),
        list(
            "values",
            vec![numeral("999999999999999999999").unwrap(), boolean(true)],
        )
        .unwrap(),
    ];
    roundtrip(&serialize(&items).unwrap());
    assert!(
        serialize(&[Item::new(ItemKind::Scalar(Scalar::Number {
            lexeme: "01".into()
        }))])
        .is_err()
    );
}
#[test]
fn region_fallback_keeps_text_and_comments() {
    let doc = parse("[[P] #keep\n x = { ]", "r").unwrap();
    assert!(matches!(&doc.items[0].kind,ItemKind::ParamText{text,..} if text == " #keep\n x = { "));
    assert!(doc.diagnostics.is_empty());
    let doc = parse("[[P] x { y = 1 } ]", "r").unwrap();
    assert!(matches!(&doc.items[0].kind, ItemKind::Param { .. }));
    assert_eq!(doc.diagnostics[0].kind, Repair::OperatorLessEntry);
}
#[test]
fn traversal_context_skip_stop_and_flat_regions() {
    let doc = parse("a = { b = 1 }\n[[P] x = { ]", "walk").unwrap();
    let mut seen = Vec::new();
    let stopped = walk_items(
        &doc.items,
        0,
        |item, depth| {
            seen.push(*depth);
            if matches!(&item.kind,ItemKind::Entry(e) if e.key=="a") {
                WalkControl::Skip
            } else {
                WalkControl::Continue(depth + 1)
            }
        },
        RegionPolicy::Read("r"),
    )
    .unwrap();
    assert!(!stopped);
    assert_eq!(seen, vec![0, 0, 1]);
    assert!(walk_items(&doc.items, (), |_, _| WalkControl::Stop, RegionPolicy::Skip).unwrap());
}
#[test]
fn nesting_boundary_does_not_use_call_stack() {
    let source = format!(
        "{}{}",
        "{".repeat(MAX_NESTING_DEPTH),
        "}".repeat(MAX_NESTING_DEPTH)
    );
    let doc = parse(&source, "deep").unwrap();
    assert!(serialize(&doc.items).is_ok());
    assert!(
        !walk_items(
            &doc.items,
            (),
            |_, _| WalkControl::Continue(()),
            RegionPolicy::Skip
        )
        .unwrap()
    );
    let normalized = without_spans(&doc.items);
    assert_eq!(normalized.len(), 1);
    let invalid = format!("{{{source}}}");
    assert_eq!(
        parse(&invalid, "deep").unwrap_err().kind,
        ErrorKind::NestingLimit
    );
    let regions = format!(
        "{}{}",
        "[[P]".repeat(MAX_NESTING_DEPTH + 1),
        "]".repeat(MAX_NESTING_DEPTH + 1)
    );
    assert_eq!(
        parse(&regions, "deep").unwrap_err().kind,
        ErrorKind::NestingLimit
    );
}
#[test]
fn operator_spellings_agree_across_parsing_writing_and_serde() {
    for (operator, spelling) in [
        (Operator::Assign, "="),
        (Operator::Greater, ">"),
        (Operator::Less, "<"),
        (Operator::GreaterEqual, ">="),
        (Operator::LessEqual, "<="),
        (Operator::NotEqual, "!="),
    ] {
        let encoded = serde_json::to_value(operator).unwrap();

        assert_eq!(operator.as_str(), spelling);
        assert_eq!(Operator::parse(spelling), Some(operator));
        assert_eq!(encoded, serde_json::json!(spelling));
        assert_eq!(
            serde_json::from_value::<Operator>(encoded).unwrap(),
            operator
        );
    }

    assert_eq!(Operator::parse("=="), None);
    assert!(serde_json::from_value::<Operator>(serde_json::json!("==")).is_err());
}

proptest! {
    #[test]
    fn arbitrary_input_never_panics(text in ".{0,400}") {
        let _ = parse(&text, "generated");
    }

    #[test]
    fn strings_survive_quoting(text in "[a-zA-Z0-9_ @.]{0,60}") {
        let value = scalar(text.clone()).unwrap();
        let written = scalar_text(&value).unwrap();
        let doc = parse(&written, "generated").unwrap();

        prop_assert!(matches!(&doc.items[0].kind,ItemKind::Scalar(Scalar::String{value,..}) if value==&text), "string changed");
    }

    #[test]
    fn generated_tree_fixpoint(values in prop::collection::vec(("[a-z]{1,8}", any::<i64>(), any::<bool>()),0..40)) {
        let mut items = Vec::new();

        for (key, number, nested) in values {
            let value = numeral(&number.to_string()).unwrap();
            let item = kv(key, Value::Scalar(value)).unwrap();
            let item = if nested {
                block("nested", vec![item]).unwrap()
            } else {
                item
            };

            items.push(item);
        }

        let text = serialize(&items).unwrap();
        let doc = parse(&text, "generated").unwrap();

        prop_assert_eq!(without_spans(&doc.items), items);
    }

    #[test]
    fn accepted_text_repairs_to_a_fixpoint(text in ".{0,300}") {
        if let Ok(doc) = parse(&text, "generated") && let Ok(written) = serialize(&doc.items) {
            let again = parse(&written, "generated").unwrap();

            prop_assert!(again.diagnostics.is_empty());
            prop_assert_eq!(serialize(&again.items).unwrap(), written);
        }
    }
}
