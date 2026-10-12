//! Zero-copy structural parsing into a flat token tape.
//!
//! The tape is the parser module's whole contract: every grammar decision, repair, region fallback,
//! and error is made here. It owns no strings, classifies no scalars, and counts no lines; callers
//! that need those derive them from token spans (see `parse`, which builds a `Document`).
use super::{Operator, Repair, is_param_name, representable::terminates};
use crate::{
    ErrorKind, MAX_NESTING_DEPTH, Span, SyntaxError,
    source::{error, quoted_end},
};
use std::ops::Range;

/// One structural event on a [`Tape`], in source order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// Key of an entry. The value follows: a scalar, or a `Header` and/or `Open`.
    Key {
        /// Assignment or comparison between key and value.
        op: Operator,
        /// Whether the key was written in quotes.
        quoted: bool,
    },
    /// Unquoted header such as `rgb` in `color = rgb { ... }`; an `Open` follows.
    Header,
    /// Start of a braced body.
    Open {
        /// Tape index of the matching `Close`.
        close: usize,
    },
    /// End of a braced body. An empty span at the end of input closes an unclosed body; the
    /// tape then also carries an `UnclosedAtEof` repair.
    Close {
        /// Tape index of the matching `Open`.
        open: usize,
    },
    /// Unquoted scalar, not yet classified as a number, boolean, variable, or string.
    Bare,
    /// Quoted scalar; escapes are not decoded.
    Quoted,
    /// Inline math such as `@[ 1 + 2 ]`, including delimiters.
    Math,
    /// Opener `[[name]` of a balanced conditional region whose body follows.
    Region {
        /// Tests absence when true.
        negated: bool,
        /// Tape index of the matching `RegionEnd`.
        end: usize,
    },
    /// Closing `]` of a balanced conditional region.
    RegionEnd {
        /// Tape index of the matching `Region`.
        start: usize,
    },
    /// Whole conditional region `[[name] body ]` whose body is kept verbatim.
    RegionText {
        /// Tests absence when true.
        negated: bool,
    },
}

/// A tape event and the source bytes it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
    line: usize,
}
impl Token {
    /// Event kind.
    #[inline]
    pub fn kind(&self) -> TokenKind {
        self.kind
    }
    /// Half-open UTF-8 byte range in the original source, including any leading BOM.
    #[inline]
    pub fn span(&self) -> Range<usize> {
        self.start..self.end
    }
    /// One-based line of the token's first byte.
    #[inline]
    pub fn line(&self) -> usize {
        self.line
    }
}

/// A repair applied while building the tape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapeDiagnostic {
    /// Repair category.
    pub kind: Repair,
    /// Source bytes that caused the repair: the stray `}`, the key or `{` of an unclosed body,
    /// or the key that lacked an operator.
    pub span: Range<usize>,
}

/// Borrowed parse of one game-definition source.
#[derive(Clone, Debug)]
pub struct Tape<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    diagnostics: Vec<TapeDiagnostic>,
}
impl<'a> Tape<'a> {
    /// Source the spans index into.
    #[inline]
    pub fn source(&self) -> &'a str {
        self.source
    }
    /// Events in source order.
    #[inline]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }
    /// Repairs in the order they were applied.
    #[inline]
    pub fn diagnostics(&self) -> &[TapeDiagnostic] {
        &self.diagnostics
    }
    /// Text of a key, header, or scalar without quotes; for regions, the parameter name without `!`.
    /// Other tokens return their source text.
    #[inline]
    pub fn text(&self, token: &Token) -> &'a str {
        let (start, end) = (token.start, token.end);
        match token.kind {
            TokenKind::Key { quoted: true, .. } | TokenKind::Quoted => {
                &self.source[start + 1..end - 1]
            }
            TokenKind::Region { negated, .. } | TokenKind::RegionText { negated } => {
                let name = &self.source[start + 2..self.name_end(token)];
                if negated { &name[1..] } else { name }
            }
            _ => &self.source[start..end],
        }
    }
    /// Verbatim body of a `RegionText`, excluding the opener and closing `]`.
    pub fn region_body(&self, token: &Token) -> &'a str {
        &self.source[self.name_end(token) + 1..token.end - 1]
    }
    fn name_end(&self, token: &Token) -> usize {
        token.start
            + self.source.as_bytes()[token.start..]
                .iter()
                .position(|&b| b == b']')
                .expect("region opener")
    }
}

/// Parses decoded game-definition source into a tape. Repairs are recorded; syntax and nesting
/// errors fail with the same error `parse` reports.
pub fn parse_tape<'a>(source: &'a str, file: &str) -> Result<Tape<'a>, SyntaxError> {
    let start = if source.starts_with('\u{feff}') { 3 } else { 0 };
    let mut parser = Parser {
        source,
        file,
        tokens: Vec::with_capacity(source.len() / 6),
        diagnostics: Vec::new(),
        levels: vec![Level::new(Lexer::new(source, start, 1), None)],
        containers: Vec::new(),
    };
    parser.run()?;
    Ok(Tape {
        source,
        tokens: parser.tokens,
        diagnostics: parser.diagnostics,
    })
}

const SPACE: u8 = 1;
const TERMINATOR: u8 = 2;
static CLASS: [u8; 256] = {
    let mut table = [0; 256];
    let mut byte = 0;
    while byte < 256 {
        if terminates(byte as u8 as char) {
            table[byte] |= TERMINATOR;
        }
        if matches!(byte as u8, b' ' | b'\t' | b'\r' | b'\n' | 11 | 12 | b';') {
            table[byte] |= SPACE;
        }
        byte += 1;
    }
    table
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lex {
    Word,
    Quoted,
    Op(Operator),
    Open,
    Close,
    Math,
    Bracket,
    Region { negated: bool, body: usize },
}
/// A lexical token and its byte range.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lexeme {
    pub kind: Lex,
    pub start: usize,
    pub end: usize,
    /// One-based line of `start`.
    pub line: usize,
}
const EMPTY: Lexeme = Lexeme {
    kind: Lex::Bracket,
    start: 0,
    end: 0,
    line: 0,
};
/// A lexical failure: message at a token start.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LexError {
    pub at: usize,
    pub message: &'static str,
}

/// Byte lexer bounded to one level: the whole document, or one region body. It counts lines as it
/// skips whitespace, which is cheaper than any later pass over the same bytes.
#[derive(Clone, Copy)]
pub(crate) struct Lexer<'a> {
    text: &'a str,
    pos: usize,
    line: usize,
}
impl<'a> Lexer<'a> {
    /// A lexer at `pos`, which is on one-based `line`.
    pub fn new(text: &'a str, pos: usize, line: usize) -> Self {
        Self { text, pos, line }
    }
    #[inline(always)]
    pub fn next(&mut self) -> Result<Option<Lexeme>, LexError> {
        let bytes = self.text.as_bytes();
        let mut pos = self.pos;
        let mut line = self.line;
        let first = loop {
            let Some(&byte) = bytes.get(pos) else {
                self.pos = pos;
                self.line = line;
                return Ok(None);
            };
            if CLASS[byte as usize] & SPACE != 0 {
                line += usize::from(byte == b'\n');
                pos += 1;
            } else if byte == b'#' {
                pos = bytes[pos..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(bytes.len(), |n| pos + n);
            } else {
                break byte;
            }
        };
        let start = pos;
        let fail = |message| LexError { at: start, message };
        let rest = &bytes[start..];
        let (kind, end) = match first {
            b'{' => (Lex::Open, start + 1),
            b'}' => (Lex::Close, start + 1),
            b']' => (Lex::Bracket, start + 1),
            b'"' => {
                let close = quoted_end(bytes, start).ok_or(fail("Unterminated quoted string"))?;
                (Lex::Quoted, close + 1)
            }
            b'@' if rest.starts_with(b"@[") || rest.starts_with(b"@\\[") => {
                let size = find(rest, b']').ok_or(fail("Unterminated inline math"))? + 1;
                (Lex::Math, start + size)
            }
            b'[' if rest.starts_with(b"[[") => {
                let opener = find(rest, b']').ok_or(fail("Unterminated parameter opener"))?;
                let name = &self.text[start + 2..start + opener];
                if !is_param_name(name.strip_prefix('!').unwrap_or(name)) {
                    return Err(fail("Invalid parameter name"));
                }
                let body = start + opener + 1;
                let close =
                    region_end(self.text, body).ok_or(fail("Unterminated parameter region"))?;
                let negated = name.starts_with('!');
                (Lex::Region { negated, body }, close + 1)
            }
            b'=' | b'<' | b'>' | b'!' => {
                let equals = rest.get(1) == Some(&b'=');
                let (op, size) = match (first, equals) {
                    (b'!', false) => return Err(fail("Unexpected '!'")),
                    (b'!', true) => (Operator::NotEqual, 2),
                    (b'<', true) => (Operator::LessEqual, 2),
                    (b'>', true) => (Operator::GreaterEqual, 2),
                    (b'<', false) => (Operator::Less, 1),
                    (b'>', false) => (Operator::Greater, 1),
                    _ => (Operator::Assign, 1),
                };
                (Lex::Op(op), start + size)
            }
            _ => {
                let size = word_len(rest);
                if size == 0 {
                    return Err(fail("Unexpected delimiter"));
                }
                if rest[size - 1] == b'?' && rest.get(size) == Some(&b'=') {
                    return Err(fail("Unsupported operator '?='"));
                }
                (Lex::Word, start + size)
            }
        };
        self.pos = end;
        self.line = match kind {
            Lex::Quoted | Lex::Math | Lex::Region { .. } => line + newlines(&bytes[start..end]),
            _ => line,
        };
        Ok(Some(Lexeme {
            kind,
            start,
            end,
            line,
        }))
    }
}
pub(crate) fn newlines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b == b'\n').count()
}
/// Length of the word at the start of `bytes`: the index of the first terminator.
#[inline]
fn word_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .position(|&b| CLASS[b as usize] & TERMINATOR != 0)
        .unwrap_or(bytes.len())
}
fn key_kind(key: Lexeme, op: Operator) -> TokenKind {
    TokenKind::Key {
        op,
        quoted: key.kind == Lex::Quoted,
    }
}
fn find(bytes: &[u8], needle: u8) -> Option<usize> {
    bytes.iter().position(|&b| b == needle)
}
fn math_open(text: &[u8]) -> bool {
    text.starts_with(b"@[") || text.starts_with(b"@\\[")
}
/// Index of the `]` closing a region body starting at `start`, skipping comments, strings, math,
/// and nested region openers.
fn region_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = start;
    let mut depth = 1;
    while index < bytes.len() {
        let rest = &bytes[index..];
        if rest[0] == b'#' {
            index += find(rest, b'\n').unwrap_or(rest.len());
        } else if rest[0] == b'"' {
            index = quoted_end(bytes, index)? + 1;
        } else if math_open(rest) {
            index += find(rest, b']')? + 1;
        } else if rest.starts_with(b"[[") {
            depth += 1;
            index += find(rest, b']')? + 1;
        } else if rest[0] == b']' {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
            index += 1;
        } else {
            index += 1;
        }
    }
    None
}

/// The document or one region body. The old parser lexed a whole level before parsing it; levels
/// keep that boundary so lexical errors keep their precedence (see `Parser::recover`).
struct Level<'a> {
    lexer: Lexer<'a>,
    /// Tokens already lexed that must be read again, in order, before lexing resumes.
    replay: [Lexeme; 4],
    replay_start: usize,
    replay_len: usize,
    /// Whether the unlexed remainder is known to lex cleanly, so error recovery scans it once.
    clean_remainder: bool,
    region: Option<RegionMark>,
}
impl<'a> Level<'a> {
    fn new(lexer: Lexer<'a>, region: Option<RegionMark>) -> Self {
        Self {
            lexer,
            replay: [EMPTY; 4],
            replay_start: 0,
            replay_len: 0,
            clean_remainder: false,
            region,
        }
    }
    /// Returns `first` and then `rest` to the front of the stream.
    fn unread(&mut self, first: Lexeme, rest: &[Lexeme]) {
        self.unread_all(rest);
        self.unread_all(&[first]);
    }
    /// Returns tokens, in order, to the front of the stream; they were read before any pending ones.
    fn unread_all(&mut self, tokens: &[Lexeme]) {
        let pending = self.replay_len;
        let mut queue = [EMPTY; 4];
        queue[..tokens.len()].copy_from_slice(tokens);
        queue[tokens.len()..tokens.len() + pending]
            .copy_from_slice(&self.replay[self.replay_start..self.replay_start + pending]);
        self.replay = queue;
        self.replay_start = 0;
        self.replay_len = tokens.len() + pending;
    }
    /// The token after the current one, without consuming it.
    fn following(&self) -> Result<Option<Lexeme>, LexError> {
        if self.replay_len > 0 {
            return Ok(Some(self.replay[self.replay_start]));
        }
        {
            let mut lexer = self.lexer;
            lexer.next()
        }
    }
    /// First lexical error in the not-yet-lexed remainder of this level.
    fn remaining_error(&mut self) -> Option<LexError> {
        if self.clean_remainder {
            return None;
        }
        let mut lexer = self.lexer;
        loop {
            match lexer.next() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(e) => return Some(e),
            }
        }
        self.clean_remainder = true;
        None
    }
}
struct RegionMark {
    lexeme: Lexeme,
    tape: usize,
    diagnostics: usize,
    containers: usize,
}
struct OpenContainer {
    tape: usize,
    span: Range<usize>,
}

enum Failure {
    Lex(LexError),
    Syntax(Range<usize>, &'static str),
    Nesting(Range<usize>),
}
impl From<LexError> for Failure {
    fn from(e: LexError) -> Self {
        Self::Lex(e)
    }
}

struct Parser<'a, 'f> {
    source: &'a str,
    file: &'f str,
    tokens: Vec<Token>,
    diagnostics: Vec<TapeDiagnostic>,
    levels: Vec<Level<'a>>,
    containers: Vec<OpenContainer>,
}
impl<'a> Parser<'a, '_> {
    fn run(&mut self) -> Result<(), SyntaxError> {
        loop {
            match self.drive() {
                Ok(()) => return Ok(()),
                Err(failure) => self.recover(failure)?,
            }
        }
    }

    /// Next token of the innermost level: replayed tokens first, then the lexer.
    #[inline(always)]
    fn next(&mut self) -> Result<Option<Lexeme>, LexError> {
        let level = self.levels.last_mut().expect("level");
        if level.replay_len > 0 {
            let token = level.replay[level.replay_start];
            level.replay_start += 1;
            level.replay_len -= 1;
            return Ok(Some(token));
        }
        level.lexer.next()
    }

    /// Parses items until the document is complete. Grammar state lives in control flow: each
    /// position that reads a token has its own call site, which keeps branches predictable.
    fn drive(&mut self) -> Result<(), Failure> {
        let mut carry = None;
        loop {
            let first = match carry.take() {
                Some(token) => token,
                None => match self.next()? {
                    Some(token) => token,
                    None if self.end_level() => continue,
                    None => return Ok(()),
                },
            };
            if !matches!(first.kind, Lex::Word | Lex::Quoted) {
                self.item(first)?;
                continue;
            }
            let Some(second) = self.next()? else {
                self.scalar(first);
                continue;
            };
            match second.kind {
                Lex::Op(op) => {
                    let Some(value) = self.next()? else {
                        return Err(Failure::Syntax(second.start..second.end, "Missing value"));
                    };
                    let key = key_kind(first, op);
                    match value.kind {
                        Lex::Open => self.open(Some(key), None, first, value)?,
                        Lex::Word => match self.next()? {
                            Some(open)
                                if open.kind == Lex::Open
                                    && self.same_line(value.start, open.start) =>
                            {
                                self.open(Some(key), Some(value), first, open)?;
                            }
                            next => {
                                self.push(key, first.start, first.end, first.line);
                                self.scalar(value);
                                carry = next;
                            }
                        },
                        Lex::Quoted | Lex::Math => {
                            self.push(key, first.start, first.end, first.line);
                            self.scalar(value);
                        }
                        _ => {
                            return Err(Failure::Syntax(
                                value.start..value.end,
                                "Expected a value",
                            ));
                        }
                    }
                }
                Lex::Open if self.same_line(first.start, second.start) => {
                    self.block(first, second)?;
                }
                _ => {
                    self.scalar(first);
                    carry = Some(second);
                }
            }
        }
    }

    /// `word {` on one line: an entry at document level, or inside a body when `word op` follows
    /// the brace; otherwise a scalar followed by an anonymous body.
    fn block(&mut self, word: Lexeme, open: Lexeme) -> Result<(), Failure> {
        if self.levels.len() > 1 || !self.containers.is_empty() {
            let after = self.next()?;
            let after_that = match after {
                Some(_) => self.next()?,
                None => None,
            };
            let entry = after.is_some_and(|t| matches!(t.kind, Lex::Word | Lex::Quoted))
                && after_that.is_some_and(|t| matches!(t.kind, Lex::Op(_)));
            let read: [Lexeme; 2] = [after.unwrap_or(EMPTY), after_that.unwrap_or(EMPTY)];
            let read = &read[..usize::from(after.is_some()) + usize::from(after_that.is_some())];
            if !entry {
                self.scalar(word);
                self.level().unread(open, read);
                return Ok(());
            }
            self.level().unread_all(read);
        }
        self.diagnostics.push(TapeDiagnostic {
            kind: Repair::OperatorLessEntry,
            span: word.start..word.end,
        });
        self.open(Some(key_kind(word, Operator::Assign)), None, word, open)
    }

    /// An item that does not start with a word.
    #[inline(always)]
    fn item(&mut self, token: Lexeme) -> Result<(), Failure> {
        match token.kind {
            Lex::Word | Lex::Quoted => unreachable!("words start entries in `drive`"),
            Lex::Math => self.scalar(token),
            Lex::Open => self.open(None, None, token, token)?,
            Lex::Close if self.containers.len() > self.level_containers() => {
                let open = self.containers.pop().expect("container").tape;
                let close = self.push(
                    TokenKind::Close { open },
                    token.start,
                    token.end,
                    token.line,
                );
                self.tokens[open].kind = TokenKind::Open { close };
            }
            Lex::Close => self.diagnostics.push(TapeDiagnostic {
                kind: Repair::StrayClosingBrace,
                span: token.start..token.end,
            }),
            Lex::Bracket | Lex::Op(_) => {
                return Err(Failure::Syntax(
                    token.start..token.end,
                    "Expected a key or value",
                ));
            }
            Lex::Region { negated, body } => self.enter_region(token, negated, body)?,
        }
        Ok(())
    }

    fn enter_region(&mut self, token: Lexeme, negated: bool, body: usize) -> Result<(), Failure> {
        if self.depth() >= MAX_NESTING_DEPTH {
            return Err(Failure::Nesting(token.start..token.end));
        }
        let tape = self.push(
            TokenKind::Region { negated, end: 0 },
            token.start,
            body,
            token.line,
        );
        let mark = RegionMark {
            lexeme: token,
            tape,
            diagnostics: self.diagnostics.len(),
            containers: self.containers.len(),
        };
        let lexer = Lexer::new(&self.source[..token.end - 1], body, token.line);
        self.levels.push(Level::new(lexer, Some(mark)));
        Ok(())
    }

    #[inline(always)]
    fn push(&mut self, kind: TokenKind, start: usize, end: usize, line: usize) -> usize {
        self.tokens.push(Token {
            kind,
            start,
            end,
            line,
        });
        self.tokens.len() - 1
    }
    #[inline(always)]
    fn scalar(&mut self, token: Lexeme) {
        let kind = match token.kind {
            Lex::Quoted => TokenKind::Quoted,
            Lex::Math => TokenKind::Math,
            _ => TokenKind::Bare,
        };
        self.push(kind, token.start, token.end, token.line);
    }
    fn level(&mut self) -> &mut Level<'a> {
        self.levels.last_mut().expect("level")
    }
    fn level_containers(&self) -> usize {
        self.levels
            .last()
            .expect("level")
            .region
            .as_ref()
            .map_or(0, |r| r.containers)
    }
    fn depth(&self) -> usize {
        self.containers.len() + self.levels.len() - 1
    }
    fn same_line(&self, from: usize, to: usize) -> bool {
        !self.source.as_bytes()[from..to].contains(&b'\n')
    }

    /// Opens a body after its `{` was read. The repair span of an unclosed body is the key when
    /// present, otherwise the `{`.
    #[inline(always)]
    fn open(
        &mut self,
        key: Option<TokenKind>,
        header: Option<Lexeme>,
        key_token: Lexeme,
        open: Lexeme,
    ) -> Result<(), Failure> {
        if self.depth() >= MAX_NESTING_DEPTH {
            return Err(self.nesting_failure());
        }
        let span = match key {
            Some(key) => {
                self.push(key, key_token.start, key_token.end, key_token.line);
                key_token.start..key_token.end
            }
            None => open.start..open.end,
        };
        if let Some(header) = header {
            self.push(TokenKind::Header, header.start, header.end, header.line);
        }
        let tape = self.push(
            TokenKind::Open { close: 0 },
            open.start,
            open.end,
            open.line,
        );
        self.containers.push(OpenContainer { tape, span });
        Ok(())
    }

    /// A nesting failure spans the token after the opener, or the level's end, as before.
    #[cold]
    fn nesting_failure(&mut self) -> Failure {
        let level = self.levels.last().expect("level");
        match level.following() {
            Ok(Some(next)) => Failure::Nesting(next.start..next.end),
            Ok(None) => Failure::Nesting(match &level.region {
                Some(region) => region.lexeme.start..region.lexeme.end,
                None => self.source.len()..self.source.len(),
            }),
            Err(e) => Failure::Lex(e),
        }
    }

    /// Handles exhaustion of the innermost level; returns false when the document is complete.
    fn end_level(&mut self) -> bool {
        let base = self.level_containers();
        if self.levels.len() == 1 {
            while let Some(open) = self.containers.pop() {
                self.diagnostics.push(TapeDiagnostic {
                    kind: Repair::UnclosedAtEof,
                    span: open.span,
                });
                let (end, line) = (self.source.len(), self.levels[0].lexer.line);
                let close = self.push(TokenKind::Close { open: open.tape }, end, end, line);
                self.tokens[open.tape].kind = TokenKind::Open { close };
            }
            return false;
        }
        let level = self.levels.pop().expect("region level");
        let region = level.region.expect("region mark");
        let verbatim = self.containers.len() > base
            || self.diagnostics[region.diagnostics..]
                .iter()
                .any(|d| d.kind != Repair::OperatorLessEntry);
        if verbatim {
            self.fall_back(region);
        } else {
            let lexeme = region.lexeme;
            let end = self.push(
                TokenKind::RegionEnd { start: region.tape },
                lexeme.end - 1,
                lexeme.end,
                level.lexer.line,
            );
            let Lex::Region { negated, .. } = lexeme.kind else {
                unreachable!("region marks hold region openers")
            };
            self.tokens[region.tape].kind = TokenKind::Region { negated, end };
        }
        true
    }

    fn fall_back(&mut self, region: RegionMark) {
        self.tokens.truncate(region.tape);
        self.diagnostics.truncate(region.diagnostics);
        self.containers.truncate(region.containers);
        let Lex::Region { negated, .. } = region.lexeme.kind else {
            unreachable!()
        };
        self.push(
            TokenKind::RegionText { negated },
            region.lexeme.start,
            region.lexeme.end,
            region.lexeme.line,
        );
    }

    /// Resolves a failure the way tokenize-then-parse did: a lexical error anywhere in an enclosing
    /// level wins (outermost first); otherwise a nesting error is fatal and any other error fails
    /// the innermost level. A failed region becomes verbatim text; a failed document is an error.
    fn recover(&mut self, failure: Failure) -> Result<(), SyntaxError> {
        let innermost = self.levels.len() - 1;
        let lexical = matches!(failure, Failure::Lex(_));
        for index in 0..self.levels.len() {
            if index == innermost && lexical {
                break;
            }
            if let Some(e) = self.levels[index].remaining_error() {
                return self.fail_level(index, Failure::Lex(e));
            }
        }
        if let Failure::Nesting(span) = failure {
            return Err(SyntaxError {
                kind: ErrorKind::NestingLimit,
                ..self.error(span, "Nesting exceeds supported limit")
            });
        }
        self.fail_level(innermost, failure)
    }
    fn fail_level(&mut self, index: usize, failure: Failure) -> Result<(), SyntaxError> {
        if index == 0 {
            return Err(match failure {
                Failure::Lex(e) => self.error(e.at..e.at, e.message),
                Failure::Syntax(span, message) => self.error(span, message),
                Failure::Nesting(_) => unreachable!("nesting failures are reported directly"),
            });
        }
        self.levels.truncate(index + 1);
        let region = self
            .levels
            .pop()
            .expect("level")
            .region
            .expect("region mark");
        self.fall_back(region);
        Ok(())
    }
    fn error(&self, span: Range<usize>, message: &str) -> SyntaxError {
        error(self.file, line_span(self.source, span), message)
    }
}

/// A span with one-based lines, counted from the start of `source`.
pub(crate) fn line_span(source: &str, span: Range<usize>) -> Span {
    let line = |at: usize| {
        source.as_bytes()[..at]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
            + 1
    };
    Span {
        start: span.start,
        end: span.end,
        line: line(span.start),
        end_line: line(span.end),
    }
}
