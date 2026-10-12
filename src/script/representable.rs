use super::{Scalar, ast::invalid};
use crate::SyntaxError;

pub(crate) const fn terminates(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\r'
            | '\n'
            | '\u{b}'
            | '\u{c}'
            | ';'
            | '{'
            | '}'
            | '['
            | ']'
            | '='
            | '<'
            | '>'
            | '!'
            | '#'
            | '"'
    )
}
/// Whether text is exactly one unquoted token.
pub fn is_bare_token(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(terminates)
}
/// Whether a key may be emitted bare, including at the document boundary.
pub fn is_bare_key(text: &str) -> bool {
    is_bare_token(text) && !text.starts_with('\u{feff}')
}
/// Whether text is a supported decimal numeral, without an exponent.
pub fn is_numeral(text: &str) -> bool {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text).as_bytes();
    let whole = digits.iter().take_while(|b| b.is_ascii_digit()).count();
    match &digits[whole..] {
        [] => whole > 0,
        [b'.', fraction @ ..] => !fraction.is_empty() && fraction.iter().all(u8::is_ascii_digit),
        _ => false,
    }
}
/// Canonicalizes numeral digits textually, preserving arbitrary precision.
pub fn canonical_numeral(text: &str) -> Result<String, SyntaxError> {
    if !is_numeral(text) {
        return Err(invalid("Invalid numeral"));
    }
    Ok(canonical_digits(text))
}
/// Canonical spelling of a text already known to be a numeral, built in one allocation.
fn canonical_digits(text: &str) -> String {
    let unsigned = text.trim_start_matches(['+', '-']);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.trim_end_matches('0');
    let negative = text.starts_with('-') && (whole != "0" || !fraction.is_empty());
    let mut lexeme = String::with_capacity(2 + whole.len() + fraction.len());
    if negative {
        lexeme.push('-');
    }
    lexeme.push_str(whole);
    if !fraction.is_empty() {
        lexeme.push('.');
        lexeme.push_str(fraction);
    }
    lexeme
}
/// Writes a finite double as decimal digits; integral doubles use their exact integer value.
pub fn decimal_lexeme(value: f64) -> Result<String, SyntaxError> {
    if !value.is_finite() {
        return Err(invalid("Non-finite number"));
    }
    let text = if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    };
    canonical_numeral(&text)
}
/// Converts a numeral to a finite double whose decimal output matches the canonical input.
/// Returns `None` for invalid numerals or when conversion changes the canonical digits.
pub fn try_number_value(text: &str) -> Option<f64> {
    let canonical = canonical_numeral(text).ok()?;
    let value: f64 = canonical.parse().ok()?;
    (decimal_lexeme(value).ok()? == canonical).then_some(value)
}
/// Converts a numeral to a double, failing if its decimal output changes the canonical digits.
/// Invalid numerals and non-finite results also return an error; syntax trees keep the original digits.
pub fn number_value(text: &str) -> Result<f64, SyntaxError> {
    try_number_value(text).ok_or_else(|| invalid("Numeral cannot be projected without rounding"))
}
/// Whether raw content can be enclosed in quotes without changing its interpretation.
pub fn is_quotable_content(text: &str) -> bool {
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' || (ch == '\\' && chars.next().is_none()) {
            return false;
        }
    }
    true
}
/// Classifies an unquoted token without evaluating variables or math.
pub fn classify_unquoted(text: &str) -> Scalar {
    if text == "yes" || text == "no" {
        Scalar::Bool {
            value: text == "yes",
        }
    } else if is_numeral(text) {
        Scalar::Number {
            lexeme: canonical_digits(text),
        }
    } else if text.starts_with('@') {
        Scalar::Variable { name: text.into() }
    } else {
        Scalar::String {
            value: text.into(),
            quoted: false,
        }
    }
}
/// Whether a string remains a string when written bare.
pub fn is_bare_string(text: &str) -> bool {
    is_bare_token(text) && matches!(classify_unquoted(text), Scalar::String { .. })
}
/// Whether a string can be written bare or quoted.
pub fn is_writable_text(text: &str) -> bool {
    is_bare_string(text) || is_quotable_content(text)
}
/// Whether a name can occur in a conditional-region opener.
pub fn is_param_name(text: &str) -> bool {
    is_bare_token(text)
}
/// Whether a variable token includes its `@` prefix and has no delimiters.
pub fn is_var_name(text: &str) -> bool {
    text.starts_with('@') && is_bare_token(text)
}
/// Whether text is one complete inline-math token.
pub fn is_math_source(text: &str) -> bool {
    let body = text
        .strip_prefix("@[")
        .or_else(|| text.strip_prefix("@\\["));
    body.and_then(|s| s.strip_suffix(']'))
        .is_some_and(|s| !s.contains(']'))
}

/// Whether text names a supported game-script operator.
pub fn is_operator(text: &str) -> bool {
    super::Operator::parse(text).is_some()
}
