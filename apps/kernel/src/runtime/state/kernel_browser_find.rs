//! Bounded agent searches over the existing, admitted and masked browser snapshot.
use super::kernel_browser_runtime::host_error;
use crate::error::DaemonError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const PAGE_SIZE: usize = 8;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    command: Find,
    #[serde(default)]
    document_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Find {
    op: String,
    tab_id: String,
    generation: u64,
    query: String,
    kind: String,
    #[serde(default)]
    offset: usize,
}

pub(super) fn params(arguments: Value) -> Result<Value, DaemonError> {
    let request: Arguments = serde_json::from_value(arguments)
        .map_err(|error| host_error(super::kernel_browser_command_error::public_reason(&error)))?;
    let command = request.command;
    if command.op != "find"
        || command.tab_id.is_empty()
        || command.tab_id.len() > 256
        || command.generation == 0
        || command.query.chars().count() > 512
        || !matches!(command.kind.as_str(), "field" | "button" | "link" | "container" | "any")
        || command.offset > 100_000
        || request
            .document_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        || command.offset > 0 && request.document_id.is_none()
    {
        return Err(host_error("find needs tab_id, generation, query (at most 512 characters), kind field/button/link/container/any, and the previous document_id beside command when continuing with offset".into()));
    }
    Ok(
        json!({"op":"snapshot","tab_id":command.tab_id,"generation":command.generation,"focused_agent":true,
        "_find":{"query":command.query,"kind":command.kind,"offset":command.offset,"document_id":request.document_id}}),
    )
}

fn short(value: &Value, key: &str) -> String {
    let mut result = String::new();
    for character in value[key].as_str().unwrap_or_default().chars() {
        if result.len() + character.len_utf8() > 128 {
            break;
        }
        result.push(character);
    }
    result
}

fn kind(node: &Value) -> Option<&'static str> {
    match node["role"].as_str().unwrap_or_default() {
        "textbox" | "searchbox" | "combobox" | "spinbutton" | "checkbox" | "radio" | "switch" => {
            Some("field")
        }
        "button" | "menuitem" | "tab" => Some("button"),
        "link" => Some("link"),
        _ => match node["node_name"].as_str().unwrap_or_default() {
            "INPUT"
                if matches!(
                    node["attributes"]["type"].as_str(),
                    Some("submit" | "button" | "reset" | "image")
                ) =>
            {
                Some("button")
            }
            "INPUT" if node["attributes"]["type"] != "hidden" => Some("field"),
            "TEXTAREA" | "SELECT" => Some("field"),
            "BUTTON" => Some("button"),
            "A" if node["attributes"]["href"].is_string() => Some("link"),
            _ if node["attributes"]["contenteditable"] == "true" => Some("field"),
            _ => None,
        },
    }
}

/// DOM layout the accessibility tree lacks: bounds by node, parents, and each document's scroll.
struct Layout<'a> {
    by_ref: BTreeMap<&'a str, &'a Value>,
    /// Refs the accessibility tree names a button or link: a control is not a box.
    controls: std::collections::BTreeSet<&'a str>,
    scroll: Vec<(f64, f64)>,
}

impl<'a> Layout<'a> {
    fn new(snapshot: &'a Value) -> Self {
        let by_ref = snapshot["dom_nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|node| Some((node["node_ref"].as_str()?, node)))
            .collect();
        let scroll = snapshot["dom_documents"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|document| {
                (
                    document["scroll"]["x"].as_f64().unwrap_or(0.0),
                    document["scroll"]["y"].as_f64().unwrap_or(0.0),
                )
            })
            .collect();
        let controls = snapshot["accessibility_nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|node| matches!(kind(node), Some("button" | "link")))
            .filter_map(|node| node["node_ref"].as_str())
            .collect();
        Self {
            by_ref,
            controls,
            scroll,
        }
    }

    fn bounds(&self, reference: &str) -> Option<&'a Value> {
        self.by_ref
            .get(reference)
            .map(|node| &node["bounds"])
            .filter(|bounds| bounds.is_object())
    }

    /// The click point in viewport pixels for a node of the top document: its bounds' centre
    /// minus the scroll. Off-screen above or left of the viewport it is not clickable until the
    /// agent scrolls to the node (`in_viewport` false, no `click`).
    fn point(&self, reference: &str, entry: &mut Value) {
        let Some(node) = self.by_ref.get(reference) else {
            return;
        };
        let Some(bounds) = self.bounds(reference) else {
            return;
        };
        if node["document_index"].as_u64().unwrap_or(0) != 0 {
            return;
        }
        let (scroll_x, scroll_y) = self.scroll.first().copied().unwrap_or((0.0, 0.0));
        let number = |key: &str| bounds[key].as_f64().unwrap_or(0.0);
        let x = (number("x") - scroll_x + number("width") / 2.0).round();
        let y = (number("y") - scroll_y + number("height") / 2.0).round();
        let visible = x >= 0.0 && y >= 0.0;
        entry["in_viewport"] = visible.into();
        if visible {
            entry["click"] = json!({"x": x as i64, "y": y as i64});
        }
    }

    fn inside(&self, reference: &str, container: &str) -> bool {
        let mut current = self.by_ref.get(reference).and_then(|n| n["parent_ref"].as_str());
        for _ in 0..64 {
            match current {
                Some(parent) if parent == container => return true,
                Some(parent) => {
                    current = self.by_ref.get(parent).and_then(|n| n["parent_ref"].as_str());
                }
                None => return false,
            }
        }
        false
    }
}

const CONTAINER_TAGS: [&str; 12] = [
    "DIV", "SECTION", "FORM", "PRE", "CODE", "P", "SPAN", "TD", "LI", "ARTICLE", "DIALOG", "FIELDSET",
];
const CONTAINER_ROLES: [&str; 6] = ["group", "region", "form", "dialog", "article", "table"];
const MAX_CONTROLS: usize = 8;

/// A box (a token field, a form, a dialog) named by its id, class, test id, label or role, with
/// the buttons and links inside it. Carries no page text: a box's text is its whole subtree.
fn container(node: &Value, query: &str, layout: &Layout<'_>, snapshot: &Value) -> Option<Value> {
    let attributes = &node["attributes"];
    let reference = node["node_ref"].as_str()?;
    let named_role = attributes["role"]
        .as_str()
        .is_some_and(|role| CONTAINER_ROLES.contains(&role));
    if node["ignored"] == true
        || layout.controls.contains(reference)
        || matches!(attributes["role"].as_str(), Some("button" | "link"))
        || !(CONTAINER_TAGS.contains(&node["node_name"].as_str().unwrap_or_default()) || named_role)
        || !node["bounds"]["width"].as_f64().is_some_and(|w| w > 0.0)
    {
        return None;
    }
    let names = ["id", "class", "data-testid", "aria-label", "name", "role"];
    if !names
        .iter()
        .filter_map(|key| attributes[key].as_str())
        .any(|text| text.to_lowercase().contains(query))
        || query.is_empty() && !(attributes["id"].is_string() || attributes["data-testid"].is_string())
    {
        return None;
    }
    let mut entry = json!({"node_ref":reference,"kind":"container","bounds":node["bounds"]});
    for key in ["id", "class", "data-testid", "aria-label", "role"] {
        let text = short(attributes, key);
        if !text.is_empty() {
            entry["attributes"][key] = text.into();
        }
    }
    let mut controls = Vec::new();
    for control in snapshot["accessibility_nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(snapshot["dom_nodes"].as_array().into_iter().flatten())
    {
        let (Some(control_ref), Some(control_kind)) = (control["node_ref"].as_str(), kind(control))
        else {
            continue;
        };
        if control["ignored"] == true
            || !matches!(control_kind, "button" | "link")
            || !layout.inside(control_ref, reference)
            || controls.iter().any(|seen: &Value| seen["node_ref"] == control_ref)
        {
            continue;
        }
        let label = ["name", "text"]
            .iter()
            .map(|key| short(control, key))
            .chain(std::iter::once(short(&control["attributes"], "aria-label")))
            .find(|text| !text.is_empty())
            .unwrap_or_default();
        let lowered = label.to_lowercase();
        let action = if lowered.contains("copy") {
            "copy"
        } else if lowered.contains("hide") {
            "hide"
        } else if ["reveal", "show", "unmask", "display"].iter().any(|w| lowered.contains(w)) {
            "reveal"
        } else {
            "other"
        };
        let mut found = json!({"node_ref":control_ref,"kind":control_kind,"name":label,"action":action});
        if let Some(bounds) = layout.bounds(control_ref) {
            found["bounds"] = bounds.clone();
        }
        layout.point(control_ref, &mut found);
        controls.push(found);
        if controls.len() == MAX_CONTROLS {
            break;
        }
    }
    entry["controls"] = controls.into();
    layout.point(reference, &mut entry);
    Some(entry)
}

pub(super) fn project(payload: Value, find: Value) -> Result<Value, DaemonError> {
    let snapshot = &payload["snapshot"];
    let document = snapshot["document_id"].as_str().ok_or_else(|| {
        host_error("find received no browser document; observe the tab again".into())
    })?;
    if find["document_id"]
        .as_str()
        .is_some_and(|expected| expected != document)
    {
        return Err(host_error(
            "The page changed during find. Start a new find without offset or document_id.".into(),
        ));
    }
    let query = find["query"].as_str().unwrap_or_default().to_lowercase();
    let requested = find["kind"].as_str().unwrap_or("any");
    let layout = Layout::new(snapshot);
    let mut nodes = BTreeMap::<String, Value>::new();
    if requested == "container" {
        for node in snapshot["dom_nodes"].as_array().into_iter().flatten() {
            if let Some(entry) = container(node, &query, &layout, snapshot) {
                nodes.insert(entry["node_ref"].as_str().unwrap_or_default().to_string(), entry);
            }
        }
    }
    for key in ["accessibility_nodes", "dom_nodes"] {
        if requested == "container" {
            break;
        }
        for node in snapshot[key].as_array().into_iter().flatten() {
            let Some(candidate_kind) = kind(node) else {
                continue;
            };
            if node["ignored"] == true || requested != "any" && requested != candidate_kind {
                continue;
            }
            let Some(reference) = node["node_ref"].as_str() else {
                continue;
            };
            let attributes = &node["attributes"];
            // Values are deliberately absent: the snapshot has already crossed the secret observation guard.
            let searchable = ["name", "description", "text", "role"]
                .iter()
                .filter_map(|key| node[key].as_str())
                .chain(
                    ["id", "name", "placeholder", "type", "aria-label"]
                        .iter()
                        .filter_map(|key| attributes[key].as_str()),
                );
            if !searchable
                .into_iter()
                .any(|text| text.to_lowercase().contains(&query))
            {
                continue;
            }
            let entry = nodes
                .entry(reference.to_string())
                .or_insert_with(|| json!({"node_ref":reference,"kind":candidate_kind}));
            for key in ["name", "description", "role", "text"] {
                let text = short(node, key);
                if !text.is_empty() {
                    entry[key] = text.into();
                }
            }
            for key in ["id", "name", "placeholder", "type"] {
                let text = short(attributes, key);
                if !text.is_empty() {
                    entry["attributes"][key] = text.into();
                }
            }
        }
    }
    // Bounds live on the DOM node; the accessibility node of the same ref has none.
    for (reference, entry) in nodes.iter_mut() {
        if entry.get("bounds").is_none() {
            if let Some(bounds) = layout.bounds(reference) {
                entry["bounds"] = bounds.clone();
            }
        }
        layout.point(reference, entry);
    }
    let offset = find["offset"].as_u64().unwrap_or(0) as usize;
    let total = nodes.len();
    let matches: Vec<_> = nodes.into_values().skip(offset).take(PAGE_SIZE).collect();
    let next = (offset + matches.len() < total).then_some(offset + matches.len());
    Ok(
        json!({"tab_id":payload["tab_id"],"generation":payload["generation"],"document_id":document,
        "query":find["query"],"kind":requested,"matches":matches,"total_matches":total,"next_offset":next,
        "snapshot_truncated":snapshot["accessibility_truncated"] == true || snapshot["dom_truncated"] == true,"guidance":"Use node_ref for Vault fill, wait or scroll. Click uses CSS x,y, not target: a match's click is its centre in viewport pixels (kind container finds a box and the Copy/Reveal buttons inside it, each with node_ref and bounds). To continue, repeat find with next_offset as offset and this document_id beside command; a changed page requires a fresh find."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_reach_every_matching_control_and_refuse_a_changed_document() {
        let nodes: Vec<_> = (0..21).map(|i| json!({"node_ref":format!("backend:{}",i+1),"role":"textbox","name":format!("Card field {i}"),"value":"not part of a find result"})).collect();
        let snapshot = json!({"tab_id":"tab","generation":1,"snapshot":{"document_id":"document","accessibility_nodes":nodes}});
        let mut offset = 0;
        let mut references = std::collections::BTreeSet::new();
        loop {
            let page = project(
                snapshot.clone(),
                json!({"query":"CARD","kind":"field","offset":offset,"document_id":"document"}),
            )
            .unwrap();
            assert!(page.to_string().len() < 16 * 1024);
            assert!(!page.to_string().contains("not part of a find result"));
            for node in page["matches"].as_array().unwrap() {
                assert!(references.insert(node["node_ref"].as_str().unwrap().to_string()));
            }
            match page["next_offset"].as_u64() {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(references.len(), 21);
        assert!(project(
            snapshot,
            json!({"query":"","kind":"any","offset":8,"document_id":"old-document"})
        )
        .is_err());
    }

    #[test]
    fn find_validates_the_query_and_continuation_before_reading_the_browser() {
        let command = json!({"op":"find","tab_id":"tab","generation":1,"query":"","kind":"field"});
        assert_eq!(
            params(json!({"command":command})).unwrap()["op"],
            "snapshot"
        );
        for (key, value) in [
            ("query", json!("x".repeat(513))),
            ("kind", json!("script")),
            ("offset", json!(8)),
            ("_agent_input", json!(true)),
        ] {
            let mut invalid = command.clone();
            invalid[key] = value;
            assert!(params(json!({"command":invalid})).is_err());
        }
        let mut next = command;
        next["offset"] = 8.into();
        assert!(params(json!({"command":next,"document_id":"document"})).is_ok());
    }

    /// A shown-once token page: a box holding the token, with Copy and Reveal buttons inside it.
    /// The accessibility tree carries names and roles; only the DOM nodes carry layout bounds.
    fn token_page() -> Value {
        json!({"tab_id":"tab","generation":3,"snapshot":{"document_id":"document",
            "dom_documents":[{"document_index":0,"url":"https://dash.example.com/tokens","scroll":{"x":0,"y":500}}],
            "accessibility_nodes":[
                {"node_ref":"backend:20","role":"button","name":"Copy"},
                {"node_ref":"backend:21","role":"button","name":"Reveal token"},
                {"node_ref":"backend:30","role":"link","name":"Back to tokens"}],
            "dom_nodes":[
                {"node_ref":"backend:10","parent_ref":"backend:1","node_name":"DIV","text":"",
                 "attributes":{"id":"token-box","class":"token-card","data-testid":"api-token"},
                 "bounds":{"x":100.0,"y":800.0,"width":600.0,"height":48.0}},
                {"node_ref":"backend:20","parent_ref":"backend:10","node_name":"DIV","text":"Copy",
                 "attributes":{"role":"button"},"bounds":{"x":620.0,"y":806.0,"width":40.0,"height":32.0}},
                {"node_ref":"backend:21","parent_ref":"backend:10","node_name":"DIV","text":"",
                 "attributes":{"aria-label":"Reveal token"},"bounds":{"x":664.0,"y":806.0,"width":32.0,"height":32.0}},
                {"node_ref":"backend:30","parent_ref":"backend:1","node_name":"A","text":"Back",
                 "attributes":{"href":"/tokens"},"bounds":{"x":10.0,"y":10.0,"width":80.0,"height":20.0}}]}})
    }

    #[test]
    fn find_accepts_the_container_kind() {
        let command = json!({"op":"find","tab_id":"tab","generation":1,"query":"token","kind":"container"});
        assert!(params(json!({"command":command})).is_ok());
    }

    #[test]
    fn find_returns_the_token_box_with_its_copy_and_reveal_buttons() {
        let page = project(
            token_page(),
            json!({"query":"token","kind":"container","offset":0,"document_id":"document"}),
        )
        .unwrap();
        let boxes = page["matches"].as_array().unwrap();
        assert_eq!(boxes.len(), 1, "{page}");
        let token_box = &boxes[0];
        assert_eq!(token_box["node_ref"], "backend:10");
        assert_eq!(token_box["kind"], "container");
        assert_eq!(token_box["bounds"]["width"], 600.0);
        let controls = token_box["controls"].as_array().unwrap();
        let by_name = |name: &str| {
            controls
                .iter()
                .find(|c| c["name"].as_str().is_some_and(|n| n.contains(name)))
                .unwrap_or_else(|| panic!("{name} in {controls:?}"))
        };
        assert_eq!(by_name("Copy")["node_ref"], "backend:20");
        assert_eq!(by_name("Copy")["action"], "copy");
        assert_eq!(by_name("Reveal")["node_ref"], "backend:21");
        assert_eq!(by_name("Reveal")["action"], "reveal");
        assert!(by_name("Reveal")["bounds"]["x"].is_number());
        assert!(controls.iter().all(|c| c["node_ref"] != "backend:30"), "a link outside the box");
    }

    #[test]
    fn find_gives_bounds_to_a_control_the_accessibility_tree_names() {
        // Row 553 (2): the button is a DIV with a role; its bounds live on the DOM node.
        let page = project(
            token_page(),
            json!({"query":"copy","kind":"button","offset":0,"document_id":"document"}),
        )
        .unwrap();
        let copy = &page["matches"][0];
        assert_eq!(copy["node_ref"], "backend:20");
        assert_eq!(copy["bounds"]["x"], 620.0);
    }

    #[test]
    fn find_names_the_click_point_in_viewport_pixels_below_the_fold() {
        let page = project(
            token_page(),
            json!({"query":"copy","kind":"button","offset":0,"document_id":"document"}),
        )
        .unwrap();
        // Page y 806 + 16, scrolled 500 down: viewport y 322; x 620 + 20.
        assert_eq!(page["matches"][0]["click"], json!({"x":640,"y":322}));
        assert_eq!(page["matches"][0]["in_viewport"], true);
        let link = project(
            token_page(),
            json!({"query":"back","kind":"link","offset":0,"document_id":"document"}),
        )
        .unwrap();
        // Page y 10..30 is above a viewport scrolled 500 down: not clickable until scrolled to.
        assert_eq!(link["matches"][0]["in_viewport"], false);
        assert!(link["matches"][0].get("click").is_none());
    }

    #[test]
    fn find_container_results_name_the_box_but_carry_no_page_text() {
        let mut page = token_page();
        page["snapshot"]["dom_nodes"][0]["text"] = json!("cfut_SHOWN_ONCE_TOKEN_VALUE");
        let found = project(
            page,
            json!({"query":"token","kind":"container","offset":0,"document_id":"document"}),
        )
        .unwrap();
        // A box's text is its whole subtree: too big for a find page, and the agent reads the page
        // through snapshot or screenshot if it wants it. The ref, name and bounds are what it needs.
        assert!(!found.to_string().contains("SHOWN_ONCE"), "{found}");
        assert!(found["matches"][0]["attributes"]["id"] == "token-box");
    }
}
