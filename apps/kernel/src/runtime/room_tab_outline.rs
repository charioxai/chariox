//! A Room Tab's accessibility tree as terminals present it to screen readers
//! and keyboard users (protocol 357): what a reader would announce, in
//! document order, as a tree.

use std::collections::HashMap;

use crate::local::RoomEnvironmentAccessibilityNode;
use crate::runtime::browser_controller_snapshot::{
    RoomBrowserAccessibilityNode, MAX_SNAPSHOT_NODES,
};

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
    // Children in document order: the snapshot's own child list, else (for a
    // node without one) the nodes naming it as parent, in snapshot order.
    let mut by_parent: HashMap<&str, Vec<&RoomBrowserAccessibilityNode>> = HashMap::new();
    for node in nodes {
        if let Some(parent) = node.parent_ref.as_deref() {
            by_parent.entry(parent).or_default().push(node);
        }
    }
    let children = |node: &RoomBrowserAccessibilityNode| -> Vec<&RoomBrowserAccessibilityNode> {
        if node.child_refs.is_empty() {
            by_parent
                .get(node.element_ref.as_str())
                .cloned()
                .unwrap_or_default()
        } else {
            node.child_refs
                .iter()
                .filter_map(|child| by_ref.get(child.as_str()).copied())
                .collect()
        }
    };
    // The controller keeps at most MAX_SNAPSHOT_NODES raw nodes, cutting the
    // deepest ones: an outline of a full snapshot may be missing text too.
    let mut truncated = nodes.len() >= MAX_SNAPSHOT_NODES;
    let mut outline = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // Depth first over the raw tree; a kept node hangs from the nearest kept
    // ancestor, so hoisted descendants keep their place in reading order.
    let mut pending: Vec<(Option<String>, &RoomBrowserAccessibilityNode)> = nodes
        .iter()
        .filter(|node| parent(node).is_none())
        .rev()
        .map(|node| (None, node))
        .collect();
    while let Some((kept_ancestor, node)) = pending.pop() {
        if !seen.insert(node.element_ref.as_str()) {
            continue;
        }
        let below = if kept(node) {
            if outline.len() == MAX_NODES {
                truncated = true;
                break;
            }
            Some(node.element_ref.clone())
        } else {
            kept_ancestor.clone()
        };
        pending.extend(
            children(node)
                .into_iter()
                .rev()
                .map(|child| (below.clone(), child)),
        );
        if !kept(node) {
            continue;
        }
        let parent_ref = kept_ancestor;
        outline.push(RoomEnvironmentAccessibilityNode {
            element_ref: node.element_ref.clone(),
            parent_ref,
            role: node.role.clone(),
            name: node.name.clone(),
            value: node.value.clone(),
            description: node.description.clone(),
            disabled: node.disabled,
            focused: node.focused,
            states: node.states.clone(),
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
            states: Vec::new(),
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
    fn hoisted_text_keeps_its_place_in_reading_order() {
        // <p><span>Hello</span> world</p>: the snapshot lists level by level
        // (p, span, " world", "Hello"); the outline reads "Hello" first.
        let mut paragraph = node("p", None, "paragraph", "");
        paragraph.child_refs = vec!["span".into(), "world".into()];
        let mut span = node("span", Some("p"), "generic", "");
        span.child_refs = vec!["hello".into()];
        let (outline, _) = outline(&[
            paragraph,
            span,
            node("world", Some("p"), "StaticText", " world"),
            node("hello", Some("span"), "StaticText", "Hello"),
        ]);
        let shape: Vec<_> = outline
            .iter()
            .map(|node| (node.element_ref.as_str(), node.parent_ref.as_deref()))
            .collect();
        assert_eq!(
            shape,
            [("p", None), ("hello", Some("p")), ("world", Some("p"))]
        );
    }

    #[test]
    fn a_full_snapshot_is_reported_as_truncated() {
        let nodes: Vec<_> = (0..MAX_SNAPSHOT_NODES)
            .map(|index| node(&format!("e{index}"), None, "generic", ""))
            .collect();
        let (outline, truncated) = outline(&nodes);
        assert!(outline.is_empty());
        assert!(truncated, "the controller may have cut deeper nodes");
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
