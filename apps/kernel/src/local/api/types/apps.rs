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
    /// Protocol 381: approved and waiting to start, usually for a free App
    /// worker slot (at most four Apps run at once).
    Queued,
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
    /// Protocol 363: an uninstalled installation that still holds its data; an
    /// update of it reinstalls the App into that data.
    pub data_kept: bool,
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
    /// Protocol 416: this command identity was evicted; it must never execute again.
    ReceiptExpired,
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
    /// Automatic restarts are exhausted; an explicit start clears quarantine.
    Quarantined,
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
    #[serde(default)]
    pub delivery_mode: NotificationDeliveryMode,
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
    /// Protocol 358: the owner's event generator connection that feeds the
    /// route. The kernel checks it with the generator and subscribes to
    /// `source_event_type` there; without it only `TestAppInboxRoute` feeds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<AppInboxConnection>,
}

/// Protocol 359: lets one installation act through one of the owner's event
/// generator connections, within the actions its signed manifest declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantAppConnectionRequest {
    pub installation_id: String,
    pub generator_id: String,
    pub connection_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeAppConnectionRequest {
    pub installation_id: String,
    pub connection_id: String,
}

/// A connection an installation may use, with the actions its manifest
/// declares for that generator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConnectionSummary {
    pub generator_id: String,
    pub connection_id: String,
    pub granted_at_ms: u64,
    pub actions: Vec<String>,
}

/// Protocol 358: an event generator connection an inbox route subscribes to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInboxConnection {
    pub generator_id: String,
    pub connection_id: String,
    pub connection_scope: String,
    /// The generator's event filter; `null` takes every occurrence.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub filter: serde_json::Value,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<AppInboxConnection>,
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
    /// The answering session. The prompt closes in the session that shows it.
    pub session_id: String,
    pub operation_id: String,
    pub files: Vec<AppFileContents>,
}

/// Protocol 394: the owner ends an installation's file requests and the
/// grants its App has not imported: all of them, or one request's
/// (`operation_id`). The App reads them as `expired` and can no longer import
/// them; files it already imported stay in its data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeAppFileGrantsRequest {
    pub installation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
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
/// caller's expected generation. User workflows are retained, and so is App
/// data unless `delete_data` (protocol 363) is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UninstallAppRequest {
    pub installation_id: String,
    pub expected_generation: String,
    /// Also delete the App's data: its storage, structured state, wakes and
    /// logs. The installation can then no longer be reinstalled into.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub delete_data: bool,
}

/// Opens the installation's view as a managed Tab in the session's Room
/// browser. The kernel serves only the active release's signed UI files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAppViewRequest {
    pub session_id: String,
    pub installation_id: String,
}

/// Protocol 380: the user's choice for an App's agent panel in this session:
/// where it sits and whether it is minimized. It wins over the App's own
/// placement; omitted fields keep their current value, and `reset` first
/// drops the choice, handing the panel back to the App.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetAppViewPanelRequest {
    pub session_id: String,
    pub installation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<crate::session::AppPanelPlacement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimized: Option<bool>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset: bool,
}

/// Protocol 361: the caller's App set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetAppSetRequest {}

/// The schema `AppSet` descriptions carry.
pub const APP_SET_SCHEMA: &str = "chariox.app-set.v1";

/// One active installation in an App set: what a kernel copy needs to
/// install it again with the same approval and configuration. App data is
/// not part of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSetInstallation {
    pub installation_id: String,
    pub app_id: String,
    pub release: AppReleaseSummary,
    /// The release's signed capabilities, which the owner approved.
    pub capabilities: serde_json::Value,
    pub automations: Vec<AppAutomationSummary>,
    pub inbox_routes: Vec<AppInboxRouteSummary>,
    pub connections: Vec<AppConnectionSummary>,
    /// Protocol 367: the deployment whose copy this installation is. Such an
    /// installation is absent from `ListAppInstallations`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
}

/// Protocol 367: the Apps a workflow publication would deploy with, read-only
/// and before any export: its pinned App plan, or else the plan its owner's
/// current App set gives. Each planned App also carries its signed
/// `capabilities` for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewDeploymentAppsRequest {
    pub session_id: String,
    pub publication_ref: String,
    /// Protocol 377: also returns this release's recorded App plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_digest: Option<String>,
}

/// Protocol 367: asks the owner once to deploy a workflow together with the
/// Apps of its publication's pinned App plan (`WorkflowPublicationDefinition.apps`).
/// The same `request_id` replays and reports the answer; a new one asks again.
/// No request can supply an approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareDeploymentAppsRequest {
    /// The publication's session, which shows the prompt.
    pub session_id: String,
    pub request_id: String,
    pub publication_ref: String,
    pub deployment_id: String,
    pub release_id: String,
    pub package_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentAppsConsentStatus {
    AwaitingApproval,
    Approved,
    Declined,
    Expired,
}

/// The owner's consent to install a deployment's Apps and share their
/// connections with its copy. An approved consent lets the kernel install
/// exactly these releases, for this deployment, without asking again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentAppsConsent {
    pub request_id: String,
    pub interaction_id: String,
    pub deployment_id: String,
    pub release_id: String,
    pub package_digest: String,
    pub status: DeploymentAppsConsentStatus,
    pub expires_at_ms: u64,
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
    pub delivery_mode: NotificationDeliveryMode,
}

/// Protocol 409: a human accepts one pending clipboard/link request. The
/// payload comes from the kernel, never from the accepting terminal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptAppHostActionRequest {
    pub session_id: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppHostAction {
    ClipboardWrite { text: String },
    OpenLink { url: String },
}

/// Protocol 410: owner-scoped restore of one saved same-generation snapshot.
/// The worker is drained and left stopped. No authority comes from the copy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreAppDataSnapshotRequest {
    pub installation_id: String,
    pub expected_generation: String,
    pub snapshot_id: String,
}
