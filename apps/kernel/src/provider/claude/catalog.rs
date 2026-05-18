//! Claude provider catalog projection.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};

use super::{normalized_claude_model, resolve_claude_executable};

const CLAUDE_KNOWN_MODELS: &[(&str, &str)] = &[
    ("sonnet", "Claude Sonnet"),
    ("opus", "Claude Opus"),
    ("haiku", "Claude Haiku"),
    ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("claude-opus-4-7", "Claude Opus 4.7"),
];

pub fn claude_provider_catalog() -> OpenCodeProviderCatalog {
    let mut models = BTreeMap::new();
    for (id, name) in claude_catalog_model_entries() {
        models.insert(id.clone(), claude_model(&id, &name));
    }
    OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "claude".to_string(),
            name: "Claude Code".to_string(),
            remote_machine_aliases: Vec::new(),
            models,
        }],
        default: BTreeMap::from([("claude".to_string(), "sonnet".to_string())]),
        connected: if resolve_claude_executable().is_ok() {
            vec!["claude".to_string()]
        } else {
            Vec::new()
        },
    }
}

fn claude_catalog_model_entries() -> Vec<(String, String)> {
    let mut entries = CLAUDE_KNOWN_MODELS
        .iter()
        .map(|(id, name)| ((*id).to_string(), (*name).to_string()))
        .collect::<Vec<_>>();
    for (id, name) in claude_config_model_entries() {
        if !entries.iter().any(|(existing, _)| existing == &id) {
            entries.push((id, name));
        }
    }
    entries
}

fn claude_config_model_entries() -> Vec<(String, String)> {
    let Some(path) = claude_config_path() else {
        return Vec::new();
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    collect_claude_model_options(value.get("additionalModelOptionsCache"), &mut entries);
    collect_claude_model_cost_keys(value.get("additionalModelCostsCache"), &mut entries);
    entries
}

fn claude_config_path() -> Option<PathBuf> {
    env::var_os("ARROBA_CLAUDE_CONFIG")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude.json")))
}

fn collect_claude_model_options(
    value: Option<&serde_json::Value>,
    entries: &mut Vec<(String, String)>,
) {
    match value {
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                match item {
                    serde_json::Value::String(id) => push_claude_model_entry(entries, id, None),
                    serde_json::Value::Object(map) => {
                        let id = ["model", "id", "value"]
                            .iter()
                            .find_map(|key| map.get(*key).and_then(|value| value.as_str()));
                        let name = ["displayName", "display_name", "label", "name"]
                            .iter()
                            .find_map(|key| map.get(*key).and_then(|value| value.as_str()));
                        if let Some(id) = id {
                            push_claude_model_entry(entries, id, name);
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(serde_json::Value::Object(map)) => {
            for (id, metadata) in map {
                let name = metadata.as_object().and_then(|metadata| {
                    ["displayName", "display_name", "label", "name"]
                        .iter()
                        .find_map(|key| metadata.get(*key).and_then(|value| value.as_str()))
                });
                push_claude_model_entry(entries, id, name);
            }
        }
        _ => {}
    }
}

fn collect_claude_model_cost_keys(
    value: Option<&serde_json::Value>,
    entries: &mut Vec<(String, String)>,
) {
    if let Some(serde_json::Value::Object(map)) = value {
        for id in map.keys() {
            push_claude_model_entry(entries, id, None);
        }
    }
}

fn push_claude_model_entry(entries: &mut Vec<(String, String)>, id: &str, name: Option<&str>) {
    let id = normalized_claude_model(id);
    if !is_supported_claude_model_ref(&id) || entries.iter().any(|(existing, _)| existing == &id) {
        return;
    }
    let name = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| display_name_for_claude_model(&id));
    entries.push((id, name));
}

fn is_supported_claude_model_ref(id: &str) -> bool {
    matches!(id, "sonnet" | "opus" | "haiku") || id.starts_with("claude-")
}

fn display_name_for_claude_model(id: &str) -> String {
    let title = id
        .trim_start_matches("claude-")
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        id.to_string()
    } else {
        format!("Claude {title}")
    }
}

fn claude_model(id: &str, name: &str) -> OpenCodeProviderModel {
    OpenCodeProviderModel {
        id: id.to_string(),
        name: name.to_string(),
        status: "available".to_string(),
        limit: None,
        variants: BTreeMap::from([
            ("low".to_string(), serde_json::json!({ "name": "Low" })),
            (
                "medium".to_string(),
                serde_json::json!({ "name": "Medium" }),
            ),
            ("high".to_string(), serde_json::json!({ "name": "High" })),
            (
                "xhigh".to_string(),
                serde_json::json!({ "name": "Extra High" }),
            ),
            ("max".to_string(), serde_json::json!({ "name": "Max" })),
        ]),
    }
}
