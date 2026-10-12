//! Owned syntax trees built from the tape.
use super::{
    tape::{Lex, Lexer, Tape, TokenKind, line_span, parse_tape},
    *,
};
use crate::{Span, SyntaxError, source::error};

/// Parses decoded game-definition source. Repairs are returned explicitly; syntax/resource errors fail.
pub fn parse(source: &str, file: &str) -> Result<Document, SyntaxError> {
    let tape = parse_tape(source, file)?;
    Ok(build(&tape, file))
}

/// Counts lines by moving from the previous offset, so cost follows the distance moved: tokens move
/// forward, and repairs move forward except for unclosed bodies, which move back once each.
struct Lines<'a> {
    bytes: &'a [u8],
    offset: usize,
    line: usize,
}
impl<'a> Lines<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            offset: 0,
            line: 1,
        }
    }
    fn at(&mut self, offset: usize) -> usize {
        let newlines = |bytes: &[u8]| bytes.iter().filter(|&&b| b == b'\n').count();
        if offset < self.offset {
            self.line -= newlines(&self.bytes[offset..self.offset]);
        } else {
            self.line += newlines(&self.bytes[self.offset..offset]);
        }
        self.offset = offset;
        self.line
    }
    fn span(&mut self, start: usize, end: usize) -> Span {
        Span {
            start,
            end,
            line: self.at(start),
            end_line: self.at(end),
        }
    }
}

enum Body {
    Root,
    Container {
        entry: Option<(String, Operator)>,
        header: Option<String>,
        span: Span,
    },
    Region {
        name: String,
        negated: bool,
        span: Span,
    },
}

fn scalar(kind: TokenKind, text: &str) -> Scalar {
    match kind {
        TokenKind::Quoted => Scalar::String {
            value: text.into(),
            quoted: true,
        },
        TokenKind::Math => Scalar::Math {
            source: text.into(),
        },
        _ => classify_unquoted(text),
    }
}

/// Builds the owned tree: owns text, classifies scalars, and attaches line numbers.
fn build(tape: &Tape<'_>, file: &str) -> Document {
    let mut lines = Lines::new(tape.source());
    let mut stack = vec![(Body::Root, Vec::new())];
    let mut key: Option<(String, Operator, Span)> = None;
    let mut header = None;
    for token in tape.tokens() {
        let range = token.span();
        let span = lines.span(range.start, range.end);
        let text = tape.text(token);
        match token.kind() {
            TokenKind::Key { op, .. } => key = Some((text.into(), op, span)),
            TokenKind::Header => header = Some(text.into()),
            kind @ (TokenKind::Bare | TokenKind::Quoted | TokenKind::Math) => {
                let value = scalar(kind, text);
                let item = match key.take() {
                    Some((key, op, key_span)) => Item {
                        span: Some(key_span.through(span)),
                        kind: ItemKind::Entry(Entry {
                            key,
                            op,
                            value: Value::Scalar(value),
                        }),
                    },
                    None => Item {
                        span: Some(span),
                        kind: ItemKind::Scalar(value),
                    },
                };
                stack.last_mut().expect("body").1.push(item);
            }
            TokenKind::Open { .. } => {
                let (entry, span) = match key.take() {
                    Some((key, op, key_span)) => (Some((key, op)), key_span),
                    None => (None, span),
                };
                let body = Body::Container {
                    entry,
                    header: header.take(),
                    span,
                };
                stack.push((body, Vec::new()));
            }
            TokenKind::Close { .. } => {
                let (
                    Body::Container {
                        entry,
                        header,
                        span: open,
                    },
                    items,
                ) = stack.pop().expect("container")
                else {
                    unreachable!("close matches a container")
                };
                let body = Container { header, items };
                let kind = match entry {
                    Some((key, op)) => ItemKind::Entry(Entry {
                        key,
                        op,
                        value: Value::Container(body),
                    }),
                    None => ItemKind::Container(body),
                };
                stack.last_mut().expect("parent").1.push(Item {
                    kind,
                    span: Some(open.through(span)),
                });
            }
            TokenKind::Region { negated, .. } => {
                let body = Body::Region {
                    name: text.into(),
                    negated,
                    span,
                };
                stack.push((body, Vec::new()));
            }
            TokenKind::RegionEnd { .. } => {
                let (
                    Body::Region {
                        name,
                        negated,
                        span: open,
                    },
                    items,
                ) = stack.pop().expect("region")
                else {
                    unreachable!("region end matches a region")
                };
                stack.last_mut().expect("parent").1.push(Item {
                    span: Some(open.through(span)),
                    kind: ItemKind::Param {
                        name,
                        negated,
                        items,
                    },
                });
            }
            TokenKind::RegionText { negated } => stack.last_mut().expect("body").1.push(Item {
                span: Some(span),
                kind: ItemKind::ParamText {
                    name: text.into(),
                    negated,
                    text: tape.region_body(token).into(),
                },
            }),
        }
    }
    let (_, items) = stack.pop().expect("root");
    let diagnostics = tape
        .diagnostics()
        .iter()
        .map(|diagnostic| {
            let span = lines.span(diagnostic.span.start, diagnostic.span.end);
            let text = match diagnostic.kind {
                Repair::StrayClosingBrace => "}".into(),
                Repair::UnclosedAtEof => "{".into(),
                Repair::OperatorLessEntry => {
                    let key = &tape.source()[diagnostic.span.clone()];
                    key.strip_prefix('"')
                        .and_then(|k| k.strip_suffix('"'))
                        .unwrap_or(key)
                        .into()
                }
            };
            Diagnostic {
                kind: diagnostic.kind,
                span,
                text,
            }
        })
        .collect();
    Document {
        file: file.into(),
        items,
        diagnostics,
    }
}

/// Reads words, inline math, and nested regions from a verbatim conditional body in source order.
/// Braces and operators are omitted; brace relationships are not inferred.
/// Returns an error if tokenization fails.
pub fn region_items(text: &str, file: &str) -> Result<Vec<Item>, SyntaxError> {
    let mut lexer = Lexer::new(text, 0);
    let mut lines = Lines::new(text);
    let mut items = Vec::new();
    loop {
        let lexeme = match lexer.next() {
            Ok(Some(lexeme)) => lexeme,
            Ok(None) => return Ok(items),
            Err(e) => return Err(error(file, line_span(text, e.at..e.at), e.message)),
        };
        let source = &text[lexeme.start..lexeme.end];
        let kind = match lexeme.kind {
            Lex::Word => ItemKind::Scalar(classify_unquoted(source)),
            Lex::Quoted => ItemKind::Scalar(Scalar::String {
                value: source[1..source.len() - 1].into(),
                quoted: true,
            }),
            Lex::Math => ItemKind::Scalar(Scalar::Math {
                source: source.into(),
            }),
            Lex::Region { negated, body } => ItemKind::ParamText {
                name: text[lexeme.start + 2 + usize::from(negated)..body - 1].into(),
                negated,
                text: text[body..lexeme.end - 1].into(),
            },
            _ => continue,
        };
        items.push(Item {
            span: Some(lines.span(lexeme.start, lexeme.end)),
            kind,
        });
    }
}
