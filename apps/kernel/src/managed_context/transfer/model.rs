use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::managed_context::package::ManagedContextPlanBinding;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ArmManagedContextTransfer {
    pub plan: ManagedContextPlanBinding,
    pub target_environment_id: String,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
    pub source_kernel_id: String,
    pub source_key_thumbprint: String,
    pub owner_user_id: String,
    pub realm_id: String,
    pub capability: String,
    pub archive_sha256: String,
    pub archive_size_bytes: u64,
    pub destination_parent: PathBuf,
    pub expires_at_ms: u64,
}

impl std::fmt::Debug for ArmManagedContextTransfer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArmManagedContextTransfer")
            .field("plan", &self.plan)
            .field("target_environment_id", &self.target_environment_id)
            .field("target_kernel_id", &self.target_kernel_id)
            .field("target_key_thumbprint", &self.target_key_thumbprint)
            .field("source_kernel_id", &self.source_kernel_id)
            .field("source_key_thumbprint", &self.source_key_thumbprint)
            .field("owner_user_id", &self.owner_user_id)
            .field("realm_id", &self.realm_id)
            .field("capability", &"[REDACTED]")
            .field("archive_sha256", &self.archive_sha256)
            .field("archive_size_bytes", &self.archive_size_bytes)
            .field("destination_parent", &self.destination_parent)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ArmedManagedContextTransfer {
    pub transfer_id: String,
    pub capability: String,
    pub expires_at_ms: u64,
}

impl std::fmt::Debug for ArmedManagedContextTransfer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArmedManagedContextTransfer")
            .field("transfer_id", &self.transfer_id)
            .field("capability", &"[REDACTED]")
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
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
    pub plan: ManagedContextPlanBinding,
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
