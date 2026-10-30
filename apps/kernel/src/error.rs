use thiserror::Error;
mod actionable_refusals;
mod provider_profile_refusal;
mod user_domain_refusal;
pub(crate) use actionable_refusals::message as actionable_refusal_message;
pub use provider_profile_refusal::ProviderProfileRefusal;
pub(crate) use user_domain_refusal::HostFailure;
pub use user_domain_refusal::UserDomainRefusalReason;

use crate::provider::ProviderRunState;
use crate::session::SessionStatus;

// MP-08 / MP-11: code-based explanations contain no provider/parser values.
// Keep executor, MCP and final socket/cache projections consistent.
pub(crate) fn external_request_failure_message(code: &str) -> &'static str {
    if let Some(message) = actionable_refusal_message(code) {
        return message;
    }
    match code {
        "session_not_found" => "session not found; check its name or list available sessions",
        _ => "external request failed; details are available only in the host terminal",
    }
}

/// A session alias the caller chose that cannot be used: the kernel's own sentence names
/// only what the caller sent and the session that holds the alias. Only that exact shape
/// qualifies, so no other text can ride on these codes.
pub(crate) fn is_session_alias_refusal(code: &str, message: &str) -> bool {
    matches!(code, "session_alias_conflict" | "invalid_session_alias")
        && message.starts_with("session alias `")
}

const STEER_UNRECORDED_SUFFIX: &str =
    "` was delivered but its event was not recorded; do not send it again";

/// The steer a provider holds whose event was not recorded: the kernel's own sentence
/// names only the steer id it generated. Only that exact shape qualifies.
pub(crate) fn is_steer_unrecorded_refusal(code: &str, message: &str) -> bool {
    code == "prompt_steer_event_unrecorded"
        && message
            .strip_prefix("steer `")
            .and_then(|rest| rest.strip_suffix(STEER_UNRECORDED_SUFFIX))
            .is_some_and(|id| {
                !id.is_empty()
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
}

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("{provider} account `{account_name}`: {}", .reason.local_message())]
    ProviderProfileRefused {
        reason: ProviderProfileRefusal,
        provider: String,
        account_name: String,
    },
    #[error("User-domain request refused")]
    UserDomainRefused { reason: UserDomainRefusalReason },
    #[error("invalid daemon configuration for `{field}`: {message}")]
    InvalidConfig {
        field: &'static str,
        message: &'static str,
    },
    #[error("failed while waiting for daemon shutdown signal: {0}")]
    ShutdownSignal(std::io::Error),
    /// A machine file, pairing or gateway request that cannot be done; `message` already
    /// says what to do next and `code` is the typed outcome the caller sees.
    #[error("{message}")]
    MachineRequest { code: String, message: String },
    /// The owner revoked a gateway's key on this kernel; it gets no request and no stream.
    #[error("this kernel's owner revoked that gateway (unpaired or revoked)")]
    GatewayKeyRevoked,
    #[error("local transport `{operation}` failed: {message}")]
    LocalTransport {
        operation: &'static str,
        message: String,
    },
    /// MP-08 / MP-11: preserve mapped executor semantics for external grants
    /// and sudo without retaining any diagnostic values or source error.
    #[error("{}", external_request_failure_message(.code))]
    ExternalRequestFailed { code: String, retryable: bool },
    #[error(
        "relay transport `{operation}` failed with code `{code}` (retryable={retryable}): {message}"
    )]
    RelayTransport {
        operation: &'static str,
        code: String,
        message: String,
        retryable: bool,
    },
    /// Internal provenance for a definitive cleanup absence reply. The relay
    /// authenticates/routes the response from its pending request's target;
    /// this is not a new wire error or an end-to-end signed error payload.
    #[error("worker `{worker_kernel_id}` reported `{resource_id}` absent ({code})")]
    RelayPeerCleanupAbsent {
        worker_kernel_id: String,
        resource_id: String,
        code: &'static str,
    },
    #[error("kernel runtime role `{role}` does not allow `{operation}`")]
    KernelRuntimeRoleDenied {
        role: &'static str,
        operation: &'static str,
    },
    #[error("authenticated relay caller does not own `{resource_id}`")]
    LeaseCallerUnauthorized { resource_id: String },
    #[error("browser controller action was cancelled (controller fenced: {controller_fenced})")]
    BrowserControllerActionCancelled { controller_fenced: bool },
    #[error(
        "browser controller restarted at generation {runtime_generation} before the operation"
    )]
    BrowserControllerRecoveryRequired { runtime_generation: u64 },
    /// App open diagnostics use the existing non-retryable request-error
    /// envelope. This internal error is not a serialized protocol type.
    #[error("App view unavailable: {message}")]
    AppViewUnavailable { message: String },
    #[error("internal invariant `{operation}` failed: {message}")]
    InternalInvariant {
        operation: &'static str,
        message: String,
    },
    #[error("worker cleanup failed for `{agent_id}`; agent retained. Resolve the reported failure and retry cleanup: {source}")]
    AgentWorkerCleanup {
        agent_id: String,
        #[source]
        source: Box<DaemonError>,
    },
    #[error("provider account copy failed: {message}")]
    ProviderAccountCopy {
        code: &'static str,
        message: &'static str,
        retryable: bool,
    },
    #[error("managed context `{operation}` failed: {message}")]
    ManagedContext {
        code: &'static str,
        operation: &'static str,
        message: String,
        retryable: bool,
    },
    #[error("session `{session_id}` was not found")]
    SessionNotFound { session_id: String },
    #[error("session alias `{alias}` is invalid: {message}")]
    InvalidSessionAlias {
        alias: String,
        message: &'static str,
    },
    #[error(
        "session alias `{alias}` is already in use by session `{session_id}` in workspace `{workspace_id}`; choose another alias or use that session"
    )]
    SessionAliasConflict {
        workspace_id: String,
        alias: String,
        session_id: String,
    },
    #[error("session reference `{session_ref}` is ambiguous: {matches:?}")]
    AmbiguousSessionRef {
        session_ref: String,
        matches: Vec<String>,
    },
    #[error("workflow `{workflow_id}` was not found in session `{session_id}`")]
    WorkflowNotFound {
        session_id: String,
        workflow_id: String,
    },
    #[error("workflow endpoint `{endpoint_id}` was not found in workflow `{workflow_id}` for session `{session_id}`")]
    WorkflowEndpointNotFound {
        session_id: String,
        workflow_id: String,
        endpoint_id: String,
    },
    #[error("workflow node `{node_id}` was not found in workflow `{workflow_id}` for session `{session_id}`")]
    WorkflowNodeNotFound {
        session_id: String,
        workflow_id: String,
        node_id: String,
    },
    #[error("workflow node `{node_id}` references missing agent `{agent_id}` in workflow `{workflow_id}` for session `{session_id}`")]
    WorkflowNodeAgentMissing {
        session_id: String,
        workflow_id: String,
        node_id: String,
        agent_id: String,
    },
    #[error("workflow node `{node_id}` in workflow `{workflow_id}` for session `{session_id}` does not support required control `{operation}` on agent `{agent_id}`")]
    WorkflowNodeControlUnsupported {
        session_id: String,
        workflow_id: String,
        node_id: String,
        agent_id: String,
        operation: &'static str,
    },
    #[error("workflow `{workflow_id}` in session `{session_id}` already has a node for agent `{agent_id}`")]
    WorkflowNodeConflict {
        session_id: String,
        workflow_id: String,
        agent_id: String,
    },
    #[error("workflow edge `{edge_id}` was not found in workflow `{workflow_id}` for session `{session_id}`")]
    WorkflowEdgeNotFound {
        session_id: String,
        workflow_id: String,
        edge_id: String,
    },
    #[error("workflow run `{workflow_run_id}` was not found in session `{session_id}`")]
    WorkflowRunNotFound {
        session_id: String,
        workflow_run_id: String,
    },
    #[error("workflow launch for endpoint `{endpoint_id}` in workflow `{workflow_id}` was rejected in session `{session_id}`: {message}")]
    WorkflowLaunchRejected {
        session_id: String,
        workflow_id: String,
        endpoint_id: String,
        message: String,
    },
    #[error("workflow edge `{from_node_id}` -> `{to_node_id}` already exists in workflow `{workflow_id}` for session `{session_id}`")]
    WorkflowEdgeConflict {
        session_id: String,
        workflow_id: String,
        from_node_id: String,
        to_node_id: String,
    },
    #[error("workflow `{workflow_id}` in session `{session_id}` has revision {current_revision}, expected {expected_revision}; refresh and retry")]
    WorkflowRevisionConflict {
        session_id: String,
        workflow_id: String,
        expected_revision: u64,
        current_revision: u64,
    },
    #[error("prompt setting `{id}` changed: expected revision {expected_revision}, current revision {current_revision}; refresh and retry")]
    PromptSettingConflict {
        id: String,
        expected_revision: u64,
        current_revision: u64,
    },
    #[error("workflow alias `{alias}` is invalid: {message}")]
    InvalidWorkflowAlias {
        alias: String,
        message: &'static str,
    },
    #[error("workflow alias `{alias}` is already in use for session `{session_id}`")]
    WorkflowAliasConflict { session_id: String, alias: String },
    #[error("workflow endpoint alias `{alias}` is invalid: {message}")]
    InvalidWorkflowEndpointAlias {
        alias: String,
        message: &'static str,
    },
    #[error("workflow endpoint alias `{alias}` is already in use for workflow `{workflow_id}` in session `{session_id}`")]
    WorkflowEndpointAliasConflict {
        session_id: String,
        workflow_id: String,
        alias: String,
    },
    #[error("workflow node reference `{reference}` is invalid in workflow `{workflow_id}` for session `{session_id}`: {message}")]
    InvalidWorkflowGraphReference {
        session_id: String,
        workflow_id: String,
        reference: String,
        message: &'static str,
    },
    #[error("workflow publication trace exposure is invalid: {message}")]
    InvalidWorkflowPublicationTraceExposure { message: String },
    #[error("workflow run `{workflow_run_id}` cannot perform `{operation}` while {status:?}")]
    InvalidWorkflowRunState {
        workflow_run_id: String,
        status: crate::session::WorkflowRunStatus,
        operation: &'static str,
    },
    #[error("workflow handoff validation failed for edge `{edge_id}` in workflow `{workflow_id}` for session `{session_id}`: {message}")]
    WorkflowHandoffValidationFailed {
        session_id: String,
        workflow_id: String,
        edge_id: String,
        message: String,
    },
    #[error("workspace claim conflict in workspace `{workspace_id}` worktree `{worktree_id}`: `{requested_operation}` for session `{requested_session_id}` conflicts with `{existing_operation}` for session `{existing_session_id}`")]
    WorkspaceClaimConflict {
        workspace_id: String,
        worktree_id: String,
        existing_session_id: String,
        existing_operation: Box<String>,
        requested_session_id: String,
        requested_operation: Box<String>,
    },
    #[error("invalid session transition for `{session_id}`: {from} -> {to}")]
    InvalidSessionTransition {
        session_id: String,
        from: SessionStatus,
        to: SessionStatus,
    },
    #[error("machine `{machine_id}` is not accepting remote execution leases")]
    RemoteLeasesDisabled { machine_id: String },
    #[error("execution lease `{lease_id}` was not found")]
    ExecutionLeaseNotFound { lease_id: String },
    #[error("leased agent `{leased_agent_id}` was not found")]
    LeasedAgentNotFound { leased_agent_id: String },
    #[error("{message}")]
    NoRemoteKernelAvailable {
        machine_ref: String,
        provider: String,
        message: String,
    },
    #[error("attachment `{attachment_id}` was not found")]
    AttachmentNotFound { attachment_id: String },
    #[error("attachment `{attachment_id}` does not belong to session `{session_id}`")]
    AttachmentNotInSession {
        session_id: String,
        attachment_id: String,
    },
    #[error("session `{session_id}` cannot perform `{operation}` while {status}")]
    SessionOperationNotAllowed {
        session_id: String,
        status: SessionStatus,
        operation: &'static str,
    },
    #[error("user `{user_id}` is not a member of session `{session_id}`")]
    SessionAccessDenied { session_id: String, user_id: String },
    #[error(
        "user `{user_id}` cannot perform `{operation}` on `{resource}` owned by `{owner_user_id}`"
    )]
    OwnershipAccessDenied {
        user_id: String,
        owner_user_id: String,
        resource: String,
        operation: &'static str,
    },
    #[error("session-scoped request `{operation}` requires authenticated user identity")]
    MissingSessionCallerIdentity { operation: String },
    #[error("session `{session_id}` has no active prompt")]
    NoActivePrompt { session_id: String },
    #[error("session `{session_id}` rejected the config change while a prompt is running")]
    ConfigChangeRejectedWhilePromptRunning { session_id: String },
    #[error("provider adapter `{adapter_key}` was not found")]
    ProviderAdapterNotFound { adapter_key: String },
    #[error("provider adapter `{adapter_key}` does not support managed workspace live sync write enforcement: {message}")]
    ProviderWorkspaceLiveSyncUnsupported {
        adapter_key: String,
        message: String,
    },
    #[error("provider adapter `{adapter_key}` could not resolve executable `{executable}`")]
    ProviderExecutableNotFound {
        adapter_key: String,
        executable: String,
    },
    #[error("provider run `{provider_run_id}` was not found")]
    ProviderRunNotFound { provider_run_id: String },
    #[error("provider run `{provider_run_id}` does not belong to session `{session_id}`")]
    ProviderRunNotInSession {
        session_id: String,
        provider_run_id: String,
    },
    #[error("provider run `{provider_run_id}` cannot perform `{operation}` while {state}")]
    InvalidProviderRunState {
        provider_run_id: String,
        state: ProviderRunState,
        operation: &'static str,
    },
    #[error(
        "session `{session_id}` has inconsistent active provider run state: active={active_provider_run_id:?}, requested={requested_provider_run_id}`"
    )]
    InconsistentActiveProviderRun {
        session_id: String,
        active_provider_run_id: Option<String>,
        requested_provider_run_id: String,
    },
    #[error("session `{session_id}` has no active provider run")]
    NoActiveProviderRun { session_id: String },
    #[error("provider run `{provider_run_id}` has no PTY process")]
    PtyProcessNotFound { provider_run_id: String },

    /// Internal proof that a steer was not submitted or was explicitly rejected.
    /// Unclassified errors may follow a provider write and cannot authorize replay.
    #[error("provider did not accept steering: {source}")]
    ProviderPromptSteerRejected {
        #[source]
        source: Box<DaemonError>,
    },
    /// The provider holds the steer, but the outside event for it was not recorded.
    /// The caller must not send it again; the message carries only the kernel's own id.
    #[error(
        "steer `{steer_id}` was delivered but its event was not recorded; do not send it again"
    )]
    SteerDeliveredEventUnrecorded { steer_id: String },
    /// A steer was written but its acknowledgement failed, so the provider may
    /// already hold it.
    #[error("provider steer for run `{provider_run_id}` was not confirmed")]
    ProviderPromptSteerUnconfirmed { provider_run_id: String },
    #[error("provider protocol `{operation}` failed for run `{provider_run_id}`: {message}")]
    ProviderProtocol {
        provider_run_id: String,
        operation: &'static str,
        message: String,
    },
    #[error("failed to spawn PTY for provider run `{provider_run_id}`: {message}")]
    PtySpawn {
        provider_run_id: String,
        message: String,
    },
    #[error("failed to clean up PTY for provider run `{provider_run_id}`: {message}")]
    PtyCleanup {
        provider_run_id: String,
        message: String,
    },
    #[error("failed to write PTY input for provider run `{provider_run_id}`: {message}")]
    PtyWrite {
        provider_run_id: String,
        message: String,
    },
    #[error(
        "failed to resize PTY for provider run `{provider_run_id}` to {cols}x{rows}: {message}"
    )]
    PtyResize {
        provider_run_id: String,
        cols: u16,
        rows: u16,
        message: String,
    },
    #[error("shell command `{command}` failed for session `{session_id}`: {message}")]
    ShellCommandFailed {
        session_id: String,
        command: String,
        message: String,
    },
    #[error("shell command `{command}` timed out for session `{session_id}` after {timeout_ms}ms")]
    ShellCommandTimedOut {
        session_id: String,
        command: String,
        timeout_ms: u64,
    },
    #[error("shell command working directory `{working_directory}` is outside session `{session_id}` worktree `{worktree_root}`")]
    ShellCommandOutsideWorktree {
        session_id: String,
        working_directory: String,
        worktree_root: String,
    },
    #[error("attachment `{attachment_id}` is not allowed to use capability `{capability}` in session `{session_id}`")]
    AttachmentCapabilityDenied {
        session_id: String,
        attachment_id: String,
        capability: &'static str,
    },
    #[error("path `{path}` is outside session `{session_id}` worktree `{worktree_root}`")]
    PathOutsideWorktree {
        session_id: String,
        path: String,
        worktree_root: String,
    },
    #[error("failed to inspect filesystem for session `{session_id}` at `{path}`: {message}")]
    FilesystemCapabilityFailed {
        session_id: String,
        path: String,
        message: String,
    },
    #[error("failed to update file for session `{session_id}` at `{path}`: {message}")]
    FileEditFailed {
        session_id: String,
        path: String,
        message: String,
    },
    #[error(
        "git inspection failed for session `{session_id}` in `{working_directory}`: {message}"
    )]
    GitCapabilityFailed {
        session_id: String,
        working_directory: String,
        message: String,
    },
    #[error("file transfer failed for session `{session_id}`: {message}")]
    TransferCapabilityFailed { session_id: String, message: String },
    #[error("invalid transfer display name `{display_name}` for session `{session_id}`")]
    InvalidTransferDisplayName {
        session_id: String,
        display_name: String,
    },
    #[error("screenshot capture failed for session `{session_id}`: {message}")]
    ScreenshotCapabilityFailed { session_id: String, message: String },
    #[error("local harness timed out waiting for terminal output for session `{session_id}` after {timeout_ms}ms")]
    LocalHarnessTimeout { session_id: String, timeout_ms: u64 },
    #[error("session history failed for {session_id:?}: {operation}: {message}")]
    SessionHistoryFailed {
        session_id: Option<String>,
        operation: &'static str,
        message: String,
    },
    #[error("agent `{agent_id}` was not found")]
    AgentNotFound { agent_id: String },
    #[error("agent `{agent_id}` does not belong to session `{session_id}`")]
    AgentNotInSession {
        session_id: String,
        agent_id: String,
    },
    #[error("agent alias `{alias}` is already in use for session `{session_id}`")]
    AgentAliasConflict { session_id: String, alias: String },
    #[error("session `{session_id}` has reached the maximum of {max_agents} agents")]
    AgentLimitReached { session_id: String, max_agents: i32 },
}

impl DaemonError {
    pub(crate) fn steer_not_submitted(self, steering: bool) -> Self {
        if steering {
            Self::ProviderPromptSteerRejected {
                source: Box::new(self),
            }
        } else {
            self
        }
    }

    /// True when a worker reports that the prompt it was asked to settle is no
    /// longer active. Prefer the stable relay code; retain text compatibility
    /// for older peers that did not project a typed refusal.
    pub(crate) fn is_no_active_prompt(&self) -> bool {
        match self {
            Self::NoActivePrompt { .. } => true,
            Self::RelayTransport { code, .. } if code == "no_active_prompt" => true,
            Self::LocalTransport { message, .. } | Self::RelayTransport { message, .. } => {
                message.contains("no active prompt")
                    || message.contains("NoActivePrompt")
                    || message.contains("no_active_prompt")
            }
            _ => false,
        }
    }
}
