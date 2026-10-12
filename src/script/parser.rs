//! Owned syntax trees built from the tape.
use super::{
    tape::{Lex, Lexer, Tape, Token, TokenKind, line_span, newlines, parse_tape},
    *,
};
use crate::{Span, SyntaxError, source::error};
use std::ops::Range;

/// Parses decoded game-definition source. Repairs are returned explicitly; syntax/resource errors fail.
pub fn parse(source: &str, file: &str) -> Result<Document, SyntaxError> {
    let tape = parse_tape(source, file)?;
    Ok(build(&tape, file))
}

/// Counts lines for repair spans by moving from the previous offset, so cost follows the distance
/// moved: repairs move forward except for unclosed bodies, which move back once each.
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

/// A span from a start line and the byte range. Only quoted strings, inline math, and regions can
/// contain a newline, so only they count their own bytes for the end line.
fn span_from(source: &str, range: Range<usize>, line: usize, multiline: bool) -> Span {
    let end_line = if multiline {
        line + newlines(&source.as_bytes()[range.clone()])
    } else {
        line
    };
    Span {
        start: range.start,
        end: range.end,
        line,
        end_line,
    }
}
fn token_span(source: &str, token: &Token) -> Span {
    let multiline = matches!(
        token.kind(),
        TokenKind::Quoted
            | TokenKind::Math
            | TokenKind::RegionText { .. }
            | TokenKind::Key { quoted: true, .. }
    );
    span_from(source, token.span(), token.line(), multiline)
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

/// Number of items in each body, in the order bodies open on the tape (the document first).
fn body_sizes(tape: &Tape<'_>) -> Vec<usize> {
    let mut sizes = vec![0];
    let mut open = vec![0];
    let mut keyed = false;
    for token in tape.tokens() {
        let starts_item = !std::mem::replace(&mut keyed, false);
        match token.kind() {
            TokenKind::Key { .. } => {
                sizes[*open.last().expect("body")] += 1;
                keyed = true;
            }
            TokenKind::Header => keyed = true,
            TokenKind::Open { .. } | TokenKind::Region { .. } => {
                if starts_item {
                    sizes[*open.last().expect("body")] += 1;
                }
                open.push(sizes.len());
                sizes.push(0);
            }
            TokenKind::Close { .. } | TokenKind::RegionEnd { .. } => {
                open.pop();
            }
            TokenKind::Bare
            | TokenKind::Quoted
            | TokenKind::Math
            | TokenKind::RegionText { .. } => {
                if starts_item {
                    sizes[*open.last().expect("body")] += 1;
                }
            }
        }
    }
    sizes
}

/// Builds the owned tree: owns text, classifies scalars, and attaches line numbers.
fn build(tape: &Tape<'_>, file: &str) -> Document {
    // Each body's vector is allocated once at its final size, counted in a cheap first pass.
    let sizes = body_sizes(tape);
    let mut next_body = 1;
    let mut stack = vec![(Body::Root, Vec::with_capacity(sizes[0]))];
    let mut key: Option<(String, Operator, Span)> = None;
    let mut header = None;
    for token in tape.tokens() {
        let span = token_span(tape.source(), token);
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
                stack.push((body, Vec::with_capacity(sizes[next_body])));
                next_body += 1;
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
                stack.push((body, Vec::with_capacity(sizes[next_body])));
                next_body += 1;
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
    let mut lines = Lines::new(tape.source());
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
    let mut lexer = Lexer::new(text, 0, 1);
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
            span: Some(span_from(
                text,
                lexeme.start..lexeme.end,
                lexeme.line,
                matches!(lexeme.kind, Lex::Quoted | Lex::Math | Lex::Region { .. }),
            )),
            kind,
        });
    }
}
