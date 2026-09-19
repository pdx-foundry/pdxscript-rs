use super::{is_param_name, representable::terminates};
use crate::{
    Span, SyntaxError,
    source::{Scanner, error, quoted_end},
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Word,
    Op,
    Open,
    Close,
    Math,
    Region,
    Bracket,
}
#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub kind: Kind,
    pub text: String,
    pub quoted: bool,
    pub span: Span,
    pub body: String,
    pub body_start: usize,
}
fn math_open(text: &str) -> bool {
    text.starts_with("@[") || text.starts_with("@\\[")
}
fn region_end(text: &str, start: usize) -> Option<usize> {
    let mut index = start;
    let mut depth = 1;
    while index < text.len() {
        let rest = &text[index..];
        if rest.starts_with('#') {
            index += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with('"') {
            index = quoted_end(text, index)? + 1;
        } else if math_open(rest) {
            index += rest.find(']')? + 1;
        } else if rest.starts_with("[[") {
            depth += 1;
            index += rest.find(']')? + 1;
        } else if rest.starts_with(']') {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
            index += 1;
        } else {
            index += rest.chars().next()?.len_utf8();
        }
    }
    None
}
pub(crate) fn tokenize(
    text: &str,
    file: &str,
    base: usize,
    line: usize,
) -> Result<Vec<Token>, SyntaxError> {
    let mut s = Scanner::new(text, base, line);
    let mut tokens = Vec::new();
    while let Some(ch) = s.byte() {
        if matches!(ch, b' ' | b'\t' | b'\r' | b'\n' | 11 | 12 | b';') {
            s.advance(1);
            continue;
        }
        if ch == b'#' {
            s.comment();
            continue;
        }
        let start = s.mark();
        let mut token = Token {
            kind: Kind::Word,
            text: String::new(),
            quoted: false,
            span: start,
            body: String::new(),
            body_start: 0,
        };
        match ch {
            b'{' | b'}' | b']' => {
                token.kind = match ch {
                    b'{' => Kind::Open,
                    b'}' => Kind::Close,
                    _ => Kind::Bracket,
                };
                token.text.push(ch as char);
                s.advance(1);
            }
            b'"' => {
                token.text = s.quoted(file)?;
                token.quoted = true;
            }
            b'@' if math_open(s.rest()) => {
                let size = s
                    .rest()
                    .find(']')
                    .ok_or_else(|| error(file, start, "Unterminated inline math"))?
                    + 1;
                token.kind = Kind::Math;
                token.text = s.rest()[..size].into();
                s.advance(size);
            }
            b'[' if s.rest().starts_with("[[") => {
                let opener = s
                    .rest()
                    .find(']')
                    .ok_or_else(|| error(file, start, "Unterminated parameter opener"))?;
                let name = &s.rest()[2..opener];
                if !is_param_name(name.strip_prefix('!').unwrap_or(name)) {
                    return Err(error(file, start, "Invalid parameter name"));
                }
                let end = region_end(text, s.offset + opener + 1)
                    .ok_or_else(|| error(file, start, "Unterminated parameter region"))?;
                token.kind = Kind::Region;
                token.text = name.into();
                token.body_start = s.base + s.offset + opener + 1;
                token.body = text[s.offset + opener + 1..end].into();
                s.advance(end + 1 - s.offset);
            }
            b'=' | b'<' | b'>' | b'!' => {
                if ch == b'!' && !s.rest().starts_with("!=") {
                    return Err(error(file, start, "Unexpected '!'"));
                }
                let size = if ch != b'=' && s.rest().as_bytes().get(1) == Some(&b'=') {
                    2
                } else {
                    1
                };
                token.kind = Kind::Op;
                token.text = s.rest()[..size].into();
                s.advance(size);
            }
            _ => {
                let size = s.rest().find(terminates).unwrap_or(s.rest().len());
                if size == 0 {
                    return Err(error(file, start, "Unexpected delimiter"));
                }
                if s.rest()[..size].ends_with('?') && s.rest().as_bytes().get(size) == Some(&b'=') {
                    return Err(error(file, start, "Unsupported operator '?='"));
                }
                token.text = s.rest()[..size].into();
                s.advance(size);
            }
        }
        token.span = s.finish(start);
        tokens.push(token);
    }
    Ok(tokens)
}
