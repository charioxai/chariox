use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use arroba_event_protocol::{
    AegsAuthorizationFlow as EventGeneratorAuthorizationFlow,
    AegsProviderResource as EventGeneratorResource,
    AegsProviderResourcePage as EventGeneratorResourcePage,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventGeneratorParty {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventGeneratorCatalogSummary {
    #[serde(default = "default_event_generator_schema_version")]
    pub schema_version: u32,
    pub generator_id: String,
    pub version: String,
    pub name: String,
    pub summary: String,
    pub provider: String,
    pub publisher: EventGeneratorParty,
    pub operator: EventGeneratorParty,
    pub verification: String,
    pub manifest_digest: String,
    pub protocol_version: u32,
    pub categories: Vec<String>,
    pub installed_count: u64,
    pub recommended: bool,
    #[serde(default = "default_event_generator_availability")]
    pub availability: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventGeneratorEventDefinition {
    pub event_type: String,
    pub version: u32,
    pub name: String,
    pub description: String,
    pub filter_schema: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventGeneratorCatalogDetail {
    #[serde(flatten)]
    pub summary: EventGeneratorCatalogSummary,
    pub authorization: Value,
    pub events: Vec<EventGeneratorEventDefinition>,
    pub signature: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecation: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCatalogCategory {
    pub id: String,
    pub name: String,
    pub service_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCatalogFacet {
    pub id: String,
    pub values: Vec<EventCatalogFacetValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCatalogFacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventGeneratorCatalogPage {
    pub services: Vec<EventGeneratorCatalogSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub categories: Vec<EventCatalogCategory>,
    pub facets: Vec<EventCatalogFacet>,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventGeneratorEventPage {
    pub events: Vec<EventGeneratorEventDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEventGeneratorCatalogLandingRequest {
    #[serde(default = "default_catalog_page_limit")]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchEventGeneratorCatalogRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default = "default_catalog_page_limit")]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowseEventGeneratorCategoryRequest {
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default = "default_catalog_page_limit")]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEventGeneratorDetailRequest {
    pub generator_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowseEventGeneratorEventsRequest {
    pub generator_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default = "default_event_page_limit")]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartEventGeneratorAuthorizationRequest {
    pub generator_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEventGeneratorResourcesRequest {
    pub generator_id: String,
    pub connection_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default = "default_catalog_page_limit")]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateWorkflowEventBindingRequest {
    pub session_id: String,
    pub publication_ref: String,
    pub generator_id: String,
    pub generator_version: String,
    pub manifest_digest: String,
    pub connection_id: String,
    pub connection_scope: String,
    pub event_type: String,
    pub event_type_version: u32,
    #[serde(default, skip_serializing_if = "is_json_null")]
    pub filter: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowEventBindingsRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowEventBindingStatusRequest {
    pub session_id: String,
    pub binding_id: String,
    pub status: crate::session::WorkflowEventBindingStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferWorkflowEventBindingRequest {
    pub source_session_id: String,
    pub binding_id: String,
    pub target_session_id: String,
    pub target_publication_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestWorkflowEventBindingRequest {
    pub session_id: String,
    pub binding_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetEventDeliveryStatusRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDeliveryStatus {
    pub configured: bool,
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aeds_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub active_route_count: usize,
}

fn default_catalog_page_limit() -> u32 {
    20
}

fn default_event_page_limit() -> u32 {
    50
}

fn default_event_generator_availability() -> String {
    "unknown".to_string()
}

fn default_event_generator_schema_version() -> u32 {
    1
}

fn is_json_null(value: &Value) -> bool {
    value.is_null()
}
