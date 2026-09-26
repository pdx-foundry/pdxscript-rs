use super::ast::invalid;
use super::*;
use crate::{ErrorKind, MAX_NESTING_DEPTH, SyntaxError};

fn quoted_text(text: &str) -> Result<String, SyntaxError> {
    if !is_quotable_content(text) {
        return Err(invalid("Content cannot be quoted without changing it"));
    }
    Ok(format!("\"{text}\""))
}
/// Canonical source for a scalar, rejecting malformed hand-built values.
pub fn scalar_text(value: &Scalar) -> Result<String, SyntaxError> {
    match value {
        Scalar::String { value, quoted } => {
            if *quoted || !is_bare_string(value) {
                quoted_text(value)
            } else {
                Ok(value.clone())
            }
        }
        Scalar::Bool { value } => Ok(if *value { "yes" } else { "no" }.into()),
        Scalar::Number { lexeme } if canonical_numeral(lexeme).as_ref() == Ok(lexeme) => {
            Ok(lexeme.clone())
        }
        Scalar::Variable { name } if is_var_name(name) => Ok(name.clone()),
        Scalar::Math { source } if is_math_source(source) => Ok(source.clone()),
        _ => Err(invalid("Unrepresentable scalar")),
    }
}
enum Write<'a> {
    Item(&'a Item, usize),
    Body(&'a Container, usize),
    ScalarBody(&'a [Item]),
    Text(String),
    Region(&'a str, bool, &'a [Item], usize),
}
fn enter(depth: usize) -> Result<(), SyntaxError> {
    if depth >= MAX_NESTING_DEPTH {
        return Err(SyntaxError {
            kind: ErrorKind::NestingLimit,
            ..invalid("Nesting exceeds supported limit")
        });
    }
    Ok(())
}
fn schedule_items<'a>(
    stack: &mut Vec<Write<'a>>,
    items: &'a [Item],
    depth: usize,
    separator: &str,
) {
    for (index, item) in items.iter().enumerate().rev() {
        stack.push(Write::Item(item, depth));
        if index != 0 {
            stack.push(Write::Text(separator.into()));
        }
    }
}
fn schedule_value<'a>(
    stack: &mut Vec<Write<'a>>,
    value: &'a Value,
    depth: usize,
) -> Result<(), SyntaxError> {
    match value {
        Value::Scalar(value) => stack.push(Write::Text(scalar_text(value)?)),
        Value::Container(body) => stack.push(Write::Body(body, depth)),
    }

    Ok(())
}

fn schedule_item<'a>(
    stack: &mut Vec<Write<'a>>,
    item: &'a Item,
    depth: usize,
) -> Result<(), SyntaxError> {
    match &item.kind {
        ItemKind::Scalar(value) => stack.push(Write::Text(scalar_text(value)?)),
        ItemKind::Container(body) => stack.push(Write::Body(body, depth)),
        ItemKind::Entry(entry) => {
            let key = if is_bare_key(&entry.key) {
                entry.key.clone()
            } else {
                quoted_text(&entry.key)?
            };

            schedule_value(stack, &entry.value, depth)?;
            stack.push(Write::Text(format!("{key} {} ", entry.op.as_str())));
        }
        ItemKind::Param {
            name,
            negated,
            items,
        } => {
            stack.push(Write::Region(name, *negated, items, depth));
        }
        ItemKind::ParamText {
            name,
            negated,
            text,
        } => {
            enter(depth)?;
            if let Some(problem) = region_text_problem(name, *negated, text) {
                return Err(invalid(&problem));
            }

            stack.push(Write::Text(format!(
                "[[{}{name}]{text}]",
                if *negated { "!" } else { "" }
            )));
        }
    }

    stack.push(Write::Text("\t".repeat(depth)));
    Ok(())
}

fn append_scalar_body(output: &mut String, items: &[Item]) -> Result<(), SyntaxError> {
    output.push_str("{ ");

    for (index, item) in items.iter().enumerate() {
        let ItemKind::Scalar(value) = &item.kind else {
            return Err(invalid("Expected a scalar in an inline container"));
        };

        if index != 0 {
            output.push(' ');
        }

        output.push_str(&scalar_text(value)?);
    }

    output.push_str(" }");
    Ok(())
}

fn schedule_body<'a>(
    stack: &mut Vec<Write<'a>>,
    body: &'a Container,
    depth: usize,
) -> Result<(), SyntaxError> {
    enter(depth)?;
    let header = match &body.header {
        Some(header) if !is_bare_token(header) => {
            return Err(invalid("Invalid container header"));
        }
        Some(header) => format!("{header} "),
        None => String::new(),
    };

    if body.items.is_empty() {
        stack.push(Write::Text("{}".into()));
    } else if body.items.iter().all(is_scalar) {
        stack.push(Write::ScalarBody(&body.items));
    } else {
        stack.push(Write::Text(format!("\n{}}}", "\t".repeat(depth))));
        schedule_items(stack, &body.items, depth + 1, "\n");
        stack.push(Write::Text("{\n".into()));
    }

    stack.push(Write::Text(header));
    Ok(())
}

fn schedule_region<'a>(
    stack: &mut Vec<Write<'a>>,
    name: &str,
    negated: bool,
    items: &'a [Item],
    depth: usize,
) -> Result<(), SyntaxError> {
    enter(depth)?;
    if !is_param_name(name) {
        return Err(invalid("Invalid parameter name"));
    }

    stack.push(Write::Text(format!(
        "{}{}]",
        if items.is_empty() { "" } else { "\n" },
        "\t".repeat(depth)
    )));
    schedule_items(stack, items, depth + 1, "\n");
    stack.push(Write::Text(format!(
        "[[{}{name}]\n",
        if negated { "!" } else { "" }
    )));
    Ok(())
}

/// Writes canonical script with tabs and a final newline. Locations, comments, and repairs are not reproduced.
pub fn serialize(items: &[Item]) -> Result<String, SyntaxError> {
    let mut stack = vec![Write::Text("\n".into())];
    schedule_items(&mut stack, items, 0, "\n\n");
    let mut output = String::new();

    while let Some(action) = stack.pop() {
        match action {
            Write::Text(text) => output.push_str(&text),
            Write::ScalarBody(items) => append_scalar_body(&mut output, items)?,
            Write::Item(item, depth) => schedule_item(&mut stack, item, depth)?,
            Write::Body(body, depth) => schedule_body(&mut stack, body, depth)?,
            Write::Region(name, negated, items, depth) => {
                schedule_region(&mut stack, name, negated, items, depth)?;
            }
        }
    }

    if output.starts_with('\u{feff}') {
        return Err(invalid(
            "A document cannot emit a leading BOM as scalar content",
        ));
    }

    Ok(output)
}
