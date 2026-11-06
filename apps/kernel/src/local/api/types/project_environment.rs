//! The Project Environment contract: one saved record per Project, its presence on this kernel and
//! the Setup runs. No request or response ever carries a Secret value out of the kernel.
use super::*;
use crate::project_environment::{ExtensionSource, FilePresence, SecretPresence};

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
    pub mcps: Vec<ProjectEnvironmentExtension>,
    pub skills: Vec<ProjectEnvironmentExtension>,
}

/// An MCP server or skill of the Project: declared by one of its files, or registered with Chariox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvironmentExtension {
    pub name: String,
    pub source: ExtensionSource,
}

/// Whether the declaring file still declares the entry on this kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionPresence {
    Present,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectEnvironmentExtensionView {
    pub name: String,
    pub source: ExtensionSource,
    pub presence: ExtensionPresence,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentEntryKind {
    Secret,
    File,
    Mcp,
    Skill,
}

/// One Secret (by name) or File (by relative path) of the Environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentEntryRef {
    pub kind: EnvironmentEntryKind,
    pub name: String,
}

/// Brings entries this kernel lacks from another kernel of the same user: the record when the
/// source's revision is newer, then the Secret values and Files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportProjectEnvironmentEntriesRequest {
    pub project_id: String,
    pub operation_id: String,
    pub source_kernel_id: String,
    /// Absent means every entry this kernel is missing (Update).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<EnvironmentEntryRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_path: Option<String>,
}

/// An idle agent of the Project on this kernel reads the Project's files and writes the Environment fields as revision 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftProjectEnvironmentRequest {
    pub project_id: String,
    pub operation_id: String,
}

/// Runs Setup on another kernel of the same user, in the checkout that kernel holds for the Project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunRemoteProjectEnvironmentSetupRequest {
    pub project_id: String,
    pub operation_id: String,
    pub target_kernel_id: String,
    /// A checkout of the Project on the target kernel; absent means its first folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_path: Option<String>,
}

/// Reads or stops (`stop`) an operation that runs on another kernel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteEnvironmentOperationRequest {
    pub operation_id: String,
    pub target_kernel_id: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stop: bool,
}

/// Reads the Project's files and the saved Environment and proposes changes; nothing is applied.
/// The kernel always compares what the files declare (no model); with `with_agent` an idle agent of the Project adds its own proposals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckProjectEnvironmentRequest {
    pub project_id: String,
    pub operation_id: String,
    pub with_agent: bool,
}

/// Accept applies one proposal as a Save; dismissing it keeps the agent from proposing it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveEnvironmentProposalRequest {
    pub project_id: String,
    pub proposal_id: String,
    pub accept: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentProposalField {
    Setup,
    Variables,
    Secret,
    File,
    Mcp,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentProposalChange {
    Add,
    Remove,
}

/// Where in the Project the agent found the reason, as one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentProposalEvidence {
    pub path: String,
    pub excerpt: String,
}

/// One change to a line of Setup or Variables, a Secret name or a File path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentProposal {
    pub id: String,
    pub field: EnvironmentProposalField,
    pub change: EnvironmentProposalChange,
    pub text: String,
    pub evidence: EnvironmentProposalEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentOperationRequest {
    pub operation_id: String,
}

/// Reads an operation. With `wait_after_updates` the answer is held (at most 20 s) until the
/// operation has more updates than the reader has seen or has ended, so a change reaches the
/// reader as soon as it happens and an idle operation costs no polling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetEnvironmentOperationRequest {
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_after_updates: Option<u64>,
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
    pub mcps: Vec<ProjectEnvironmentExtensionView>,
    pub skills: Vec<ProjectEnvironmentExtensionView>,
    pub changed_at_ms: u64,
    pub machine_label: String,
    pub checkout_path: String,
    pub setup_state: ProjectEnvironmentSetupState,
    /// Changes an agent proposed and nobody has accepted or dismissed yet.
    pub proposals: Vec<EnvironmentProposal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentOperationKind {
    Setup,
    Import,
    Draft,
    Check,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentOperationState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentOperationEntryState {
    Pending,
    Importing,
    Imported,
    Failed,
}

/// One entry of an Import: where it stands and, when it failed, why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentOperationEntry {
    pub kind: EnvironmentEntryKind,
    pub name: String,
    pub state: EnvironmentOperationEntryState,
    pub reason: Option<String>,
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
    /// Counts every change of this view, so a reader can wait for the next one.
    pub updates: u64,
    /// The last 64 KiB of output, with Secret values replaced by their names in brackets.
    pub output_tail: String,
    /// Import only: the entries and their results, in order.
    pub entries: Vec<EnvironmentOperationEntry>,
    /// Import only: the display name of the kernel the entries come from.
    pub source_label: Option<String>,
}
