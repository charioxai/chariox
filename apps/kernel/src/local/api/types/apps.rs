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
