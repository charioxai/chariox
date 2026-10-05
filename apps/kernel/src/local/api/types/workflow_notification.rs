//! Protocol 437: workflow notifications are private successful-completion output.
use super::*;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterWorkflowNotificationSourceRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub enabled: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachWorkflowNotificationRequest {
    pub session_id: String,
    pub source_id: String,
    pub publication_ref: String,
    #[serde(default)]
    pub queue_ref: Option<String>,
    #[serde(default = "default_ttl_days")]
    pub ttl_days: u32,
}
fn default_ttl_days() -> u32 {
    7
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListWorkflowNotificationsRequest {
    pub session_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNotificationSource {
    pub source_id: String,
    pub owner_user_id: String,
    pub kernel_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub enabled: bool,
    pub available: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNotificationSubscription {
    pub subscription_id: String,
    pub source_id: String,
    pub owner_user_id: String,
    pub target_kernel_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub publication_id: String,
    pub endpoint_id: String,
    pub queue_id: String,
    pub ttl_days: u32,
    pub source_available: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNotificationEnvelope {
    pub source_id: String,
    pub occurrence_id: String,
    pub output: crate::session::WorkflowOutputPayload,
    /// Kernel-derived causal workflow IDs; client requests cannot supply these.
    pub ancestry: Vec<String>,
    pub deadline_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNotificationAck {
    Accepted,
    Duplicate,
    Expired,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNotificationDiagnostic {
    pub source_id: String,
    pub occurrence_id: String,
    pub code: String,
}
