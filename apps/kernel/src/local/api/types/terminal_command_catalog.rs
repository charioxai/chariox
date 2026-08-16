use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetTerminalCommandCatalogRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetTerminalOperationRegistryRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOperationContract {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intents: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_context: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_targets: Vec<String>,
    pub input_schema: serde_json::Value,
    pub result_kind: String,
    pub mutation: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_projections: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_surfaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parity_variants: Vec<String>,
    pub presentation_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOperationRegistry {
    pub revision: String,
    pub operations: Vec<TerminalOperationContract>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCommandCatalogNodeKind {
    Group,
    Command,
    PromptPrefix,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCommandCatalogExecutionTarget {
    Kernel,
    TerminalLocal,
    PromptPrefix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCommandCatalogSurface {
    Session,
    WaitingRoom,
    WorkflowScreen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCommandCatalogNode {
    pub id: String,
    pub label: String,
    pub description: String,
    pub value: String,
    pub kind: TerminalCommandCatalogNodeKind,
    pub execution_target: TerminalCommandCatalogExecutionTarget,
    pub surfaces: Vec<TerminalCommandCatalogSurface>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intents: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TerminalCommandCatalogNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCommandCatalog {
    pub revision: String,
    pub nodes: Vec<TerminalCommandCatalogNode>,
}
