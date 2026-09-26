use super::{
    lexer::{Kind, Token, tokenize},
    *,
};
use crate::{
    ErrorKind, MAX_NESTING_DEPTH, Span, SyntaxError,
    source::{Cursor, error},
};

#[derive(Clone)]
enum Attach {
    Root,
    Container {
        entry: Option<(String, Operator)>,
        header: Option<String>,
        span: Span,
    },
    Region(Token),
}
struct Frame {
    cursor: Cursor<Token>,
    items: Vec<Item>,
    attach: Attach,
    diagnostics: Vec<Diagnostic>,
    depth: usize,
    end: Span,
}
impl Frame {
    fn new(tokens: Vec<Token>, attach: Attach, depth: usize, end: Span) -> Self {
        Self {
            cursor: Cursor::new(tokens),
            items: Vec::new(),
            attach,
            diagnostics: Vec::new(),
            depth,
            end,
        }
    }
}
fn limit(file: &str, span: Span) -> SyntaxError {
    SyntaxError {
        kind: ErrorKind::NestingLimit,
        ..error(file, span, "Nesting exceeds supported limit")
    }
}
fn as_scalar(token: &Token) -> Scalar {
    match token.kind {
        Kind::Math => Scalar::Math {
            source: token.text.clone(),
        },
        _ if token.quoted => Scalar::String {
            value: token.text.clone(),
            quoted: true,
        },
        _ => classify_unquoted(&token.text),
    }
}
/// Parses decoded game-definition source. Repairs are returned explicitly; syntax/resource errors fail.
pub fn parse(source: &str, file: &str) -> Result<Document, SyntaxError> {
    let (text, base) = source
        .strip_prefix('\u{feff}')
        .map_or((source, 0), |s| (s, 3));
    let end = Span {
        start: source.len(),
        end: source.len(),
        line: source.bytes().filter(|&b| b == b'\n').count() + 1,
        end_line: source.bytes().filter(|&b| b == b'\n').count() + 1,
    };
    let tokens = tokenize(text, file, base, 1)?;
    parse_tokens(tokens, file, end)
}
fn parse_tokens(tokens: Vec<Token>, file: &str, end: Span) -> Result<Document, SyntaxError> {
    let mut frames = vec![Frame::new(tokens, Attach::Root, 0, end)];
    loop {
        let step = advance_frame(&mut frames, file);
        if let Err(e) = step {
            if e.kind == ErrorKind::NestingLimit {
                return Err(e);
            }
            let Some(index) = frames
                .iter()
                .rposition(|f| matches!(f.attach, Attach::Region(_)))
            else {
                return Err(e);
            };
            let Attach::Region(token) = frames[index].attach.clone() else {
                unreachable!()
            };
            frames.truncate(index);
            frames
                .last_mut()
                .expect("region parent")
                .items
                .push(region_text(token));
            continue;
        }
        if frames.last().expect("root frame").cursor.peek().is_some()
            || matches!(
                frames.last().expect("frame").attach,
                Attach::Container { .. }
            )
        {
            continue;
        }
        let complete = frames.pop().expect("completed frame");
        match complete.attach {
            Attach::Root => {
                return Ok(Document {
                    file: file.into(),
                    items: complete.items,
                    diagnostics: complete.diagnostics,
                });
            }
            Attach::Container { .. } => unreachable!("containers completed by advance_frame"),
            Attach::Region(token) => {
                complete_region(
                    frames.last_mut().expect("region parent"),
                    token,
                    complete.items,
                    complete.diagnostics,
                );
            }
        }
    }
}
fn complete_region(
    parent: &mut Frame,
    token: Token,
    items: Vec<Item>,
    mut diagnostics: Vec<Diagnostic>,
) {
    let needs_verbatim_body = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.kind != Repair::OperatorLessEntry);
    if needs_verbatim_body {
        parent.items.push(region_text(token));
        return;
    }

    let (name, negated) = region_name(&token);
    parent.items.push(Item {
        span: Some(token.span),
        kind: ItemKind::Param {
            name,
            negated,
            items,
        },
    });
    parent.diagnostics.append(&mut diagnostics);
}

fn region_name(token: &Token) -> (String, bool) {
    (
        token.text.strip_prefix('!').unwrap_or(&token.text).into(),
        token.text.starts_with('!'),
    )
}
fn region_text(token: Token) -> Item {
    let (name, negated) = region_name(&token);
    Item {
        span: Some(token.span),
        kind: ItemKind::ParamText {
            name,
            negated,
            text: token.body,
        },
    }
}
fn complete_container(frames: &mut Vec<Frame>, close: Option<Token>) {
    let mut child = frames.pop().expect("container");
    let Attach::Container {
        entry,
        header,
        span,
    } = child.attach
    else {
        unreachable!()
    };
    let unclosed_at_eof = close.is_none();
    let end = close.map_or(child.end, |t| t.span);
    if unclosed_at_eof {
        child.diagnostics.push(Diagnostic {
            kind: Repair::UnclosedAtEof,
            span,
            text: "{".into(),
        });
    }
    let body = Container {
        header,
        items: child.items,
    };
    let kind = match entry {
        Some((key, op)) => ItemKind::Entry(Entry {
            key,
            op,
            value: Value::Container(body),
        }),
        None => ItemKind::Container(body),
    };
    let parent = frames.last_mut().expect("container parent");
    parent.items.push(Item {
        kind,
        span: Some(span.through(end)),
    });
    parent.diagnostics.append(&mut child.diagnostics);
    // Container frames share their parent's token stream by transferring ownership.
    parent.cursor = child.cursor;
}
fn push_container(frames: &mut Vec<Frame>, file: &str, attach: Attach) -> Result<(), SyntaxError> {
    let parent = frames.last_mut().expect("parent");
    let depth = parent.depth + 1;
    if depth > MAX_NESTING_DEPTH {
        return Err(limit(
            file,
            parent.cursor.nth(0).map_or(parent.end, |t| t.span),
        ));
    }
    let cursor = std::mem::replace(&mut parent.cursor, Cursor::new(Vec::new()));
    let end = parent.end;
    frames.push(Frame {
        cursor,
        items: Vec::new(),
        attach,
        diagnostics: Vec::new(),
        depth,
        end,
    });
    Ok(())
}
fn advance_frame(frames: &mut Vec<Frame>, file: &str) -> Result<(), SyntaxError> {
    let frame = frames.last_mut().expect("frame");
    let Some(first) = frame.cursor.take() else {
        if matches!(frame.attach, Attach::Container { .. }) {
            complete_container(frames, None);
        }
        return Ok(());
    };
    match first.kind {
        Kind::Close if matches!(frame.attach, Attach::Container { .. }) => {
            complete_container(frames, Some(first))
        }
        Kind::Close => frame.diagnostics.push(Diagnostic {
            kind: Repair::StrayClosingBrace,
            span: first.span,
            text: first.text,
        }),
        Kind::Open => push_container(
            frames,
            file,
            Attach::Container {
                entry: None,
                header: None,
                span: first.span,
            },
        )?,
        Kind::Bracket | Kind::Op => return Err(error(file, first.span, "Expected a key or value")),
        Kind::Region => {
            let depth = frame.depth + 1;
            if depth > MAX_NESTING_DEPTH {
                return Err(limit(file, first.span));
            }
            // Tokenization errors inside a region are syntax fallbacks too.
            match tokenize(&first.body, file, first.body_start, first.span.line) {
                Ok(tokens) => {
                    let end = first.span;
                    frames.push(Frame::new(tokens, Attach::Region(first), depth, end));
                }
                Err(_) => frame.items.push(region_text(first)),
            }
        }
        Kind::Word if frame.cursor.peek().is_some_and(|t| t.kind == Kind::Op) => {
            let op_token = frame.cursor.take().expect("operator");
            let op = Operator::parse(&op_token.text)
                .ok_or_else(|| error(file, op_token.span, "Unsupported operator"))?;
            let value = frame
                .cursor
                .take()
                .ok_or_else(|| error(file, op_token.span, "Missing value"))?;
            let header = value.kind == Kind::Word
                && !value.quoted
                && frame
                    .cursor
                    .peek()
                    .is_some_and(|t| t.kind == Kind::Open && t.span.line == value.span.line);
            if value.kind == Kind::Open || header {
                let header = if header {
                    frame.cursor.take();
                    Some(value.text)
                } else {
                    None
                };
                push_container(
                    frames,
                    file,
                    Attach::Container {
                        entry: Some((first.text, op)),
                        header,
                        span: first.span,
                    },
                )?;
            } else if matches!(value.kind, Kind::Word | Kind::Math) {
                frame.items.push(Item {
                    span: Some(first.span.through(value.span)),
                    kind: ItemKind::Entry(Entry {
                        key: first.text,
                        op,
                        value: Value::Scalar(as_scalar(&value)),
                    }),
                });
            } else {
                return Err(error(file, value.span, "Expected a value"));
            }
        }
        Kind::Word => {
            let same_line_block = frame
                .cursor
                .peek()
                .is_some_and(|t| t.kind == Kind::Open && t.span.line == first.span.line);
            let entry_body = frame.cursor.nth(1).is_some_and(|t| t.kind == Kind::Word)
                && frame.cursor.nth(2).is_some_and(|t| t.kind == Kind::Op);
            if same_line_block && (matches!(frame.attach, Attach::Root) || entry_body) {
                frame.cursor.take();
                frame.diagnostics.push(Diagnostic {
                    kind: Repair::OperatorLessEntry,
                    span: first.span,
                    text: first.text.clone(),
                });
                push_container(
                    frames,
                    file,
                    Attach::Container {
                        entry: Some((first.text, Operator::Assign)),
                        header: None,
                        span: first.span,
                    },
                )?;
            } else {
                frame.items.push(Item {
                    span: Some(first.span),
                    kind: ItemKind::Scalar(as_scalar(&first)),
                });
            }
        }
        Kind::Math => frame.items.push(Item {
            span: Some(first.span),
            kind: ItemKind::Scalar(as_scalar(&first)),
        }),
    }
    Ok(())
}
/// Reads words, inline math, and nested regions from a verbatim conditional body in source order.
/// Braces and operators are omitted; brace relationships are not inferred.
/// Returns an error if tokenization fails.
pub fn region_items(text: &str, file: &str) -> Result<Vec<Item>, SyntaxError> {
    Ok(tokenize(text, file, 0, 1)?
        .into_iter()
        .filter_map(|token| match token.kind {
            Kind::Word | Kind::Math => Some(Item {
                span: Some(token.span),
                kind: ItemKind::Scalar(as_scalar(&token)),
            }),
            Kind::Region => Some(region_text(token)),
            _ => None,
        })
        .collect())
}
