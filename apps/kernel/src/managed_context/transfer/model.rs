use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArmManagedContextTransfer {
    pub context_id: String,
    pub target_environment_id: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub owner_user_id: String,
    pub realm_id: String,
    pub project_id: String,
    pub archive_sha256: String,
    pub archive_size_bytes: u64,
    pub destination_parent: PathBuf,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArmedManagedContextTransfer {
    pub transfer_id: String,
    pub capability: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedContextTransferCaller {
    pub kernel_id: String,
    pub key_thumbprint: String,
    pub owner_user_id: String,
    pub realm_id: String,
    pub target_environment_id: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ManagedContextTransferPhase {
    Armed,
    Receiving,
    ReadyToImport,
    Importing,
    Failed,
    Consumed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedContextTransferStatus {
    pub transfer_id: String,
    pub phase: ManagedContextTransferPhase,
    pub accepted_bytes: u64,
    pub archive_size_bytes: u64,
    pub expires_at_ms: u64,
    pub import_receipt_sha256: Option<String>,
    pub import_receipt_json: Option<String>,
    pub failure_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadyManagedContextImport {
    pub transfer_id: String,
    pub archive_path: PathBuf,
    pub context_id: String,
    pub project_id: String,
    pub archive_sha256: String,
    pub destination_root: PathBuf,
    pub target_environment_id: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManagedContextImportClaim {
    Claimed(ReadyManagedContextImport),
    InProgress(ManagedContextTransferStatus),
    Terminal(ManagedContextTransferStatus),
}
