use thiserror::Error;

use crate::provider::ProviderRunState;
use crate::session::SessionStatus;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("invalid daemon configuration for `{field}`: {message}")]
    InvalidConfig {
        field: &'static str,
        message: &'static str,
    },
    #[error("failed while waiting for daemon shutdown signal: {0}")]
    ShutdownSignal(std::io::Error),
    #[error("local transport `{operation}` failed: {message}")]
    LocalTransport {
        operation: &'static str,
        message: String,
    },
    #[error("session `{session_id}` was not found")]
    SessionNotFound { session_id: String },
    #[error("session alias `{alias}` is invalid: {message}")]
    InvalidSessionAlias {
        alias: String,
        message: &'static str,
    },
    #[error("session alias `{alias}` is already in use for workspace `{workspace_id}`")]
    SessionAliasConflict { workspace_id: String, alias: String },
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
    #[error("workflow run `{workflow_run_id}` cannot perform `{operation}` while {status:?}")]
    InvalidWorkflowRunState {
        workflow_run_id: String,
        status: crate::session::WorkflowRunStatus,
        operation: &'static str,
    },
    #[error("workflow output validation failed for edge `{edge_id}` in workflow `{workflow_id}` for session `{session_id}`: {message}")]
    WorkflowOutputValidationFailed {
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
        existing_operation: String,
        requested_session_id: String,
        requested_operation: String,
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
    #[error("no live remote kernel on machine `{machine_ref}` can host provider `{provider}`")]
    NoRemoteKernelAvailable {
        machine_ref: String,
        provider: String,
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
    #[error("provider adapter `{adapter_key}` does not support required managed I/O write enforcement: {message}")]
    ProviderManagedIoUnsupported {
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
