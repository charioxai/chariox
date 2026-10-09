//! Protocol 437: workflow notifications are private kernel completion notifications.
use super::*;
pub use chariox_app_runtime::app_outbox::NotificationDeliveryMode;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterWorkflowNotificationSourceRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub enabled: bool,
    #[serde(default)]
    pub output_fields: Option<Vec<String>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachWorkflowNotificationRequest {
    pub session_id: String,
    pub source_id: String,
    pub publication_ref: String,
    #[serde(default)]
    pub queue_ref: Option<String>,
    #[serde(default = "default_ttl_days")]
    pub ttl_days: u32,
    #[serde(default)]
    pub events: WorkflowNotificationEvents,
    #[serde(default)]
    pub filters: serde_json::Value,
    #[serde(default)]
    pub delivery_mode: NotificationDeliveryMode,
}
fn default_ttl_days() -> u32 {
    7
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListWorkflowNotificationsRequest {
    pub session_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNotificationSource {
    pub source_id: String,
    pub owner_user_id: String,
    pub kernel_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub enabled: bool,
    pub available: bool,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub output_fields: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNotificationSubscription {
    pub subscription_id: String,
    pub source_id: String,
    pub owner_user_id: String,
    pub target_kernel_id: String,
    #[serde(default)]
    pub target_kind: WorkflowNotificationTargetKind,
    pub session_id: String,
    pub workflow_id: String,
    pub publication_id: String,
    pub endpoint_id: String,
    pub queue_id: String,
    pub ttl_days: u32,
    pub source_available: bool,
    #[serde(default)]
    pub source_kernel_id: String,
    #[serde(default)]
    pub events: WorkflowNotificationEvents,
    #[serde(default)]
    pub filters: serde_json::Value,
    #[serde(default)]
    pub delivery_mode: NotificationDeliveryMode,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNotificationEnvelope {
    pub source_id: String,
    pub occurrence_id: String,
    pub output: Option<crate::session::WorkflowOutputPayload>,
    pub status: WorkflowNotificationStatus,
    pub subject: Option<String>,
    pub fields: serde_json::Value,
    /// Kernel-derived causal workflow IDs; client requests cannot supply these.
    pub ancestry: Vec<String>,
    pub deadline_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNotificationAck {
    Accepted,
    Duplicate,
    Expired,
    Filtered,
    LoopDropped,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNotificationDiagnostic {
    pub source_id: String,
    pub occurrence_id: String,
    pub code: String,
}

/// Source kernels produce both events; subscribers choose which to accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNotificationEvents {
    #[default]
    Success,
    Failure,
    Both,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNotificationStatus {
    Success,
    Failure,
}
impl WorkflowNotificationEvents {
    pub fn accepts(self, status: WorkflowNotificationStatus) -> bool {
        self == Self::Both
            || matches!(
                (self, status),
                (Self::Success, WorkflowNotificationStatus::Success)
                    | (Self::Failure, WorkflowNotificationStatus::Failure)
            )
    }
}
/// Picker metadata only; no outputs, prompts, history, credentials or owner profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowNotificationSourceSummary {
    pub source_id: String,
    pub kernel_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub name: String,
    pub events: WorkflowNotificationEvents,
    pub fields: Vec<String>,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetachWorkflowNotificationRequest {
    pub session_id: String,
    pub subscription_id: String,
}

/// Subscribers belong to workflow endpoints. Run-scoped admission is deferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNotificationTargetKind {
    #[default]
    WorkflowEndpoint,
}
