use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct JsonRpcMessage {
    #[serde(default)]
    pub(super) id: Option<Value>,
    #[serde(default)]
    pub(super) method: Option<String>,
    #[serde(default)]
    pub(super) params: Option<Value>,
    #[serde(default)]
    pub(super) result: Option<Value>,
    #[serde(default)]
    pub(super) error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct JsonRpcError {
    #[serde(default)]
    pub(super) message: Option<String>,
}
