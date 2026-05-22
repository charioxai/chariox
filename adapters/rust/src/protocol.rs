use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub request_type: ConnectorAdapterRequestType,
    pub connector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<ConnectorAdapterOperationValidation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<ConnectorAdapterCredential>,
    pub timeout_ms: u64,
    pub max_response_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorAdapterRequestType {
    Validate,
    Prepare,
    Call,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterOperationValidation {
    pub name: String,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterCredential {
    pub id: String,
    pub secret: String,
    pub injection: UserCredentialInjectionConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterPrepareResult {
    #[serde(default)]
    pub credential_targets: Vec<ConnectorAdapterCredentialTarget>,
    pub prepared_config: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectorAdapterCredentialTarget {
    Host {
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UserCredentialInjectionConfig {
    Header {
        name: String,
        value: String,
    },
    Query {
        name: String,
    },
    Basic {
        username: String,
    },
    Hmac {
        timestamp_header: String,
        signature_header: String,
    },
    Pty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorAdapterResponse {
    pub id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
