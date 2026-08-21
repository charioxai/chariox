use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListManagedEnvironmentCatalogRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetManagedEnvironmentRequest {
    pub environment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateManagedEnvironmentRequest {
    pub client_request_id: String,
    pub name: String,
    pub region: String,
    pub compute_class: String,
    pub auto_stop_policy: ManagedEnvironmentAutoStopPolicy,
    pub context_plan: ManagedEnvironmentContextPlanInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestManagedEnvironmentLifecycleRequest {
    pub environment_id: String,
    pub action: ManagedEnvironmentLifecycleAction,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedEnvironmentLifecycleAction {
    Start,
    Stop,
    Restart,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentAutoStopPolicy {
    pub minimum_runtime_seconds: u64,
    pub idle_delay_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentContextPlanInput {
    pub source_target_id: Option<String>,
    pub kernel_context: ManagedEnvironmentKernelContextSelection,
    pub development_setup: ManagedEnvironmentDevelopmentSetup,
    pub provider_accounts: ManagedEnvironmentProviderAccounts,
    pub git_credentials: ManagedEnvironmentGitCredentials,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedEnvironmentKernelContextSelection {
    Empty,
    SourceKernel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ManagedEnvironmentDevelopmentSetup {
    Empty,
    SourceProject {
        project_id: String,
        repositories: Vec<ManagedEnvironmentRepositorySelection>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentRepositorySelection {
    pub role: ManagedEnvironmentRepositoryRole,
    pub workspace_id: String,
    pub worktree_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedEnvironmentRepositoryRole {
    Primary,
    Supporting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ManagedEnvironmentProviderAccounts {
    None,
    Selected {
        accounts: Vec<ManagedEnvironmentProviderAccountSelection>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentProviderAccountSelection {
    pub provider: String,
    pub account_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ManagedEnvironmentGitCredentials {
    None,
    Selected { credential_ids: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentCatalog {
    pub compute_classes: Vec<ManagedEnvironmentComputeClassOption>,
    pub context_sources: Vec<ManagedEnvironmentContextSourceOption>,
    pub environments: Vec<ManagedEnvironmentSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentComputeClassOption {
    pub compute_class: String,
    pub regions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentContextSourceOption {
    pub source_target_id: String,
    pub machine_id: String,
    pub kernel_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentSummary {
    pub environment_id: String,
    pub account_id: String,
    pub created_by_user_id: String,
    pub name: String,
    pub region: String,
    pub compute_class: String,
    pub desired_state: ManagedEnvironmentDesiredState,
    pub observed_state: ManagedEnvironmentObservedState,
    pub desired_revision: u64,
    pub observed_revision: u64,
    pub runtime_machine_id: Option<String>,
    pub runtime_release_digest: Option<String>,
    pub context_plan: ManagedEnvironmentContextPlan,
    pub context_manifest_digest: Option<String>,
    pub auto_stop_policy: ManagedEnvironmentAutoStopPolicy,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedEnvironmentDesiredState {
    Running,
    Stopped,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedEnvironmentObservedState {
    Requested,
    Provisioning,
    Bootstrapping,
    AwaitingContext,
    Ready,
    Starting,
    Stopping,
    Stopped,
    Deleting,
    Deleted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentContextPlan {
    pub schema_version: u32,
    pub context_id: String,
    pub plan_digest: String,
    pub source: Option<ManagedEnvironmentContextSource>,
    pub kernel_context: ManagedEnvironmentKernelContextSelection,
    pub development_setup: ManagedEnvironmentDevelopmentSetup,
    pub provider_accounts: ManagedEnvironmentProviderAccounts,
    pub git_credentials: ManagedEnvironmentGitCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentContextSource {
    pub source_target_id: String,
    pub relay_realm_id: String,
    pub machine_id: String,
    pub kernel_id: String,
    pub key_thumbprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentOperationSummary {
    pub operation_id: String,
    pub environment_id: String,
    pub requested_by_user_id: String,
    pub kind: ManagedEnvironmentOperationKind,
    pub idempotency_key: String,
    pub request_digest: String,
    pub desired_revision: u64,
    pub status: ManagedEnvironmentOperationStatus,
    pub attempt: u64,
    pub retryable: bool,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedEnvironmentOperationKind {
    Create,
    Start,
    Stop,
    Restart,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedEnvironmentOperationStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedEnvironmentResult {
    pub environment: ManagedEnvironmentSummary,
    pub operation: ManagedEnvironmentOperationSummary,
}
