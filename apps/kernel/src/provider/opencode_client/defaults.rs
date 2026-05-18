//! OpenCode config and agent-list defaults projection.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::DaemonError;

use super::{OpenCodeClient, OpenCodeSelectedModel};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenCodeConfiguredDefaults {
    pub model: Option<String>,
    pub variant: Option<String>,
    pub selected_agent: Option<String>,
    pub agent_model: Option<String>,
    pub agent_variant: Option<String>,
    pub top_level_model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OpenCodeConfig {
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(rename = "default_agent", default)]
    pub(super) default_agent: Option<String>,
    #[serde(default)]
    pub(super) agent: BTreeMap<String, OpenCodeConfigAgent>,
    #[serde(default)]
    pub(super) mode: BTreeMap<String, OpenCodeConfigAgent>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct OpenCodeConfigAgent {
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpenCodeAgentInfo {
    pub(super) name: String,
    pub(super) mode: String,
    pub(super) hidden: Option<bool>,
    pub(super) model: Option<OpenCodeSelectedModel>,
    pub(super) variant: Option<String>,
}

impl OpenCodeClient {
    pub fn configured_defaults(&self) -> Result<OpenCodeConfiguredDefaults, DaemonError> {
        let config: OpenCodeConfig = match self.send_json_request("GET", "/config", None) {
            Ok(config) => config,
            Err(DaemonError::ProviderProtocol {
                operation: "opencode_http",
                message,
                ..
            }) if message == "OpenCode returned HTTP 404" => {
                return Ok(OpenCodeConfiguredDefaults::default());
            }
            Err(error) => return Err(error),
        };
        let agents = match self.send_json_request("GET", "/agent", None) {
            Ok::<serde_json::Value, _>(value) => parse_agent_infos(value),
            Err(DaemonError::ProviderProtocol {
                operation: "opencode_http",
                message,
                ..
            }) if message == "OpenCode returned HTTP 404" => Vec::new(),
            Err(error) => return Err(error),
        };
        Ok(resolve_configured_defaults(&config, &agents))
    }
}

pub(super) fn resolve_configured_defaults(
    config: &OpenCodeConfig,
    agents: &[OpenCodeAgentInfo],
) -> OpenCodeConfiguredDefaults {
    let selected_agent = config
        .default_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "build".to_string());
    let config_agent = config
        .agent
        .get(&selected_agent)
        .or_else(|| config.mode.get(&selected_agent));
    let listed_agent = agents.iter().find(|agent| {
        agent.name == selected_agent && agent.mode != "subagent" && agent.hidden != Some(true)
    });
    let top_level_model = config
        .model
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let config_agent_model = config_agent
        .and_then(|agent| agent.model.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let listed_agent_model = listed_agent
        .and_then(|agent| agent.model.as_ref())
        .map(|model| format!("{}/{}", model.provider_id, model.model_id));
    let agent_model = config_agent_model.clone().or(listed_agent_model.clone());
    let config_agent_variant = config_agent
        .and_then(|agent| agent.variant.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let listed_agent_variant = listed_agent
        .and_then(|agent| agent.variant.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let agent_variant = config_agent_variant
        .clone()
        .or(listed_agent_variant.clone());

    OpenCodeConfiguredDefaults {
        model: agent_model.clone().or(top_level_model.clone()),
        variant: agent_variant.clone(),
        selected_agent: Some(selected_agent),
        agent_model,
        agent_variant,
        top_level_model,
    }
}

pub(super) fn parse_agent_infos(value: serde_json::Value) -> Vec<OpenCodeAgentInfo> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(parse_agent_info)
        .collect()
}

fn parse_agent_info(value: &serde_json::Value) -> Option<OpenCodeAgentInfo> {
    let object = value.as_object()?;
    let name = object.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }

    let mode = object
        .get("mode")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("primary")
        .to_string();
    let hidden = object.get("hidden").and_then(|value| value.as_bool());
    let model = parse_agent_model(object.get("model"));
    let variant = object
        .get("variant")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    Some(OpenCodeAgentInfo {
        name: name.to_string(),
        mode,
        hidden,
        model,
        variant,
    })
}

fn parse_agent_model(value: Option<&serde_json::Value>) -> Option<OpenCodeSelectedModel> {
    let value = value?;
    if let Some(model) = value.as_object() {
        let provider_id = model
            .get("providerID")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let model_id = model
            .get("modelID")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        return Some(OpenCodeSelectedModel {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        });
    }

    let raw = value.as_str()?.trim();
    let (provider_id, model_id) = raw.split_once('/')?;
    let provider_id = provider_id.trim();
    let model_id = model_id.trim();
    if provider_id.is_empty() || model_id.is_empty() {
        return None;
    }

    Some(OpenCodeSelectedModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    })
}
