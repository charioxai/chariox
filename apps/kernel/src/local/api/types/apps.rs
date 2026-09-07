use super::*;

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
