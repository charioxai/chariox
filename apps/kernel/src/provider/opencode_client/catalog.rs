//! OpenCode provider catalog contracts and fetch operation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

use super::OpenCodeClient;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeProviderCatalog {
    pub all: Vec<OpenCodeProviderInfo>,
    pub default: BTreeMap<String, String>,
    pub connected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeProviderInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub remote_machine_aliases: Vec<String>,
    #[serde(default)]
    pub models: BTreeMap<String, OpenCodeProviderModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeProviderModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub limit: Option<OpenCodeProviderModelLimit>,
    #[serde(default)]
    pub variants: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeProviderModelLimit {
    pub context: u64,
    #[serde(default)]
    pub input: Option<u64>,
    #[serde(default)]
    pub output: Option<u64>,
}

impl OpenCodeClient {
    pub fn provider_catalog(&self) -> Result<OpenCodeProviderCatalog, DaemonError> {
        self.send_json_request("GET", "/provider", None)
    }
}
