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
    nodes.iter().map(|item| match &item.kind {
        ItemKind::Scalar(s) => scalar(s),
        ItemKind::Container(c) => body(c),
        ItemKind::Entry(e) => json!({"kind":"entry","key":e.key,"op":e.op,"value":match &e.value { Value::Scalar(s) => scalar(s), Value::Container(c) => body(c) }}),
        ItemKind::Param { name, negated, items: children } => json!({"kind":"param","name":name,"negated":negated,"items":items(children)}),
        ItemKind::ParamText { name, negated, text } => json!({"kind":"param-text","name":name,"negated":negated,"text":text}),
    }).collect()
}
