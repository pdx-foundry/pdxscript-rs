//! CWT syntax with raw values, attached annotations, and explicit diagnostics.
use crate::{
    MAX_NESTING_DEPTH, Span, SyntaxError,
    source::{Cursor, Scanner},
};
use serde::{Deserialize, Serialize};

/// CWT operator spelling; distinct from game-script comparisons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operator {
    /// Assignment.
    #[serde(rename = "=")]
    Assign,
    /// CWT comparison.
    #[serde(rename = "==")]
    Equal,
}

/// Uninterpreted scalar spelling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scalar {
    /// Text excluding surrounding quotes.
    pub text: String,
    /// Whether the scalar was quoted.
    pub quoted: bool,
    /// Source location including quotes when present.
    pub span: Span,
}

/// A scalar or ordered CWT block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum Value {
    /// Raw scalar text, never boolean/numeric interpretation.
    Scalar(Scalar),
    /// Braced sequence preserving duplicates and bare values.
    Block {
        /// Ordered body.
        nodes: Vec<Node>,
        /// Opening through closing brace.
        span: Span,
    },
}

impl Value {
    /// Location of this value.
    pub fn span(&self) -> Span {
        match self {
            Self::Scalar(s) => s.span,
            Self::Block { span, .. } => *span,
        }
    }
}

/// Semantic CWT comment retained with its original spelling and location.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// Number of hashes, distinguishing options from documentation.
    pub hashes: usize,
    /// Text following the hashes, excluding surrounding whitespace.
    pub text: String,
    /// Complete source span.
    pub span: Span,
}

/// A keyed rule or bare value with annotations bound to this entry only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Absent for a bare value.
    pub key: Option<Scalar>,
    /// Present exactly when the key is present.
    pub op: Option<Operator>,
    /// Raw value or nested rules.
    pub value: Value,
    /// Preceding option and documentation comments in source order.
    pub annotations: Vec<Annotation>,
    /// Key/value through the final token, excluding preceding annotations.
    pub span: Span,
}

/// Input that could not be bound or parsed; never silently discarded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// `syntax` marks malformed input; `orphan-annotation` marks unattached metadata;
    /// `nesting-limit` marks input beyond the supported nesting depth.
    pub kind: String,
    /// Source location.
    pub span: Span,
    /// Diagnostic explanation.
    pub message: String,
}

/// A partial or complete parse. Nonempty diagnostics must be surfaced by consumers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// Caller-supplied filename.
    pub file: String,
    /// Parsed nodes, excluding failed statements.
    pub nodes: Vec<Node>,
    /// Unparsed source and unattached metadata.
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone)]
enum Token {
    Scalar(Scalar),
    Op(Operator, Span),
    Open(Span),
    Close(Span),
    Annotation(Annotation),
}

impl Token {
    fn span(&self) -> Span {
        match self {
            Self::Scalar(s) => s.span,
            Self::Op(_, s) | Self::Open(s) | Self::Close(s) => *s,
            Self::Annotation(a) => a.span,
        }
    }
}

fn tokenize(text: &str, file: &str, end: Span) -> (Vec<Token>, Option<SyntaxError>) {
    let mut s = Scanner::new(text, 0, 1);
    if text.starts_with('\u{feff}') {
        s.advance(3);
    }
    let mut tokens = Vec::new();
    while let Some(ch) = s.byte() {
        if matches!(ch, b' ' | b'\t' | b'\r' | b'\n') {
            s.advance(1);
            continue;
        }
        let start = s.mark();
        let token = match ch {
            b'#' => {
                let hashes = s.rest().bytes().take_while(|&b| b == b'#').count();
                s.advance(hashes);
                let content = s.rest().split('\n').next().unwrap_or("").trim().to_owned();
                s.comment();
                if !(2..=4).contains(&hashes) {
                    continue;
                }
                Token::Annotation(Annotation {
                    hashes,
                    text: content,
                    span: s.finish(start),
                })
            }
            b'{' => {
                s.advance(1);
                Token::Open(s.finish(start))
            }
            b'}' => {
                s.advance(1);
                Token::Close(s.finish(start))
            }
            b'=' => {
                let equal = s.rest().starts_with("==");
                s.advance(if equal { 2 } else { 1 });
                Token::Op(
                    if equal {
                        Operator::Equal
                    } else {
                        Operator::Assign
                    },
                    s.finish(start),
                )
            }
            b'"' => match s.quoted(file) {
                Ok(text) => Token::Scalar(Scalar {
                    text,
                    quoted: true,
                    span: s.finish(start),
                }),
                Err(mut e) => {
                    e.span = e.span.through(end);
                    return (tokens, Some(e));
                }
            },
            _ => {
                let size = s
                    .rest()
                    .find([' ', '\t', '\r', '\n', '{', '}', '=', '#', '"'])
                    .unwrap_or(s.rest().len());
                let text = s.rest()[..size].to_owned();
                s.advance(size);
                Token::Scalar(Scalar {
                    text,
                    quoted: false,
                    span: s.finish(start),
                })
            }
        };
        tokens.push(token);
    }
    (tokens, None)
}

struct Frame {
    nodes: Vec<Node>,
    annotations: Vec<Annotation>,
    key: Option<Scalar>,
    op: Option<Operator>,
    open: Span,
    start: Span,
    attached: Vec<Annotation>,
}

impl Frame {
    fn root() -> Self {
        Self {
            nodes: Vec::new(),
            annotations: Vec::new(),
            key: None,
            op: None,
            open: Span::default(),
            start: Span::default(),
            attached: Vec::new(),
        }
    }
}

fn diagnose_orphans(annotations: &mut Vec<Annotation>, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.extend(annotations.drain(..).map(|a| Diagnostic {
        kind: "orphan-annotation".into(),
        span: a.span,
        message: a.text,
    }));
}

/// Parses CWT, retaining recoverable diagnostics and partial nodes instead of hiding unparsed input.
pub fn parse(source: &str, file: &str) -> Document {
    let line = source.bytes().filter(|&b| b == b'\n').count() + 1;
    let end = Span {
        start: source.len(),
        end: source.len(),
        line,
        end_line: line,
    };
    let (tokens, lexical_error) = tokenize(source, file, end);
    let mut diagnostics = Vec::new();
    if let Some(e) = lexical_error {
        diagnostics.push(Diagnostic {
            kind: "syntax".into(),
            span: e.span,
            message: e.message,
        });
    }
    let nodes = parse_tokens(tokens, end, &mut diagnostics);
    diagnostics.sort_by_key(|d| (d.span.start, d.kind.clone()));
    Document {
        file: file.into(),
        nodes,
        diagnostics,
    }
}

fn parse_tokens(tokens: Vec<Token>, end: Span, diagnostics: &mut Vec<Diagnostic>) -> Vec<Node> {
    let mut cursor = Cursor::new(tokens);
    let mut frames = vec![Frame::root()];
    while let Some(token) = cursor.take() {
        let frame = frames.last_mut().expect("root");
        match token {
            Token::Annotation(a) => frame.annotations.push(a),
            Token::Close(close) => {
                diagnose_orphans(&mut frame.annotations, diagnostics);
                if frames.len() == 1 {
                    diagnostics.push(Diagnostic {
                        kind: "syntax".into(),
                        span: close,
                        message: "Unmatched closing brace".into(),
                    });
                    continue;
                }
                let child = frames.pop().expect("child");
                frames.last_mut().expect("parent").nodes.push(Node {
                    key: child.key,
                    op: child.op,
                    value: Value::Block {
                        nodes: child.nodes,
                        span: child.open.through(close),
                    },
                    annotations: child.attached,
                    span: child.start.through(close),
                });
            }
            Token::Op(_, span) => diagnostics.push(Diagnostic {
                kind: "syntax".into(),
                span,
                message: "Operator has no key".into(),
            }),
            first => {
                let start = first.span();
                let Some((key, op, value)) = recognize_statement(first, &mut cursor) else {
                    diagnostics.push(Diagnostic {
                        kind: "syntax".into(),
                        span: start,
                        message: "Missing assignment value".into(),
                    });
                    diagnose_orphans(&mut frame.annotations, diagnostics);
                    continue;
                };

                let attached = std::mem::take(&mut frame.annotations);
                match value {
                    Token::Scalar(value) => frame.nodes.push(Node {
                        span: start.through(value.span),
                        key,
                        op,
                        value: Value::Scalar(value),
                        annotations: attached,
                    }),
                    Token::Open(open) => {
                        if frames.len() > MAX_NESTING_DEPTH {
                            diagnostics.push(Diagnostic {
                                kind: "nesting-limit".into(),
                                span: start.through(end),
                                message:
                                    "Nesting exceeds supported limit; remaining input unparsed"
                                        .into(),
                            });
                            break;
                        }
                        frames.push(Frame {
                            key,
                            op,
                            start,
                            open,
                            attached,
                            ..Frame::root()
                        });
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    finish_frames(frames, end, diagnostics)
}

fn recognize_statement(
    first: Token,
    cursor: &mut Cursor<Token>,
) -> Option<(Option<Scalar>, Option<Operator>, Token)> {
    let Token::Scalar(key) = &first else {
        return Some((None, None, first));
    };
    let Some(Token::Op(op, _)) = cursor.peek() else {
        return Some((None, None, first));
    };
    let op = *op;
    cursor.take();

    if !matches!(cursor.peek(), Some(Token::Scalar(_) | Token::Open(_))) {
        return None;
    }

    Some((Some(key.clone()), Some(op), cursor.take().expect("value")))
}

fn finish_frames(
    mut frames: Vec<Frame>,
    end: Span,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Node> {
    while frames.len() > 1 {
        let mut child = frames.pop().expect("child");
        diagnose_orphans(&mut child.annotations, diagnostics);
        diagnostics.push(Diagnostic {
            kind: "syntax".into(),
            span: child.start.through(end),
            message: "Unclosed block; contents remain unparsed".into(),
        });
        diagnose_orphans(&mut child.attached, diagnostics);
    }
    let mut root = frames.pop().expect("root");
    diagnose_orphans(&mut root.annotations, diagnostics);
    root.nodes
}
