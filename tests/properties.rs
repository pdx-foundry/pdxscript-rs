use pdxscript::script::*;
use proptest::prelude::*;
fn trees() -> impl Strategy<Value = Item> {
    let leaf = prop_oneof![
        any::<bool>().prop_map(boolean),
        any::<i128>().prop_map(|n| numeral(&n.to_string()).unwrap()),
        "[a-z][a-z0-9_]{0,15}".prop_map(|s| scalar(s).unwrap()),
        "[a-z0-9 @]{0,15}".prop_map(|s| quoted(s).unwrap()),
        "[a-z]{1,8}".prop_map(|s| var_ref(format!("@{s}")).unwrap()),
        "[a-z0-9 +]{0,15}".prop_map(|s| inline_math(format!("@[{s}]")).unwrap()),
    ]
    .prop_map(|s| Item::new(ItemKind::Scalar(s)));
    leaf.prop_recursive(5, 100, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5)
                .prop_map(|items| block("body", items).unwrap()),
            prop::collection::vec(inner.clone(), 0..5)
                .prop_map(|items| Item::new(ItemKind::Container(container(items, None).unwrap()))),
            (prop::collection::vec(inner, 0..5), any::<bool>())
                .prop_map(|(items, negated)| param_block("P", items, negated).unwrap()),
            Just(param_text("P", " x = { # retained\n", false).unwrap()),
        ]
    })
}
proptest! {
    #[test]
    fn all_constructible_forms_roundtrip(tree in trees()) {
        let written=serialize(&[tree]).unwrap();let doc=parse(&written,"generated").unwrap();
        prop_assert!(doc.diagnostics.is_empty());
        prop_assert_eq!(serialize(&doc.items).unwrap(),written);
    }
}
