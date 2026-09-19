use super::representable::*;
use crate::{Span, SyntaxError, source::error};
use serde::{Deserialize, Serialize};

/// Operators accepted by game-definition scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operator {
    /// Assignment.
    #[serde(rename = "=")]
    Assign,
    /// Greater than.
    #[serde(rename = ">")]
    Greater,
    /// Less than.
    #[serde(rename = "<")]
    Less,
    /// Greater than or equal.
    #[serde(rename = ">=")]
    GreaterEqual,
    /// Less than or equal.
    #[serde(rename = "<=")]
    LessEqual,
    /// Not equal.
    #[serde(rename = "!=")]
    NotEqual,
}
impl Operator {
    /// Exact source spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assign => "=",
            Self::Greater => ">",
            Self::Less => "<",
            Self::GreaterEqual => ">=",
            Self::LessEqual => "<=",
            Self::NotEqual => "!=",
        }
    }
    /// Recognizes only supported game-script operators.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "=" => Some(Self::Assign),
            ">" => Some(Self::Greater),
            "<" => Some(Self::Less),
            ">=" => Some(Self::GreaterEqual),
            "<=" => Some(Self::LessEqual),
            "!=" => Some(Self::NotEqual),
            _ => None,
        }
    }
}
/// A scalar preserves exact text rather than projecting numbers or interpreting variables.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Scalar {
    /// String content, with a quote-preservation hint.
    #[serde(rename = "str")]
    String {
        /// Raw content; escapes are not decoded.
        value: String,
        /// Whether the writer must retain quotes.
        quoted: bool,
    },
    /// Canonical decimal digits, without floating-point conversion.
    #[serde(rename = "num")]
    Number {
        /// Canonical decimal spelling.
        lexeme: String,
    },
    /// A yes/no token.
    #[serde(rename = "bool")]
    Bool {
        /// Boolean represented by the token.
        value: bool,
    },
    /// Unresolved script variable.
    #[serde(rename = "var")]
    Variable {
        /// Name including `@`.
        name: String,
    },
    /// Unevaluated inline math.
    #[serde(rename = "math")]
    Math {
        /// Entire source token including delimiters.
        source: String,
    },
}
/// An ordered body, optionally preceded by a header such as `hsv`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Container {
    /// Unquoted header at value position.
    pub header: Option<String>,
    /// Ordered children, including duplicate entries and bare values.
    pub items: Vec<Item>,
}
/// A right-hand-side value; conditional regions cannot occur here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// One scalar.
    Scalar(Scalar),
    /// A braced body.
    Container(Container),
}
/// A key, operator, and value; keys are never classified as scalars.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Raw key text.
    pub key: String,
    /// Assignment or comparison.
    pub op: Operator,
    /// Assigned or compared value.
    pub value: Value,
}
/// Syntax at item position; the wrapper holds optional source metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum ItemKind {
    /// Keyed entry.
    Entry(Entry),
    /// Bare scalar.
    Scalar(Scalar),
    /// Anonymous container.
    Container(Container),
    /// Balanced conditional parameter body.
    Param {
        /// Parameter name.
        name: String,
        /// Tests absence when true.
        negated: bool,
        /// Balanced body.
        items: Vec<Item>,
    },
    /// Conditional text whose structure depends on substitution.
    ParamText {
        /// Parameter name.
        name: String,
        /// Tests absence when true.
        negated: bool,
        /// Verbatim body, including comments.
        text: String,
    },
}
/// An ordered syntax item with optional source location.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// The syntax represented by this item.
    pub kind: ItemKind,
    /// Source location, absent for constructed items.
    pub span: Option<Span>,
}
impl Item {
    /// Constructs an item without a source location; serialization checks representability.
    pub fn new(kind: ItemKind) -> Self {
        Self { kind, span: None }
    }
}
/// A documented repair to malformed source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Repair category.
    pub kind: Repair,
    /// Source location of the repair.
    pub span: Span,
    /// Source text that caused the repair.
    pub text: String,
}
/// The only repairs made by the game-script parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Repair {
    /// Skipped an unmatched closing brace.
    StrayClosingBrace,
    /// Closed a container at end of input.
    UnclosedAtEof,
    /// Inserted an assignment before a same-line entry-shaped block.
    OperatorLessEntry,
}
/// Parsed source and all applied repairs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// Caller-supplied diagnostic filename.
    pub file: String,
    /// Ordered top-level items.
    pub items: Vec<Item>,
    /// Repairs; strict callers may reject any nonempty list.
    pub diagnostics: Vec<Diagnostic>,
}
pub(crate) fn invalid(message: &str) -> SyntaxError {
    error("<constructed>", Span::default(), message)
}
/// Constructs a representable string, automatically quoted by the serializer when necessary.
pub fn scalar(value: impl Into<String>) -> Result<Scalar, SyntaxError> {
    let value = value.into();
    if !is_writable_text(&value) {
        return Err(invalid("Unrepresentable string"));
    }
    Ok(Scalar::String {
        value,
        quoted: false,
    })
}
/// Constructs an explicitly quoted string without decoding escapes.
pub fn quoted(value: impl Into<String>) -> Result<Scalar, SyntaxError> {
    let value = value.into();
    if !is_quotable_content(&value) {
        return Err(invalid("Unrepresentable quoted content"));
    }
    Ok(Scalar::String {
        value,
        quoted: true,
    })
}
/// Constructs a numeral without passing through a floating-point value.
pub fn numeral(text: &str) -> Result<Scalar, SyntaxError> {
    Ok(Scalar::Number {
        lexeme: canonical_numeral(text)?,
    })
}
/// Constructs a boolean scalar.
pub fn boolean(value: bool) -> Scalar {
    Scalar::Bool { value }
}
/// Constructs an unresolved variable reference including `@`.
pub fn var_ref(name: impl Into<String>) -> Result<Scalar, SyntaxError> {
    let name = name.into();
    if !is_var_name(&name) {
        return Err(invalid("Invalid variable reference"));
    }
    Ok(Scalar::Variable { name })
}
/// Constructs an unevaluated inline-math token.
pub fn inline_math(source: impl Into<String>) -> Result<Scalar, SyntaxError> {
    let source = source.into();
    if !is_math_source(&source) {
        return Err(invalid("Invalid inline math"));
    }
    Ok(Scalar::Math { source })
}
/// Constructs a body with an optional unquoted header; depth is checked when serialized.
pub fn container(items: Vec<Item>, header: Option<String>) -> Result<Container, SyntaxError> {
    if header.as_ref().is_some_and(|s| !is_bare_token(s)) {
        return Err(invalid("Invalid container header"));
    }
    Ok(Container { items, header })
}
/// Constructs a keyed entry, preserving comparison operators and duplicate keys.
pub fn entry(key: impl Into<String>, op: Operator, value: Value) -> Result<Item, SyntaxError> {
    let key = key.into();
    if !is_bare_token(&key) && !is_quotable_content(&key) {
        return Err(invalid("Invalid key"));
    }
    Ok(Item::new(ItemKind::Entry(Entry { key, op, value })))
}
/// Constructs an assignment.
pub fn kv(key: impl Into<String>, value: Value) -> Result<Item, SyntaxError> {
    entry(key, Operator::Assign, value)
}
/// Constructs a scalar comparison.
pub fn cmp(key: impl Into<String>, op: Operator, value: Scalar) -> Result<Item, SyntaxError> {
    entry(key, op, Value::Scalar(value))
}
/// Constructs an assignment to an ordered body.
pub fn block(key: impl Into<String>, items: Vec<Item>) -> Result<Item, SyntaxError> {
    kv(key, Value::Container(container(items, None)?))
}
/// Constructs an assignment to a scalar list.
pub fn list(key: impl Into<String>, values: Vec<Scalar>) -> Result<Item, SyntaxError> {
    block(
        key,
        values
            .into_iter()
            .map(|v| Item::new(ItemKind::Scalar(v)))
            .collect(),
    )
}
/// Constructs a balanced conditional region.
pub fn param_block(
    name: impl Into<String>,
    items: Vec<Item>,
    negated: bool,
) -> Result<Item, SyntaxError> {
    let name = name.into();
    if !is_param_name(&name) {
        return Err(invalid("Invalid parameter name"));
    }
    Ok(Item::new(ItemKind::Param {
        name,
        negated,
        items,
    }))
}
/// Constructs conditional text only when it reparses as the same verbatim region.
pub fn param_text(
    name: impl Into<String>,
    text: impl Into<String>,
    negated: bool,
) -> Result<Item, SyntaxError> {
    let name = name.into();
    let text = text.into();
    if let Some(problem) = region_text_problem(&name, negated, &text) {
        return Err(invalid(&problem));
    }
    Ok(Item::new(ItemKind::ParamText {
        name,
        negated,
        text,
    }))
}
/// Explains why a proposed verbatim region cannot round-trip, or returns `None`.
pub fn region_text_problem(name: &str, negated: bool, text: &str) -> Option<String> {
    if !is_param_name(name) {
        return Some("Invalid parameter name".into());
    }
    let source = format!("[[{}{name}]{text}]", if negated { "!" } else { "" });
    match super::parse(&source, "<region>") {
        Ok(doc) if doc.diagnostics.is_empty() && doc.items.len() == 1 => match &doc.items[0].kind {
            ItemKind::ParamText {
                name: n,
                negated: b,
                text: t,
            } if n == name && *b == negated && t == text => None,
            _ => Some("Body does not read back as the same verbatim region".into()),
        },
        Ok(_) => Some("Body does not read back as one region".into()),
        Err(e) => Some(e.to_string()),
    }
}
/// Whether a proposed verbatim region reads back unchanged.
pub fn is_region_text(name: &str, negated: bool, text: &str) -> bool {
    region_text_problem(name, negated, text).is_none()
}

/// Whether an item is one of the five scalar forms.
pub fn is_scalar(item: &Item) -> bool {
    matches!(item.kind, ItemKind::Scalar(_))
}
