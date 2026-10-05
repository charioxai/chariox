use crate::runtime::browser_controller_snapshot::RoomBrowserDomNode;

// MP-08/MP-10/MP-11: passive rendered geometry includes noninteractive shapes.
const MAX_VISIBLE_ELEMENTS: usize = 128;
pub(super) fn browser_geometry_elements(
    nodes: &[RoomBrowserDomNode],
) -> (Vec<serde_json::Value>, bool) {
    let mut visible = nodes.iter().filter(|node| {
        node.node_type == 1
            && node.rendered
            && node.bounds.is_some_and(|b| b.width > 0.0 && b.height > 0.0)
            && !matches!(
                node.node_name.as_str(),
                "HTML" | "BODY" | "SCRIPT" | "STYLE" | "NOSCRIPT" | "TEMPLATE"
            )
    });
    let elements = visible
        .by_ref()
        .take(MAX_VISIBLE_ELEMENTS)
        .map(|node| {
            let appearance = node
                .attributes
                .iter()
                .filter_map(|(key, value)| {
                    key.strip_prefix("chariox-rendered-")
                        .map(|key| (key.to_string(), serde_json::Value::String(value.clone())))
                })
                .collect::<serde_json::Map<_, _>>();
            serde_json::json!({
                "kind":"element", "field_id":node.element_ref, "parent_field_id":node.parent_ref,
                "tag":node.node_name.to_ascii_lowercase(), "id":node.attributes.get("id"),
                "role":node.attributes.get("role"), "class":node.attributes.get("class"),
                "bounds":node.bounds, "appearance":appearance,
            })
        })
        .collect();
    (elements, visible.next().is_some())
}
