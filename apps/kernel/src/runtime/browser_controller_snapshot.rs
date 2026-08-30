use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

const MAX_SNAPSHOT_NODES: usize = 5_000;
const MAX_SNAPSHOT_STRING_BYTES: usize = 2_048;
const MAX_NODE_ATTRIBUTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct BrowserControllerStructuredSnapshot {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) snapshot_revision: u64,
    pub(crate) accessibility_nodes: Vec<BrowserControllerAccessibilityNode>,
    pub(crate) dom_nodes: Vec<BrowserControllerDomNode>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct BrowserControllerAccessibilityNode {
    pub(crate) node_ref: String,
    pub(crate) parent_ref: Option<String>,
    pub(crate) child_refs: Vec<String>,
    pub(crate) role: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) value: String,
    pub(crate) ignored: bool,
    pub(crate) disabled: bool,
    pub(crate) focused: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct BrowserControllerDomNode {
    pub(crate) node_ref: String,
    pub(crate) parent_ref: Option<String>,
    pub(crate) node_type: u32,
    pub(crate) node_name: String,
    pub(crate) text: String,
    pub(crate) attributes: BTreeMap<String, String>,
    pub(crate) bounds: Option<BrowserControllerNodeBounds>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub(crate) struct BrowserControllerNodeBounds {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RoomBrowserStructuredSnapshot {
    pub(crate) session_id: String,
    pub(crate) environment_id: String,
    pub(crate) runtime_generation: u64,
    pub(crate) tab_id: String,
    pub(crate) document_revision: u64,
    pub(crate) snapshot_revision: u64,
    pub(crate) accessibility_nodes: Vec<RoomBrowserAccessibilityNode>,
    pub(crate) dom_nodes: Vec<RoomBrowserDomNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RoomBrowserAccessibilityNode {
    pub(crate) element_ref: String,
    pub(crate) parent_ref: Option<String>,
    pub(crate) child_refs: Vec<String>,
    pub(crate) role: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) value: String,
    pub(crate) ignored: bool,
    pub(crate) disabled: bool,
    pub(crate) focused: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RoomBrowserDomNode {
    pub(crate) element_ref: String,
    pub(crate) parent_ref: Option<String>,
    pub(crate) node_type: u32,
    pub(crate) node_name: String,
    pub(crate) text: String,
    pub(crate) attributes: BTreeMap<String, String>,
    pub(crate) bounds: Option<BrowserControllerNodeBounds>,
}

impl BrowserControllerStructuredSnapshot {
    pub(crate) fn validate(
        &self,
        expected_target_id: &str,
        expected_document_id: &str,
    ) -> Result<(), String> {
        if self.browser_generation == 0 || self.snapshot_revision == 0 {
            return Err(
                "browser controller snapshot returned a zero generation or revision".to_string(),
            );
        }
        if self.target_id != expected_target_id || self.document_id != expected_document_id {
            return Err(
                "browser controller snapshot changed target or document identity".to_string(),
            );
        }
        validate_accessibility_nodes(&self.accessibility_nodes)?;
        validate_dom_nodes(&self.dom_nodes)
    }

    pub(crate) fn controller_node_refs(&self) -> BTreeSet<String> {
        self.accessibility_nodes
            .iter()
            .map(|node| node.node_ref.clone())
            .chain(self.dom_nodes.iter().map(|node| node.node_ref.clone()))
            .collect()
    }

    pub(crate) fn into_room_snapshot(
        self,
        session_id: String,
        environment_id: String,
        runtime_generation: u64,
        tab_id: String,
        document_revision: u64,
        references: &BTreeMap<String, String>,
    ) -> Result<RoomBrowserStructuredSnapshot, String> {
        let accessibility_nodes = self
            .accessibility_nodes
            .into_iter()
            .map(|node| {
                Ok(RoomBrowserAccessibilityNode {
                    element_ref: required_reference(references, &node.node_ref)?.to_string(),
                    parent_ref: optional_reference(references, node.parent_ref.as_deref()),
                    child_refs: node
                        .child_refs
                        .iter()
                        .filter_map(|reference| references.get(reference).cloned())
                        .collect(),
                    role: node.role,
                    name: node.name,
                    description: node.description,
                    value: node.value,
                    ignored: node.ignored,
                    disabled: node.disabled,
                    focused: node.focused,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let dom_nodes = self
            .dom_nodes
            .into_iter()
            .map(|node| {
                Ok(RoomBrowserDomNode {
                    element_ref: required_reference(references, &node.node_ref)?.to_string(),
                    parent_ref: optional_reference(references, node.parent_ref.as_deref()),
                    node_type: node.node_type,
                    node_name: node.node_name,
                    text: node.text,
                    attributes: node.attributes,
                    bounds: node.bounds,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(RoomBrowserStructuredSnapshot {
            session_id,
            environment_id,
            runtime_generation,
            tab_id,
            document_revision,
            snapshot_revision: self.snapshot_revision,
            accessibility_nodes,
            dom_nodes,
        })
    }
}

fn required_reference<'a>(
    references: &'a BTreeMap<String, String>,
    controller_node_ref: &str,
) -> Result<&'a str, String> {
    references
        .get(controller_node_ref)
        .map(String::as_str)
        .ok_or_else(|| "kernel omitted an element reference for a controller node".to_string())
}

fn optional_reference(
    references: &BTreeMap<String, String>,
    controller_node_ref: Option<&str>,
) -> Option<String> {
    controller_node_ref.and_then(|reference| references.get(reference).cloned())
}

fn validate_accessibility_nodes(
    nodes: &[BrowserControllerAccessibilityNode],
) -> Result<(), String> {
    if nodes.len() > MAX_SNAPSHOT_NODES {
        return Err(
            "browser controller accessibility snapshot exceeded its node bound".to_string(),
        );
    }
    let mut references = BTreeSet::new();
    for node in nodes {
        validate_node_reference(&node.node_ref)?;
        if !references.insert(node.node_ref.as_str()) {
            return Err(format!(
                "browser controller accessibility snapshot duplicated {}",
                node.node_ref
            ));
        }
        validate_optional_reference(node.parent_ref.as_deref())?;
        for child_ref in &node.child_refs {
            validate_node_reference(child_ref)?;
        }
        for value in [&node.role, &node.name, &node.description, &node.value] {
            validate_bounded_string(value)?;
        }
    }
    Ok(())
}

fn validate_dom_nodes(nodes: &[BrowserControllerDomNode]) -> Result<(), String> {
    if nodes.len() > MAX_SNAPSHOT_NODES {
        return Err("browser controller DOM snapshot exceeded its node bound".to_string());
    }
    let mut references = BTreeSet::new();
    for node in nodes {
        validate_node_reference(&node.node_ref)?;
        if !references.insert(node.node_ref.as_str()) {
            return Err(format!(
                "browser controller DOM snapshot duplicated {}",
                node.node_ref
            ));
        }
        validate_optional_reference(node.parent_ref.as_deref())?;
        validate_bounded_string(&node.node_name)?;
        validate_bounded_string(&node.text)?;
        if node.attributes.len() > MAX_NODE_ATTRIBUTES {
            return Err(format!(
                "browser controller DOM node {} exceeded its attribute bound",
                node.node_ref
            ));
        }
        for (name, value) in &node.attributes {
            validate_bounded_string(name)?;
            validate_bounded_string(value)?;
        }
        if is_form_control(&node.node_name) {
            if let Some(value) = node.attributes.get("value") {
                if value != "[redacted]" {
                    return Err(format!(
                        "browser controller DOM node {} exposed a form value",
                        node.node_ref
                    ));
                }
            }
        }
        if let Some(bounds) = node.bounds {
            if ![bounds.x, bounds.y, bounds.width, bounds.height]
                .into_iter()
                .all(f64::is_finite)
                || bounds.width < 0.0
                || bounds.height < 0.0
            {
                return Err(format!(
                    "browser controller DOM node {} returned invalid bounds",
                    node.node_ref
                ));
            }
        }
    }
    Ok(())
}

fn validate_optional_reference(reference: Option<&str>) -> Result<(), String> {
    match reference {
        Some(reference) => validate_node_reference(reference),
        None => Ok(()),
    }
}

fn validate_node_reference(reference: &str) -> Result<(), String> {
    let Some(sequence) = reference.strip_prefix("backend:") else {
        return Err("browser controller snapshot returned an invalid node reference".to_string());
    };
    if sequence.is_empty() || !sequence.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("browser controller snapshot returned an invalid node reference".to_string());
    }
    Ok(())
}

fn validate_bounded_string(value: &str) -> Result<(), String> {
    if value.len() > MAX_SNAPSHOT_STRING_BYTES {
        return Err("browser controller snapshot exceeded its string bound".to_string());
    }
    Ok(())
}

fn is_form_control(node_name: &str) -> bool {
    matches!(
        node_name.to_ascii_lowercase().as_str(),
        "input" | "textarea" | "option"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_raw_form_values_and_invalid_bounds() {
        let mut snapshot = valid_snapshot();
        snapshot.dom_nodes[0]
            .attributes
            .insert("value".to_string(), "secret".to_string());
        assert!(snapshot.validate("target-a", "loader-a").is_err());

        let mut snapshot = valid_snapshot();
        snapshot.dom_nodes[0].attributes.clear();
        snapshot.dom_nodes[0].bounds = Some(BrowserControllerNodeBounds {
            x: 0.0,
            y: 0.0,
            width: f64::NAN,
            height: 10.0,
        });
        assert!(snapshot.validate("target-a", "loader-a").is_err());
    }

    fn valid_snapshot() -> BrowserControllerStructuredSnapshot {
        BrowserControllerStructuredSnapshot {
            browser_generation: 1,
            target_id: "target-a".to_string(),
            document_id: "loader-a".to_string(),
            snapshot_revision: 1,
            accessibility_nodes: Vec::new(),
            dom_nodes: vec![BrowserControllerDomNode {
                node_ref: "backend:1".to_string(),
                parent_ref: None,
                node_type: 1,
                node_name: "INPUT".to_string(),
                text: String::new(),
                attributes: BTreeMap::from([("value".to_string(), "[redacted]".to_string())]),
                bounds: None,
            }],
        }
    }
}
