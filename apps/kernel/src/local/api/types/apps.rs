use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginAppPublisherEnrollmentRequest {
    pub session_id: String,
    pub request_id: String,
    pub publisher_id: String,
    pub key_id: String,
    pub public_key_base64: String,
    /// Exact decimal revision; kept as text across JavaScript transports.
    pub expected_revision: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppPublisherEnrollmentRequest {
    pub request_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppPublisherEnrollmentPhase {
    Pending,
    Approved,
    Denied,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppPublisherEnrollmentSummary {
    pub request_id: String,
    pub phase: AppPublisherEnrollmentPhase,
    pub publisher_id: String,
    pub key_id: String,
    pub key_fingerprint: String,
    /// Historical approval only; later revocation can supersede this revision.
    pub approved_revision: Option<String>,
    pub interaction_id: Option<String>,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginAppInstallRequest {
    pub session_id: String,
    pub request_id: String,
    pub upload_handle: String,
    pub expected_package_digest: String,
}
/// Replaces the release of the caller's installation, fenced on the
/// generation the caller read; progress uses the install operation requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginAppUpdateRequest {
    pub session_id: String,
    pub request_id: String,
    pub installation_id: String,
    pub expected_generation: String,
    pub upload_handle: String,
    pub expected_package_digest: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInstallOperationRequest {
    pub request_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppInstallOperationPhase {
    Preparing,
    AwaitingApproval,
    Starting,
    Committed,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInstallOperationSummary {
    pub request_id: String,
    pub phase: AppInstallOperationPhase,
    pub installation_id: Option<String>,
    pub generation: Option<String>,
    /// Expected upload digest while preparing; authority requires verification.
    pub package_digest: String,
    pub interaction_id: Option<String>,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListAppInstallationsRequest {
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInstallationRequest {
    pub installation_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginAppPackageUploadRequest {
    pub request_id: String,
    pub expected_size: u64,
    pub sha256: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutAppPackageUploadChunkRequest {
    pub handle: String,
    pub offset: u64,
    pub data_base64: String,
    pub chunk_sha256: String,
}

impl std::fmt::Debug for PutAppPackageUploadChunkRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PutAppPackageUploadChunkRequest")
            .field("handle", &self.handle)
            .field("offset", &self.offset)
            .field("encoded_bytes", &self.data_base64.len())
            .field("chunk_sha256", &self.chunk_sha256)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppPackageUploadRequest {
    pub handle: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppPackageUploadSummary {
    pub handle: String,
    pub phase: AppPackageUploadPhase,
    pub expected_size: u64,
    pub accepted_bytes: u64,
    pub sha256: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppPackageUploadPhase {
    Receiving,
    Finalized,
    Aborted,
}

/// Client projection only. Approval handles and host paths remain private.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppReleaseSummary {
    pub version: String,
    pub publisher_id: String,
    pub package_digest: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInstallationSummary {
    pub installation_id: String,
    pub app_id: String,
    /// Decimal strings preserve generations beyond JavaScript's safe integers.
    pub generation: String,
    pub active_release: Option<AppReleaseSummary>,
    pub pending_generation: Option<String>,
    pub admission_paused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateSummary {
    pub base_generation: String,
    pub generation: String,
    pub release: AppReleaseSummary,
    pub phase: AppUpdatePhase,
    pub decision: AppCapabilityDecisionStatus,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppUpdatePhase {
    Staged,
    Quiescing,
    Prepared,
    Committed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCapabilityDecisionStatus {
    Pending,
    Approved,
    Declined,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppRequestErrorCode {
    InvalidRequest,
    Unauthorized,
    NotFound,
    Busy,
    LimitExceeded,
    Conflict,
    DigestMismatch,
    StorageUnavailable,
}

/// Protocol 345: owner-scoped App worker control. The kernel derives the
/// owner; requests cannot name an owner, generation or host path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppWorkerRequest {
    pub installation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppWorkerAction {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlAppWorkerRequest {
    pub installation_id: String,
    pub action: AppWorkerAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppWorkerPhase {
    NotStarted,
    Starting,
    Running,
    /// Stopped while idle; the next use starts it on demand.
    Dormant,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppWorkerSummary {
    pub installation_id: String,
    pub phase: AppWorkerPhase,
    /// False after a user stop; on-demand use does not restart it.
    pub enabled: bool,
    pub failure: Option<String>,
    pub updated_at_ms: Option<u64>,
}

/// Protocol 345: App automations route one App event to one workflow endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureAppAutomationRequest {
    pub installation_id: String,
    pub automation_id: String,
    /// Zero creates; replacements name the current revision.
    pub expected_revision: u64,
    pub event_name: String,
    pub session_id: String,
    pub publication_ref: String,
    pub queue_ref: Option<String>,
    pub scheduled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisableAppAutomationRequest {
    pub installation_id: String,
    pub automation_id: String,
    pub expected_revision: u64,
}

/// Protocol 348: the App's own `log.write` entries, oldest first, after an
/// optional sequence (for paging and following).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetAppLogsRequest {
    pub installation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_sequence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppLogEntrySummary {
    pub sequence: String,
    pub at_ms: u64,
    pub level: String,
    pub message: String,
    pub fields: serde_json::Value,
}

/// Protocol 353: routes one external event type to an App's declared
/// incoming (or both-direction) event. The route grants the App nothing else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAppInboxRouteRequest {
    pub installation_id: String,
    pub route_id: String,
    pub event_name: String,
    pub source_event_type: String,
    pub source_event_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInboxRouteRequest {
    pub installation_id: String,
    pub route_id: String,
}

/// Accepts one occurrence for a route as a source would, for drills and
/// local development. The payload must match the App's signed schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestAppInboxRouteRequest {
    pub installation_id: String,
    pub route_id: String,
    pub occurrence_id: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInboxRouteSummary {
    pub route_id: String,
    pub event_name: String,
    pub source_event_type: String,
    pub source_event_version: u32,
    pub active: bool,
    /// Occurrences by outcome; `failed` ones exhausted their attempts.
    pub pending: u64,
    pub delivered: u64,
    pub failed: u64,
    pub expired: u64,
}

/// Protocol 354: the owner's answer to an App's file request (`host.pick_file`):
/// the chosen files' names and bytes. Only the final name component is kept;
/// no host path reaches the App.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantAppFileRequest {
    /// The session showing the request; its prompt closes once answered.
    pub session_id: String,
    pub operation_id: String,
    pub files: Vec<AppFileContents>,
}

/// Protocol 355: the owner takes a copy of a file an App offered with
/// `files.export`, once; the terminal decides where it is saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveAppFileExportRequest {
    /// The session showing the offer; its prompt closes once answered.
    pub session_id: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppFileContents {
    pub name: String,
    pub contents_base64: String,
}

/// Protocol 347: stops the worker and deactivates the installation at the
/// caller's expected generation. App data and user workflows are retained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UninstallAppRequest {
    pub installation_id: String,
    pub expected_generation: String,
}

/// Opens the installation's view as a managed Tab in the session's Room
/// browser. The kernel serves only the active release's signed UI files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAppViewRequest {
    pub session_id: String,
    pub installation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppAutomationStatus {
    Active,
    Paused,
    Broken,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppAutomationSummary {
    pub automation_id: String,
    pub revision: u64,
    pub event_name: String,
    pub event_version: u32,
    pub session_id: String,
    pub publication_id: String,
    pub endpoint_id: String,
    pub queue_id: String,
    pub scheduled: bool,
    pub status: AppAutomationStatus,
}
