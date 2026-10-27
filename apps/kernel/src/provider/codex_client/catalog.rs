//! Codex model-list response mapping into Chariox provider catalog shape.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::Value;

use crate::error::DaemonError;
use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};

use super::CodexClient;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CodexModelListResponse {
    pub(super) data: Vec<CodexModel>,
    #[serde(rename = "nextCursor", default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CodexModel {
    id: String,
    model: String,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(default)]
    hidden: bool,
    #[serde(rename = "supportedReasoningEfforts", default)]
    supported_reasoning_efforts: Vec<CodexReasoningEffort>,
    #[serde(rename = "isDefault", default)]
    is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexReasoningEffort {
    #[serde(rename = "reasoningEffort")]
    reasoning_effort: String,
}

pub(super) fn codex_catalog_from_models(models: Vec<CodexModel>) -> OpenCodeProviderCatalog {
    let mut catalog_models = BTreeMap::new();
    let mut default = BTreeMap::new();
    let mut first_model = None;

    for model in models.into_iter().filter(|model| !model.hidden) {
        let model_id = model.model.clone();
        if first_model.is_none() {
            first_model = Some(model_id.clone());
        }
        if model.is_default {
            default.insert("codex".to_string(), model_id.clone());
        }
        let variants = model
            .supported_reasoning_efforts
            .into_iter()
            .map(|entry| (entry.reasoning_effort, Value::Object(Default::default())))
            .collect::<BTreeMap<_, _>>();
        catalog_models.insert(
            model_id.clone(),
            OpenCodeProviderModel {
                id: model_id,
                name: model.display_name.unwrap_or_else(|| model.id.clone()),
                status: "active".to_string(),
                limit: None,
                variants,
            },
        );
    }

    if default.is_empty() {
        if let Some(model) = first_model {
            default.insert("codex".to_string(), model);
        }
    }

    OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            remote_machine_aliases: Vec::new(),
            models: catalog_models,
        }],
        default,
        connected: vec!["codex".to_string()],
    }
}

const MAX_CATALOG_PAGES: usize = 128;
const MAX_CATALOG_MODELS: usize = 10_000;

fn load_codex_catalog(
    mut request_page: impl FnMut(Value) -> Result<CodexModelListResponse, DaemonError>,
) -> Result<OpenCodeProviderCatalog, DaemonError> {
    let mut params = serde_json::json!({});
    let mut models = Vec::new();
    let mut cursors = BTreeSet::new();
    for _ in 0..MAX_CATALOG_PAGES {
        let response = request_page(params)?;
        if models.len().saturating_add(response.data.len()) > MAX_CATALOG_MODELS {
            return Err(catalog_pagination_error(
                "model count exceeds the catalog limit",
            ));
        }
        models.extend(response.data);
        let Some(cursor) = response.next_cursor else {
            return Ok(codex_catalog_from_models(models));
        };
        if !cursors.insert(cursor.clone()) {
            return Err(catalog_pagination_error(
                "model/list repeated a pagination cursor",
            ));
        }
        params = serde_json::json!({"cursor": cursor});
    }
    Err(catalog_pagination_error(
        "model/list exceeded the page limit",
    ))
}

fn catalog_pagination_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "codex.provider_catalog",
        message: message.to_string(),
    }
}

impl CodexClient {
    pub fn provider_catalog(&self) -> Result<OpenCodeProviderCatalog, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        load_codex_catalog(|params| {
            self.send_request(&mut socket, &mut next_request_id, "model/list", params)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn model(id: &str, hidden: bool, default: bool) -> Value {
        serde_json::json!({"id": id, "model": id, "displayName": id,
            "hidden": hidden, "isDefault": default,
            "supportedReasoningEfforts": [{"reasoningEffort": "xhigh"}]})
    }

    #[test]
    fn discovers_visible_models_and_default_from_later_pages() {
        let mut pages = VecDeque::from([
            serde_json::json!({"data": [model("first", false, false)], "nextCursor": "opaque-page-2"}),
            serde_json::json!({"data": [], "nextCursor": "opaque-page-3"}),
            serde_json::json!({"data": [model("future-model", false, true), model("hidden-model", true, false)], "nextCursor": null}),
        ]);
        let mut requests = Vec::new();
        let catalog = load_codex_catalog(|params| {
            requests.push(params);
            Ok(serde_json::from_value(pages.pop_front().expect("unexpected extra page")).unwrap())
        })
        .unwrap();
        assert_eq!(
            requests,
            [
                serde_json::json!({}),
                serde_json::json!({"cursor":"opaque-page-2"}),
                serde_json::json!({"cursor":"opaque-page-3"})
            ]
        );
        assert_eq!(catalog.default["codex"], "future-model");
        let models = &catalog.all[0].models;
        assert_eq!(models.len(), 2);
        assert!(models["future-model"].variants.contains_key("xhigh"));
        assert!(!models.contains_key("hidden-model"));
    }

    #[test]
    fn accepts_legacy_single_page_without_cursor() {
        let catalog = load_codex_catalog(|_| {
            Ok(
                serde_json::from_value(serde_json::json!({"data": [model("first", false, false)]}))
                    .unwrap(),
            )
        })
        .unwrap();
        assert_eq!(catalog.default["codex"], "first");
    }

    #[test]
    fn later_page_failure_does_not_publish_a_partial_catalog() {
        let mut calls = 0;
        let result = load_codex_catalog(|_| {
            calls += 1;
            if calls == 1 {
                Ok(serde_json::from_value(serde_json::json!({"data": [model("first", false, false)], "nextCursor":"next"})).unwrap())
            } else {
                Err(DaemonError::LocalTransport {
                    operation: "model/list",
                    message: "fixture page failure".to_string(),
                })
            }
        });
        assert!(
            result.is_err(),
            "a failed continuation must not look like a complete catalog"
        );
        assert_eq!(calls, 2);
    }

    #[test]
    fn cursor_cycle_is_an_error_instead_of_an_infinite_refresh() {
        let mut calls = 0;
        let result = load_codex_catalog(|_| {
            calls += 1;
            assert!(calls <= 3, "pagination failed to stop its cursor cycle");
            Ok(
                serde_json::from_value(serde_json::json!({"data": [], "nextCursor":"same"}))
                    .unwrap(),
            )
        });
        assert!(result.is_err());
        assert_eq!(calls, 2);
    }

    #[test]
    fn endlessly_advancing_cursors_fail_with_bounded_work() {
        let mut calls = 0;
        let result = load_codex_catalog(|_| {
            calls += 1;
            Ok(serde_json::from_value(
                serde_json::json!({"data": [], "nextCursor": format!("page-{calls}")}),
            )
            .unwrap())
        });
        assert!(result.is_err());
        assert_eq!(calls, MAX_CATALOG_PAGES);
    }

    #[test]
    fn oversized_inventory_is_rejected_instead_of_truncated() {
        let mut calls = 0;
        let result = load_codex_catalog(|_| {
            calls += 1;
            Ok(serde_json::from_value(serde_json::json!({
                "data": vec![model("fixture", false, false); MAX_CATALOG_MODELS / 2 + 1],
                "nextCursor": format!("page-{calls}"),
            }))
            .unwrap())
        });
        assert!(result.is_err());
        assert_eq!(calls, 2);
    }
}
