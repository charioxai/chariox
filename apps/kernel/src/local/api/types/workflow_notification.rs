//! MP-08/MP-10/MP-11: private completion notifications use the shared local protocol.
use super::*;

/// Every undelivered occurrence, workflow and App alike, is cancelled after this long (7 days).
pub const NOTIFICATION_RETENTION_MS: u64 = chariox_app_runtime::app_outbox::PENDING_LIFETIME_MS;

/// One exit of a workflow is one notification source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterWorkflowNotificationSourceRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub exit_node_id: String,
    pub enabled: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachWorkflowNotificationRequest {
    pub session_id: String,
    pub source_id: String,
    pub publication_ref: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListWorkflowNotificationsRequest {
    pub session_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNotificationSource {
    pub kind: NotificationSourceKind,
    pub source_id: String,
    pub owner_user_id: String,
    pub kernel_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub exit_node_id: String,
    pub enabled: bool,
    pub available: bool,
    /// The workflow's alias, or its id.
    #[serde(default)]
    pub name: String,
    /// Position among the workflow's exits, as the Exit inspector numbers them.
    #[serde(default)]
    pub exit_ordinal: u32,
    /// `name`, or `name-<exit_ordinal>` when more than one exit of the workflow notifies.
    #[serde(default)]
    pub display_name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNotificationSubscription {
    pub subscription_id: String,
    pub source_id: String,
    /// The user whose authority the subscription acts for: the workflow owner
    /// for an entry point, the agent's room owner for an agent.
    pub owner_user_id: String,
    /// Who created the subscription (a room guest for a guest's agent).
    #[serde(default)]
    pub created_by_user_id: String,
    pub target_kernel_id: String,
    #[serde(default)]
    pub target_kind: WorkflowNotificationTargetKind,
    pub session_id: String,
    /// Empty for an agent target.
    pub workflow_id: String,
    /// The agent trigger id for an agent target.
    pub publication_id: String,
    /// The agent id for an agent target.
    pub endpoint_id: String,
    pub queue_id: String,
    pub source_available: bool,
    #[serde(default)]
    pub source_kernel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNotificationEnvelope {
    pub source_id: String,
    pub display_name: String,
    pub occurrence_id: String,
    pub output: crate::session::WorkflowOutputPayload,
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
    LoopDropped,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNotificationDiagnostic {
    pub source_id: String,
    pub occurrence_id: String,
    pub code: String,
}

/// Picker metadata only; no outputs, prompts, history, credentials or owner profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowNotificationSourceSummary {
    pub kind: NotificationSourceKind,
    pub source_id: String,
    pub kernel_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub exit_node_id: String,
    pub exit_ordinal: u32,
    pub name: String,
    pub display_name: String,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetachWorkflowNotificationRequest {
    pub session_id: String,
    pub subscription_id: String,
}

/// What receives the notification: a workflow entry point (always queues a
/// run), an agent (always queues a prompt) or an outside agent's access grant (always
/// appends one event to the grant's own event stream).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNotificationTargetKind {
    #[default]
    WorkflowEndpoint,
    Agent,
    OutsideAgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSourceKind {
    WorkflowExit,
    OwnerAction,
    /// Any run of a workflow that ended, completed or failed.
    WorkflowRun,
    /// An event an installed App's route accepted.
    AppEvent,
}
