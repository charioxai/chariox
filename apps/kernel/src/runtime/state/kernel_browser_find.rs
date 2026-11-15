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
        || !matches!(command.kind.as_str(), "field" | "button" | "link" | "any")
        || command.offset > 100_000
        || request
            .document_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        || command.offset > 0 && request.document_id.is_none()
    {
        return Err(host_error("find needs tab_id, generation, query (at most 512 characters), kind field/button/link/any, and the previous document_id beside command when continuing with offset".into()));
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
    let mut nodes = BTreeMap::<String, Value>::new();
    for key in ["accessibility_nodes", "dom_nodes"] {
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
            if node["bounds"].is_object() {
                entry["bounds"] = node["bounds"].clone();
            }
        }
    }
    let offset = find["offset"].as_u64().unwrap_or(0) as usize;
    let total = nodes.len();
    let matches: Vec<_> = nodes.into_values().skip(offset).take(PAGE_SIZE).collect();
    let next = (offset + matches.len() < total).then_some(offset + matches.len());
    Ok(
        json!({"tab_id":payload["tab_id"],"generation":payload["generation"],"document_id":document,
        "query":find["query"],"kind":requested,"matches":matches,"total_matches":total,"next_offset":next,
        "snapshot_truncated":snapshot["accessibility_truncated"] == true || snapshot["dom_truncated"] == true,"guidance":"Use node_ref for Vault fill, wait or scroll. Click uses screenshot CSS x,y, not target. To continue, repeat find with next_offset as offset and this document_id beside command; a changed page requires a fresh find."}),
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
}
