//! Claude provider catalog projection.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};

use super::launch_args::normalized_claude_model;
use super::resolve_claude_executable;

pub(crate) const CLAUDE_HEADLESS_PROVIDER_ID: &str = "claude-headless";
pub(crate) const CLAUDE_PRINT_PROVIDER_ID: &str = "claude-p";

// Offline mirror of the official Claude Code model names (2.1.296).
// The live catalog below replaces this with the installed harness's picker.
const CLAUDE_KNOWN_MODELS: &[(&str, &str)] = &[
    ("claude-haiku-4-5", "Haiku 4.5"),
    ("claude-haiku-5-5", "Haiku 5.5"),
    ("claude-sonnet-4-6", "Sonnet 4.6"),
    ("claude-sonnet-5", "Sonnet 5"),
    ("claude-sonnet-5-5", "Sonnet 5.5"),
    ("claude-opus-4-7", "Opus 4.7"),
    ("claude-opus-4-8", "Opus 4.8"),
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-fable-5", "Fable 5"),
    ("claude-fable-5-1", "Fable 5.1"),
];

pub fn claude_provider_catalog() -> OpenCodeProviderCatalog {
    let mut models = BTreeMap::new();
    for (id, name) in claude_catalog_model_entries() {
        models.insert(id.clone(), claude_model(&id, &name));
    }
    let providers = [
        (CLAUDE_HEADLESS_PROVIDER_ID, "Claude headless"),
        (CLAUDE_PRINT_PROVIDER_ID, "Claude -p"),
    ];
    let connected = if resolve_claude_executable().is_ok() {
        providers.iter().map(|(id, _)| (*id).to_string()).collect()
    } else {
        Vec::new()
    };
    OpenCodeProviderCatalog {
        all: providers
            .iter()
            .map(|(id, name)| OpenCodeProviderInfo {
                id: (*id).to_string(),
                name: (*name).to_string(),
                remote_machine_aliases: Vec::new(),
                models: models.clone(),
            })
            .collect(),
        default: providers
            .iter()
            .map(|(id, _)| ((*id).to_string(), "claude-sonnet-5-5".to_string()))
            .collect(),
        connected,
    }
}

/// Use the selected product profile, never an ambient provider login.
pub(crate) fn claude_provider_catalog_with_environment(
    environment: &BTreeMap<String, String>,
) -> OpenCodeProviderCatalog {
    let fallback = claude_provider_catalog();
    let Ok(executable) = resolve_claude_executable() else {
        return fallback;
    };
    match super::usage_probe::probe_claude_models(&executable, environment)
        .ok()
        .and_then(|models| catalog_from_native_models(&models))
    {
        Some(catalog) => catalog,
        None => {
            crate::logging::warn_with_fields(
                "daemon.provider.claude",
                "Native model metadata unavailable; using the offline native model list",
                serde_json::json!({}),
            );
            fallback
        }
    }
}

fn catalog_from_native_models(value: &serde_json::Value) -> Option<OpenCodeProviderCatalog> {
    let mut models = BTreeMap::new();
    let mut default = None;
    for option in value.as_array()? {
        let native_value = option.get("value").and_then(serde_json::Value::as_str)?;
        let id = option
            .get("resolvedModel")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(native_value);
        if !is_supported_claude_model_ref(id) {
            continue;
        }
        if native_value == "default" {
            default = Some(id.to_string());
            continue;
        }
        let name = option
            .get("displayName")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .unwrap_or(id);
        let mut model = claude_model(id, name);
        model.variants = option
            .get("supportedEffortLevels")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(|effort| (effort.to_string(), serde_json::json!({})))
            .collect();
        models.insert(id.to_string(), model);
    }
    if models.is_empty() {
        return None;
    }
    let default = default
        .filter(|id| models.contains_key(id))
        .unwrap_or_else(|| {
            models
                .keys()
                .next()
                .expect("nonempty native models")
                .clone()
        });
    let mut catalog = claude_provider_catalog();
    for provider in &mut catalog.all {
        provider.models = models.clone();
        catalog.default.insert(provider.id.clone(), default.clone());
    }
    Some(catalog)
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
    env::var_os("CHARIOX_CLAUDE_CONFIG")
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
        .unwrap_or_else(|| id.clone());
    entries.push((id, name));
}

fn is_supported_claude_model_ref(id: &str) -> bool {
    id.starts_with("claude-")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_uses_native_concrete_ids_and_names() {
        let catalog = claude_provider_catalog();
        for provider in catalog.all {
            assert_eq!(provider.models["claude-haiku-5-5"].name, "Haiku 5.5");
            assert_eq!(provider.models["claude-sonnet-5-5"].name, "Sonnet 5.5");
            for alias in ["haiku", "sonnet", "opus", "fable"] {
                assert!(!provider.models.contains_key(alias));
            }
            assert!(provider
                .models
                .values()
                .all(|model| !model.name.contains("(latest)")));
        }
    }

    #[test]
    fn unknown_model_without_native_display_name_keeps_its_id() {
        let mut entries = Vec::new();
        push_claude_model_entry(&mut entries, "claude-future-7-2", None);
        assert_eq!(
            entries,
            [("claude-future-7-2".into(), "claude-future-7-2".into())]
        );
    }

    #[test]
    fn native_picker_resolves_aliases_and_preserves_names_and_efforts() {
        let value = serde_json::json!([
            {"value":"default", "resolvedModel":"claude-opus-5-5", "displayName":"Default (recommended)"},
            {"value":"opus", "resolvedModel":"claude-opus-5-5", "displayName":"Opus", "supportedEffortLevels":["low","max"]},
            {"value":"haiku", "resolvedModel":"claude-haiku-5-5", "displayName":"Haiku", "supportedEffortLevels":["low"]}
        ]);
        let catalog = catalog_from_native_models(&value).unwrap();
        assert_eq!(catalog.default["claude-p"], "claude-opus-5-5");
        for provider in catalog.all {
            assert_eq!(provider.models.len(), 2);
            assert_eq!(provider.models["claude-opus-5-5"].name, "Opus");
            assert_eq!(
                provider.models["claude-haiku-5-5"]
                    .variants
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                ["low"]
            );
        }
        assert!(catalog_from_native_models(&serde_json::json!([
            {"value":"haiku", "displayName":"Haiku"}
        ]))
        .is_none());
    }

    // MP-08 / MP-10 / MP-11: a model that executes through the official CLI
    // must also be selectable by both web and TUI through the shared catalog.
    #[test]
    fn catalog_offers_validated_haiku_5_5_in_both_claude_modes() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("claude-model-catalog");
        let config = root.path().join("model-cache.json");
        std::fs::write(&config, "{}").unwrap();
        std::env::set_var("CHARIOX_CLAUDE_CONFIG", &config);
        let catalog = claude_provider_catalog();
        std::env::remove_var("CHARIOX_CLAUDE_CONFIG");
        for provider in ["claude-p", "claude-headless"] {
            let endpoint = catalog
                .all
                .iter()
                .find(|entry| entry.id == provider)
                .unwrap();
            let model = endpoint
                .models
                .get("claude-haiku-5-5")
                .expect("real Claude Haiku 5.5 must remain available after a model switch");
            assert_eq!(model.name, "Haiku 5.5");
        }
    }

    #[test]
    fn catalog_includes_current_claude_models() {
        let catalog = claude_provider_catalog();
        for provider in catalog.all {
            assert!(provider.models.contains_key("claude-fable-5-1"));
            assert!(provider.models.contains_key("claude-haiku-4-5"));
            assert!(provider.models.contains_key("claude-sonnet-5"));
            assert!(provider.models.contains_key("claude-opus-4-8"));
            assert!(provider.models.contains_key("claude-fable-5"));
        }
    }
}
