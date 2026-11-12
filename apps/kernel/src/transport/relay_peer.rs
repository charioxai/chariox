#[path = "room_browser_snapshot_wire.rs"]
mod snapshot_wire;

use serde::{Deserialize, Serialize};

pub use super::relay_peer_agents::{
    PeerAgentAddress, PeerAgentOutcome, PeerAgentRefusal, PeerAgentStamp, PeerCallerAuthority,
    PeerFollow, PeerOutcomeAck,
};
use crate::agent::GitWorktreePlacement;
use crate::execution_lease::{ExecutionLease, LeasedAgent, RemoteWorkflowTurnContext};
use crate::history::{HistoryAttributionConfidence, HistoryEventKind, HistoryEventTurnContext};
use crate::io::WorkspaceIdentity;
use crate::mcp::CharioxMcpServerConfig;
use crate::session::{PromptCancellation, PromptCompletion, PromptOrigin, PromptSubmissionOutcome};
use crate::skill::CharioxSkillPackage;
use crate::terminal::TerminalOutputKind;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayManagedContextCapability(String);

impl RelayManagedContextCapability {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for RelayManagedContextCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted managed-context capability]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayManagedContextChunk(String);

impl RelayManagedContextChunk {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for RelayManagedContextChunk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted managed-context chunk]")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayManagedSliceToken(String);

impl RelayManagedSliceToken {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for RelayManagedSliceToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted managed-slice relay token]")
    }
}

/// Version 44 requires cancellable uploads and upload execution receipt recovery.
/// Version 45 requires cancellable browser configuration and execution receipt recovery.
/// Version 46 requires cancellable browser lifecycle operations and receipt recovery.
/// Version 47 carries the workspace kind in managed-context import receipts.
/// Version 48 carries private browser-cookie import commands and bounded results.
/// Version 50 carries bounded provider-run termination metadata across leased execution.
/// Version 51 carries kernel-owned project-environment setup dispatch and status.
/// Version 52 carries the source-selected managed-context plan in import arm requests.
/// Version 53 carries the home-authoritative setup attempt on worker redispatch.
/// Version 54 carries recipe and lockfile input attestations for worker setup.
/// Version 55 carries definition-derived executable path entries for worker setup.
/// Version 56 carries the originating home prompt for forwarded worker runtime tools.
/// Version 57 rejects worker kernels that cannot supply the required origin turn
/// for `chariox.send_agent_message`; mixed-version peers fail before dispatch.
/// Version 58 carries the home-owned Room browser capability in extension manifests.
/// Version 59 adds read-only exact-home-prompt receipt queries for leased workers.
/// Version 60 requires exact home-prompt and worker-run identities for leased cancellation
/// and carries durable exact-identity receipts for queued prompt steers.
/// Version 62 also persists worker admission receipts for original leased prompts.
/// Version 63 requires the leased setup worker to receive a home persistence
/// acknowledgment before validating a utility-generated Project definition.
/// Version 64 binds signed daemon admission to the canonical kernel subject.
/// Version 65 installs an authenticated selected Project environment on a leased worker.
/// Version 66 queries and adjusts the leased worker Project through its own authority.
/// Version 67 binds Computer credential input to its approved native display target (MP-08).
/// Version 68 updates/dismisses ephemeral login UI through the existing interaction bridge (MP-08/MP-10/MP-11).
/// Version 69 drops the workflow event capability flags (local protocol 365):
/// `RemoteWorkflowTurnContext` loses `event_context_enabled` /
/// `event_actions_enabled` and leased provider-run projections lose
/// `workflow_event_actions_enabled`, which a peer at v68 or below still requires.
/// Version70 combines Apps Phase1 and autonomous Room observation contracts (MP-08/MP-10).
/// MP-08/MP-10/MP-11: version 86 combines the multidomain and main worker shapes.
/// MP-08/MP-11: reserved peer89 shares the owned-desktop local contract.
/// MP-08/MP-10/MP-11: version 90 negotiates bounded stripes.
/// MP-08/MP-11: version 92 adds Room AT-SPI target actions and owner Computer revoke.
/// Older worker contracts are rejected before dispatch.
/// Version 101 (main) binds the embedded local response union to local protocol 491,
/// restores owner-private workflow notifications to the Browser/Computer peer
/// union and pairs exact home-prompt cancellation admission (MP-08/MP-10/MP-11).
/// Version 102 reconciles the agent-model union and its leased watcher contract (MP-08/MP-09/MP-10/MP-11).
/// Version 114 integrates the 102 agent-model/leased-watcher union, main's
/// identity-preserving renewal and the native display binary-event transport
/// (MP-08/MP-09/MP-10/MP-11), and includes main's 101 kernel ownership,
/// waiting-room and Room workflow contracts (#974). Generated consumers follow
/// this allocated version.
/// MP-08/MP-10/MP-11: allocated 133 carries the local 525 resumption union.
/// MP-08/MP-10/MP-11: allocated 136 carries the local 528 Browser state fields.
/// MP-08/MP-10/MP-11: allocated 137 carries local529 page display placement.
/// Allocated 138 carries the local 530 band gestures and auxiliary popups.
/// MP-08/MP-10/MP-11: allocated 140 negotiates chunks-v2 multiplexed transport.
/// Allocated143 carries535 delivery registration with525 Room resume+attach.
/// MP-08/MP-10/MP-11: allocated144 carries the visible-page default crop contract.
/// MP-08/MP-10/MP-11: allocated 148 carries the local 540 warm omnibox list.
/// MP-08/MP-10/MP-11: allocated 158 carries the local 550 mirror v2 wire contract.
/// Allocated 160 carries the local 552 `parked_input` reply field.
/// MP-08/MP-10/MP-11: allocated 164 carries the local 556 `state_wait`.
/// MP-08/MP-10/MP-11: allocated 175 carries the local 567 `fonts` op `absent`.
/// MP-08/MP-10/MP-11: allocated 202 carries the local 596 state revision order and `latest_prompt_count`.
/// Allocated 209 carries the local 603 dictation service/setup contract.
/// Allocated 216 carries the local 610 union.
/// Agent-model (145) and event-core (169) contracts were integrated in 570/178.
/// The Room terminal service (`RoomTerminal`, formerly provisional 177) and its peer forwards ride this number too.
/// Allocated 181 adds exact-home-check assessment wire identities.
/// Allocated 197 carries the outside `steer` event between peers.
/// Allocated 215 installs independent provider accounts over the encrypted peer channel.
/// Allocated 220 unions the staging 216 line with the external agents 215 line.
/// MP-08/MP-10/MP-11: allocated 232 carries the local 629 browser resize command.
/// Peer 232 also carries protocol 627 attachment byte sizes in shared history.
/// MP-08/MP-09/MP-10/MP-11: landing union 239 carries local 636 (peers 233 dialogs, 234 outside events,
/// 235 binary Room exec and 239 archived-agent discovery with the saved provider thread handoff).
/// MP-08/MP-09/MP-10/MP-11: allocated 361 carries structured login retry feedback.
/// MP-08/MP-09/MP-10/MP-11: landing union 242 carries local 640 (peers 233 dialogs, 234 outside events,
/// 235 binary Room exec, 239 archived-agent discovery with the saved provider thread handoff and 242 the typed
/// failure cause on provider run terminations).
/// MP-08/MP-09/MP-10/MP-11: landing union 243 carries local 642 (peers 233 dialogs, 234 outside events,
/// failure cause on provider run terminations) and 243 (that failure renamed to the turn outcome kind and reason).
/// Allocated 240 carries the local 638 `AgentMove` peer messages.
/// Allocated 241 carries the composed Move resource receipts and probe messages of local 639.
/// Allocated 430 pairs the embedded local response union with local 880 (integration received events).
/// MP-08 / MP-09 / MP-10 / MP-11: owned process observations and outcomes.
/// MP-08/MP-09/MP-10/MP-11: landing union 362 pairs the embedded local response union with local 911
/// (361 with peers 430 received events and 350 owned process observations and outcomes).
/// Allocated 364 pairs the embedded local response union with local 913 (`processes` in `SessionState`).
/// 370 pairs the embedded local request/response union with local 919 (the user-side browser frame stream removed, `display_ack` `paused`).
/// MP-08/MP-10/MP-11: allocated 371 pairs the embedded local response union with local 920.
/// 374 pairs the embedded local response union with local 923 (`CredentialVaultUnlocked`).
/// Landing union 374 pairs the embedded local union with local 923 (it carries peers 364, 370 and 371).
/// Allocated 375 pairs the embedded local response union with local 924 (opaque reimage `providerId`).
/// MP-08/MP-09/MP-10/MP-11: 376 pairs local 925 (the relay close's `kernel_reload`).
/// 377 pairs the embedded local union with local 926 (direct data channel liveness).
/// 378 pairs the embedded local union with local 927 (`SessionProcess.state`, `StartSessionProcessNow`).
/// Allocated 379 pairs the embedded local response union with local 928 (`question` and `resolution` on outside interaction events).
/// Allocated 380 carries the local 931 `schedule` and `notification` prompt origins.
/// Allocated 386 carries the local 932 HTTP agent trigger variants with the `http` prompt origin.
/// Landing union 381 pairs the embedded local union with local 932; it carries peers 375 to 380 and
/// takes 381 for the HTTP trigger variants (allocated as 386).
/// Browser header 382 carries the local 933 header state.
/// MP-08/MP-10/MP-11: 383 acknowledges the leased projection drain; it pairs with local 934.
/// MP-08/MP-10/MP-11: 384 binds `CompleteLeasedPrompt` to the exact home prompt and worker provider run, so a late
/// completion cannot end the prompt that replaced it; it pairs with local 935.
/// MP-08/MP-10/MP-11: 385 pairs the embedded local union with local 936 (non-blocking task-owned questions).
/// Browser downloads 386 carries local 937 download state and cancellation.
/// MP-08/MP-10/MP-11: 387 pairs the embedded local union with local 938 (kernel-resolved access holder).
/// MP-08 / MP-09 / MP-10 / MP-11: 403 carries local954 process disk I/O.
/// MP-08/MP-10/MP-11: peer 390 carries Vault fill audience masking.
/// MP-08/MP-09/MP-10/MP-11: 392 carries local 943 `env` and `file` credentials.
/// Resource register 393 carries local 944 resource metadata and tool calls.
/// Vault TOTP 394 carries local 945 factor credential metadata.
/// VAUDIT S3 397 carries local 948 (kernel-browser Vault capture and fill).
/// Peer 399 carries the shared RelayStatus local protocol field (local 950).
/// Peer 409 carries the final integrated local 960 Vault and Machines contract.
/// Peer 412 carries local963 one-use Vault capture bindings.
/// Peer 414 carries local965 integrated Vault and machine contracts.
/// Peer432 carries local993 workspace artifact read pagination.
/// Peer415 carries local966 WASM Apps plus Landing965 contracts.
/// MP-08 / MP-09 / MP-10 / MP-11: 435 carries bounded lossless Browser snapshots.
/// OSS boundary: 416 pairs the embedded local union with local 967 (retired Cloud controls removed).
/// Allocated 424 carries local983 (the waiting-command row and the outside `process` event).
/// Peer425 carries local984 Vault unlock durations.
/// Peer 433 carries local994 per-exit sources, App agent targets and blocked-task notifications.
/// Peer 437 carries the local998 notification listener contract.
/// Peer 439 carries local1000: the run finished source kind, event connection selections in the managed-context plan
/// binding and Vault provider logins in the context package.
/// Peer 440 carries local 1001: peers across kernels of one owner.
/// MP-08 / MP-09 / MP-10 / MP-11: peer 441 sends the kernel's large messages to the relay in
/// chunks-v2 frames (`x-chariox-upload`); toward a relay that does not answer the offer the
/// kernel writes whole messages, and the relay keeps reading whole messages from 440 kernels.
/// MP-08 / MP-09 / MP-10 / MP-11: peer 442 sends a request's ciphertext as a `CXR3` frame, to a relay that
/// answers `x-chariox-relay-requests: binary-v1`, and sizes binary chunks to the link. It adds
/// `SetLeasedAgentDeadline`, `DeleteRoomPeer` and the `chariox.file.copy` requests
/// `ReadLeasedAgentFile`, `WriteLeasedAgentFile` and `DiscardLeasedAgentFile`.
/// MP-08 / MP-09 / MP-10 / MP-11: peer 444 (files and sub-agent panes) adds the machine file requests `StatMachinePath`,
/// `ListMachineDirectory`, `ReadMachineFile` and `DeleteMachinePath` and the transfer requests
/// `StartFileTransfer`, `GetFileTransfer` and `CancelFileTransfer` between kernels of one owner, and
/// renames the answers `LeasedAgentFileRead` and `LeasedAgentFileRefused` to `FileSliceRead` and
/// `FileRefused`, which both file services use.
pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 451;
/// MP-08/MP-10/MP-11: runtime admission requires the integrated agent contract,
/// independently of a relay's ability to downgrade its binary transport.
pub const MINIMUM_RELAY_PEER_RUNTIME_VERSION: u32 = RELAY_PEER_PROTOCOL_VERSION;
/// MP-08/MP-11: typed leased Computer grants require peer 109.
pub(crate) const MINIMUM_LEASED_COMPUTER_PEER_VERSION: u32 = 109;
pub const REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE: &str =
    "provider_launch_credential_required";
/// The worker refused a leased prompt before admission because the selected
/// account cannot start work there; repeating the prompt cannot repair it.
pub const REMOTE_PROVIDER_ACCOUNT_UNUSABLE_CODE: &str = "provider_account_unusable";
pub(crate) const ADMIT_LEASED_PROMPT_ACCOUNT_OPERATION: &str = "admit leased prompt account";
pub(crate) const ENSURE_REMOTE_PROVIDER_ACCOUNT_OPERATION: &str = "ensure remote provider account";
/// The worker refused a completion because the prompt it names is not the leased
/// agent's current prompt (already finished, or replaced by a later one).
pub const LEASED_PROMPT_NOT_CURRENT_CODE: &str = "leased_prompt_not_current";
pub const LEASED_PROMPT_RUN_CHANGED_CODE: &str = "leased_prompt_run_changed";

// MP-08/MP-09/MP-10/MP-11 A10: start and queries use the exact admitted lease.
// Preserve the serialized peer action and its existing public Rust fields.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve the versioned leased-process request shape."
)]
pub enum LeasedWatchedProcessAction {
    Start {
        wake: crate::durable_state::agent_lifecycle::AgentWake,
        binding: crate::durable_state::agent_lifecycle::LeasedProcessBinding,
        cwd: Option<String>,
    },
    Query {
        wake_id: String,
        binding: crate::durable_state::agent_lifecycle::LeasedProcessBinding,
    },
    Cancel {
        wake_id: String,
        binding: crate::durable_state::agent_lifecycle::LeasedProcessBinding,
    },
}
/// Home-selected execution identity for a leased prompt. Contains no credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayAgentExecutionProfile {
    pub provider: String,
    pub account_profile: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl From<&crate::agent::AgentInstance> for RelayAgentExecutionProfile {
    fn from(agent: &crate::agent::AgentInstance) -> Self {
        Self {
            provider: agent.provider().to_string(),
            account_profile: agent.provider_account_profile().to_string(),
            model: agent.model().map(str::to_string),
            effort: agent.effort().map(str::to_string),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayManagedContextTransferPhase {
    Armed,
    Receiving,
    ReadyToImport,
    Importing,
    Consumed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayManagedContextImportedRepository {
    #[serde(
        default,
        skip_serializing_if = "crate::managed_context::development::DevelopmentWorkspaceKind::is_git"
    )]
    pub workspace_kind: crate::managed_context::development::DevelopmentWorkspaceKind,
    pub repository_id: String,
    pub role: crate::managed_context::development::DevelopmentRepositoryRole,
    pub target_directory: String,
    pub destination_path: String,
    pub head_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayManagedContextImportReceipt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<crate::managed_context::owner_managed::OwnerManagedDestination>,
    pub transfer_id: String,
    pub archive_sha256: String,
    pub plan_digest: String,
    pub development: RelayManagedDevelopmentContextImportReceipt,
    pub kernel_context: RelayManagedKernelContextImportReceipt,
    pub receipt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayManagedDevelopmentContextImportReceipt {
    Empty,
    FromSource {
        project_id: String,
        destination_root: String,
        primary_repository_id: String,
        repositories: Vec<RelayManagedContextImportedRepository>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayManagedKernelContextImportReceipt {
    Empty,
    FromKernel {
        context_id: String,
        source_kernel_id: String,
        source_key_thumbprint: String,
        snapshot_sha256: String,
        extension_count: usize,
        dependency_count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayManagedContextTransferStatus {
    pub transfer_id: String,
    pub phase: RelayManagedContextTransferPhase,
    pub accepted_bytes: u64,
    pub archive_size_bytes: u64,
    pub expires_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<RelayManagedContextImportReceipt>,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_prompt_id: Option<String>,
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
pub struct RemoteProviderAccountSyncContext {
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub execution_lease_id: String,
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

/// MP-08/MP-11 peer109: value-free, home-selected lease and desktop authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeasedComputerGrant {
    pub epoch: String,
    pub home_kernel_id: String,
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
    pub desktop_kernel_id: String,
    pub target: crate::local::KernelDesktopTarget,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRoomBrowserRuntimeToolCall {
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

impl std::fmt::Debug for RemoteRoomBrowserRuntimeToolCall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteRoomBrowserRuntimeToolCall")
            .field("tool_name", &self.tool_name)
            .field("arguments", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteRoomBrowserRuntimeToolResult(
    pub crate::transport::runtime_tools::RuntimeToolResult,
);

impl std::fmt::Debug for RemoteRoomBrowserRuntimeToolResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RemoteRoomBrowserRuntimeToolResult(<redacted>)")
    }
}

/// An agent's terminal tool call, forwarded by the worker its provider runs on
/// to the home kernel that owns the Room and the agent's identity. Arguments
/// and results carry typed text and terminal output, so they stay out of logs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteTerminalRuntimeToolCall {
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

impl std::fmt::Debug for RemoteTerminalRuntimeToolCall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteTerminalRuntimeToolCall")
            .field("tool_name", &self.tool_name)
            .field("arguments", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteTerminalRuntimeToolResult(pub crate::transport::runtime_tools::RuntimeToolResult);

impl std::fmt::Debug for RemoteTerminalRuntimeToolResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RemoteTerminalRuntimeToolResult(<redacted>)")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteRoomComputerObservationCall {
    Snapshot {
        observer: String,
    },
    ScreenStatus,
    Ocr {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<String>,
    },
    FindText {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<String>,
    },
}

impl std::fmt::Debug for RemoteRoomComputerObservationCall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Snapshot { .. } => formatter.write_str("Snapshot(<redacted>)"),
            Self::ScreenStatus => formatter.write_str("ScreenStatus"),
            Self::Ocr { .. } => formatter.write_str("Ocr(<redacted>)"),
            Self::FindText { .. } => formatter.write_str("FindText(<redacted>)"),
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteRoomComputerObservationResult(
    pub crate::transport::runtime_tools::RuntimeToolResult,
);

impl std::fmt::Debug for RemoteRoomComputerObservationResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RemoteRoomComputerObservationResult(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteCredentialSecretInjection {
    Browser { target_url: String },
    Computer,
    Pty,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteCredentialSecretInput(String);

impl RemoteCredentialSecretInput {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn from_zeroizing(mut value: zeroize::Zeroizing<String>) -> Self {
        Self(std::mem::take(&mut *value))
    }

    pub fn into_zeroizing(mut self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(std::mem::take(&mut self.0))
    }
}

impl Drop for RemoteCredentialSecretInput {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RemoteCredentialSecretInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted remote credential secret input]")
    }
}

/// A file's bytes (base64) inside an encrypted kernel-to-kernel message; it can hold secrets.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteFileBytes(String);

impl RemoteFileBytes {
    pub fn encode(bytes: &[u8]) -> Self {
        use base64::Engine;
        Self(base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn decode(&self) -> Option<zeroize::Zeroizing<Vec<u8>>> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&self.0)
            .ok()
            .map(zeroize::Zeroizing::new)
    }
}

/// An MCP registration or a skill inside an encrypted kernel-to-kernel response; an MCP may carry literal settings.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteProjectRegistration(pub crate::project_environment::Registration);

impl std::fmt::Debug for RemoteProjectRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted Project registration]")
    }
}

impl Drop for RemoteFileBytes {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl std::fmt::Debug for RemoteFileBytes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted Project file bytes]")
    }
}

/// One provider launch secret carried only inside an encrypted kernel-to-kernel
/// request. The worker validates the provider/profile binding, moves the value
/// into its in-memory launch environment, and never writes it to a provider
/// credential file or durable kernel state.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteProviderLaunchCredential {
    pub provider: String,
    pub account_profile: String,
    pub secret_input: RemoteCredentialSecretInput,
}

impl std::fmt::Debug for RemoteProviderLaunchCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteProviderLaunchCredential")
            .field("provider", &self.provider)
            .field("account_profile", &self.account_profile)
            .field("secret_input", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteNativeInteractionContext {
    pub home_session_id: String,
    pub home_agent_id: String,
    pub leased_agent_id: String,
    pub worker_provider_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_prompt_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredRemoteMcp {
    pub config: CharioxMcpServerConfig,
    pub definition_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredRemoteSkill {
    pub name: String,
    pub version_hash: String,
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

/// A10: what a worker may know of a home sudo window: a revoke handle, its
/// revision and the time left when the home sent it. It is a narrowing fence
/// only; the home rechecks its own window on every forwarded privileged call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedSudoGrant {
    pub entry_id: String,
    pub revision: u64,
    pub remaining_ms: u64,
    /// The window's first fence. A worker that does not hold the window
    /// refuses any later one, so a worker restart ends elevation visibly.
    #[serde(default)]
    pub initial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeasedPromptReceiptPhase {
    Active,
    Completed,
    SteerDispatching,
    SteerAccepted,
    SteerRejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedPromptReceipt {
    pub home_prompt_id: String,
    pub worker_provider_run_id: String,
    pub phase: LeasedPromptReceiptPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_home_prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_lease_id: Option<String>,
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
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve the existing RelayPeerRequest typed actor payload layout"
)]
pub enum RelayPeerRequest {
    AgentMove {
        request: crate::agent_move::peer::AgentMovePeerRequest,
    },
    /// A kernel of the same user starts, reads or stops a Setup run on this kernel.
    RunProjectEnvironmentSetup {
        project_id: String,
        operation_id: String,
        checkout_path: Option<String>,
    },
    GetProjectEnvironmentOperation {
        operation_id: String,
    },
    CancelProjectEnvironmentOperation {
        operation_id: String,
    },
    /// A kernel of the same user asks what this kernel holds of a Project's Environment.
    ReadProjectEnvironment {
        project_id: String,
    },
    PullProjectEnvironmentSecret {
        project_id: String,
        name: String,
    },
    /// One slice of a Project file, starting at `offset`; a message stays well under the relay's bound.
    PullProjectEnvironmentFile {
        project_id: String,
        path: String,
        offset: u64,
    },
    /// The registration of an MCP server or skill the Environment lists, as another kernel can write it.
    PullProjectEnvironmentRegistration {
        project_id: String,
        entry_kind: crate::local::EnvironmentEntryKind,
        name: String,
    },
    ListWorkflowNotificationSources {
        protocol_version: u32,
    },
    SubscribeWorkflowNotifications {
        protocol_version: u32,
        source_id: String,
        target_ref: crate::local::WorkflowNotificationSubscription,
        /// The version of the follow the destination asks for; the source ignores it when it has
        /// already seen a newer one of this subscription.
        generation: u64,
    },
    UnsubscribeWorkflowNotifications {
        protocol_version: u32,
        subscription_id: String,
        /// As `SubscribeWorkflowNotifications::generation`.
        generation: u64,
    },
    DeliverWorkflowNotification {
        protocol_version: u32,
        subscription_id: String,
        envelope: crate::local::WorkflowNotificationEnvelope,
    },
    /// An agent on the sending kernel messages an ordinary agent of a room here; `urgent` steers.
    PeerAgentMessage {
        protocol_version: u32,
        request_id: String,
        sender: PeerAgentStamp,
        target: PeerAgentAddress,
        message: String,
        urgent: bool,
        reply_requested: bool,
        caller_authority: PeerCallerAuthority,
    },
    PeerAgentCancel {
        protocol_version: u32,
        request_id: String,
        sender: PeerAgentStamp,
        target: PeerAgentAddress,
        caller_authority: PeerCallerAuthority,
    },
    SubscribePeerAgent {
        protocol_version: u32,
        request_id: String,
        sender: PeerAgentStamp,
        target: PeerAgentAddress,
        subscription_id: String,
        generation: u64,
    },
    UnsubscribePeerAgent {
        protocol_version: u32,
        subscription_id: String,
        generation: u64,
    },
    /// A room agent on the sending kernel creates an ordinary agent in a room here.
    SpawnRoomPeer {
        protocol_version: u32,
        request_id: String,
        sender: PeerAgentStamp,
        room: String,
        provider: String,
        account_profile: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        alias: Option<String>,
        prompt: Option<String>,
        execution_mode: Option<crate::provider::AgentExecutionMode>,
        permission_level: Option<crate::provider::AgentPermissionLevel>,
        caller_authority: PeerCallerAuthority,
        follow: Option<PeerFollow>,
    },
    /// The creator of a peer here has ended: delete the agent its spawn `request_id` created,
    /// wherever it is now, or see to it that the spawn creates none.
    DeleteRoomPeer {
        protocol_version: u32,
        request_id: String,
        sender: PeerAgentStamp,
    },
    /// A followed peer's event, or the reply to a message, back to the kernel that asked.
    DeliverPeerAgentOutcome {
        protocol_version: u32,
        delivery_id: String,
        outcome: PeerAgentOutcome,
    },
    RoomBrowserController {
        session_id: String,
        slice_id: String,
        command: super::room_browser_controller::RoomBrowserControllerCommand,
    },
    // MP-11: old workers must reject this request instead of ignoring its audience.
    #[serde(rename = "open_room_shared_display_with_audience")]
    OpenRoomSharedDisplay {
        session_id: String,
        slice_id: String,
        lease_id: String,
        viewer_public_key: String,
        audience: super::room_shared_display::RoomDisplayAudience,
        offer: crate::slice::RoomDisplayOffer,
        viewport: crate::session::CanonicalViewport,
        target: super::room_shared_display::RoomDisplayTarget,
    },
    RoomSharedDisplay {
        session_id: String,
        slice_id: String,
        lease_id: String,
        command: super::room_shared_display::RoomDisplayCommand,
    },
    OpenRoomDisplay {
        session_id: String,
        slice_id: String,
        viewer_public_key: String,
    },
    CaptureRoomScreenshot {
        session_id: String,
        slice_id: String,
    },
    ReadRoomScreenshotChunk {
        session_id: String,
        slice_id: String,
        artifact_id: String,
        offset: u64,
        max_bytes: u32,
    },
    ObserveRoomComputer {
        session_id: String,
        slice_id: String,
        call: RemoteRoomComputerObservationCall,
    },
    Ping {
        value: String,
    },
    InstallManagedSliceRelayToken {
        slice_id: String,
        owner_kernel_id: String,
        owner_machine_id: String,
        activation_nonce: String,
        relay_token: RelayManagedSliceToken,
        expires_at_ms: u64,
        relay_recovery_token: RelayManagedSliceToken,
        recovery_expires_at_ms: u64,
    },
    ConfirmManagedSliceRelayToken {
        slice_id: String,
        owner_kernel_id: String,
        worker_kernel_id: String,
        activation_nonce: String,
    },
    RefreshManagedSliceRelayToken {
        slice_id: String,
        owner_kernel_id: String,
        worker_kernel_id: String,
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
        account_profile: String,
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
    UpdateLeasedAgentProfile {
        leased_agent_id: String,
        provider: String,
        account_profile: String,
        model: Option<String>,
        effort: Option<String>,
    },
    UpdateLeasedAgentMetaMode {
        leased_agent_id: String,
        active: bool,
    },
    UpdateLeasedAgentRemoteExtensionManifest {
        leased_agent_id: String,
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    /// The worker enforces the agent's deadline itself, so the run stops with the home offline.
    SetLeasedAgentDeadline {
        leased_agent_id: String,
        deadline_ms: u64,
    },
    /// `chariox.file.copy` from the leased agent: one slice of a file in its workspace, starting
    /// at `offset`.
    ReadLeasedAgentFile {
        leased_agent_id: String,
        path: String,
        offset: u64,
    },
    /// `chariox.file.copy` to the leased agent: one slice of a file arriving into its workspace.
    /// The file appears under `path` only when its last slice lands with digest `sha256`.
    WriteLeasedAgentFile {
        leased_agent_id: String,
        path: String,
        offset: u64,
        total_size: u64,
        sha256: String,
        overwrite: bool,
        content: RemoteFileBytes,
    },
    /// Files on this machine for another kernel of the same owner. What it is, by absolute path.
    StatMachinePath {
        path: String,
    },
    /// A page of a folder's entries, sorted by name, starting at entry `offset`.
    ListMachineDirectory {
        path: String,
        offset: u64,
    },
    /// One slice of a file, starting at `offset`; the answer is `FileSliceRead`.
    ReadMachineFile {
        path: String,
        offset: u64,
    },
    /// Deletes a file (only while its revision is still `expect_revision`) or an empty folder: the
    /// source of a move, once the destination holds a verified copy.
    DeleteMachinePath {
        path: String,
        #[serde(default)]
        expect_revision: Option<String>,
    },
    /// A file transfer whose destination is this machine, started by another kernel of the owner.
    StartFileTransfer {
        request: crate::local::StartFileTransferRequest,
    },
    GetFileTransfer {
        job_id: String,
    },
    CancelFileTransfer {
        job_id: String,
    },
    /// Drops what an unfinished `WriteLeasedAgentFile` staged for `path`.
    DiscardLeasedAgentFile {
        leased_agent_id: String,
        path: String,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        required_skills: Option<Vec<RequiredRemoteSkill>>,
        #[serde(
            default,
            skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
        )]
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_launch_credential: Option<RemoteProviderLaunchCredential>,
    },
    SendLeasedNativeProviderInput {
        leased_agent_id: String,
        provider_run_id: String,
        attachment_id: String,
        data_base64: String,
    },
    ResizeLeasedProviderTerminal {
        leased_agent_id: String,
        provider_run_id: String,
        cols: u16,
        rows: u16,
    },
    /// One Room terminal operation for the agent `leased_agent_id` runs as,
    /// typed as `actor` (the home Room's identity).
    RoomTerminal {
        leased_agent_id: String,
        actor: crate::room_terminal::TerminalActor,
        operation: crate::local::RoomTerminalOperation,
    },
    SubmitLeasedPrompt {
        leased_agent_id: String,
        expected_profile: RelayAgentExecutionProfile,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_resume_state: Option<crate::provider::ProviderResumeState>,
        prompt: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        hidden_system_context: String,
        #[serde(default)]
        attachments: Vec<RelayPromptAttachment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow_context: Option<RemoteWorkflowTurnContext>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        git_context: Option<RemoteGitTurnContext>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        required_mcps: Vec<RequiredRemoteMcp>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        required_skills: Option<Vec<RequiredRemoteSkill>>,
        #[serde(
            default,
            skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
        )]
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_launch_credential: Option<RemoteProviderLaunchCredential>,
    },
    /// A10: install or replace the worker fence for one exact leased home
    /// prompt's sudo window; `remaining_ms == 0` ends the window's fences.
    UpdateLeasedSudo {
        leased_agent_id: String,
        home_prompt_id: String,
        grant: LeasedSudoGrant,
    },
    GetLeasedPromptReceipt {
        leased_agent_id: String,
        home_prompt_id: String,
    },
    ReconcileLeasedPromptSteerReceipt {
        leased_agent_id: String,
        steer_id: String,
        target_home_prompt_id: String,
        worker_provider_run_id: String,
        execution_lease_id: String,
    },
    SteerLeasedPrompt {
        leased_agent_id: String,
        steer_id: String,
        target_home_prompt_id: String,
        prompt: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        hidden_system_context: String,
        #[serde(default)]
        attachments: Vec<RelayPromptAttachment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        required_skills: Option<Vec<RequiredRemoteSkill>>,
    },
    DrainLeasedRuntimeProjection {
        leased_agent_id: String,
        provider_run_id: String,
        #[serde(default)]
        pump_output: bool,
        /// Peer 383: the last projection sequence the home applied for this leased
        /// agent. The worker keeps a drained projection until this acknowledges it.
        acknowledged_projection_sequence: u64,
    },
    /// Peer 384: names the exact prompt it completes, like `CancelLeasedPrompt`.
    CompleteLeasedPrompt {
        leased_agent_id: String,
        home_prompt_id: String,
        worker_provider_run_id: String,
    },
    ObserveLeasedGitAfter {
        leased_agent_id: String,
        provider_run_id: String,
    },
    CancelLeasedPrompt {
        leased_agent_id: String,
        home_prompt_id: String,
        worker_provider_run_id: String,
    },
    LeasedWatchedProcess {
        leased_agent_id: String,
        action: LeasedWatchedProcessAction,
    },
    ForwardWorkflowRuntimeTool {
        context: RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
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
    SetLeasedComputerGrant {
        leased_agent_id: String,
        epoch: String,
        grant: Option<LeasedComputerGrant>,
    },
    CheckLeasedComputerGrant {
        context: RemoteExtensionInvocationContext,
        epoch: String,
    },
    ForwardRoomBrowserRuntimeTool {
        context: RemoteExtensionInvocationContext,
        call: RemoteRoomBrowserRuntimeToolCall,
    },
    ForwardTerminalRuntimeTool {
        context: RemoteExtensionInvocationContext,
        call: RemoteTerminalRuntimeToolCall,
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
    ForwardNativeTurnInteraction {
        context: RemoteNativeInteractionContext,
        interaction: crate::session::RuntimeInteraction,
    },
    UpdateNativeInteraction {
        context: RemoteNativeInteractionContext,
        interaction_id: String,
        login: Option<crate::session::RuntimeProviderLogin>,
    },
    EnsureRemoteSkillPackages {
        context: RemoteSkillSyncContext,
        packages: Vec<CharioxSkillPackage>,
    },
    RequestProviderAccountCopy {
        request: crate::local::CopyProviderAccountToKernelRequest,
    },
    InstallProviderAccountCopy {
        binding: crate::provider_account_copy::ProviderAccountCopyBinding,
        credential: crate::provider_account_copy::ProviderAccountCopyCredential,
    },
    EnsureRemoteProviderAccount {
        context: RemoteProviderAccountSyncContext,
        materialization: crate::account_profile::ProviderAccountMaterialization,
    },
    CheckRemoteMcpAvailability {
        context: RemoteMcpCheckContext,
        required_mcps: Vec<RequiredRemoteMcp>,
    },
    ArmManagedContextImport {
        plan: crate::managed_context::package::ManagedContextPlanBinding,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        target_environment_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        destination: Option<crate::managed_context::owner_managed::OwnerManagedDestination>,
        target_kernel_id: String,
        target_key_thumbprint: String,
        capability: RelayManagedContextCapability,
        archive_sha256: String,
        archive_size_bytes: u64,
    },
    BeginManagedContextImport {
        transfer_id: String,
        capability: RelayManagedContextCapability,
    },
    UploadManagedContextChunk {
        transfer_id: String,
        capability: RelayManagedContextCapability,
        offset: u64,
        data_base64: RelayManagedContextChunk,
        chunk_sha256: String,
    },
    FinalizeManagedContextImport {
        transfer_id: String,
        capability: RelayManagedContextCapability,
    },
    GetManagedContextImportStatus {
        transfer_id: String,
        capability: RelayManagedContextCapability,
    },
}

impl RelayPeerRequest {
    /// Off-lock callers can wait for both bounded native account probes plus
    /// reply cleanup. App-locked callers retain their configured response bound.
    pub(crate) fn off_lock_response_timeout(
        &self,
        configured: std::time::Duration,
    ) -> std::time::Duration {
        if matches!(
            self,
            Self::EnsureRemoteProviderAccount { .. } | Self::InstallProviderAccountCopy { .. }
        ) {
            configured.max(std::time::Duration::from_secs(150))
        } else {
            configured
        }
    }

    /// Charges transport setup to a forwarded decision's fixed expiry.
    pub(crate) fn rebase_decision_budget(&mut self) {
        if let Self::ForwardNativeTurnInteraction { interaction, .. } = self {
            interaction.rebase_decision_budget();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayPeerResponse {
    ProjectEnvironmentRead {
        source_label: String,
        record: Option<crate::project_environment::ProjectEnvironmentRecord>,
        /// Secret names the source's Vault holds a value for.
        secrets: Vec<String>,
        /// Listed Files that exist in the source's checkout.
        files: Vec<String>,
        /// Listed MCP registrations and skills this source holds.
        mcps: Vec<String>,
        skills: Vec<String>,
    },
    ProjectEnvironmentOperation {
        operation: crate::local::EnvironmentOperationView,
    },
    /// The source will not hand the Environment over (unknown Project, Vault locked, ...).
    ProjectEnvironmentRefused {
        reason: String,
    },
    ProjectEnvironmentSecretPulled {
        value: Option<RemoteCredentialSecretInput>,
    },
    /// `None` when the source holds no registration of that name.
    ProjectEnvironmentRegistrationPulled {
        registration: Option<RemoteProjectRegistration>,
    },
    ProjectEnvironmentFilePulled {
        /// The whole file's size, so the receiver knows when it has everything.
        size_bytes: u64,
        /// SHA-256 of the whole source file, binding every slice to its contents; absent when
        /// the file is not there.
        sha256: Option<String>,
        /// The slice at the requested offset; absent when the file is not there.
        content: Option<RemoteFileBytes>,
    },
    AgentMove {
        response: crate::agent_move::peer::AgentMovePeerResponse,
    },
    LeasedWatchedProcess {
        receipt: Option<crate::durable_state::agent_lifecycle::LeasedProcessReceipt>,
    },
    WorkflowNotificationSources {
        protocol_version: u32,
        sources: Vec<crate::local::WorkflowNotificationSourceSummary>,
    },
    WorkflowNotificationSubscribed {
        subscription: crate::local::WorkflowNotificationSubscription,
    },
    WorkflowNotificationUnsubscribed,
    /// The source ignored a subscribe or unsubscribe: it has already seen this newer generation
    /// of the subscription.
    WorkflowNotificationSuperseded {
        generation: u64,
    },
    WorkflowNotificationAccepted {
        ack: crate::local::WorkflowNotificationAck,
    },
    PeerAgentAccepted {
        protocol_version: u32,
        duplicate: bool,
        target: PeerAgentStamp,
    },
    PeerAgentSubscribed {
        protocol_version: u32,
        subscription_id: String,
        generation: u64,
        target: PeerAgentStamp,
    },
    RoomPeerSpawned {
        protocol_version: u32,
        duplicate: bool,
        agent: PeerAgentStamp,
        alias: String,
    },
    /// `duplicate` when there was no peer to delete: it was already gone, or its spawn had not
    /// created it and now will not.
    RoomPeerDeleted {
        protocol_version: u32,
        duplicate: bool,
    },
    PeerAgentOutcomeAck {
        protocol_version: u32,
        ack: PeerOutcomeAck,
    },
    PeerAgentRefused {
        protocol_version: u32,
        refusal: PeerAgentRefusal,
    },
    RoomBrowserController {
        session_id: String,
        slice_id: String,
        #[serde(with = "snapshot_wire")]
        result: super::room_browser_controller::RoomBrowserControllerResult,
    },
    RoomSharedDisplay {
        session_id: String,
        slice_id: String,
        result: super::room_shared_display::RoomDisplayResult,
    },
    RoomDisplayOpened {
        session_id: String,
        slice_id: String,
        endpoint: crate::slice::SliceDisplayEndpoint,
    },
    RoomScreenshotCaptured {
        session_id: String,
        slice_id: String,
        artifact: crate::local::RoomEnvironmentScreenshotArtifact,
    },
    RoomScreenshotChunk {
        session_id: String,
        slice_id: String,
        chunk: crate::local::RoomEnvironmentScreenshotChunk,
    },
    RoomComputerObserved {
        session_id: String,
        slice_id: String,
        result: RemoteRoomComputerObservationResult,
    },
    Pong {
        value: String,
        daemon_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        relay_peer_protocol_version: Option<u32>,
    },
    ManagedSliceRelayTokenInstalled {
        slice_id: String,
        activation_nonce: String,
        relay_peer_protocol_version: u32,
    },
    ManagedSliceRelayTokenActivated {
        slice_id: String,
        activation_nonce: String,
        relay_peer_protocol_version: u32,
    },
    ManagedSliceRelayTokenRefreshed {
        slice_id: String,
        relay_token: RelayManagedSliceToken,
        expires_at_ms: u64,
        relay_recovery_token: RelayManagedSliceToken,
        recovery_expires_at_ms: u64,
        relay_peer_protocol_version: u32,
    },
    ManagedSliceRelayTokenFailed {
        code: String,
        retryable: bool,
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
    LeasedAgentProfileUpdated {
        leased_agent: LeasedAgent,
    },
    LeasedAgentMetaModeUpdated {
        leased_agent: LeasedAgent,
    },
    FileSliceRead {
        /// The whole file's size, so the home knows when it has everything.
        size_bytes: u64,
        /// SHA-256 of the whole file; sent with the slice at offset 0.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
        /// Changes whenever the file is written or replaced; every slice of one copy carries the
        /// same one.
        revision: String,
        content: RemoteFileBytes,
    },
    LeasedAgentFileWritten {
        /// Bytes staged so far; the home continues from here.
        received: u64,
        /// The published file's digest, once the last slice landed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    LeasedAgentFileDiscarded,
    MachinePathStat {
        stat: crate::local::KernelFileStat,
    },
    MachineDirectory {
        entries: Vec<crate::local::KernelDirectoryEntry>,
        /// More entries follow at `offset` + `entries.len()`.
        truncated: bool,
    },
    MachinePathDeleted,
    FileTransfer {
        job: crate::local::FileTransferJob,
    },
    /// The other kernel will not do that to that path; repeating the request cannot change it.
    FileRefused {
        reason: String,
    },
    LeasedAgentRemoteExtensionManifestUpdated {
        leased_agent_id: String,
    },
    LeasedAgentDeadlineSet {
        leased_agent_id: String,
    },
    LeasedNativeProviderRunLaunched {
        provider_run: crate::provider::RuntimeProviderRun,
    },
    LeasedNativeProviderInputSent {
        byte_count: usize,
    },
    LeasedProviderTerminalResized {
        provider_run_id: String,
        cols: u16,
        rows: u16,
    },
    RoomTerminal {
        result: crate::local::RoomTerminalResult,
    },
    LeasedPromptSubmitted {
        provider_run_id: String,
        outcome: PromptSubmissionOutcome,
    },
    LeasedPromptReceiptQueried {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        receipt: Option<LeasedPromptReceipt>,
    },
    LeasedPromptSteered {
        provider_run_id: String,
        steer_id: String,
        replayed: bool,
    },
    LeasedRuntimeProjectionDrained {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event: Option<RelayPeerEvent>,
        /// Present when the worker retains `event` until the home acknowledges it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        projection_sequence: Option<u64>,
    },
    LeasedPromptCompleted {
        provider_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_diagnostic: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_termination: Option<crate::provider::ProviderRunTermination>,
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
    LeasedSudoUpdated,
    WorkflowRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    WorkspaceLiveSyncRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
        final_artifact_states: Vec<RemoteWorkspaceLiveSyncArtifactState>,
    },
    WorkspaceLiveSyncRuntimeToolFinalized,
    CapabilityRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill_package: Option<CharioxSkillPackage>,
        #[serde(
            default,
            skip_serializing_if = "crate::extension::RemoteExtensionManifest::is_empty"
        )]
        remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    },
    MetaRuntimeToolHandled {
        result: crate::transport::runtime_tools::RuntimeToolResult,
    },
    LeasedComputerGrantUpdated {},
    LeasedComputerGrantChecked {
        grant: LeasedComputerGrant,
    },
    RoomBrowserRuntimeToolHandled {
        result: RemoteRoomBrowserRuntimeToolResult,
    },
    TerminalRuntimeToolHandled {
        result: RemoteTerminalRuntimeToolResult,
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
        secret_input: RemoteCredentialSecretInput,
    },
    WorkspaceLiveSyncChangeApplied {
        target_result: crate::git_observer::WorkspaceLiveSyncTargetResult,
    },
    NativeInteractionResolved {
        resolution: crate::provider::ProviderNativeInteractionResolution,
    },
    NativeInteractionUpdated {},
    RemoteSkillPackagesEnsured {
        materialized: Vec<RemoteSkillMaterialization>,
    },
    ProviderAccountCopyFailed {
        code: String,
    },
    ProviderAccountCopyInstalled {
        receipt: crate::provider_account_copy::ProviderAccountCopyReceipt,
    },
    RemoteProviderAccountEnsured {
        provider: String,
        account_profile: String,
    },
    RemoteMcpAvailabilityChecked {
        results: Vec<RemoteMcpAvailability>,
    },
    ManagedContextImportArmed {
        transfer_id: String,
        capability: RelayManagedContextCapability,
        expires_at_ms: u64,
        max_chunk_bytes: usize,
        relay_peer_protocol_version: u32,
    },
    ManagedContextImportStatus {
        status: RelayManagedContextTransferStatus,
    },
    ManagedContextImportFailed {
        code: String,
        retryable: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_prompt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_termination: Option<crate::provider::ProviderRunTermination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<crate::provider::TurnOutcome>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn leased_prompt_cancellation_requires_exact_prompt_and_run_at_protocol_66() {
        assert_eq!(
            RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let request = RelayPeerRequest::CancelLeasedPrompt {
            leased_agent_id: "leased-agent-1".to_string(),
            home_prompt_id: "home-prompt-1".to_string(),
            worker_provider_run_id: "worker-run-1".to_string(),
        };
        let snapshot = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(
            snapshot,
            serde_json::json!({
                "kind": "cancel_leased_prompt",
                "leased_agent_id": "leased-agent-1",
                "home_prompt_id": "home-prompt-1",
                "worker_provider_run_id": "worker-run-1"
            })
        );
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(snapshot.clone())
                .expect("exact cancellation request should deserialize"),
            request
        );

        for missing_field in ["home_prompt_id", "worker_provider_run_id"] {
            let mut old_request = snapshot.clone();
            old_request
                .as_object_mut()
                .expect("request snapshot should be an object")
                .remove(missing_field);
            assert!(
                serde_json::from_value::<RelayPeerRequest>(old_request).is_err(),
                "mixed-version cancellation without {missing_field} must fail closed"
            );
        }
    }

    #[test]
    fn remote_room_browser_capability_manifest_is_versioned_at_protocol_66() {
        assert_eq!(
            RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let request = RelayPeerRequest::UpdateLeasedAgentRemoteExtensionManifest {
            leased_agent_id: "leased-agent-1".to_string(),
            remote_extension_manifest: crate::extension::RemoteExtensionManifest {
                room_browser_available: true,
                ..crate::extension::RemoteExtensionManifest::default()
            },
        };
        let snapshot = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(
            snapshot.pointer("/remote_extension_manifest/room_browser_available"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(snapshot)
                .expect("request should deserialize"),
            request
        );
    }

    #[test]
    fn leased_completion_provider_termination_shape_is_versioned() {
        assert_eq!(
            RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let completion = RelayProjectedCompletion {
            failure: None,
            message_id: "assistant-msg-1".to_string(),
            completed_at_ms: 1_234,
            home_prompt_id: Some("home-prompt-1".to_string()),
            provider_termination: Some(crate::provider::ProviderRunTermination::process_exit(
                137, 1_234,
            )),
        };
        let snapshot = serde_json::to_value(completion).expect("completion should serialize");
        assert_eq!(
            snapshot.pointer("/provider_termination/category"),
            Some(&serde_json::json!("process_exit")),
        );
        assert_eq!(
            snapshot.pointer("/provider_termination/reason"),
            Some(&serde_json::json!(
                "provider process exited with status 137"
            )),
        );
        assert_eq!(
            snapshot.pointer("/provider_termination/timestamp_ms"),
            Some(&serde_json::json!(1_234)),
        );
        assert_eq!(
            snapshot.pointer("/provider_termination/failure/kind"),
            Some(&serde_json::json!("provider_crashed")),
        );
        let serialized = serde_json::to_string(&snapshot).expect("completion should encode");
        let hash = Sha256::digest(serialized.as_bytes());
        assert_eq!(
            format!("{hash:x}"),
            "0d86c18bf510a18529d2304a1aa8c56035268d59f44f513f494856e9d77dc479",
        );
    }

    #[test]
    fn leased_failure_without_termination_shape_is_versioned() {
        let completion = RelayProjectedCompletion {
            message_id: "failed-turn".into(),
            completed_at_ms: 1234,
            home_prompt_id: Some("home-prompt".into()),
            provider_termination: None,
            failure: Some(crate::provider::TurnOutcome {
                kind: crate::provider::TurnOutcomeKind::UsageLimit,
                retryable: false,
                reason: "You've hit your usage limit.".into(),
            }),
        };
        let snapshot = serde_json::to_value(&completion).unwrap();
        assert_eq!(
            snapshot,
            serde_json::json!({
                "message_id": "failed-turn", "completed_at_ms": 1234,
                "home_prompt_id": "home-prompt", "failure": {
                    "kind": "usage_limit", "retryable": false,
                    "reason": "You've hit your usage limit.",
                },
            })
        );
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(encoded.as_bytes())),
            "78963c7a1dc75eb895450962abe3502d1b08b29b64e578e77c059d286abd1e15"
        );
        let old: RelayProjectedCompletion = serde_json::from_value(serde_json::json!({
            "message_id": "completed-turn", "completed_at_ms": 1234,
        }))
        .unwrap();
        assert_eq!(old.failure, None);
    }

    #[test]
    fn leased_completion_provider_termination_categories_are_wire_distinct() {
        let cases = [
            (
                crate::provider::ProviderRunTermination::signal("SIGTERM", 1_234),
                "signal",
            ),
            (
                crate::provider::ProviderRunTermination::explicit_provider_error(
                    "codex",
                    "provider rejected request",
                    1_235,
                ),
                "explicit_provider_error",
            ),
        ];

        for (provider_termination, category) in cases {
            let completion = RelayProjectedCompletion {
                failure: None,
                message_id: "assistant-msg-1".to_string(),
                completed_at_ms: provider_termination.timestamp_ms,
                home_prompt_id: Some("home-prompt-1".to_string()),
                provider_termination: Some(provider_termination),
            };
            let snapshot = serde_json::to_value(completion)
                .expect("termination category completion should serialize");
            assert_eq!(
                snapshot.pointer("/provider_termination/category"),
                Some(&serde_json::json!(category)),
            );
        }
    }

    #[test]
    fn leased_prompt_receipt_query_and_steer_reconciliation_are_versioned_at_protocol_66() {
        assert_eq!(
            RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let request = RelayPeerRequest::GetLeasedPromptReceipt {
            leased_agent_id: "leased-agent-1".to_string(),
            home_prompt_id: "home-prompt-1".to_string(),
        };
        let request_snapshot = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(
            request_snapshot,
            serde_json::json!({
                "kind": "get_leased_prompt_receipt",
                "leased_agent_id": "leased-agent-1",
                "home_prompt_id": "home-prompt-1"
            })
        );
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(request_snapshot)
                .expect("request should deserialize"),
            request
        );
        let reconcile = RelayPeerRequest::ReconcileLeasedPromptSteerReceipt {
            leased_agent_id: "leased-agent-1".to_string(),
            steer_id: "queued-steer-1".to_string(),
            target_home_prompt_id: "active-home-prompt-1".to_string(),
            worker_provider_run_id: "worker-run-1".to_string(),
            execution_lease_id: "lease-1".to_string(),
        };
        let reconcile_snapshot =
            serde_json::to_value(&reconcile).expect("reconcile request should serialize");
        assert_eq!(
            reconcile_snapshot,
            serde_json::json!({
                "kind": "reconcile_leased_prompt_steer_receipt",
                "leased_agent_id": "leased-agent-1",
                "steer_id": "queued-steer-1",
                "target_home_prompt_id": "active-home-prompt-1",
                "worker_provider_run_id": "worker-run-1",
                "execution_lease_id": "lease-1"
            })
        );
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(reconcile_snapshot)
                .expect("reconcile request should deserialize"),
            reconcile
        );

        let active = RelayPeerResponse::LeasedPromptReceiptQueried {
            receipt: Some(LeasedPromptReceipt {
                home_prompt_id: "home-prompt-1".to_string(),
                worker_provider_run_id: "worker-run-1".to_string(),
                phase: LeasedPromptReceiptPhase::Active,
                target_home_prompt_id: None,
                execution_lease_id: None,
            }),
        };
        let active_snapshot = serde_json::to_value(&active).expect("response should serialize");
        assert_eq!(
            active_snapshot,
            serde_json::json!({
                "kind": "leased_prompt_receipt_queried",
                "receipt": {
                    "home_prompt_id": "home-prompt-1",
                    "worker_provider_run_id": "worker-run-1",
                    "phase": "active"
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<RelayPeerResponse>(active_snapshot)
                .expect("active receipt should deserialize"),
            active
        );

        let completed = RelayPeerResponse::LeasedPromptReceiptQueried {
            receipt: Some(LeasedPromptReceipt {
                home_prompt_id: "home-prompt-1".to_string(),
                worker_provider_run_id: "worker-run-1".to_string(),
                phase: LeasedPromptReceiptPhase::Completed,
                target_home_prompt_id: None,
                execution_lease_id: None,
            }),
        };
        let completed_snapshot =
            serde_json::to_value(&completed).expect("completed response should serialize");
        assert_eq!(
            completed_snapshot,
            serde_json::json!({
                "kind": "leased_prompt_receipt_queried",
                "receipt": {
                    "home_prompt_id": "home-prompt-1",
                    "worker_provider_run_id": "worker-run-1",
                    "phase": "completed"
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<RelayPeerResponse>(completed_snapshot)
                .expect("completed receipt should deserialize"),
            completed
        );

        for (phase, phase_wire) in [
            (
                LeasedPromptReceiptPhase::SteerDispatching,
                "steer_dispatching",
            ),
            (LeasedPromptReceiptPhase::SteerRejected, "steer_rejected"),
        ] {
            let original_prompt = RelayPeerResponse::LeasedPromptReceiptQueried {
                receipt: Some(LeasedPromptReceipt {
                    home_prompt_id: "home-prompt-1".to_string(),
                    worker_provider_run_id: String::new(),
                    phase,
                    target_home_prompt_id: None,
                    execution_lease_id: Some("lease-1".to_string()),
                }),
            };
            let snapshot = serde_json::to_value(&original_prompt)
                .expect("original-prompt admission receipt should serialize");
            assert_eq!(
                snapshot,
                serde_json::json!({
                    "kind": "leased_prompt_receipt_queried",
                    "receipt": {
                        "home_prompt_id": "home-prompt-1",
                        "worker_provider_run_id": "",
                        "phase": phase_wire,
                        "execution_lease_id": "lease-1"
                    }
                })
            );
            assert_eq!(
                serde_json::from_value::<RelayPeerResponse>(snapshot)
                    .expect("original-prompt admission receipt should deserialize"),
                original_prompt
            );
        }

        let none = RelayPeerResponse::LeasedPromptReceiptQueried { receipt: None };
        let none_snapshot = serde_json::to_value(&none).expect("empty response should serialize");
        assert_eq!(
            none_snapshot,
            serde_json::json!({ "kind": "leased_prompt_receipt_queried" })
        );
        assert_eq!(
            serde_json::from_value::<RelayPeerResponse>(none_snapshot)
                .expect("empty response should deserialize"),
            none
        );

        let steer_receipts = [
            (
                LeasedPromptReceiptPhase::SteerDispatching,
                "steer_dispatching",
            ),
            (LeasedPromptReceiptPhase::SteerAccepted, "steer_accepted"),
            (LeasedPromptReceiptPhase::SteerRejected, "steer_rejected"),
        ];
        for (phase, phase_wire) in steer_receipts {
            let response = RelayPeerResponse::LeasedPromptReceiptQueried {
                receipt: Some(LeasedPromptReceipt {
                    home_prompt_id: "queued-steer-1".to_string(),
                    worker_provider_run_id: "worker-run-1".to_string(),
                    phase,
                    target_home_prompt_id: Some("active-home-prompt-1".to_string()),
                    execution_lease_id: Some("lease-1".to_string()),
                }),
            };
            let snapshot =
                serde_json::to_value(&response).expect("queued-steer receipt should serialize");
            assert_eq!(
                snapshot,
                serde_json::json!({
                    "kind": "leased_prompt_receipt_queried",
                    "receipt": {
                        "home_prompt_id": "queued-steer-1",
                        "worker_provider_run_id": "worker-run-1",
                        "phase": phase_wire,
                        "target_home_prompt_id": "active-home-prompt-1",
                        "execution_lease_id": "lease-1"
                    }
                })
            );
            assert_eq!(
                serde_json::from_value::<RelayPeerResponse>(snapshot)
                    .expect("queued-steer receipt should deserialize"),
                response
            );
        }
    }

    #[test]
    fn retired_workflow_failure_forwarding_is_not_an_accepted_peer_protocol_path() {
        let request = serde_json::json!({
            "kind": "forward_workflow_provider_failure",
            "context": {
                "home_kernel_id": "home",
                "home_session_id": "room",
                "home_agent_id": "agent",
                "workflow_run_id": "run",
                "workflow_node_run_id": "node",
                "delivery_token": "test-turn"
            },
            "message": "provider capacity exhausted"
        });
        assert!(
            serde_json::from_value::<RelayPeerRequest>(request).is_err(),
            "provider failures must use correlated runtime projection, not a second RPC"
        );
        assert!(
            serde_json::from_value::<RelayPeerResponse>(serde_json::json!({
                "kind": "workflow_provider_failure_handled"
            }))
            .is_err()
        );
    }

    #[test]
    fn managed_slice_relay_token_keeps_wire_shape_and_redacts_debug_output() {
        let request = RelayPeerRequest::InstallManagedSliceRelayToken {
            slice_id: "slice-1".to_string(),
            owner_kernel_id: "kernel-owner".to_string(),
            owner_machine_id: "machine-owner".to_string(),
            activation_nonce: "activation-1".to_string(),
            relay_token: RelayManagedSliceToken::new("secret-relay-token".to_string()),
            expires_at_ms: 300_000,
            relay_recovery_token: RelayManagedSliceToken::new("secret-recovery-token".to_string()),
            recovery_expires_at_ms: 2_592_000_000,
        };
        let encoded = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(encoded["kind"], "install_managed_slice_relay_token");
        assert_eq!(encoded["relay_token"], "secret-relay-token");
        assert_eq!(encoded["relay_recovery_token"], "secret-recovery-token");

        let debug = format!("{request:?}");
        assert!(debug.contains("[redacted managed-slice relay token]"));
        assert!(!debug.contains("secret-relay-token"));
        assert!(!debug.contains("secret-recovery-token"));

        let decoded: RelayPeerRequest =
            serde_json::from_value(encoded).expect("request should deserialize");
        assert_eq!(decoded, request);
    }

    #[test]
    fn managed_slice_relay_refresh_response_redacts_token() {
        let response = RelayPeerResponse::ManagedSliceRelayTokenRefreshed {
            slice_id: "slice-1".to_string(),
            relay_token: RelayManagedSliceToken::new("refreshed-secret".to_string()),
            expires_at_ms: 600_000,
            relay_recovery_token: RelayManagedSliceToken::new(
                "refreshed-recovery-secret".to_string(),
            ),
            recovery_expires_at_ms: 2_592_600_000,
            relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        };
        let debug = format!("{response:?}");
        assert!(debug.contains("[redacted managed-slice relay token]"));
        assert!(!debug.contains("refreshed-secret"));
        assert!(!debug.contains("refreshed-recovery-secret"));
    }

    #[test]
    fn managed_slice_activation_confirmation_has_versioned_nonce_wire_shape() {
        let request = RelayPeerRequest::ConfirmManagedSliceRelayToken {
            slice_id: "slice-1".to_string(),
            owner_kernel_id: "kernel-owner".to_string(),
            worker_kernel_id: "kernel-worker".to_string(),
            activation_nonce: "activation-1".to_string(),
        };
        let request_value = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(request_value["kind"], "confirm_managed_slice_relay_token");
        assert_eq!(request_value["activation_nonce"], "activation-1");
        assert_eq!(
            serde_json::from_value::<RelayPeerRequest>(request_value)
                .expect("request should deserialize"),
            request
        );

        let response = RelayPeerResponse::ManagedSliceRelayTokenActivated {
            slice_id: "slice-1".to_string(),
            activation_nonce: "activation-1".to_string(),
            relay_peer_protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        };
        let response_value = serde_json::to_value(&response).expect("response should serialize");
        assert_eq!(
            response_value["kind"],
            "managed_slice_relay_token_activated"
        );
        assert_eq!(
            response_value["relay_peer_protocol_version"],
            RELAY_PEER_PROTOCOL_VERSION
        );
        assert_eq!(
            serde_json::from_value::<RelayPeerResponse>(response_value)
                .expect("response should deserialize"),
            response
        );
    }
}

#[cfg(test)]
mod native_approval_protocol_tests {
    #[test]
    fn native_approval_origin_relay_shape_is_versioned() {
        use sha2::{Digest, Sha256};
        assert_eq!(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            crate::test_support::expected_local_protocol_version()
        );
        let snapshot = serde_json::json!({"kind": "forward_native_turn_interaction",
            "context": {"home_session_id":"home-session", "home_agent_id":"home-agent",
                "leased_agent_id":"lease", "worker_provider_run_id":"run", "home_prompt_id":"home-prompt-A"},
            "interaction": {"id":"approval", "agent_id":"worker-agent", "kind":"permission", "level":"warning",
                "message":"Allow?", "choices":[], "native_origin":{"scope":"prompt", "provider_run_id":"run", "prompt_id":"worker-prompt-A"},
                "requested_at_ms":1}
        });
        let request: super::RelayPeerRequest = serde_json::from_value(snapshot.clone()).unwrap();
        let mut legacy = snapshot.clone();
        legacy["kind"] = serde_json::json!("forward_native_interaction");
        assert!(serde_json::from_value::<super::RelayPeerRequest>(legacy).is_err());
        assert_eq!(serde_json::to_value(request).unwrap(), snapshot);
        let encoded = serde_json::to_vec(&snapshot).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(encoded)),
            "dee36b9bf4d4a4e4877a46ea81e1be63d696f78ebc07ebf90d964606c6d93084"
        );
    }
}

#[cfg(test)]
mod leased_agent_deadline_shapes {
    use super::*;
    use sha2::{Digest, Sha256};

    fn pinned<T>(snapshot: serde_json::Value, hash: &str)
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let decoded: T = serde_json::from_value(snapshot.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), snapshot);
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
            ),
            hash
        );
    }

    // The worker enforces a leased agent's deadline (peer 442): the home asks, then
    // records the deadline only once the worker confirms it.
    #[test]
    fn the_worker_deadline_request_and_its_confirmation_keep_their_wire_shape() {
        pinned::<RelayPeerRequest>(
            serde_json::json!({"kind":"set_leased_agent_deadline","leased_agent_id":"lease","deadline_ms":1793700040000_u64}),
            "93d5696247b76e96c9188e3f65be1642338f562a4223c8ba2c390ef378477684",
        );
        pinned::<RelayPeerResponse>(
            serde_json::json!({"kind":"leased_agent_deadline_set","leased_agent_id":"lease"}),
            "b64a7791b7687bf3b9396ec2375733cf1d2d364d2df873918921d2ede940be75",
        );
    }
}

#[cfg(test)]
mod owner_managed_context_shapes {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn mp08_mp11_owner_managed_arm_shape_requires_peer_117() {
        assert_eq!(
            RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let value: serde_json::Value = serde_json::from_str(r#"{"kind":"arm_managed_context_import","plan":{"contextId":"context","planDigest":"digest","destination":{"kind":"owner_managed_machine","machineId":"machine","kernelId":"kernel"},"kernelContext":"source_kernel_without_credentials","development":{"kind":"empty"},"providerAccounts":{"kind":"none"},"gitCredentials":{"kind":"none"}},"destination":{"kind":"owner_managed_machine","machineId":"machine","kernelId":"kernel"},"target_kernel_id":"kernel","target_key_thumbprint":"thumbprint","capability":"capability","archive_sha256":"archive","archive_size_bytes":128}"#).unwrap();
        let request: RelayPeerRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), value);
        assert_eq!(
            format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap())),
            "972bfa82e0e37d399f829a7cca14b7260100b207c610d86ccc2c0039eaab6bf4"
        );
        assert!(value.get("target_environment_id").is_none());
    }
}

#[cfg(test)]
mod workflow_notification_shapes {
    use super::*;
    use crate::local::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn mp08_mp10_completion_peer_current_snapshot() {
        assert_eq!(
            RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let sub = WorkflowNotificationSubscription {
            subscription_id: "sub".into(),
            source_id: "source".into(),
            owner_user_id: "owner".into(),
            created_by_user_id: "owner".into(),
            source_kernel_id: "source-kernel".into(),
            target_kernel_id: "target-kernel".into(),
            target_kind: WorkflowNotificationTargetKind::WorkflowEndpoint,
            session_id: "session".into(),
            workflow_id: "workflow".into(),
            publication_id: "publication".into(),
            endpoint_id: "endpoint".into(),
            queue_id: "queue".into(),
            source_available: true,
            agent_id: None,
            trigger_id: None,
        };
        let env = WorkflowNotificationEnvelope {
            source_id: "source".into(),
            display_name: "Reviewer-2".into(),
            occurrence_id: "run".into(),
            output: crate::session::WorkflowOutputPayload::new("approved", vec![]),
            ancestry: vec!["source-workflow".into()],
            deadline_ms: 604800000,
        };
        let requests = vec![
            RelayPeerRequest::ListWorkflowNotificationSources {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            },
            RelayPeerRequest::SubscribeWorkflowNotifications {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                source_id: "source".into(),
                target_ref: sub.clone(),
                generation: 3,
            },
            RelayPeerRequest::DeliverWorkflowNotification {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                subscription_id: "sub".into(),
                envelope: env,
            },
            RelayPeerRequest::UnsubscribeWorkflowNotifications {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                subscription_id: "sub".into(),
                generation: 4,
            },
        ];
        let responses = vec![
            RelayPeerResponse::WorkflowNotificationSources {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                sources: vec![WorkflowNotificationSourceSummary {
                    kind: crate::local::NotificationSourceKind::WorkflowExit,
                    source_id: "source".into(),
                    kernel_id: "kernel".into(),
                    session_id: "session".into(),
                    workflow_id: "workflow".into(),
                    exit_node_id: "node-2".into(),
                    exit_ordinal: 2,
                    name: "Reviewer".into(),
                    display_name: "Reviewer-2".into(),
                    available: true,
                }],
            },
            RelayPeerResponse::WorkflowNotificationSubscribed { subscription: sub },
            RelayPeerResponse::WorkflowNotificationAccepted {
                ack: WorkflowNotificationAck::Accepted,
            },
            RelayPeerResponse::WorkflowNotificationUnsubscribed,
            RelayPeerResponse::WorkflowNotificationSuperseded { generation: 5 },
        ];
        let mut wire = serde_json::json!({"requests":requests,"responses":responses});
        // Pin the wire shape independently of the generated current peer number.
        for collection in ["requests", "responses"] {
            for message in wire[collection].as_array_mut().unwrap() {
                if let Some(version) = message.get_mut("protocol_version") {
                    assert_eq!(*version, RELAY_PEER_PROTOCOL_VERSION);
                    *version = serde_json::json!(0);
                }
            }
        }
        assert_eq!(
            format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
            "02a8d98603e23bde427cd5c238c4dcf487071f955d4c6848ca31c82f2b6228ee"
        );
    }
}

#[cfg(test)]
mod room_terminal_peer_shapes {
    use super::*;
    use crate::local::{RoomTerminalOperation, RoomTerminalResult};
    use crate::room_terminal::TerminalActor;
    use sha2::{Digest, Sha256};

    fn assert_wire<T>(wire: serde_json::Value, expected_sha256: &str)
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let typed: T = serde_json::from_value(wire.clone()).unwrap();
        let encoded = serde_json::to_value(typed).unwrap();
        assert_eq!(encoded, wire);
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&encoded).unwrap())
            ),
            expected_sha256
        );
    }

    #[test]
    fn room_terminal_peer_request_and_response_shapes() {
        let request = RelayPeerRequest::RoomTerminal {
            leased_agent_id: "leased-1".into(),
            actor: TerminalActor::Agent("agent-2".into()),
            operation: RoomTerminalOperation::Close {
                terminal_id: "term-1".into(),
            },
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"kind": "room_terminal", "leased_agent_id": "leased-1",
                "actor": {"kind": "agent", "agent_id": "agent-2"},
                "operation": {"operation": "close", "terminal_id": "term-1"}})
        );
        assert_wire::<RelayPeerRequest>(
            wire,
            "eacffce23693fe298745f52ede63de4ea2f7015a31296ebf4d52f349e270988b",
        );
        let response = RelayPeerResponse::RoomTerminal {
            result: RoomTerminalResult::Closed,
        };
        assert_wire::<RelayPeerResponse>(
            serde_json::to_value(&response).unwrap(),
            "97df114510c195b53e132e9bc4bc8fb9cad7d9bfe806b33ad8530022e46a03c9",
        );
    }

    #[test]
    fn forwarded_terminal_tool_peer_request_and_response_shapes() {
        let request = RelayPeerRequest::ForwardTerminalRuntimeTool {
            context: RemoteExtensionInvocationContext {
                home_kernel_id: "home".into(),
                home_session_id: "room-1".into(),
                home_agent_id: "agent-1".into(),
                leased_agent_id: "leased-1".into(),
                worker_provider_run_id: "run-1".into(),
                worker_kernel_id: Some("worker".into()),
                worker_machine_id: None,
            },
            call: RemoteTerminalRuntimeToolCall {
                tool_name: "chariox.terminal_read".into(),
                arguments: serde_json::json!({"terminal_id": "term-1", "home_kernel": true}),
            },
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"kind": "forward_terminal_runtime_tool",
                "context": {"home_kernel_id": "home", "home_session_id": "room-1",
                    "home_agent_id": "agent-1", "leased_agent_id": "leased-1",
                    "worker_provider_run_id": "run-1", "worker_kernel_id": "worker"},
                "call": {"tool_name": "chariox.terminal_read",
                    "arguments": {"terminal_id": "term-1", "home_kernel": true}}})
        );
        assert_wire::<RelayPeerRequest>(
            wire,
            "a351eac8619e213b3af207f645f068c2fc28a8046125b98d26c447b0861f2139",
        );
        let response = RelayPeerResponse::TerminalRuntimeToolHandled {
            result: RemoteTerminalRuntimeToolResult(
                crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({"closed": true}),
                },
            ),
        };
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"kind": "terminal_runtime_tool_handled",
                "result": {"ok": true, "payload": {"closed": true}}})
        );
        assert_wire::<RelayPeerResponse>(
            wire,
            "48fbcd5f06df3e69938eb9c3d53b4d84f3c73901fa1c953aa2e364f3b098ae2a",
        );
    }
}

#[cfg(test)]
mod multidomain_union_tests {
    #[test]
    fn md_notes_room_observation_protocol_74_snapshot_and_hash() {
        use sha2::{Digest, Sha256};
        assert_eq!(
            super::RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        assert_eq!(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            crate::test_support::expected_local_protocol_version()
        );
        let request = serde_json::json!({"kind":"room_browser_controller","session_id":"room-1","slice_id":"slice-1","command":{"kind":"note_observation","target_id":"target-1","document_id":"doc-1"}});
        let response = serde_json::json!({"kind":"room_browser_controller","session_id":"room-1","slice_id":"slice-1","result":{"kind":"note_observation","observation":{"target_id":"target-1","document_id":"doc-1","url":"https://example.test/","selection":null}}});
        let decoded: super::RelayPeerRequest = serde_json::from_value(request.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), request);
        let decoded: super::RelayPeerResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), response);
        let snapshot = serde_json::json!({"request":request,"response":response});
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
            ),
            "9e4c866474f43f003928723345c0313c768824139d9eb765931d688d303d0785"
        );
    }
}

#[cfg(test)]
mod notes_protocol_probe_tests {
    #[test]
    fn md_notes_protocol_probe_preserves_legacy_pong_and_pins_union_advertisement() {
        use sha2::{Digest, Sha256};
        assert_eq!(
            super::RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let current = serde_json::json!({"kind":"pong","value":"md-notes-protocol","daemon_id":"worker-1","relay_peer_protocol_version":74});
        let legacy =
            serde_json::json!({"kind":"pong","value":"md-notes-protocol","daemon_id":"worker-1"});
        for (value, expected) in [(&current, Some(74)), (&legacy, None)] {
            let decoded: super::RelayPeerResponse = serde_json::from_value(value.clone()).unwrap();
            assert!(
                matches!(&decoded, super::RelayPeerResponse::Pong { relay_peer_protocol_version, .. } if *relay_peer_protocol_version == expected)
            );
            assert_eq!(serde_json::to_value(decoded).unwrap(), *value);
        }
        let snapshot = serde_json::json!({"current":current,"legacy":legacy});
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
            ),
            "5c9e2f881c4ebce7f3ee9f4af71ec364547a4f51f451f46d457e5809885c29ac"
        );
    }
}

#[cfg(test)]
mod leased_watched_process_protocol_tests {
    use super::*;
    #[test]
    fn mp08_mp10_mp11_leased_process_query_shape_is_bound_to_peer102() {
        assert_eq!(
            RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        let binding = crate::durable_state::agent_lifecycle::LeasedProcessBinding {
            worker_kernel_id: "worker".into(),
            worker_machine_id: "machine".into(),
            execution_lease_id: "lease".into(),
            leased_agent_id: "leased".into(),
            home_prompt_id: "prompt".into(),
            worker_provider_run_id: "run".into(),
        };
        let request = RelayPeerRequest::LeasedWatchedProcess {
            leased_agent_id: "leased".into(),
            action: LeasedWatchedProcessAction::Query {
                wake_id: "wake".into(),
                binding,
            },
        };
        let value = serde_json::to_value(request).unwrap();
        let expected = serde_json::json!({"kind":"leased_watched_process","leased_agent_id":"leased","action":{"Query":{"wake_id":"wake","binding":{"worker_kernel_id":"worker","worker_machine_id":"machine","execution_lease_id":"lease","leased_agent_id":"leased","home_prompt_id":"prompt","worker_provider_run_id":"run"}}}});
        assert_eq!(value, expected);
        use sha2::{Digest, Sha256};
        assert_eq!(
            format!("{:x}", Sha256::digest(value.to_string())),
            "2432555231cea1828a200e1a9519230b8689bf51496fe714ec1a35cd239122d6"
        );
    }
}

#[cfg(test)]
mod mp08_transport_compatibility_tests {
    #[test]
    fn mp08_mp11_transport_downgrade_cannot_admit_legacy_worker_contracts() {
        assert_eq!(
            super::RELAY_PEER_PROTOCOL_VERSION,
            crate::test_support::expected_relay_peer_protocol_version()
        );
        // MP-08/MP-10/MP-11: transport negotiation does not grant an older
        // worker the integrated task, wake and leased-watcher contract.
        for (version, compatible) in [
            (None, false),
            (Some(89), false),
            (Some(90), false),
            (Some(94), false),
            (Some(96), false),
            (Some(102), false),
            (Some(114), false),
            (Some(121), false),
            (Some(128), false),
            (Some(super::RELAY_PEER_PROTOCOL_VERSION - 1), false),
            (Some(super::RELAY_PEER_PROTOCOL_VERSION), true),
            (Some(super::RELAY_PEER_PROTOCOL_VERSION + 1), true),
        ] {
            let binding: crate::agent::RemoteAgentBinding =
                serde_json::from_value(serde_json::json!({
                    "worker_kernel_id":"worker","worker_machine_id":"machine",
                    "execution_lease_id":"lease","leased_agent_id":"agent",
                    "relay_peer_protocol_version":version,
                }))
                .unwrap();
            assert_eq!(binding.relay_peer_protocol_compatible(), compatible);
        }
    }
}
