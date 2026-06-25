use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCommandDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub value: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCommandCatalogSource {
    Shipped,
    Discovered,
    Merged,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCommandCatalogDiscovery {
    None,
    ProviderApi,
    CustomFiles,
    Driver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCommandCatalog {
    pub provider: String,
    pub source: ProviderCommandCatalogSource,
    pub discovery: ProviderCommandCatalogDiscovery,
    #[serde(default)]
    pub commands: Vec<ProviderCommandDescriptor>,
}

pub fn default_provider_command_catalogs() -> BTreeMap<String, ProviderCommandCatalog> {
    BTreeMap::from([
        (
            "opencode".to_string(),
            ProviderCommandCatalog {
                provider: "opencode".to_string(),
                source: ProviderCommandCatalogSource::Shipped,
                discovery: ProviderCommandCatalogDiscovery::None,
                commands: Vec::new(),
            },
        ),
        (
            "claude".to_string(),
            ProviderCommandCatalog {
                provider: "claude".to_string(),
                source: ProviderCommandCatalogSource::Shipped,
                discovery: ProviderCommandCatalogDiscovery::None,
                commands: Vec::new(),
            },
        ),
        (
            "codex".to_string(),
            ProviderCommandCatalog {
                provider: "codex".to_string(),
                source: ProviderCommandCatalogSource::Shipped,
                discovery: ProviderCommandCatalogDiscovery::None,
                commands: Vec::new(),
            },
        ),
        (
            "pi".to_string(),
            ProviderCommandCatalog {
                provider: "pi".to_string(),
                source: ProviderCommandCatalogSource::Shipped,
                discovery: ProviderCommandCatalogDiscovery::Driver,
                commands: Vec::new(),
            },
        ),
    ])
}
