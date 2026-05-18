//! Codex model-list response mapping into Arroba provider catalog shape.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::error::DaemonError;
use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};

use super::CodexClient;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CodexModelListResponse {
    pub(super) data: Vec<CodexModel>,
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

impl CodexClient {
    pub fn provider_catalog(&self) -> Result<OpenCodeProviderCatalog, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexModelListResponse = self.send_request(
            &mut socket,
            &mut next_request_id,
            "model/list",
            serde_json::json!({}),
        )?;
        Ok(codex_catalog_from_models(response.data))
    }
}
