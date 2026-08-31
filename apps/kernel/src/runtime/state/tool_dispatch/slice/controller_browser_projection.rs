use std::collections::{BTreeMap, BTreeSet};

use crate::error::DaemonError;
use crate::runtime::browser_controller_snapshot::{
    RoomBrowserAccessibilityNode, RoomBrowserDomNode, RoomBrowserStructuredSnapshot,
};

const MAX_CONTROLLER_BROWSER_TEXT_BYTES: usize = 256 * 1024;
const MAX_CONTROLLER_BROWSER_TEXT_QUERY_BYTES: usize = 64 * 1024;
pub(super) fn controller_browser_status_surfaces(
    snapshot: Option<&RoomBrowserStructuredSnapshot>,
) -> serde_json::Map<String, serde_json::Value> {
    let Some(snapshot) = snapshot else {
        return empty_surfaces();
    };
    let accessibility = snapshot
        .accessibility_nodes
        .iter()
        .map(|node| (node.element_ref.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let dom = snapshot
        .dom_nodes
        .iter()
        .map(|node| (node.element_ref.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut references = Vec::new();
    let mut seen = BTreeSet::new();
    for reference in snapshot
        .accessibility_nodes
        .iter()
        .map(|node| node.element_ref.as_str())
        .chain(
            snapshot
                .dom_nodes
                .iter()
                .map(|node| node.element_ref.as_str()),
        )
    {
        if seen.insert(reference) {
            references.push(reference);
        }
    }

    let mut fields = Vec::new();
    let mut buttons = Vec::new();
    let mut links = Vec::new();
    let mut focused_element = serde_json::Value::Null;
    for reference in references {
        let accessibility_node = accessibility.get(reference).copied();
        let dom_node = dom.get(reference).copied();
        if accessibility_node.is_some_and(|node| node.ignored) && dom_node.is_none() {
            continue;
        }
        if !browser_element_visible(accessibility_node, dom_node) {
            continue;
        }
        let Some(kind) = browser_element_kind(accessibility_node, dom_node) else {
            if accessibility_node.is_some_and(|node| node.focused) {
                focused_element =
                    browser_element_summary("element", reference, accessibility_node, dom_node);
            }
            continue;
        };
        let summary = browser_element_summary(kind, reference, accessibility_node, dom_node);
        if accessibility_node.is_some_and(|node| node.focused) {
            focused_element = summary.clone();
        }
        match kind {
            "field" => fields.push(summary),
            "button" => buttons.push(summary),
            "link" => links.push(summary),
            _ => {}
        }
    }

    serde_json::Map::from_iter([
        ("fields".to_string(), serde_json::Value::Array(fields)),
        ("buttons".to_string(), serde_json::Value::Array(buttons)),
        ("links".to_string(), serde_json::Value::Array(links)),
        ("focusedElement".to_string(), focused_element),
        (
            "snapshot_revision".to_string(),
            serde_json::Value::from(snapshot.snapshot_revision),
        ),
    ])
}

pub(super) fn controller_browser_status_compatibility(
    url: &str,
    host: &str,
    title: &str,
    surfaces: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "url": url,
        "host": host,
        "title": title,
        "focusedElement": surfaces.get("focusedElement").cloned().unwrap_or(serde_json::Value::Null),
        "fields": surfaces.get("fields").cloned().unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        "buttons": surfaces.get("buttons").cloned().unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        "links": surfaces.get("links").cloned().unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        "snapshot_revision": surfaces.get("snapshot_revision").cloned().unwrap_or(serde_json::Value::Null),
    })
}

pub(super) fn controller_browser_find(
    status: &serde_json::Value,
    query: &str,
    kind: &str,
) -> Result<serde_json::Value, String> {
    if !matches!(kind, "field" | "button" | "link" | "any") {
        return Err(format!("unsupported browser element kind `{kind}`"));
    }
    let query_lower = query.to_ascii_lowercase();
    let mut matches = Vec::new();
    for (candidate_kind, key) in [
        ("field", "fields"),
        ("button", "buttons"),
        ("link", "links"),
    ] {
        if kind != "any" && kind != candidate_kind {
            continue;
        }
        let Some(candidates) = status.get(key).and_then(serde_json::Value::as_array) else {
            continue;
        };
        matches.extend(
            candidates
                .iter()
                .filter(|candidate| {
                    [
                        "selector",
                        "id",
                        "name",
                        "role",
                        "label",
                        "placeholder",
                        "text",
                        "type",
                    ]
                    .into_iter()
                    .filter_map(|key| candidate.get(key).and_then(serde_json::Value::as_str))
                    .any(|value| value.to_ascii_lowercase().contains(&query_lower))
                })
                .cloned(),
        );
    }
    Ok(serde_json::json!({
        "query": query,
        "kind": kind,
        "matches": matches,
    }))
}

pub(super) fn controller_browser_element_ref(
    selector: Option<&str>,
    field_id: Option<&str>,
    operation: &'static str,
) -> Result<String, DaemonError> {
    let reference = field_id
        .or_else(|| selector.filter(|value| value.starts_with("element-")))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    reference.map(str::to_string).ok_or_else(|| DaemonError::LocalTransport {
        operation,
        message: "controller-backed browser actions require an opaque field_id returned by slice_browser_status or slice_browser_find".to_string(),
    })
}

pub(super) fn controller_browser_action_tool_result(
    slice_id: &str,
    agent_id: &str,
    execution: crate::runtime::state::BrowserControllerActionExecution<
        crate::runtime::browser_controller_action::RoomBrowserActionResult,
    >,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let result = execution.value;
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "actor_id": execution.actor_id,
            "action_id": execution.action_id,
            "session_id": result.session_id,
            "environment_id": result.environment_id,
            "runtime_generation": result.runtime_generation,
            "tab_id": result.tab_id,
            "document_revision": result.document_revision,
            "browser": {
                "ok": true,
                "field_id": result.element_ref,
                "selector": serde_json::Value::Null,
                "action_kind": result.action_kind,
                "dialog_opened": result.dialog_opened,
                "attempts": result.attempts,
                "elapsed_ms": result.elapsed_ms,
            },
        }),
    }
}

pub(super) fn controller_secret_paste_tool_result(
    slice_id: &str,
    agent_id: &str,
    credential_id: &str,
    submitted: bool,
    execution: crate::runtime::state::BrowserControllerActionExecution<
        crate::runtime::browser_controller_action::RoomBrowserActionResult,
    >,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let result = execution.value;
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "actor_id": execution.actor_id,
            "action_id": execution.action_id,
            "credential_id": credential_id,
            "submitted": submitted,
            "session_id": result.session_id,
            "environment_id": result.environment_id,
            "runtime_generation": result.runtime_generation,
            "tab_id": result.tab_id,
            "document_revision": result.document_revision,
            "browser": {
                "ok": true,
                "action_kind": result.action_kind,
                "attempts": result.attempts,
                "elapsed_ms": result.elapsed_ms,
            },
        }),
    }
}

pub(super) fn controller_browser_dialog_tool_result(
    slice_id: &str,
    agent_id: &str,
    execution: crate::runtime::state::BrowserControllerActionExecution<
        crate::runtime::browser_controller_action::RoomBrowserDialogResult,
    >,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let result = execution.value;
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "actor_id": execution.actor_id,
            "action_id": execution.action_id,
            "session_id": result.session_id,
            "environment_id": result.environment_id,
            "runtime_generation": result.runtime_generation,
            "tab_id": result.tab_id,
            "document_revision": result.document_revision,
            "browser": {
                "ok": true,
                "action": result.action,
            },
        }),
    }
}

pub(super) fn controller_browser_document_text(snapshot: &RoomBrowserStructuredSnapshot) -> String {
    let mut text = String::new();
    let mut seen = BTreeSet::new();
    for candidate in snapshot
        .dom_nodes
        .iter()
        .map(|node| node.text.as_str())
        .chain(
            snapshot
                .accessibility_nodes
                .iter()
                .filter(|node| !node.ignored)
                .flat_map(|node| [node.name.as_str(), node.description.as_str()]),
        )
    {
        let candidate = candidate.trim();
        if candidate.is_empty() || !seen.insert(candidate) {
            continue;
        }
        append_bounded_text(&mut text, candidate);
        if text.len() >= MAX_CONTROLLER_BROWSER_TEXT_BYTES {
            break;
        }
    }
    text
}

pub(super) fn validate_controller_browser_text_query(query: &str) -> Result<(), String> {
    if query.len() > MAX_CONTROLLER_BROWSER_TEXT_QUERY_BYTES {
        return Err(format!(
            "browser text query exceeds {MAX_CONTROLLER_BROWSER_TEXT_QUERY_BYTES} UTF-8 bytes"
        ));
    }
    Ok(())
}

pub(super) fn controller_browser_wait_for_text_result(
    slice_id: &str,
    agent_id: &str,
    environment: crate::session::RoomEnvironmentSnapshot,
    query: &str,
    waited_ms: u64,
    matched: bool,
    timeout_ms: u64,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: matched,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "session_id": environment.session_id,
            "environment_id": environment.environment_id,
            "runtime_generation": environment.runtime_generation,
            "tab_id": environment.focused_tab_id,
            "browser": {
                "ok": matched,
                "text": query,
                "waited_ms": waited_ms,
                "timeout_ms": timeout_ms,
                "error": (!matched).then_some("timeout"),
            },
        }),
    }
}

fn append_bounded_text(output: &mut String, candidate: &str) {
    let separator_bytes = usize::from(!output.is_empty());
    let remaining = MAX_CONTROLLER_BROWSER_TEXT_BYTES
        .saturating_sub(output.len())
        .saturating_sub(separator_bytes);
    if remaining == 0 {
        return;
    }
    let mut end = candidate.len().min(remaining);
    while !candidate.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return;
    }
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(&candidate[..end]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_browser_find_preserves_opaque_references_and_kind_filtering() {
        let status = serde_json::json!({
            "fields": [{"field_id": "element-1", "label": "Email", "text": ""}],
            "buttons": [{"field_id": "element-2", "label": "Continue", "text": "Continue"}],
            "links": [{"field_id": "element-3", "label": "Help", "text": "Help"}],
        });

        let result = controller_browser_find(&status, "cont", "button")
            .expect("supported kind should return matches");

        assert_eq!(result["matches"][0]["field_id"], "element-2");
        assert_eq!(result["matches"].as_array().map(Vec::len), Some(1));
        assert!(controller_browser_find(&status, "", "image").is_err());
    }

    #[test]
    fn controller_browser_action_requires_a_kernel_issued_element_reference() {
        assert_eq!(
            controller_browser_element_ref(None, Some("element-7"), "test")
                .expect("opaque reference should be accepted"),
            "element-7"
        );
        assert!(controller_browser_element_ref(Some("#password"), None, "test").is_err());
    }

    #[test]
    fn controller_browser_text_bound_never_splits_utf8() {
        let mut output = "a".repeat(MAX_CONTROLLER_BROWSER_TEXT_BYTES - 2);

        append_bounded_text(&mut output, "😀");

        assert!(output.len() <= MAX_CONTROLLER_BROWSER_TEXT_BYTES);
        assert!(std::str::from_utf8(output.as_bytes()).is_ok());
        assert!(!output.ends_with('\n'));
    }
}

fn empty_surfaces() -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::from_iter([
        ("fields".to_string(), serde_json::Value::Array(Vec::new())),
        ("buttons".to_string(), serde_json::Value::Array(Vec::new())),
        ("links".to_string(), serde_json::Value::Array(Vec::new())),
        ("focusedElement".to_string(), serde_json::Value::Null),
    ])
}

fn browser_element_kind(
    accessibility: Option<&RoomBrowserAccessibilityNode>,
    dom: Option<&RoomBrowserDomNode>,
) -> Option<&'static str> {
    let role = accessibility
        .map(|node| node.role.as_str())
        .or_else(|| dom.and_then(|node| node.attributes.get("role").map(String::as_str)))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let tag = dom
        .map(|node| node.node_name.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let input_type = dom
        .and_then(|node| node.attributes.get("type"))
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(
        role.as_str(),
        "textbox"
            | "searchbox"
            | "combobox"
            | "checkbox"
            | "radio"
            | "switch"
            | "slider"
            | "spinbutton"
    ) || matches!(tag.as_str(), "textarea" | "select")
        || tag == "input" && !matches!(input_type.as_str(), "button" | "submit" | "reset" | "image")
        || dom.is_some_and(|node| {
            node.attributes
                .get("contenteditable")
                .is_some_and(|value| value != "false")
        })
    {
        return Some("field");
    }
    if role == "button"
        || tag == "button"
        || tag == "input" && matches!(input_type.as_str(), "button" | "submit" | "reset" | "image")
    {
        return Some("button");
    }
    if role == "link" || tag == "a" && dom.is_some_and(|node| node.attributes.contains_key("href"))
    {
        return Some("link");
    }
    None
}

fn browser_element_visible(
    accessibility: Option<&RoomBrowserAccessibilityNode>,
    dom: Option<&RoomBrowserDomNode>,
) -> bool {
    match dom {
        Some(node) => node
            .bounds
            .is_some_and(|bounds| bounds.width > 0.0 && bounds.height > 0.0),
        None => accessibility.is_some_and(|node| !node.ignored),
    }
}

fn browser_element_summary(
    kind: &str,
    element_ref: &str,
    accessibility: Option<&RoomBrowserAccessibilityNode>,
    dom: Option<&RoomBrowserDomNode>,
) -> serde_json::Value {
    let attribute = |name: &str| {
        dom.and_then(|node| node.attributes.get(name))
            .map(String::as_str)
            .unwrap_or_default()
    };
    let role = accessibility
        .map(|node| node.role.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| attribute("role"));
    let label = accessibility
        .map(|node| node.name.as_str())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let label = attribute("aria-label");
            (!label.is_empty()).then_some(label)
        })
        .or_else(|| {
            let placeholder = attribute("placeholder");
            (!placeholder.is_empty()).then_some(placeholder)
        })
        .unwrap_or_default();
    let text = dom
        .map(|node| node.text.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(label);
    let disabled = accessibility.is_some_and(|node| node.disabled)
        || dom.is_some_and(|node| node.attributes.contains_key("disabled"));
    let read_only = dom.is_some_and(|node| {
        node.attributes.contains_key("readonly")
            || node
                .attributes
                .get("aria-readonly")
                .is_some_and(|value| value == "true")
    });

    serde_json::json!({
        "kind": kind,
        "selector": serde_json::Value::Null,
        "field_id": element_ref,
        "tag": dom.map(|node| node.node_name.to_ascii_lowercase()).unwrap_or_default(),
        "type": attribute("type"),
        "name": attribute("name"),
        "id": attribute("id"),
        "role": role,
        "label": label,
        "placeholder": attribute("placeholder"),
        "text": text,
        "disabled": disabled,
        "readOnly": read_only,
    })
}
