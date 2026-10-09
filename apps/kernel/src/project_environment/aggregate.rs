//! MP-02 / MP-03 / MP-08: Project specification, independent of sessions and machines.
//! P01 projects references only; values and authorization never belong in this contract.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentLineage {
    pub project_id: String,
    pub environment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequirementScope {
    Project,
    Folder { folder_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequirementOrigin {
    Detected {
        folder_id: String,
        relative_path: String,
        line: Option<u32>,
        evidence_digest: String,
    },
    DetectedMetadata {
        source: String,
        reference: String,
    },
    UserAdded {
        user_id: String,
        revision: u64,
    },
    Migrated {
        source: String,
        digest: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentFileKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentFile {
    pub relative_path: String,
    pub kind: EnvironmentFileKind,
    pub content_digest: Option<String>,
    pub folder_id: String,
    pub user_selected: bool,
    pub git_ignored: Option<bool>,
    pub byte_count: Option<u64>,
    pub credential_filter_verdict: EnvironmentCredentialFilterVerdict,
    pub transfer_inclusion: EnvironmentTransferInclusion,
    pub reason: Option<String>,
    pub secret_looking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentInstallScope {
    Project,
    User,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPlatform {
    pub os: String,
    pub architecture: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentProbe {
    Executable {
        name: String,
        arguments: Vec<String>,
        timeout_ms: u32,
    },
    Tcp {
        host: String,
        port: u16,
        timeout_ms: u32,
    },
    Http {
        url: String,
        expected_status: u16,
        timeout_ms: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentVaultReference {
    pub service: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentProvider {
    Codex,
    Claude,
    #[serde(rename = "opencode")]
    OpenCode,
}

/// Distinct from ExtensionKind::Connector (Chariox event connectors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolKind {
    Mcp,
    Skill,
    InstructionFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCommand {
    pub executable: String,
    pub arguments: Vec<String>,
    pub timeout_ms: u32,
    pub output_limit_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentExecutionBoundary {
    DisposableWorker,
    ProjectFolder,
}

/// References to the original recipe retain its evidence without projecting shell values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacySetupReference {
    pub definition_digest: String,
    pub origin: crate::session::ProjectEnvironmentDefinitionOrigin,
    pub source: crate::session::ProjectEnvironmentDefinitionSource,
    pub target_platform: String,
    pub source_path: Option<String>,
    pub inputs: Vec<crate::session::ProjectEnvironmentInput>,
    pub path_entries: Vec<crate::session::ProjectEnvironmentPathEntry>,
    pub setup_step_kinds: Vec<crate::session::ProjectEnvironmentSetupStepKind>,
    pub setup_command_digests: Vec<String>,
    pub validation_command_digests: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequirementSpec {
    Files {
        folder_id: String,
        entries: Vec<EnvironmentFile>,
    },
    Software {
        identity: String,
        version_constraint: Option<String>,
        platform: Option<EnvironmentPlatform>,
        install_scope: EnvironmentInstallScope,
        install_source: Option<String>,
        detect_only: bool,
    },
    Services {
        identity: String,
        target_binding: Option<EnvironmentTargetBinding>,
        probe: Option<EnvironmentProbe>,
        health_expectation: Option<EnvironmentHealthExpectation>,
        depends_on: Vec<String>,
    },
    Variables {
        name: String,
        locator: ProjectEnvironmentLocator,
    },
    Secrets {
        name: String,
        vault: Option<EnvironmentVaultReference>,
    },
    Accounts {
        provider: EnvironmentProvider,
        linked_profile_ref: Option<String>,
        required_capabilities: Vec<String>,
    },
    AgentTools {
        tool_kind: AgentToolKind,
        registry_ref: String,
        package_digest: Option<String>,
        scope: RequirementScope,
        runtime_requirements: Vec<String>,
        vault_refs: Vec<EnvironmentVaultReference>,
        compatible_providers: Vec<EnvironmentProvider>,
    },
    CharioxApps {
        app_id: String,
        version: Option<String>,
        content_digest: Option<String>,
        required_grants: Vec<String>,
    },
    SetupChecks {
        commands: Vec<EnvironmentCommand>,
        probes: Vec<EnvironmentProbe>,
        boundary: EnvironmentExecutionBoundary,
        source_path: Option<String>,
        source_digest: Option<String>,
        legacy: Option<LegacySetupReference>,
    },
    MachineNeeds {
        platforms: Vec<EnvironmentPlatform>,
        minimum_cpus: Option<u32>,
        minimum_memory_bytes: Option<u64>,
        minimum_free_disk_bytes: Option<u64>,
        gpu: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub requirement_id: String,
    pub title: String,
    pub scope: RequirementScope,
    pub origins: Vec<RequirementOrigin>,
    pub spec: RequirementSpec,
    pub depends_on: Vec<String>,
    pub platform_variants: Vec<EnvironmentPlatform>,
    pub required: bool,
    /// Original value-free manifest entry, including exclusion, locator and usage evidence.
    pub legacy_entry: Option<ProjectEnvironmentEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentGitReference {
    pub url: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentFolder {
    pub folder_id: String,
    pub portable_folder_key: String,
    pub label: String,
    /// Empty when this saved folder is no longer attached to the local Project.
    pub local_workspace_binding: String,
    pub optional_git: Option<EnvironmentGitReference>,
    pub requirements: Vec<Requirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentProposal {
    pub proposal_id: String,
    pub requirement: Requirement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentObservationStatus {
    Ready,
    Missing,
    Outdated,
    NeedsYourInput,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentTargetBinding {
    pub machine_id: String,
    pub target_instance_generation: String,
    pub slice_ref: Option<String>,
}

/// Measured facts are an allowlist, never arbitrary probe output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentMeasuredFacts {
    pub platform: Option<EnvironmentPlatform>,
    pub version: Option<String>,
    pub content_digest: Option<String>,
    pub present: Option<bool>,
    pub cpus: Option<u32>,
    pub memory_bytes: Option<u64>,
    pub free_disk_bytes: Option<u64>,
    pub gpu: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineObservation {
    pub environment_id: String,
    pub requirement_id: String,
    pub target: EnvironmentTargetBinding,
    pub revision_digest: String,
    pub status: EnvironmentObservationStatus,
    pub checked_at_ms: Option<u64>,
    pub measured_facts: EnvironmentMeasuredFacts,
    pub reason_code: String,
    pub safe_summary: String,
    pub check_operation_id: Option<String>,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentOperationKind {
    Detect,
    Apply,
    Check,
    Export,
    Import,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentOperationPhase {
    Requested,
    Preparing,
    AwaitingInput,
    Running,
    Validating,
    Ready,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentItemResult {
    pub requirement_id: String,
    pub status: EnvironmentObservationStatus,
    pub reason_code: String,
    pub safe_summary: String,
    pub receipt_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentReceipt {
    pub receipt_id: String,
    pub requirement_id: Option<String>,
    pub target: EnvironmentTargetBinding,
    pub revision_digest: String,
    pub artifact_digest: String,
    pub recorded_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCancellation {
    pub requested_at_ms: u64,
    pub settled_at_ms: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentOperation {
    pub operation_id: String,
    pub attempt: u32,
    pub local_project_id: String,
    pub revision_digest: String,
    pub target: EnvironmentTargetBinding,
    pub kind: EnvironmentOperationKind,
    pub phase: EnvironmentOperationPhase,
    pub selected_items: Vec<String>,
    pub per_item_opt_ins: Vec<EnvironmentItemOptIn>,
    pub per_item_results: Vec<EnvironmentItemResult>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub cancellation: Option<EnvironmentCancellation>,
    pub receipts: Vec<EnvironmentReceipt>,
    pub recovery_state: EnvironmentRecoveryState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironment {
    pub schema_version: u32,
    pub delivered_capabilities: DeliveredEnvironmentCapabilities,
    pub local_project_id: String,
    pub lineage: EnvironmentLineage,
    /// Zero is the unsaved read-through revision. P01 never saves a specification.
    pub revision: u64,
    pub content_digest: String,
    pub parent_revision_digest: Option<String>,
    pub evidence_digest: Option<String>,
    pub project_requirements: Vec<Requirement>,
    pub folders: Vec<EnvironmentFolder>,
    pub proposals: Vec<EnvironmentProposal>,
    pub reviewed_at_ms: Option<u64>,
    pub reviewed_by: Option<String>,
    pub observations: Vec<MachineObservation>,
    pub operations: Vec<EnvironmentOperation>,
    pub legacy_manifest: Option<ProjectEnvironmentManifest>,
    pub legacy_reviewed_manifest: Option<ProjectEnvironmentManifest>,
    pub legacy_review: Option<ProjectEnvironmentReview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentCredentialFilterVerdict {
    NotChecked,
    Clear,
    NeedsVault,
    Rejected,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentTransferInclusion {
    Include,
    Exclude,
    ReviewRequired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentHealthExpectation {
    Reachable,
    HttpStatus { status: u16 },
    CommandExit { exit_code: i32 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentCapability {
    Get,
    Detect,
    PreviewDiff,
    Save,
    Plan,
    Apply,
    Check,
    GetOperation,
    CancelOperation,
    RetryOperation,
    Export,
    PreviewImport,
    CommitImport,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveredEnvironmentCapabilities {
    pub enabled_environment_operations: Vec<EnvironmentCapability>,
    pub supported_schema: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentOptInDecision {
    Accepted,
    Excluded,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentItemOptIn {
    pub requirement_id: String,
    pub decision: EnvironmentOptInDecision,
    pub recorded_at_ms: u64,
    pub user_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentRecoveryState {
    Pending,
    Resumable {
        checkpoint_digest: String,
        last_settled_requirement_id: Option<String>,
    },
    Settled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentFolderSpecification {
    pub folder_id: String,
    pub portable_folder_key: String,
    pub label: String,
    pub optional_git: Option<EnvironmentGitReference>,
    pub requirements: Vec<Requirement>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionDraft {
    pub project_requirements: Vec<Requirement>,
    pub folders: Vec<EnvironmentFolderSpecification>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSealedPayloadDescriptor {
    pub artifact_id: String,
    pub content_digest: String,
    pub byte_count: u64,
    pub target_kernel_id: String,
    pub target_key_thumbprint: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentExportSource {
    pub kernel_id: String,
    pub project_label: String,
    pub exported_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPortableFolder {
    pub folder_id: String,
    pub portable_folder_key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentExportEnvelope {
    pub schema_version: u32,
    pub lineage: EnvironmentLineage,
    pub source_revision_digest: String,
    pub folder_map: Vec<EnvironmentPortableFolder>,
    pub requirement_spec: EnvironmentRevisionDraft,
    pub selected_file_manifest: Vec<EnvironmentFile>,
    pub artifact_digests: Vec<String>,
    pub source_metadata: EnvironmentExportSource,
    pub sealed_target_payload_descriptor: Option<EnvironmentSealedPayloadDescriptor>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentDiffKind {
    Added,
    Changed,
    Removed,
    Unchanged,
    Conflict,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRequirementDiff {
    pub requirement_id: String,
    pub kind: EnvironmentDiffKind,
    pub before: Option<Requirement>,
    pub after: Option<Requirement>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRevisionDiff {
    pub project_id: String,
    pub expected_revision: u64,
    pub source_digest: String,
    pub target_digest: String,
    pub requirements: Vec<EnvironmentRequirementDiff>,
    pub folder_map: Vec<EnvironmentFolderBinding>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentFolderBinding {
    pub folder_id: String,
    pub local_workspace_binding: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnvironmentImportChoice {
    New,
    Update {
        project_id: String,
        expected_revision: u64,
    },
    Replace {
        project_id: String,
        expected_revision: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentImportFileDiff {
    pub folder_id: String,
    pub relative_path: String,
    pub kind: EnvironmentDiffKind,
    pub before_digest: Option<String>,
    pub after_digest: Option<String>,
    pub byte_count: u64,
    pub credential_filter_verdict: EnvironmentCredentialFilterVerdict,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentImportPreview {
    pub preview_id: String,
    pub lineage: EnvironmentLineage,
    pub source_digest: String,
    pub matching_project_ids: Vec<String>,
    pub choice: EnvironmentImportChoice,
    pub folder_map: Vec<EnvironmentFolderBinding>,
    pub revision_diff: Option<EnvironmentRevisionDiff>,
    pub file_diff: Vec<EnvironmentImportFileDiff>,
    pub platform_gaps: Vec<String>,
    pub omitted_items: Vec<String>,
    pub expires_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentApplyPlan {
    pub plan_id: String,
    pub project_id: String,
    pub revision_digest: String,
    pub target: EnvironmentTargetBinding,
    pub selected_items: Vec<String>,
    pub required_opt_ins: Vec<String>,
    pub conflicts: Vec<EnvironmentItemResult>,
    pub expires_at_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentProviderAuthorizationLink {
    pub provider: EnvironmentProvider,
    pub linked_profile_ref: Option<String>,
    pub authorization_url: String,
    pub expires_at_ms: u64,
}
