use super::*;
use crate::SyntaxError;

/// How traversal treats the body of a verbatim conditional region.
#[derive(Clone, Copy, Debug)]
pub enum RegionPolicy<'a> {
    /// Leave the body uninterpreted.
    Skip,
    /// Tokenize the body as a flat item sequence using this diagnostic filename.
    Read(&'a str),
}
/// Visitor decision, including the context to pass to children.
#[derive(Clone, Debug)]
pub enum WalkControl<C> {
    /// Visit children with this context.
    Continue(C),
    /// Skip this item's descendants.
    Skip,
    /// Stop the entire traversal.
    Stop,
}
/// Returns owned children in order. Entry values are wrapped as items without source metadata.
pub fn item_children(item: &Item, regions: RegionPolicy<'_>) -> Result<Vec<Item>, SyntaxError> {
    Ok(match &item.kind {
        ItemKind::Entry(entry) => vec![Item::new(match &entry.value {
            Value::Scalar(s) => ItemKind::Scalar(s.clone()),
            Value::Container(c) => ItemKind::Container(c.clone()),
        })],
        ItemKind::Container(c) => c.items.clone(),
        ItemKind::Param { items, .. } => items.clone(),
        ItemKind::ParamText { text, .. } => match regions {
            RegionPolicy::Skip => Vec::new(),
            RegionPolicy::Read(file) => region_items(text, file)?,
        },
        ItemKind::Scalar(_) => Vec::new(),
    })
}
/// Pre-order traversal with child context, subtree skipping, and early termination. Returns whether stopped.
pub fn walk_items<C: Clone>(
    items: &[Item],
    context: C,
    mut visit: impl FnMut(&Item, &C) -> WalkControl<C>,
    regions: RegionPolicy<'_>,
) -> Result<bool, SyntaxError> {
    let mut pending: Vec<_> = items
        .iter()
        .rev()
        .cloned()
        .map(|i| (i, context.clone()))
        .collect();
    while let Some((item, context)) = pending.pop() {
        let child_context = match visit(&item, &context) {
            WalkControl::Stop => return Ok(true),
            WalkControl::Skip => continue,
            WalkControl::Continue(c) => c,
        };
        let children = match item.kind {
            ItemKind::Entry(e) => vec![Item::new(match e.value {
                Value::Scalar(s) => ItemKind::Scalar(s),
                Value::Container(c) => ItemKind::Container(c),
            })],
            ItemKind::Container(c) => c.items,
            ItemKind::Param { items, .. } => items,
            ItemKind::ParamText { text, .. } => match regions {
                RegionPolicy::Skip => Vec::new(),
                RegionPolicy::Read(file) => region_items(&text, file)?,
            },
            ItemKind::Scalar(_) => Vec::new(),
        };
        pending.extend(
            children
                .into_iter()
                .rev()
                .map(|i| (i, child_context.clone())),
        );
    }
    Ok(false)
}
/// Copies a tree with all source locations removed for semantic comparison.
pub fn without_spans(items: &[Item]) -> Vec<Item> {
    let mut result = items.to_vec();
    let mut pending: Vec<_> = result.iter_mut().collect();
    while let Some(item) = pending.pop() {
        item.span = None;
        match &mut item.kind {
            ItemKind::Entry(Entry {
                value: Value::Container(c),
                ..
            })
            | ItemKind::Container(c) => pending.extend(c.items.iter_mut()),
            ItemKind::Param { items, .. } => pending.extend(items.iter_mut()),
            _ => (),
        }
    }
    result
}
