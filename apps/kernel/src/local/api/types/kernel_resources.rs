use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One resource a client shows, with the revision of the copy it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelResourceWatch {
    pub resource: String,
    #[serde(default = "empty_params")]
    pub params: Value,
    #[serde(default)]
    pub known_revision: Option<String>,
}

/// Replaces the whole watch set of one `kernel_resources` subscription: the relay
/// subscription id over a relay, the `Subscribe` request id on a direct socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchKernelResourcesRequest {
    pub subscription_request_id: String,
    pub watches: Vec<KernelResourceWatch>,
}

/// A watch the kernel will not answer; it is never answered with an empty snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelResourceRefusal {
    pub resource: String,
    pub params: Value,
    pub code: String,
    pub message: String,
}

fn empty_params() -> Value {
    Value::Object(serde_json::Map::new())
}
