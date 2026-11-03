//! The Project Environment contract: one saved record per Project, its presence on this kernel and
//! the Setup runs. No request or response ever carries a Secret value out of the kernel.
use super::*;
use crate::project_environment::{FilePresence, SecretPresence};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetProjectEnvironmentRequest {
    pub project_id: String,
    /// A folder of the Project on this kernel; absent means the Project's first folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvironmentSetupField {
    pub script: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvironmentSecretName {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvironmentFilePath {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveProjectEnvironmentRequest {
    pub project_id: String,
    pub expected_revision: u64,
    pub setup: ProjectEnvironmentSetupField,
    pub variables: String,
    pub secrets: Vec<ProjectEnvironmentSecretName>,
    pub files: Vec<ProjectEnvironmentFilePath>,
}

/// The owner puts a Secret's value into this kernel's Vault under the Project scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetProjectEnvironmentSecretRequest {
    pub project_id: String,
    pub name: String,
    pub value: VaultPassphrase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunProjectEnvironmentSetupRequest {
    pub project_id: String,
    /// Chosen by the client; running the same id again answers the existing operation.
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentOperationRequest {
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvironmentSecretView {
    pub name: String,
    pub presence: SecretPresence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEnvironmentFilePresenceKind {
    Present,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvironmentFileView {
    pub path: String,
    pub presence: ProjectEnvironmentFilePresenceKind,
    pub size_bytes: Option<u64>,
}

impl ProjectEnvironmentFileView {
    pub fn new(path: String, presence: FilePresence) -> Self {
        let (presence, size_bytes) = match presence {
            FilePresence::Present { size_bytes } => (
                ProjectEnvironmentFilePresenceKind::Present,
                Some(size_bytes),
            ),
            FilePresence::Missing => (ProjectEnvironmentFilePresenceKind::Missing, None),
        };
        Self {
            path,
            presence,
            size_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ProjectEnvironmentSetupState {
    /// Never run on this checkout, or the script changed since the last run.
    NotRun,
    #[serde(rename_all = "camelCase")]
    Ok {
        operation_id: String,
        ended_at_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        operation_id: String,
        ended_at_ms: u64,
        exit_code: Option<i32>,
        reason: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Cancelled {
        operation_id: String,
        ended_at_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    Running {
        operation_id: String,
        started_at_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvironmentView {
    pub project_id: String,
    /// False until the first Save; then revision 0 and every field empty.
    pub exists: bool,
    pub revision: u64,
    pub setup: ProjectEnvironmentSetupField,
    pub variables: String,
    pub secrets: Vec<ProjectEnvironmentSecretView>,
    pub files: Vec<ProjectEnvironmentFileView>,
    pub changed_at_ms: u64,
    pub machine_label: String,
    pub checkout_path: String,
    pub setup_state: ProjectEnvironmentSetupState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentOperationKind {
    Setup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentOperationState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentOperationView {
    pub operation_id: String,
    pub project_id: String,
    pub checkout_path: String,
    pub kind: EnvironmentOperationKind,
    pub state: EnvironmentOperationState,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub reason: Option<String>,
    /// The last 64 KiB of output, with Secret values replaced by their names in brackets.
    pub output_tail: String,
}
