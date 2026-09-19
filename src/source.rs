use serde::{Deserialize, Serialize};

/// Half-open UTF-8 byte range and one-based source lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// First byte, inclusive.
    pub start: usize,
    /// Last byte, exclusive.
    pub end: usize,
    /// Line containing the first byte.
    pub line: usize,
    /// Line containing the last token.
    pub end_line: usize,
}
impl Span {
    pub(crate) fn through(self, end: Self) -> Self {
        Self {
            end: end.end,
            end_line: end.end_line,
            ..self
        }
    }
}
/// Distinguishes malformed syntax from a resource limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    /// Text does not satisfy the selected language grammar.
    Syntax,
    /// Nesting exceeded the supported bound; never a region-text fallback.
    NestingLimit,
}
/// A source-qualified failure. Repairs are returned separately by each parser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxError {
    /// Diagnostic file label supplied by the caller.
    pub file: String,
    /// Location of the failure.
    pub span: Span,
    /// Failure category.
    pub kind: ErrorKind,
    /// Explanation for a reader.
    pub message: String,
}
impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.span.line, self.message)
    }
}
impl std::error::Error for SyntaxError {}
pub(crate) fn error(file: &str, span: Span, message: impl Into<String>) -> SyntaxError {
    SyntaxError {
        file: file.into(),
        span,
        kind: ErrorKind::Syntax,
        message: message.into(),
    }
}

pub(crate) struct Scanner<'a> {
    pub text: &'a str,
    pub offset: usize,
    pub line: usize,
    pub base: usize,
}
impl<'a> Scanner<'a> {
    pub fn new(text: &'a str, base: usize, line: usize) -> Self {
        Self {
            text,
            offset: 0,
            base,
            line,
        }
    }
    pub fn rest(&self) -> &'a str {
        &self.text[self.offset..]
    }
    pub fn byte(&self) -> Option<u8> {
        self.rest().as_bytes().first().copied()
    }
    pub fn mark(&self) -> Span {
        Span {
            start: self.base + self.offset,
            end: self.base + self.offset,
            line: self.line,
            end_line: self.line,
        }
    }
    pub fn advance(&mut self, bytes: usize) {
        self.line += self.text[self.offset..self.offset + bytes]
            .bytes()
            .filter(|&b| b == b'\n')
            .count();
        self.offset += bytes;
    }
    pub fn finish(&self, start: Span) -> Span {
        Span {
            end: self.base + self.offset,
            end_line: self.line,
            ..start
        }
    }
    pub fn comment(&mut self) {
        let bytes = self.rest().find('\n').unwrap_or(self.rest().len());
        self.advance(bytes);
    }
    pub fn quoted(&mut self, file: &str) -> Result<String, SyntaxError> {
        let start = self.mark();
        let end = quoted_end(self.text, self.offset)
            .ok_or_else(|| error(file, start, "Unterminated quoted string"))?;
        let value = self.text[self.offset + 1..end].to_owned();
        self.advance(end + 1 - self.offset);
        Ok(value)
    }
}
pub(crate) fn quoted_end(text: &str, open: usize) -> Option<usize> {
    let mut escaped = false;
    for (offset, ch) in text[open + 1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(open + 1 + offset);
        }
    }
    None
}
pub(crate) struct Cursor<T> {
    pub tokens: Vec<T>,
    pub index: usize,
}
impl<T> Cursor<T> {
    pub fn new(tokens: Vec<T>) -> Self {
        Self { tokens, index: 0 }
    }
    pub fn peek(&self) -> Option<&T> {
        self.tokens.get(self.index)
    }
    pub fn nth(&self, n: usize) -> Option<&T> {
        self.tokens.get(self.index + n)
    }
}
impl<T: Clone> Cursor<T> {
    pub fn take(&mut self) -> Option<T> {
        let token = self.peek()?.clone();
        self.index += 1;
        Some(token)
    }
}
