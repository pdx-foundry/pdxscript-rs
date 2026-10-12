use pdxscript::script::*;
use proptest::prelude::*;

fn kinds(source: &str) -> Vec<(TokenKind, &str)> {
    let tape = parse_tape(source, "test").unwrap();
    tape.tokens()
        .iter()
        .map(|token| (token.kind(), tape.text(token)))
        .collect()
}

fn key(op: Operator, quoted: bool) -> TokenKind {
    TokenKind::Key { op, quoted }
}
fn region(negated: bool, end: usize) -> TokenKind {
    TokenKind::Region { negated, end }
}

#[test]
fn entries_headers_and_links() {
    use TokenKind::*;
    let assign = Operator::Assign;
    assert_eq!(
        kinds("\"a b\" = 1 color = rgb { 1 } x >= @[ 2 ]"),
        vec![
            (key(assign, true), "a b"),
            (Bare, "1"),
            (key(assign, false), "color"),
            (Header, "rgb"),
            (Open { close: 6 }, "{"),
            (Bare, "1"),
            (Close { open: 4 }, "}"),
            (key(Operator::GreaterEqual, false), "x"),
            (Math, "@[ 2 ]"),
        ]
    );
}

#[test]
fn regions_balance_or_fall_back_to_text() {
    use TokenKind::*;
    let tape = parse_tape("[[P] a [[!Q] b ] ] [[R] } ]", "test").unwrap();
    let tokens = tape.tokens();
    let summary: Vec<_> = tokens.iter().map(|t| (t.kind(), tape.text(t))).collect();
    assert_eq!(
        summary,
        vec![
            (region(false, 5), "P"),
            (Bare, "a"),
            (region(true, 4), "Q"),
            (Bare, "b"),
            (RegionEnd { start: 2 }, "]"),
            (RegionEnd { start: 0 }, "]"),
            (RegionText { negated: false }, "R"),
        ]
    );
    assert_eq!(tape.region_body(&tokens[6]), " } ");
    assert_eq!(tokens[6].span(), 19..27);
    assert!(tape.diagnostics().is_empty());
}

#[test]
fn repairs_and_byte_offsets() {
    let source = "\u{feff}} x = {";
    let tape = parse_tape(source, "test").unwrap();
    let repairs: Vec<_> = tape
        .diagnostics()
        .iter()
        .map(|d| (d.kind, d.span.clone()))
        .collect();
    assert_eq!(
        repairs,
        vec![
            (Repair::StrayClosingBrace, 3..4),
            (Repair::UnclosedAtEof, 5..6)
        ]
    );
    let close = tape.tokens().last().unwrap();
    assert_eq!(close.kind(), TokenKind::Close { open: 1 });
    assert_eq!(close.span(), source.len()..source.len());
}

#[test]
fn errors_match_parse() {
    for source in ["a = ", "= 1", "a = \"open", "[[P] a = ] ! b"] {
        assert_eq!(
            parse_tape(source, "test").unwrap_err(),
            parse(source, "test").unwrap_err()
        );
    }
}

proptest! {
    #[test]
    fn links_are_reciprocal(source in "([a-z]{1,3}|[ =\n{}]|\\[\\[P\\]|\\]|\"q\"){0,40}") {
        let Ok(tape) = parse_tape(&source, "generated") else { return Ok(()) };
        for (index, token) in tape.tokens().iter().enumerate() {
            match token.kind() {
                TokenKind::Open { close } => {
                    prop_assert_eq!(tape.tokens()[close].kind(), TokenKind::Close { open: index });
                }
                TokenKind::Region { end, .. } => {
                    prop_assert_eq!(tape.tokens()[end].kind(), TokenKind::RegionEnd { start: index });
                }
                _ => {}
            }
            prop_assert!(token.span().end <= source.len());
        }
    }
}
