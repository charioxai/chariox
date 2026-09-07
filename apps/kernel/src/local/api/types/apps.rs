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
    StorageUnavailable,
}
