use pdxscript::script::*;
use serde_json::{Value as Json, json};
fn scalar(value: &Scalar) -> Json {
    serde_json::to_value(value).unwrap()
}
fn body(container: &Container) -> Json {
    let mut value = json!({"kind":"container","items":items(&container.items)});
    if let Some(header) = &container.header {
        value["header"] = json!(header);
    }
    value
}
pub fn items(nodes: &[Item]) -> Vec<Json> {
    nodes
        .iter()
        .map(|item| match &item.kind {
            ItemKind::Scalar(s) => scalar(s),
            ItemKind::Container(c) => body(c),
            ItemKind::Entry(entry) => {
                let value = match &entry.value {
                    Value::Scalar(scalar_value) => scalar(scalar_value),
                    Value::Container(container) => body(container),
                };
                json!({"kind":"entry","key":entry.key,"op":entry.op,"value":value})
            }
            ItemKind::Param {
                name,
                negated,
                items: children,
            } => json!({"kind":"param","name":name,"negated":negated,"items":items(children)}),
            ItemKind::ParamText {
                name,
                negated,
                text,
            } => json!({"kind":"param-text","name":name,"negated":negated,"text":text}),
        })
        .collect()
}

/// Builds the shared success response used by the conformance runner and fixture checks.
pub fn response(document: &Document) -> Result<Json, pdxscript::SyntaxError> {
    let canonical = serialize(&document.items)?;
    let normalized_items = items(&document.items);
    let diagnostics: Vec<_> = document
        .diagnostics
        .iter()
        .map(|diagnostic| {
            json!({
                "kind": diagnostic.kind,
                "line": diagnostic.span.line,
                "text": diagnostic.text,
            })
        })
        .collect();

    Ok(json!({
        "items": normalized_items,
        "canonical": canonical,
        "diagnostics": diagnostics,
    }))
}
