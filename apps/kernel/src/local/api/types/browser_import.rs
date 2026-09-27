use super::*;

/// Consent metadata only. Cookie values never enter the terminal request path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserImportSelection {
    pub session_id: String,
    pub attachment_id: String,
    pub environment_id: String,
    pub runtime_generation: u64,
    pub tab_id: String,
    pub document_revision: u64,
    pub source_store_id: String,
    pub domains: Vec<String>,
    pub partition_sites: Vec<String>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareBrowserImportRequest {
    pub selection: BrowserImportSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveBrowserImportRequest {
    pub request_id: String,
    pub selection: BrowserImportSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserImportSourceRequest {
    pub request_id: String,
    pub selection: BrowserImportSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelBrowserImportRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserImportConsentStatus {
    Prepared,
    Approved,
    SourceClaimed,
    SourceAuthorized,
    Cancelled,
}
