use serde::Deserialize;
use serde_json::Value;

use crate::error::DaemonError;
use crate::local::{
    ManagedEnvironmentAutoStopPolicy, ManagedEnvironmentComputeClassOption,
    ManagedEnvironmentContextPlan, ManagedEnvironmentContextSource,
    ManagedEnvironmentContextSourceOption, ManagedEnvironmentDesiredState,
    ManagedEnvironmentDevelopmentSetup, ManagedEnvironmentGitCredentials,
    ManagedEnvironmentKernelContextSelection, ManagedEnvironmentObservedState,
    ManagedEnvironmentOperationKind, ManagedEnvironmentOperationStatus,
    ManagedEnvironmentOperationSummary, ManagedEnvironmentProviderAccountSelection,
    ManagedEnvironmentProviderAccounts, ManagedEnvironmentReimageReceipt,
    ManagedEnvironmentReimageReceiptStatus, ManagedEnvironmentReimageResult,
    ManagedEnvironmentRepositoryRole, ManagedEnvironmentRepositorySelection,
    ManagedEnvironmentResult, ManagedEnvironmentSummary,
};
use crate::managed_context::outbound_service::{
    ManagedContextTransferTarget, ManagedContextTransferTicket,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OptionsResponse {
    pub(super) compute_classes: Vec<ComputeClassOption>,
    pub(super) context_sources: Vec<ContextSourceOption>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnvironmentsResponse {
    pub(super) environments: Vec<EnvironmentSummary>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnvironmentDetailsResponse {
    pub(super) environment: EnvironmentSummary,
    #[allow(dead_code)]
    pub(super) operations: Vec<OperationSummary>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnvironmentResult {
    environment: EnvironmentSummary,
    operation: OperationSummary,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReimageResult {
    environment: EnvironmentSummary,
    operation: OperationSummary,
    receipt: ReimageReceipt,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReimageReceipt {
    receipt_id: String,
    environment_id: String,
    operation_id: String,
    previous_generation: u64,
    generation: u64,
    status: ManagedEnvironmentReimageReceiptStatus,
    fresh_equivalent: bool,
    provider_server_id: String,
    previous_provider_image_id: Option<String>,
    provider_image_id: String,
    provider_profile_id: String,
    provider_profile_digest: String,
    runtime_release_digest: String,
    old_machine_id: Option<String>,
    new_machine_id: Option<String>,
    old_kernel_id: Option<String>,
    new_kernel_id: Option<String>,
    old_relay_realm_id: Option<String>,
    new_relay_realm_id: Option<String>,
    old_relay_target_id: Option<String>,
    new_relay_target_id: Option<String>,
    old_bootstrap_grant_id: Option<String>,
    new_bootstrap_grant_id: Option<String>,
    old_credential_ids: Value,
    new_credential_ids: Value,
    runtime_evidence: Value,
    source_evidence: Value,
    residue_checks: Value,
    revocations: Value,
    billing_observation: Value,
    resource_observation: Value,
    cleanup_state: Value,
    rollback_state: Value,
    receipt_digest: Option<String>,
    failure_code: Option<String>,
    failure_message: Option<String>,
    requested_at: String,
    completed_at: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContextTransferTicket {
    environment_id: String,
    context_plan: ContextPlan,
    target: ContextTransferTarget,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextTransferTarget {
    relay_realm_id: String,
    machine_id: String,
    kernel_id: String,
    relay_public_key: String,
    key_thumbprint: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ComputeClassOption {
    compute_class: String,
    regions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContextSourceOption {
    source_target_id: String,
    machine_id: String,
    kernel_id: String,
    label: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnvironmentSummary {
    environment_id: String,
    account_id: String,
    created_by_user_id: String,
    name: String,
    region: String,
    compute_class: String,
    managed_repository_root: String,
    desired_state: ManagedEnvironmentDesiredState,
    observed_state: ManagedEnvironmentObservedState,
    desired_revision: u64,
    observed_revision: u64,
    runtime_machine_id: Option<String>,
    #[serde(default)]
    runtime_kernel_id: Option<String>,
    runtime_release_digest: Option<String>,
    context_plan: ContextPlan,
    context_manifest_digest: Option<String>,
    auto_stop_policy: AutoStopPolicy,
    last_error_code: Option<String>,
    last_error_message: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutoStopPolicy {
    minimum_runtime_seconds: u64,
    idle_delay_seconds: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextPlan {
    schema_version: u32,
    context_id: String,
    plan_digest: String,
    source: Option<ContextSource>,
    kernel_context: ManagedEnvironmentKernelContextSelection,
    development_setup: DevelopmentSetup,
    provider_accounts: ProviderAccounts,
    git_credentials: GitCredentials,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextSource {
    source_target_id: String,
    relay_realm_id: String,
    machine_id: String,
    kernel_id: String,
    key_thumbprint: String,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum DevelopmentSetup {
    Empty,
    SourceProject {
        project_id: String,
        repositories: Vec<RepositorySelection>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositorySelection {
    role: ManagedEnvironmentRepositoryRole,
    workspace_id: String,
    worktree_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum ProviderAccounts {
    None,
    Selected {
        accounts: Vec<ProviderAccountSelection>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderAccountSelection {
    provider: String,
    account_profile: String,
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum GitCredentials {
    None,
    Selected { credential_ids: Vec<String> },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationSummary {
    operation_id: String,
    environment_id: String,
    requested_by_user_id: String,
    kind: ManagedEnvironmentOperationKind,
    idempotency_key: String,
    request_digest: String,
    desired_revision: u64,
    status: ManagedEnvironmentOperationStatus,
    attempt: u64,
    retryable: bool,
    failure_code: Option<String>,
    failure_message: Option<String>,
    completed_at: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<ComputeClassOption> for ManagedEnvironmentComputeClassOption {
    fn from(value: ComputeClassOption) -> Self {
        Self {
            compute_class: value.compute_class,
            regions: value.regions,
        }
    }
}

impl From<ContextSourceOption> for ManagedEnvironmentContextSourceOption {
    fn from(value: ContextSourceOption) -> Self {
        Self {
            source_target_id: value.source_target_id,
            machine_id: value.machine_id,
            kernel_id: value.kernel_id,
            label: value.label,
        }
    }
}

impl From<EnvironmentSummary> for ManagedEnvironmentSummary {
    fn from(value: EnvironmentSummary) -> Self {
        Self {
            environment_id: value.environment_id,
            account_id: value.account_id,
            created_by_user_id: value.created_by_user_id,
            name: value.name,
            region: value.region,
            compute_class: value.compute_class,
            managed_repository_root: value.managed_repository_root,
            desired_state: value.desired_state,
            observed_state: value.observed_state,
            desired_revision: value.desired_revision,
            observed_revision: value.observed_revision,
            runtime_machine_id: value.runtime_machine_id,
            runtime_kernel_id: value.runtime_kernel_id,
            runtime_release_digest: value.runtime_release_digest,
            context_plan: value.context_plan.into(),
            context_manifest_digest: value.context_manifest_digest,
            auto_stop_policy: value.auto_stop_policy.into(),
            last_error_code: value.last_error_code,
            last_error_message: value.last_error_message,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<AutoStopPolicy> for ManagedEnvironmentAutoStopPolicy {
    fn from(value: AutoStopPolicy) -> Self {
        Self {
            minimum_runtime_seconds: value.minimum_runtime_seconds,
            idle_delay_seconds: value.idle_delay_seconds,
        }
    }
}

impl From<ContextPlan> for ManagedEnvironmentContextPlan {
    fn from(value: ContextPlan) -> Self {
        Self {
            schema_version: value.schema_version,
            context_id: value.context_id,
            plan_digest: value.plan_digest,
            source: value.source.map(Into::into),
            kernel_context: value.kernel_context,
            development_setup: value.development_setup.into(),
            provider_accounts: value.provider_accounts.into(),
            git_credentials: value.git_credentials.into(),
        }
    }
}

impl From<ContextSource> for ManagedEnvironmentContextSource {
    fn from(value: ContextSource) -> Self {
        Self {
            source_target_id: value.source_target_id,
            relay_realm_id: value.relay_realm_id,
            machine_id: value.machine_id,
            kernel_id: value.kernel_id,
            key_thumbprint: value.key_thumbprint,
        }
    }
}

impl From<DevelopmentSetup> for ManagedEnvironmentDevelopmentSetup {
    fn from(value: DevelopmentSetup) -> Self {
        match value {
            DevelopmentSetup::Empty => Self::Empty,
            DevelopmentSetup::SourceProject {
                project_id,
                repositories,
            } => Self::SourceProject {
                project_id,
                repositories: repositories.into_iter().map(Into::into).collect(),
            },
        }
    }
}

impl From<RepositorySelection> for ManagedEnvironmentRepositorySelection {
    fn from(value: RepositorySelection) -> Self {
        Self {
            role: value.role,
            workspace_id: value.workspace_id,
            worktree_id: value.worktree_id,
        }
    }
}

impl From<ProviderAccounts> for ManagedEnvironmentProviderAccounts {
    fn from(value: ProviderAccounts) -> Self {
        match value {
            ProviderAccounts::None => Self::None,
            ProviderAccounts::Selected { accounts } => Self::Selected {
                accounts: accounts.into_iter().map(Into::into).collect(),
            },
        }
    }
}

impl From<ProviderAccountSelection> for ManagedEnvironmentProviderAccountSelection {
    fn from(value: ProviderAccountSelection) -> Self {
        Self {
            provider: value.provider,
            account_profile: value.account_profile,
        }
    }
}

impl From<GitCredentials> for ManagedEnvironmentGitCredentials {
    fn from(value: GitCredentials) -> Self {
        match value {
            GitCredentials::None => Self::None,
            GitCredentials::Selected { credential_ids } => Self::Selected { credential_ids },
        }
    }
}

impl From<OperationSummary> for ManagedEnvironmentOperationSummary {
    fn from(value: OperationSummary) -> Self {
        Self {
            operation_id: value.operation_id,
            environment_id: value.environment_id,
            requested_by_user_id: value.requested_by_user_id,
            kind: value.kind,
            idempotency_key: value.idempotency_key,
            request_digest: value.request_digest,
            desired_revision: value.desired_revision,
            status: value.status,
            attempt: value.attempt,
            retryable: value.retryable,
            failure_code: value.failure_code,
            failure_message: value.failure_message,
            completed_at: value.completed_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<EnvironmentResult> for ManagedEnvironmentResult {
    fn from(value: EnvironmentResult) -> Self {
        Self {
            environment: value.environment.into(),
            operation: value.operation.into(),
        }
    }
}

impl From<ReimageReceipt> for ManagedEnvironmentReimageReceipt {
    fn from(value: ReimageReceipt) -> Self {
        Self {
            receipt_id: value.receipt_id,
            environment_id: value.environment_id,
            operation_id: value.operation_id,
            previous_generation: value.previous_generation,
            generation: value.generation,
            status: value.status,
            fresh_equivalent: value.fresh_equivalent,
            provider_server_id: value.provider_server_id,
            previous_provider_image_id: value.previous_provider_image_id,
            provider_image_id: value.provider_image_id,
            provider_profile_id: value.provider_profile_id,
            provider_profile_digest: value.provider_profile_digest,
            runtime_release_digest: value.runtime_release_digest,
            old_machine_id: value.old_machine_id,
            new_machine_id: value.new_machine_id,
            old_kernel_id: value.old_kernel_id,
            new_kernel_id: value.new_kernel_id,
            old_relay_realm_id: value.old_relay_realm_id,
            new_relay_realm_id: value.new_relay_realm_id,
            old_relay_target_id: value.old_relay_target_id,
            new_relay_target_id: value.new_relay_target_id,
            old_bootstrap_grant_id: value.old_bootstrap_grant_id,
            new_bootstrap_grant_id: value.new_bootstrap_grant_id,
            old_credential_ids: value.old_credential_ids,
            new_credential_ids: value.new_credential_ids,
            runtime_evidence: value.runtime_evidence,
            source_evidence: value.source_evidence,
            residue_checks: value.residue_checks,
            revocations: value.revocations,
            billing_observation: value.billing_observation,
            resource_observation: value.resource_observation,
            cleanup_state: value.cleanup_state,
            rollback_state: value.rollback_state,
            receipt_digest: value.receipt_digest,
            failure_code: value.failure_code,
            failure_message: value.failure_message,
            requested_at: value.requested_at,
            completed_at: value.completed_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<ReimageResult> for ManagedEnvironmentReimageResult {
    fn from(value: ReimageResult) -> Self {
        Self {
            environment: value.environment.into(),
            operation: value.operation.into(),
            receipt: value.receipt.into(),
        }
    }
}

impl ContextTransferTicket {
    pub(super) fn into_ticket(self) -> Result<ManagedContextTransferTicket, DaemonError> {
        let context_plan = ManagedEnvironmentContextPlan::from(self.context_plan);
        let context_plan = serde_json::from_value(
            serde_json::to_value(context_plan).map_err(ticket_decode_error)?,
        )
        .map_err(ticket_decode_error)?;
        Ok(ManagedContextTransferTicket {
            environment_id: self.environment_id,
            context_plan,
            target: ManagedContextTransferTarget {
                relay_realm_id: self.target.relay_realm_id,
                machine_id: self.target.machine_id,
                kernel_id: self.target.kernel_id,
                relay_public_key: self.target.relay_public_key,
                key_thumbprint: self.target.key_thumbprint,
            },
        })
    }
}

fn ticket_decode_error(error: impl std::fmt::Display) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "decode managed context transfer ticket",
        message: error.to_string(),
    }
}
