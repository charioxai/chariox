//! MP-08: Full protocol-487 Environment contract. P01 serves Get only.
use super::*;
use crate::project_environment::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetProjectEnvironmentRequest {
    pub project_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectProjectEnvironmentRequest {
    pub project_id: String,
    pub operation_id: String,
    pub folder_ids: Vec<String>,
    pub target: EnvironmentTargetBinding,
    pub provider: Option<EnvironmentProvider>,
    pub allow_model_folders: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewEnvironmentDiffRequest {
    pub project_id: String,
    pub expected_revision: u64,
    pub draft: EnvironmentRevisionDraft,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveProjectEnvironmentRevisionRequest {
    pub project_id: String,
    pub expected_revision: u64,
    pub expected_content_digest: String,
    pub draft: EnvironmentRevisionDraft,
    pub accepted_proposal_ids: Vec<String>,
    pub excluded_proposal_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanProjectEnvironmentRequest {
    pub project_id: String,
    pub expected_revision: u64,
    pub revision_digest: String,
    pub target: EnvironmentTargetBinding,
    pub selected_items: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyProjectEnvironmentRequest {
    pub project_id: String,
    pub operation_id: String,
    pub plan_id: String,
    pub expected_revision: u64,
    pub revision_digest: String,
    pub target: EnvironmentTargetBinding,
    pub selected_items: Vec<String>,
    pub per_item_opt_ins: Vec<EnvironmentItemOptIn>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckProjectEnvironmentRequest {
    pub project_id: String,
    pub operation_id: String,
    pub revision_digest: String,
    pub target: EnvironmentTargetBinding,
    pub selected_items: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentOperationRequest {
    pub project_id: String,
    pub operation_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryEnvironmentOperationRequest {
    pub project_id: String,
    pub operation_id: String,
    pub expected_attempt: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportProjectEnvironmentRequest {
    pub project_id: String,
    pub operation_id: String,
    pub expected_revision: u64,
    pub revision_digest: String,
    pub selected_items: Vec<String>,
    pub selected_files: Vec<EnvironmentFile>,
    pub destination: EnvironmentExportDestination,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentExportDestination {
    File {
        path: String,
    },
    Kernel {
        kernel_id: String,
        target_key_thumbprint: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewEnvironmentImportRequest {
    pub artifact_id: String,
    pub artifact_digest: String,
    pub choice: EnvironmentImportChoice,
    pub folder_map: Vec<EnvironmentFolderBinding>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitEnvironmentImportRequest {
    pub operation_id: String,
    pub preview_id: String,
    pub preview_digest: String,
    pub choice: EnvironmentImportChoice,
}

impl LocalDaemonRequest {
    pub(crate) fn unsupported_environment_capability(&self) -> Option<EnvironmentCapability> {
        Some(match self {
            Self::DetectProjectEnvironment(_) => EnvironmentCapability::Detect,
            Self::PreviewEnvironmentDiff(_) => EnvironmentCapability::PreviewDiff,
            Self::SaveProjectEnvironmentRevision(_) => EnvironmentCapability::Save,
            Self::PlanProjectEnvironment(_) => EnvironmentCapability::Plan,
            Self::ApplyProjectEnvironment(_) => EnvironmentCapability::Apply,
            Self::CheckProjectEnvironment(_) => EnvironmentCapability::Check,
            Self::GetEnvironmentOperation(_) => EnvironmentCapability::GetOperation,
            Self::CancelEnvironmentOperation(_) => EnvironmentCapability::CancelOperation,
            Self::RetryEnvironmentOperation(_) => EnvironmentCapability::RetryOperation,
            Self::ExportProjectEnvironment(_) => EnvironmentCapability::Export,
            Self::PreviewEnvironmentImport(_) => EnvironmentCapability::PreviewImport,
            Self::CommitEnvironmentImport(_) => EnvironmentCapability::CommitImport,
            _ => return None,
        })
    }
}
