use std::collections::{BTreeMap, BTreeSet};

use crate::error::DaemonError;
use crate::runtime::browser_controller_snapshot::{
    RoomBrowserAccessibilityNode, RoomBrowserDomNode, RoomBrowserStructuredSnapshot,
};

const MAX_CONTROLLER_BROWSER_TEXT_BYTES: usize = 256 * 1024;
const MAX_CONTROLLER_BROWSER_TEXT_QUERY_BYTES: usize = 64 * 1024;
// Provider tool-output caps start near 10k tokens (Codex); 24 KiB of JSON stays under
// that while a heavy page (Wikipedia, ~240 KB of links) would otherwise be refused.
const MAX_CONTROLLER_BROWSER_STATUS_BYTES: usize = 24 * 1024;
const MAX_CONTROLLER_BROWSER_STATUS_ITEM_CHARS: usize = 200;
const MAX_CONTROLLER_BROWSER_STATUS_TAB_BYTES: usize = 6 * 1024;
const BROWSER_STATUS_ELEMENT_KEYS: [&str; 3] = ["fields", "buttons", "links"];

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

/// Bounds a status payload to the tool-output cap. Elements are paged in
/// fields, buttons, links order; `page.next_cursor` continues where this stopped.
pub(super) fn bound_controller_browser_status(
    mut payload: serde_json::Value,
    cursor: usize,
) -> serde_json::Value {
    if cursor == 0 && payload.to_string().len() <= MAX_CONTROLLER_BROWSER_STATUS_BYTES {
        return payload;
    }
    let Some(browser) = payload
        .get_mut("browser")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return payload;
    };
    let mut totals = serde_json::Map::new();
    let mut elements = Vec::new();
    for key in BROWSER_STATUS_ELEMENT_KEYS {
        let items = match browser.insert(key.to_string(), serde_json::json!([])) {
            Some(serde_json::Value::Array(items)) => items,
            _ => Vec::new(),
        };
        totals.insert(key.to_string(), items.len().into());
        elements.extend(items.into_iter().map(|item| (key, item)));
    }
    let current_tab = payload.get("tab_id").cloned();
    // Titles, URLs and the tab inventory share the cap with the element pages.
    truncate_status_strings(&mut payload);
    if let Some(tabs) = payload
        .get_mut("tabs")
        .and_then(serde_json::Value::as_array_mut)
    {
        tabs.sort_by_key(|tab| tab.get("tab_id") != current_tab.as_ref());
        let (mut used, mut kept) = (0, Vec::new());
        let total_tabs = tabs.len();
        for tab in tabs.drain(..) {
            used += tab.to_string().len();
            if used <= MAX_CONTROLLER_BROWSER_STATUS_TAB_BYTES {
                kept.push(tab);
            }
        }
        if kept.len() < total_tabs {
            payload["tabs_omitted"] = (total_tabs - kept.len()).into();
        }
        payload["tabs"] = kept.into();
    }
    let total = elements.len();
    let mut used = payload.to_string().len() + 512;
    let mut next_cursor = cursor.min(total);
    for (key, mut item) in elements.into_iter().skip(cursor) {
        truncate_status_strings(&mut item);
        used += item.to_string().len() + 1;
        if used > MAX_CONTROLLER_BROWSER_STATUS_BYTES && next_cursor > cursor {
            break;
        }
        payload["browser"][key]
            .as_array_mut()
            .expect("status element lists were just inserted")
            .push(item);
        next_cursor += 1;
    }
    payload["page"] = serde_json::json!({
        "cursor": cursor,
        "total": totals,
        "next_cursor": (next_cursor < total).then_some(next_cursor),
    });
    // Never return an oversized tool result, even if a future metadata field
    // grows independently of the element pages. Keep the continuation handle.
    if payload.to_string().len() > MAX_CONTROLLER_BROWSER_STATUS_BYTES {
        payload = serde_json::json!({
            "session_id": payload["session_id"], "environment_id": payload["environment_id"],
            "tab_id": payload["tab_id"], "document_revision": payload["document_revision"],
            "browser": {"fields": payload["browser"]["fields"], "buttons": payload["browser"]["buttons"], "links": payload["browser"]["links"]},
            "page": payload["page"],
        });
    }
    payload
}

fn truncate_status_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            if let Some((end, _)) = text
                .char_indices()
                .nth(MAX_CONTROLLER_BROWSER_STATUS_ITEM_CHARS)
            {
                text.truncate(end);
                text.push('…');
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(truncate_status_strings),
        serde_json::Value::Object(map) => map.values_mut().for_each(truncate_status_strings),
        _ => {}
    }
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

pub(super) fn controller_browser_document_text(snapshot: &RoomBrowserStructuredSnapshot) -> String {
    // Script, style and template contents are page code, not document text.
    let code = snapshot
        .dom_nodes
        .iter()
        .filter(|node| {
            ["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE"]
                .iter()
                .any(|tag| node.node_name.eq_ignore_ascii_case(tag))
        })
        .map(|node| node.element_ref.as_str())
        .collect::<BTreeSet<_>>();
    let mut text = String::new();
    let mut seen = BTreeSet::new();
    for candidate in snapshot
        .dom_nodes
        .iter()
        .filter(|node| {
            !code.contains(node.element_ref.as_str())
                && !node
                    .parent_ref
                    .as_deref()
                    .is_some_and(|parent| code.contains(parent))
        })
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

    fn synthetic_status(link_count: usize, link_text: &str) -> serde_json::Value {
        let dom_nodes = (0..link_count)
            .map(|index| RoomBrowserDomNode {
                element_ref: format!("element-{index}"),
                parent_ref: None,
                document_index: 0,
                node_type: 1,
                node_name: "A".to_string(),
                text: format!("{link_text} {index}"),
                attributes: BTreeMap::from([("href".to_string(), format!("/wiki/{index}"))]),
                bounds: Some(
                    crate::runtime::browser_controller_snapshot::BrowserControllerNodeBounds {
                        x: 0.0,
                        y: 0.0,
                        width: 10.0,
                        height: 10.0,
                    },
                ),
            })
            .collect();
        let snapshot = RoomBrowserStructuredSnapshot {
            session_id: "s".to_string(),
            environment_id: "e".to_string(),
            runtime_generation: 1,
            browser_generation: 1,
            tab_id: "tab".to_string(),
            document_revision: 1,
            snapshot_revision: 1,
            accessibility_nodes: Vec::new(),
            accessibility_truncated: false,
            dom_documents: Vec::new(),
            shadow_roots: Vec::new(),
            dom_nodes,
        };
        let surfaces = controller_browser_status_surfaces(Some(&snapshot));
        serde_json::json!({
            "source": "browser_controller",
            "tab_id": "tab",
            "tabs": [{"tab_id": "tab", "url": "https://de.wikipedia.org/wiki/Hamburg"}],
            "browser": controller_browser_status_compatibility(
                "https://de.wikipedia.org/wiki/Hamburg",
                "de.wikipedia.org",
                "Hamburg",
                &surfaces,
            ),
        })
    }

    #[test]
    fn controller_browser_status_of_a_heavy_page_is_bounded_and_resumable() {
        let status = synthetic_status(2_000, &"Hamburg ".repeat(40));
        assert!(status.to_string().chars().count() > 200_000);

        let mut seen = Vec::new();
        let mut cursor = 0;
        loop {
            let page = bound_controller_browser_status(status.clone(), cursor);
            let text = page.to_string();
            assert!(text.chars().count() <= 24 * 1024, "{} chars", text.len());
            assert_eq!(page["page"]["total"]["links"], 2_000);
            seen.extend(
                page["browser"]["links"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|link| link["field_id"].as_str().unwrap().to_string()),
            );
            match page["page"]["next_cursor"].as_u64() {
                Some(next) => cursor = next as usize,
                None => break,
            }
        }
        let expected = (0..2_000)
            .map(|i| format!("element-{i}"))
            .collect::<Vec<_>>();
        assert_eq!(seen, expected);
    }

    #[test]
    fn controller_browser_status_bounds_oversized_titles_and_tab_inventories() {
        let long = "T".repeat(30_000);
        let mut status = synthetic_status(0, "Link");
        status["title"] = long.clone().into();
        status["browser"]["title"] = long.clone().into();
        status["browser"]["url"] = format!("https://example.test/{long}").into();
        let bounded = bound_controller_browser_status(status, 0);
        assert!(
            bounded.to_string().len() <= 24 * 1024,
            "{}",
            bounded.to_string().len()
        );
        assert!(bounded["page"]["next_cursor"].is_null());

        let mut single = synthetic_status(0, "Link");
        single["tabs"] = serde_json::json!([{"tab_id": "focused", "title": long, "url": long}]);
        single["tab_id"] = "focused".into();
        let bounded = bound_controller_browser_status(single, 0);
        assert!(bounded.to_string().len() <= MAX_CONTROLLER_BROWSER_STATUS_BYTES);
        assert_eq!(bounded["tabs"][0]["tab_id"], "focused");
        assert!(
            bounded["tabs"][0]["title"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 201
        );

        let mut status = synthetic_status(5, "Link");
        status["tabs"] = (0..200)
            .map(|i| serde_json::json!({"tab_id": format!("tab-{i}"), "title": long, "url": format!("https://example.test/{i}/{long}")}))
            .collect();
        status["tab_id"] = "tab-150".into();
        let bounded = bound_controller_browser_status(status, 0);
        assert!(
            bounded.to_string().len() <= 24 * 1024,
            "{}",
            bounded.to_string().len()
        );
        assert_eq!(bounded["tabs"][0]["tab_id"], "tab-150");
        assert_eq!(
            bounded["tabs_omitted"],
            200 - bounded["tabs"].as_array().unwrap().len()
        );
    }

    #[test]
    fn controller_browser_status_of_a_small_page_is_unchanged() {
        let status = synthetic_status(3, "Link");

        assert_eq!(bound_controller_browser_status(status.clone(), 0), status);
    }
}
