//! A Room Tab's accessibility tree as terminals present it to screen readers
//! and keyboard users (protocol 357): what a reader would announce, in
//! document order, as a tree.

use std::collections::HashMap;

use crate::local::RoomEnvironmentAccessibilityNode;
use crate::runtime::browser_controller_snapshot::RoomBrowserAccessibilityNode;

/// Bound on the outline a terminal receives for one Tab.
const MAX_NODES: usize = 2000;

/// Leaves out what a reader would not announce: ignored nodes, the text boxes
/// inside a text run, unnamed layout wrappers, and text that only repeats its
/// parent's name (a button's label). The children of a left-out node hang from
/// its nearest kept ancestor.
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
                && parent(node).is_some_and(|parent| parent.name == node.name)))
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
    let visible: Vec<_> = nodes.iter().filter(|node| kept(node)).collect();
    let truncated = visible.len() > MAX_NODES;
    let outline = visible
        .into_iter()
        .take(MAX_NODES)
        .map(|node| RoomEnvironmentAccessibilityNode {
            element_ref: node.element_ref.clone(),
            parent_ref: kept_ancestor(node),
            role: node.role.clone(),
            name: node.name.clone(),
            value: node.value.clone(),
            description: node.description.clone(),
            disabled: node.disabled,
            focused: node.focused,
        })
        .collect();
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
                ("e5", Some("e1")),
                ("e8", Some("e1"))
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
