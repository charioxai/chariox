//! A Room Tab's accessibility tree as terminals present it to screen readers
//! and keyboard users (protocol 357): what a reader would announce, in
//! document order, as a tree.

use std::collections::HashMap;

use crate::local::RoomEnvironmentAccessibilityNode;
use crate::runtime::browser_controller_snapshot::RoomBrowserAccessibilityNode;

/// Bound on the outline a terminal receives for one Tab.
const MAX_NODES: usize = 2000;

/// Leaves out what a reader would not announce: ignored nodes, the text boxes
/// inside a text run, unnamed layout wrappers, and text its parent's name
/// already says (a button's label). The children of a left-out node hang from
/// its nearest kept ancestor. Nodes come out in document order (depth first).
pub(crate) fn outline(
    nodes: &[RoomBrowserAccessibilityNode],
) -> (Vec<RoomEnvironmentAccessibilityNode>, bool) {
    let by_ref: HashMap<&str, &RoomBrowserAccessibilityNode> = nodes
        .iter()
        .map(|node| (node.element_ref.as_str(), node))
        .collect();
    let parent = |node: &RoomBrowserAccessibilityNode| {
        node.parent_ref
            .as_deref()
            .and_then(|parent| by_ref.get(parent).copied())
    };
    let kept = |node: &RoomBrowserAccessibilityNode| {
        !(node.ignored
            || node.role == "InlineTextBox"
            || (matches!(node.role.as_str(), "generic" | "none" | "presentation")
                && node.name.is_empty())
            || (node.role == "StaticText"
                && parent(node).is_some_and(|parent| parent.name.contains(node.name.as_str()))))
    };
    let kept_ancestor = |node: &RoomBrowserAccessibilityNode| {
        let mut current = parent(node);
        for _ in 0..nodes.len() {
            match current {
                Some(ancestor) if kept(ancestor) => return Some(ancestor.element_ref.clone()),
                Some(ancestor) => current = parent(ancestor),
                None => break,
            }
        }
        None
    };
    let mut children: HashMap<Option<String>, Vec<&RoomBrowserAccessibilityNode>> = HashMap::new();
    for node in nodes.iter().filter(|node| kept(node)) {
        children.entry(kept_ancestor(node)).or_default().push(node);
    }
    let mut outline = Vec::new();
    let mut truncated = false;
    let mut pending: Vec<(Option<String>, &RoomBrowserAccessibilityNode)> = children
        .get(&None)
        .map(|roots| roots.iter().rev().map(|node| (None, *node)).collect())
        .unwrap_or_default();
    while let Some((parent_ref, node)) = pending.pop() {
        if outline.len() == MAX_NODES {
            truncated = true;
            break;
        }
        if let Some(below) = children.get(&Some(node.element_ref.clone())) {
            pending.extend(
                below
                    .iter()
                    .rev()
                    .map(|child| (Some(node.element_ref.clone()), *child)),
            );
        }
        outline.push(RoomEnvironmentAccessibilityNode {
            element_ref: node.element_ref.clone(),
            parent_ref,
            role: node.role.clone(),
            name: node.name.clone(),
            value: node.value.clone(),
            description: node.description.clone(),
            disabled: node.disabled,
            focused: node.focused,
        });
    }
    (outline, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(
        element_ref: &str,
        parent: Option<&str>,
        role: &str,
        name: &str,
    ) -> RoomBrowserAccessibilityNode {
        RoomBrowserAccessibilityNode {
            element_ref: element_ref.to_string(),
            parent_ref: parent.map(str::to_string),
            child_refs: Vec::new(),
            role: role.to_string(),
            name: name.to_string(),
            description: String::new(),
            value: String::new(),
            ignored: false,
            disabled: false,
            focused: false,
        }
    }

    #[test]
    fn the_outline_keeps_what_a_reader_announces_as_a_tree() {
        let mut ignored = node("e2", Some("e1"), "generic", "hidden");
        ignored.ignored = true;
        let (outline, truncated) = outline(&[
            node("e1", None, "RootWebArea", "Todo"),
            ignored,
            node("e3", Some("e2"), "generic", ""),
            node("e4", Some("e3"), "textbox", "New todo"),
            node("e5", Some("e3"), "button", "Add"),
            node("e6", Some("e5"), "StaticText", "Add"),
            node("e7", Some("e6"), "InlineTextBox", "Add"),
            node("e8", Some("e3"), "StaticText", "2 open"),
            node("e9", Some("e5"), "generic", ""),
            node("e10", Some("e3"), "button", "Delete Buy milk"),
            node("e11", Some("e10"), "StaticText", "Delete"),
            // Snapshots list nodes level by level; the outline is depth first.
            node("e12", Some("e4"), "StaticText", "milk"),
        ]);
        assert!(!truncated);
        let shape: Vec<_> = outline
            .iter()
            .map(|node| (node.element_ref.as_str(), node.parent_ref.as_deref()))
            .collect();
        assert_eq!(
            shape,
            [
                ("e1", None),
                ("e4", Some("e1")),
                ("e12", Some("e4")),
                ("e5", Some("e1")),
                ("e8", Some("e1")),
                ("e10", Some("e1")),
            ]
        );
    }

    #[test]
    fn a_long_page_is_bounded() {
        let nodes: Vec<_> = (0..MAX_NODES + 5)
            .map(|index| node(&format!("e{index}"), None, "listitem", "item"))
            .collect();
        let (outline, truncated) = outline(&nodes);
        assert_eq!(outline.len(), MAX_NODES);
        assert!(truncated);
    }
}
