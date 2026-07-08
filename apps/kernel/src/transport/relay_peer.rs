use serde::{Deserialize, Serialize};

use crate::agent::GitWorktreePlacement;
use crate::execution_lease::{ExecutionLease, LeasedAgent, RemoteWorkflowTurnContext};
use crate::history::{HistoryAttributionConfidence, HistoryEventKind, HistoryEventTurnContext};
use crate::io::WorkspaceIdentity;
use crate::mcp::ArrobaMcpServerConfig;
use crate::session::{PromptCancellation, PromptCompletion, PromptOrigin, PromptSubmissionOutcome};
use crate::skill::ArrobaSkillPackage;
use crate::terminal::TerminalOutputKind;

pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayPromptAttachment {
    pub url: String,
    pub mime: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspaceLiveSyncContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
    pub worker_kernel_id: String,
    pub worker_machine_id: String,
    pub worker_provider_run_id: String,
    pub worker_worktree_path: String,
    pub worker_workspace_identity: WorkspaceIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSkillSyncContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSkillMaterialization {
    pub name: String,
    pub version_hash: String,
    pub materialized_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMcpCheckContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteExtensionInvocationContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
    pub worker_provider_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_kernel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_machine_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteCredentialSecretInjection {
    Browser { target_url: String },
    Pty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteNativeInteractionContext {
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
    pub worker_provider_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredRemoteMcp {
    pub config: ArrobaMcpServerConfig,
    pub definition_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMcpAvailability {
    pub name: String,
    pub expected_hash: String,
    pub status: RemoteMcpAvailabilityStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteGitTurnContext {
    pub home_session_id: String,
    pub home_agent_id: String,
    pub home_prompt_id: String,
    pub home_turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_origin: Option<PromptOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_turn_id: Option<String>,
    pub prompt_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteGitObservation {
    pub kind: HistoryEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, serde_json::Value>,
    pub context: HistoryEventTurnContext,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_agent_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_prompt_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_turn_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution_confidence: Option<HistoryAttributionConfidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspaceLiveSyncApplyContext {
    pub home_session_id: String,
    pub link_id: String,
    pub link_name: String,
    pub source_agent_id: String,
    pub source_worktree_path: String,
    pub target_user_id: String,
    pub target_machine_id: String,
    pub target_kernel_id: String,
    pub target_repo_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RemoteMcpAvailabilityStatus {
    Available,
    Missing,
    DefinitionMismatch { worker_hash: String },
    MissingCommand { command: String },
    MissingEnv { names: Vec<String> },
    Invalid { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspaceLiveSyncArtifactState {
    pub path: String,
    pub exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspaceLiveSyncInvocationMetadata {
    pub invocation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_tool_call_id: Option<String>,
    #[serde(default = "default_workspace_live_sync_invocation_attempt")]
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl RemoteWorkspaceLiveSyncInvocationMetadata {
    pub fn new(
        provider_run_id: &str,
        tool_name: &str,
        provider_tool_call_id: Option<String>,
    ) -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let started_at_ms = crate::session::unix_epoch_ms();
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            invocation_id: format!("{provider_run_id}:{tool_name}:{started_at_ms}:{sequence}"),
            provider_tool_call_id,
            attempt: 1,
            idempotency_key: None,
        }
    }
}

fn default_workspace_live_sync_invocation_attempt() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerRequest {
    Ping {
        value: String,
    },
    CreateExecutionLease {
        home_kernel_id: String,
        home_session_id: String,
        home_agent_id: String,
        #[serde(default)]
        home_agent_metaagent: bool,
        owner_user_id: String,
    },
    DestroyExecutionLease {
        lease_id: String,
    },
    SpawnLeasedAgent {
        lease_id: String,
        provider: String,
        model: Option<String>,
        effort: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_mode: Option<crate::provider::AgentExecutionMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission_level: Option<crate::provider::AgentPermissionLevel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree_placement: Option<GitWorktreePlacement>,
    },
    DestroyLeasedAgent {
        leased_agent_id: String,
    },
    UpdateLeasedAgentConfig {
        leased_agent_id: String,
        execution_mode: crate::provider::AgentExecutionMode,
        permission_level: crate::provider::AgentPermissionLevel,
    },
    UpdateLeasedAgentMetaMode {
        leased_agent_id: String,
        active: bool,
    },
    UpdateLeasedAgentRemoteExtensionManifest {
        leased_agent_id: String,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    LaunchLeasedNativeProviderRun {
        leased_agent_id: String,
        adapter_key: String,
        provider: String,
        account_profile: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        structured_endpoint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        required_mcps: Vec<RequiredRemoteMcp>,
        #[serde(
            default,
            skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
        )]
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    SendLeasedNativeProviderInput {
        leased_agent_id: String,
        provider_run_id: String,
        attachment_id: String,
        data_base64: String,
    },
    SubmitLeasedPrompt {
        leased_agent_id: String,
        prompt: String,
        #[serde(default)]
        attachments: Vec<RelayPromptAttachment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow_context: Option<RemoteWorkflowTurnContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        git_context: Option<RemoteGitTurnContext>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        required_mcps: Vec<RequiredRemoteMcp>,
        #[serde(
            default,
            skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
        )]
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    DrainLeasedRuntimeProjection {
        leased_agent_id: String,
        provider_run_id: String,
        #[serde(default)]
        pump_output: bool,
    },
    CompleteLeasedPrompt {
        leased_agent_id: String,
    },
    ObserveLeasedGitAfter {
        leased_agent_id: String,
        provider_run_id: String,
    },
    CancelLeasedPrompt {
        leased_agent_id: String,
    },
    ForwardWorkflowRuntimeTool {
        context: RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ForwardWorkflowProviderFailure {
        context: RemoteWorkflowTurnContext,
        message: String,
    },
    ForwardWorkspaceLiveSyncRuntimeTool {
        context: RemoteWorkspaceLiveSyncContext,
        metadata: RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<RemoteWorkspaceLiveSyncArtifactState>,
    },
    FinalizeWorkspaceLiveSyncRuntimeTool {
        context: RemoteWorkspaceLiveSyncContext,
        metadata: RemoteWorkspaceLiveSyncInvocationMetadata,
        tool_name: String,
        arguments: serde_json::Value,
        initial_artifact_states: Vec<RemoteWorkspaceLiveSyncArtifactState>,
        final_artifact_states: Vec<RemoteWorkspaceLiveSyncArtifactState>,
    },
    ForwardCapabilityRuntimeTool {
        context: RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ForwardMetaRuntimeTool {
        context: RemoteWorkspaceLiveSyncContext,
        tool_name: String,
        arguments: serde_json::Value,
    },
    InvokeHomeExtensionTool {
        context: RemoteExtensionInvocationContext,
        #[serde(default)]
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        tool: crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    },
    InvokeHomeMcpProxy {
        context: RemoteExtensionInvocationContext,
        #[serde(default)]
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        name: String,
        tool: crate::extension::RemoteExtensionTool,
        payload: serde_json::Value,
    },
    CancelHomeExtensionInvocation {
        context: RemoteExtensionInvocationContext,
        #[serde(default)]
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
    },
    InvokeHomeCredentialTool {
        context: RemoteExtensionInvocationContext,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ResolveHomeCredentialSecret {
        context: RemoteExtensionInvocationContext,
        credential_id: String,
        injection: RemoteCredentialSecretInjection,
    },
    ApplyWorkspaceLiveSyncChange {
        context: RemoteWorkspaceLiveSyncApplyContext,
        change: crate::git_observer::WorkspaceLiveSyncChange,
    },
    ForwardNativeInteraction {
        context: RemoteNativeInteractionContext,
        interaction: crate::session::RuntimeInteraction,
    },
    EnsureRemoteSkillPackages {
        context: RemoteSkillSyncContext,
        packages: Vec<ArrobaSkillPackage>,
    },
    CheckRemoteMcpAvailability {
        context: RemoteMcpCheckContext,
        required_mcps: Vec<RequiredRemoteMcp>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerResponse {
    Pong {
        value: String,
        daemon_id: String,
    },
    ExecutionLeaseCreated {
        lease: ExecutionLease,
        #[serde(default)]
        relay_peer_protocol_version: u32,
    },
    ExecutionLeaseDestroyed {
        lease_id: String,
    },
    LeasedAgentSpawned {
        leased_agent: LeasedAgent,
    },
    LeasedAgentDestroyed {
        leased_agent_id: String,
    },
    LeasedAgentConfigUpdated {
        leased_agent: LeasedAgent,
    },
    LeasedAgentMetaModeUpdated {
        leased_agent: LeasedAgent,
    },
    LeasedAgentRemoteExtensionManifestUpdated {
        leased_agent_id: String,
    },
    LeasedNativeProviderRunLaunched {
        provider_run: crate::provider::RuntimeProviderRun,
    },
    LeasedNativeProviderInputSent {
        byte_count: usize,
    },
    LeasedPromptSubmitted {
        provider_run_id: String,
        outcome: PromptSubmissionOutcome,
    },
    LeasedRuntimeProjectionDrained {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event: Option<RelayPeerEvent>,
    },
    LeasedPromptCompleted {
        provider_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_diagnostic: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        git_observations: Vec<RemoteGitObservation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_live_sync_change: Option<crate::git_observer::WorkspaceLiveSyncChange>,
        completion: PromptCompletion,
    },
    LeasedGitObserved {
        provider_run_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        git_observations: Vec<RemoteGitObservation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_live_sync_change: Option<crate::git_observer::WorkspaceLiveSyncChange>,
    },
    LeasedPromptCancelled {
        cancellation: PromptCancellation,
    },
    WorkflowRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    WorkflowProviderFailureHandled,
    WorkspaceLiveSyncRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
        final_artifact_states: Vec<RemoteWorkspaceLiveSyncArtifactState>,
    },
    WorkspaceLiveSyncRuntimeToolFinalized,
    CapabilityRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill_package: Option<ArrobaSkillPackage>,
        #[serde(
            default,
            skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
        )]
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    MetaRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    HomeExtensionToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    HomeMcpProxyHandled {
        response: serde_json::Value,
    },
    HomeExtensionInvocationCancelled {
        invocation_id: String,
        cancelled: bool,
    },
    HomeCredentialToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    HomeCredentialSecretResolved {
        credential_id: String,
        secret_input: String,
    },
    WorkspaceLiveSyncChangeApplied {
        target_result: crate::git_observer::WorkspaceLiveSyncTargetResult,
    },
    NativeInteractionResolved {
        resolution: crate::provider::ProviderNativeInteractionResolution,
    },
    RemoteSkillPackagesEnsured {
        materialized: Vec<RemoteSkillMaterialization>,
    },
    RemoteMcpAvailabilityChecked {
        results: Vec<RemoteMcpAvailability>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayProjectedOutputChunk {
    pub kind: TerminalOutputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayProjectedCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayProjectedPrompt {
    pub prompt_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerEvent {
    LeasedRuntimeProjection {
        home_session_id: String,
        home_agent_id: String,
        provider_run_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_run: Option<crate::provider::RuntimeProviderRun>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prompts: Vec<RelayProjectedPrompt>,
        output_chunks: Vec<RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<RelayProjectedCompletion>,
    },
}
