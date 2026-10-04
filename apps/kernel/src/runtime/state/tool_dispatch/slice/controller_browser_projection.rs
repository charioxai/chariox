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
        let mut summary = browser_element_summary(kind, reference, accessibility_node, dom_node);
        if summary["text"].as_str().unwrap_or_default().is_empty() {
            let text = browser_descendant_text(snapshot, reference);
            summary["text"] = serde_json::Value::String(text.clone());
            if summary["label"].as_str().unwrap_or_default().is_empty() {
                summary["label"] = serde_json::Value::String(text);
            }
        }
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
        "diagnostic": matches.is_empty().then_some("No rendered target matched within bounded discovery; refresh, try kind=any, or inspect the shared Computer view."),
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

pub(super) fn controller_browser_tab_tool_result(
    slice_id: &str,
    agent_id: &str,
    action: crate::runtime::browser_controller_tab::BrowserTabAction,
    execution: crate::runtime::state::BrowserControllerActionExecution<
        crate::session::RoomEnvironmentSnapshot,
    >,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let environment = execution.value;
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "actor_id": execution.actor_id,
            "action_id": execution.action_id,
            "session_id": environment.session_id,
            "environment_id": environment.environment_id,
            "runtime_generation": environment.runtime_generation,
            "action": action.as_str(),
            "focused_tab_id": environment.focused_tab_id,
            "tabs": environment.tabs,
        }),
    }
}

pub(super) fn controller_browser_history_tool_result(
    slice_id: &str,
    agent_id: &str,
    tab_id: &str,
    action: crate::runtime::browser_controller_history::BrowserHistoryAction,
    execution: crate::runtime::state::BrowserControllerActionExecution<
        crate::session::RoomEnvironmentSnapshot,
    >,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let environment = execution.value;
    let tab = environment
        .tabs
        .iter()
        .find(|tab| tab.tab_id == tab_id)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "runtime_tool_slice_browser_history",
            message: "browser history result omitted its stable Room tab".to_string(),
        })?;
    let document_revision = tab.document_revision;
    let url = tab.url.clone();
    Ok(crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "actor_id": execution.actor_id,
            "action_id": execution.action_id,
            "session_id": environment.session_id,
            "environment_id": environment.environment_id,
            "runtime_generation": environment.runtime_generation,
            "action": action.as_str(),
            "tab_id": tab_id,
            "document_revision": document_revision,
            "url": url,
            "focused_tab_id": environment.focused_tab_id,
            "tabs": environment.tabs,
        }),
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

pub(super) fn controller_browser_events_tool_result(
    slice_id: &str,
    agent_id: &str,
    session_id: &str,
    batch: crate::runtime::browser_controller_event::RoomBrowserEventBatch,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let events = batch
        .events
        .into_iter()
        .map(|event| {
            serde_json::json!({
                "event_id": event.event_id,
                "kind": event.kind,
                "tab_id": event.tab_id,
                "document_id": event.document_id,
                "data": event.data,
            })
        })
        .collect::<Vec<_>>();
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: !batch.replay_gap,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "session_id": session_id,
            "browser_generation": batch.browser_generation,
            "events": events,
            "next_cursor": batch.next_cursor,
            "replay_gap": batch.replay_gap,
        }),
    }
}

pub(super) fn controller_browser_downloads_tool_result(
    slice_id: &str,
    agent_id: &str,
    result: crate::runtime::browser_controller_file_transfer::RoomBrowserDownloadsResult,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: result.enabled,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "session_id": result.session_id,
            "environment_id": result.environment_id,
            "runtime_generation": result.runtime_generation,
            "tab_id": result.tab_id,
            "document_revision": result.document_revision,
            "enabled": result.enabled,
        }),
    }
}

pub(super) fn controller_browser_upload_tool_result(
    slice_id: &str,
    agent_id: &str,
    result: crate::runtime::browser_controller_file_transfer::RoomBrowserUploadResult,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "session_id": result.session_id,
            "environment_id": result.environment_id,
            "runtime_generation": result.runtime_generation,
            "tab_id": result.tab_id,
            "document_revision": result.document_revision,
            "field_id": result.element_ref,
            "file_count": result.file_count,
            "total_bytes": result.total_bytes,
        }),
    }
}

pub(super) fn controller_browser_permission_tool_result(
    slice_id: &str,
    agent_id: &str,
    result: crate::runtime::browser_controller_permission::RoomBrowserPermissionResult,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller",
            "slice_id": slice_id,
            "agent_id": agent_id,
            "session_id": result.session_id,
            "environment_id": result.environment_id,
            "runtime_generation": result.runtime_generation,
            "tab_id": result.tab_id,
            "document_revision": result.document_revision,
            "permission": result.permission,
            "setting": result.setting,
        }),
    }
}

// DOM text is ordered by rendered node identity. AX names represent the same
// content and are deliberately not appended as a second document transcript.
pub(super) fn controller_browser_document_text(snapshot: &RoomBrowserStructuredSnapshot) -> String {
    let dom = snapshot
        .dom_nodes
        .iter()
        .map(|node| (node.element_ref.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut text = String::new();
    let mut previous = None;
    for node in snapshot.dom_nodes.iter().filter(|node| {
        node.rendered && (node.node_type == 3 || node.node_name.eq_ignore_ascii_case("br"))
    }) {
        let (block, cell, excluded) = browser_text_ancestry(node, &dom);
        if excluded {
            continue;
        }
        if node.node_name.eq_ignore_ascii_case("br") {
            if text.len() < MAX_CONTROLLER_BROWSER_TEXT_BYTES {
                text.push('\n');
            }
            continue;
        }
        if node.text.is_empty() {
            continue;
        }
        if let Some((last_block, last_cell)) = previous {
            if last_block != block {
                append_text_separator(&mut text, '\n');
            } else if last_cell != cell && cell.is_some() {
                append_text_separator(&mut text, '\t');
            }
        }
        let remaining = MAX_CONTROLLER_BROWSER_TEXT_BYTES.saturating_sub(text.len());
        let mut end = node.text.len().min(remaining);
        while !node.text.is_char_boundary(end) {
            end -= 1;
        }
        text.push_str(&node.text[..end]);
        previous = Some((block, cell));
        if text.len() >= MAX_CONTROLLER_BROWSER_TEXT_BYTES {
            break;
        }
    }
    text
}

fn append_text_separator(text: &mut String, separator: char) {
    if text.len() < MAX_CONTROLLER_BROWSER_TEXT_BYTES && !text.ends_with(separator) {
        text.push(separator);
    }
}

fn browser_text_ancestry<'a>(
    node: &'a RoomBrowserDomNode,
    dom: &BTreeMap<&'a str, &'a RoomBrowserDomNode>,
) -> (Option<&'a str>, Option<&'a str>, bool) {
    let mut parent = node.parent_ref.as_deref();
    let mut block = None;
    let mut cell = None;
    for _ in 0..dom.len() {
        let Some(ancestor) = parent.and_then(|reference| dom.get(reference)).copied() else {
            break;
        };
        match ancestor.node_name.to_ascii_uppercase().as_str() {
            "SCRIPT" | "STYLE" | "NOSCRIPT" | "TEMPLATE" | "INPUT" | "TEXTAREA" | "SELECT" => {
                return (block, cell, true)
            }
            "TD" | "TH" if cell.is_none() => cell = Some(ancestor.element_ref.as_str()),
            "P" | "DIV" | "LI" | "TR" | "PRE" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6"
            | "SECTION" | "ARTICLE" | "HEADER" | "FOOTER" | "BLOCKQUOTE"
                if block.is_none() =>
            {
                block = Some(ancestor.element_ref.as_str())
            }
            _ => {}
        }
        parent = ancestor.parent_ref.as_deref();
    }
    (block, cell, false)
}

fn browser_descendant_text(snapshot: &RoomBrowserStructuredSnapshot, reference: &str) -> String {
    let dom = snapshot
        .dom_nodes
        .iter()
        .map(|node| (node.element_ref.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut text = String::new();
    for node in snapshot
        .dom_nodes
        .iter()
        .filter(|node| node.node_type == 3 && node.rendered)
    {
        let mut parent = node.parent_ref.as_deref();
        for _ in 0..dom.len() {
            if parent == Some(reference) {
                let mut end = node.text.len().min(2048usize.saturating_sub(text.len()));
                while !node.text.is_char_boundary(end) {
                    end -= 1;
                }
                text.push_str(&node.text[..end]);
                break;
            }
            parent = parent
                .and_then(|reference| dom.get(reference))
                .and_then(|node| node.parent_ref.as_deref());
            if parent.is_none() {
                break;
            }
        }
        if text.len() >= 2048 {
            break;
        }
    }
    text.trim().to_string()
}

pub(super) fn controller_browser_text_page_tool_result(
    slice_id: &str,
    agent_id: &str,
    snapshot: &RoomBrowserStructuredSnapshot,
    page: &crate::runtime::browser_controller_snapshot::BrowserTextPage,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "source": "browser_controller", "slice_id": slice_id, "agent_id": agent_id,
            "session_id": snapshot.session_id, "environment_id": snapshot.environment_id,
            "runtime_generation": snapshot.runtime_generation, "tab_id": snapshot.tab_id,
            "document_revision": snapshot.document_revision, "snapshot_revision": snapshot.snapshot_revision,
            "text": page.text, "offset": page.offset, "next_offset": page.next_offset,
            "total_bytes": page.total_bytes,
        }),
    }
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

    fn room_snapshot(nodes: serde_json::Value) -> RoomBrowserStructuredSnapshot {
        let controller: crate::runtime::browser_controller_snapshot::BrowserControllerStructuredSnapshot = serde_json::from_value(serde_json::json!({
            "browser_generation": 1, "target_id": "target", "document_id": "document", "snapshot_revision": 1,
            "accessibility_nodes": [], "dom_documents": [{"document_index": 0, "url": "http://fixture.test", "owner_node_ref": null}], "dom_nodes": nodes,
        })).unwrap();
        let references = controller
            .controller_node_refs()
            .into_iter()
            .enumerate()
            .map(|(i, reference)| (reference, format!("element-{i}")))
            .collect();
        controller
            .into_room_snapshot(
                "room".into(),
                "environment".into(),
                1,
                "tab".into(),
                1,
                &references,
            )
            .unwrap()
    }

    fn node(id: u64, parent: Option<u64>, tag: &str, text: &str) -> serde_json::Value {
        serde_json::json!({"node_ref": format!("backend:{id}"), "parent_ref": parent.map(|id| format!("backend:{id}")), "document_index": 0,
            "node_type": if tag == "#text" {3} else {1}, "node_name": tag, "text": text, "attributes": {}, "rendered": true, "clickable": false,
            "bounds": {"x": 10, "y": 10, "width": 100, "height": 30}})
    }

    #[test]
    fn mp08_p1_generic_clickable_and_custom_options_have_opaque_targets() {
        let mut clickable = node(1, None, "SPAN", "");
        clickable["clickable"] = true.into();
        let mut option = node(3, None, "DIV", "");
        option["attributes"] = serde_json::json!({"role":"option"});
        let mut hidden = node(5, None, "DIV", "");
        hidden["clickable"] = true.into();
        hidden["rendered"] = false.into();
        let snapshot = room_snapshot(serde_json::json!([
            clickable,
            node(2, Some(1), "#text", "Listener choice"),
            option,
            node(4, Some(3), "#text", "Custom choice"),
            hidden,
            node(6, None, "SPAN", "Inert")
        ]));
        let status = serde_json::Value::Object(controller_browser_status_surfaces(Some(&snapshot)));
        for query in ["Listener choice", "Custom choice"] {
            let found = controller_browser_find(&status, query, "any").unwrap();
            assert_eq!(found["matches"].as_array().unwrap().len(), 1);
            assert!(found["matches"][0]["field_id"]
                .as_str()
                .unwrap()
                .starts_with("element-"));
            assert!(found["matches"][0]["selector"].is_null());
        }
        assert_eq!(status["buttons"].as_array().unwrap().len(), 2);
        assert_eq!(
            controller_browser_find(&status, "Inert", "any").unwrap()["matches"],
            serde_json::json!([])
        );
    }

    #[test]
    fn mp08_p3_rendered_text_preserves_rows_and_people_without_ax_duplication() {
        let mut hidden = node(20, None, "#text", "hidden");
        hidden["rendered"] = false.into();
        let mut snapshot = room_snapshot(serde_json::json!([
            node(1, None, "LI", ""),
            node(2, Some(1), "#text", "Same person"),
            node(3, None, "LI", ""),
            node(4, Some(3), "#text", "Same person"),
            node(5, None, "TR", ""),
            node(6, Some(5), "TD", ""),
            node(7, Some(6), "#text", "Monday"),
            node(8, Some(5), "TD", ""),
            node(9, Some(8), "#text", "7"),
            node(10, None, "TR", ""),
            node(11, Some(10), "TD", ""),
            node(12, Some(11), "#text", "Tuesday"),
            node(13, Some(10), "TD", ""),
            node(14, Some(13), "#text", "7"),
            node(15, None, "SCRIPT", ""),
            node(16, Some(15), "#text", "pollution"),
            node(17, None, "PRE", ""),
            node(18, Some(17), "#text", "Visible code\n  indented"),
            hidden
        ]));
        snapshot
            .accessibility_nodes
            .push(RoomBrowserAccessibilityNode {
                element_ref: "element-avatar".into(),
                parent_ref: None,
                child_refs: vec![],
                role: "img".into(),
                name: "Same person".into(),
                description: "".into(),
                value: "".into(),
                ignored: false,
                disabled: false,
                focused: false,
                states: vec![],
            });
        assert_eq!(
            controller_browser_document_text(&snapshot),
            "Same person\nSame person\nMonday\t7\nTuesday\t7\nVisible code\n  indented"
        );
    }

    #[test]
    fn mp08_p3_exact_text_preserves_edge_newlines_and_breaks() {
        let snapshot = room_snapshot(serde_json::json!([
            node(1, None, "PRE", ""),
            node(2, Some(1), "#text", "\n  indented\n\n"),
            node(3, None, "P", ""),
            node(4, Some(3), "#text", "before"),
            node(5, Some(3), "BR", ""),
            node(6, Some(3), "BR", ""),
            node(7, Some(3), "#text", "after\n"),
        ]));
        assert_eq!(
            controller_browser_document_text(&snapshot),
            "\n  indented\n\nbefore\n\nafter\n"
        );
    }

    #[test]
    fn mp08_p2_selected_value_and_option_states_are_observable() {
        let mut snapshot = room_snapshot(serde_json::json!([
            node(1, None, "SELECT", ""),
            node(2, None, "DIV", "")
        ]));
        for (index, role, value, states) in [
            (0, "combobox", "日本", vec![]),
            (1, "option", "", vec!["selected".to_string()]),
        ] {
            snapshot
                .accessibility_nodes
                .push(RoomBrowserAccessibilityNode {
                    element_ref: snapshot.dom_nodes[index].element_ref.clone(),
                    parent_ref: None,
                    child_refs: vec![],
                    role: role.into(),
                    name: "Choice".into(),
                    description: "".into(),
                    value: value.into(),
                    ignored: false,
                    disabled: false,
                    focused: false,
                    states,
                });
        }
        let status = serde_json::Value::Object(controller_browser_status_surfaces(Some(&snapshot)));
        assert_eq!(status["fields"][0]["value"], "日本");
        assert_eq!(
            status["buttons"][0]["states"],
            serde_json::json!(["selected"])
        );
    }

    #[test]
    fn mp08_p3_escaped_page_stays_valid_through_actual_provider_history_bounds() {
        let snapshot = room_snapshot(serde_json::json!([]));
        let page = crate::runtime::browser_controller_snapshot::BrowserTextPage {
            text: "\u{1}".repeat(1024),
            offset: 0,
            next_offset: Some(1024),
            total_bytes: 2048,
            query: Some("\u{1}".repeat(2048)),
        };
        let result = controller_browser_text_page_tool_result("slice", "agent", &snapshot, &page);
        let record = serde_json::json!({"id":"call", "tool":"slice_browser_text", "status":"completed", "output":serde_json::to_string(&result.payload).unwrap()});
        let bytes = serde_json::to_vec(&record).unwrap();
        let bounded = crate::provider_output_policy::output_bounds::bounded_terminal_output_bytes(
            &crate::terminal::TerminalOutputKind::ProviderTool,
            &bytes,
        );
        assert_eq!(
            bounded, bytes,
            "a complete page must survive JSON escaping without head/tail cuts"
        );
        let decoded: serde_json::Value = serde_json::from_slice(&bounded).unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(decoded["output"].as_str().unwrap()).unwrap();
        assert_eq!(payload["text"].as_str().unwrap(), page.text);
        assert_eq!(payload["next_offset"], 1024);
    }

    #[test]
    fn mp08_p3_text_bound_never_splits_utf8() {
        let snapshot = room_snapshot(serde_json::json!([node(
            1,
            None,
            "#text",
            &"😀".repeat(MAX_CONTROLLER_BROWSER_TEXT_BYTES)
        )]));
        let text = controller_browser_document_text(&snapshot);
        assert_eq!(text.len(), MAX_CONTROLLER_BROWSER_TEXT_BYTES);
        assert!(std::str::from_utf8(text.as_bytes()).is_ok());
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
    if matches!(role.as_str(), "button" | "option" | "menuitem" | "tab")
        || tag == "button"
        || tag == "input" && matches!(input_type.as_str(), "button" | "submit" | "reset" | "image")
    {
        return Some("button");
    }
    if role == "link" || tag == "a" && dom.is_some_and(|node| node.attributes.contains_key("href"))
    {
        return Some("link");
    }
    dom.filter(|node| node.node_type == 1 && node.clickable)
        .map(|_| "button")
}

fn browser_element_visible(
    accessibility: Option<&RoomBrowserAccessibilityNode>,
    dom: Option<&RoomBrowserDomNode>,
) -> bool {
    match dom {
        Some(node) => {
            node.rendered
                && node
                    .bounds
                    .is_some_and(|bounds| bounds.width > 0.0 && bounds.height > 0.0)
        }
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
        "states": accessibility.map(|node| node.states.as_slice()).unwrap_or_default(),
        "value": if matches!(role.to_ascii_lowercase().as_str(), "combobox" | "listbox") || dom.is_some_and(|node| node.node_name.eq_ignore_ascii_case("select")) {
            accessibility.map(|node| node.value.as_str()).unwrap_or_default()
        } else { "" },
    })
}
