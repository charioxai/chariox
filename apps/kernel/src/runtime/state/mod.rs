//! Shared runtime-state facade and async orchestration wiring.
//!
//! Domain modules own the concrete session/provider/prompt/workflow and workspace live sync mutations.
//! This root keeps the public `KernelRuntimeState` entry points, shared fields, and cross-domain
//! plumbing that would otherwise create cycles between those modules.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use tokio::sync::{Mutex, Notify};

use crate::agent::AgentServiceStore;
use crate::app::{
    ActiveTurnStore, AttachedProviderTranscriptCursorStore, DaemonApp,
    ExternalProviderSessionIndexStore, PromptActivityStore, PromptWorkspaceClaimStore,
    ProviderLaunchFailureRetryStore, ProviderProcessTrackingStore, WorkflowDesignEventStore,
};
use crate::attachment::AttachmentServiceStore;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::history::{OperationalHistoryStore, SessionHistoryEntry};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::{ProviderProcessServiceStore, ProviderRunOperationLanes};
use crate::runtime::metaagent_event::MetaagentEventStore;
use crate::session::{SessionStateOwner, SessionStateStore};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

mod workspace_live_sync;
use workspace_live_sync::*;
mod workspace_live_sync_workspace_context;
use workspace_live_sync_workspace_context::*;
mod agent_move_abort;
mod agent_move_composition;
#[cfg(test)]
mod agent_move_launch_tests;
mod agent_move_peer;
mod agent_move_preflight;
mod agent_move_queue;
mod agent_move_runtime;
mod agent_move_slices;
mod agent_move_source;
mod agent_move_status;
mod agent_move_target;
mod agent_move_transport;
mod context_handoff;
mod failed_request_owned_state;
mod native_prompt_handoff;
use context_handoff::PendingAgentContextHandoff;
pub(crate) use context_handoff::PendingAgentContextHandoffStore;
mod app_automation_owned_state;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_connection_runtime;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_control_requests;
mod app_event_delivery_owned_state;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_event_pump_runtime;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_inbox_runtime;
mod app_runtime_state;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_set_runtime;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_wake_pump_runtime;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod workflow_publication_app_copy;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod workflow_publication_apps_runtime;
#[cfg(all(
    test,
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
pub(crate) use workflow_publication_app_copy::fixture_copy_request_id;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_deployment_consent_runtime;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_file_pick_runtime;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_foreground_runtime;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_host_runtime;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_validation_pump_runtime;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_view_host;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_view_poll;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod app_view_runtime;
mod computer_secret_input_runtime_state;
mod config_runtime_state;
mod critical_approval_passkey;
mod kernel_access;
mod outside_events;
pub(crate) use outside_events::is_outside_event_request;
mod sudo;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod user_app_view_browser;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod user_app_view_restore;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
mod user_app_view_runtime;
#[cfg(test)]
pub(crate) use critical_approval_passkey::critical_approval_audit_payload;
#[cfg(test)]
pub(crate) use sudo::leased::LEASED_SUDO_PEER_PROTOCOL_VERSION;
pub(crate) use sudo::SudoWindowProjection;
#[cfg(test)]
pub(crate) use sudo::SUDO_WINDOW_LENGTH_FOR_TEST;
pub(crate) use sudo::{is_sudo_control, is_sudo_prompt, sudo_window_minutes};
mod passkey_prompts;
#[cfg(test)]
pub(crate) use passkey_prompts::PASSKEY_ALREADY_ANSWERED;
mod native_catalog_refresh;
mod owner_context_review;
mod project_environment_export;
mod project_environment_files;
mod project_environment_manifest;
mod project_environment_placement;
mod project_environment_remote;
mod project_environment_review;
mod project_environment_worker_export;
mod project_environment_workspaces;
mod project_prompt_promotion;
mod provider_output_deadline_store;
mod provider_reload;
mod room_computer_readiness;
#[cfg(test)]
mod room_computer_readiness_tests;
pub(crate) use provider_output_deadline_store::ProviderOutputDeadlineStore;
pub(crate) use provider_reload::*;
#[cfg(all(
    test,
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
mod app_event_path_tests;
mod browser_import_consent;
mod browser_import_execution_gate;
#[cfg(test)]
mod computer_hold_tests;
mod environment_execution_gate;
mod event_delivery_runtime_state;
mod human_browser_action_runtime_state;
mod human_environment_action_runtime_state;
mod leased_agent_operations;
mod managed_activity_runtime_state;
mod managed_kernel_quiescence_state;
pub(crate) use managed_kernel_quiescence_state::{
    ManagedKernelAdmissionGuard, ManagedKernelQuiescenceChallenge, ManagedKernelQuiescenceGate,
    ManagedKernelQuiescenceOutcome,
};
mod claude_permission_rules;
mod provider_launch_defaults_owned_state;
mod provider_relaunch_runtime;
mod provider_reload_pending_runtime;
mod provider_run_read_state;
mod public_history;
mod publication_activation;
mod room_browser_artifact;
mod room_browser_controller;
mod room_browser_controller_admission;
mod room_browser_manifest_sync;
mod room_browser_start_failure;
mod room_computer_access;
mod room_computer_observation;
mod room_display;
mod room_environment_health;
mod room_environment_placement;
mod room_environment_state;
mod room_exec_runtime_state;
mod room_screenshot;
mod room_secret_observation;
mod room_shared_display;
mod room_terminal_runtime_state;
mod runtime_tool_call_activity;
mod usage_snapshot;
mod visible_region_capture;
pub(crate) use runtime_tool_call_activity::RuntimeToolCallActivity;

#[derive(Clone)]
pub(crate) struct KernelRuntimeState {
    external_command_authority: Option<ExternalCommandAuthority>,
    room_provider_origin: Option<(String, String)>,
    room_request_origin: Option<(String, LocalDaemonRequest)>,
    /// MP-08 / MP-11: the transport admitted this command from a terminal
    /// (local token or relay client), not from a granted or launched agent.
    terminal_caller: bool,
    relay_peer_authority: Option<crate::runtime::relay_peer_authority::RelayPeerAuthority>,
    forwarded_peer_binding: Option<forwarded_peer_authority::ForwardedPeerBinding>,
    #[cfg(test)]
    app_lock_wait_probe: Option<Arc<tokio::sync::Notify>>,
    #[cfg(test)]
    queue_promotion_exit_probe: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
    provider_history_appends: provider_history_runtime::ProviderHistoryAppends,
    leased_agent_operations: leased_agent_operations::LeasedAgentOperations,
    detached_workflow_provider_launches: Arc<std::sync::Mutex<BTreeSet<String>>>,
    /// Recovery operations this kernel life dispatched. One from an earlier life has no provider
    /// behind it any more, so restart recovery may supersede it instead of waiting for it.
    restart_recovery_dispatched: Arc<std::sync::Mutex<BTreeSet<String>>>,
    /// What the person granted with "Allow for this session" on Claude permission prompts, per
    /// agent, so a restarted Claude process starts with it again.
    claude_session_grants: claude_permission_rules::StandingGrants,
    /// The only strong handle on the terminal wake driver. The driver holds a runtime copy made
    /// before this field was set, and the Room terminal service and wake registry hold it weakly,
    /// so dropping the runtime frees the kernel instead of leaving a cycle.
    terminal_wake_driver: Option<Arc<terminal_wake::TerminalWakeDriver>>,
    /// MP-08/MP-09/MP-11: terminals retain this callback weakly so it cannot
    /// own the runtime state that owns the terminal service.
    managed_terminal_activity: Option<Arc<dyn Fn() + Send + Sync>>,
    owned: KernelRuntimeOwnedState,
}

#[derive(Clone)]
struct KernelRuntimeOwnedState {
    app_control: crate::runtime::app_control::AppControlService,
    critical_approval_passkeys: critical_approval_passkey::CriticalApprovalPasskeys,
    passkey_prompts: Arc<passkey_prompts::PasskeyPromptBoard>,
    kernel_access: crate::runtime::kernel_access::AccessStore,
    sudo_turns: sudo::SudoStore,
    /// Window timer proof of life: entry -> (armed revision, alerted revision).
    sudo_timers: Arc<std::sync::Mutex<BTreeMap<String, (u64, u64)>>>,
    sudo_timer_changes: Arc<RuntimeChangeSignal>,
    sudo_end_wakes: Arc<std::sync::Mutex<BTreeMap<String, (crate::local::KernelSudoTurn, String)>>>,
    sudo_verified_at: Arc<std::sync::Mutex<BTreeMap<String, (std::time::Instant, u64)>>>,
    /// Worker side of A10 leased sudo windows, keyed by backing agent.
    leased_sudo_grants: Arc<std::sync::Mutex<BTreeMap<String, sudo::leased::WorkerSudoGrant>>>,
    /// Home side of A10: windows whose first worker fence was confirmed.
    leased_sudo_fenced: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
    config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
    session_store: SessionStateStore,
    agent_store: AgentServiceStore,
    attachment_store: AttachmentServiceStore,
    provider_store: ProviderProcessServiceStore,
    agent_moves: crate::agent_move::AgentMoveStore,
    pending_provider_launch_credentials: Arc<
        std::sync::Mutex<
            BTreeMap<String, crate::provider::ProviderCredentialEnvironment>,
        >,
    >,
    workflow_provider_launch_lock: Arc<std::sync::Mutex<()>>,
    workflow_instance_provision_lock: Arc<std::sync::Mutex<()>>,
    workflow_entry_claims: workflow_queue_durable::WorkflowEntryClaims,
    publication_activation: Arc<publication_activation::PublicationActivation>,
    provider_process_tracking: ProviderProcessTrackingStore,
    provider_launch_failure_retries: ProviderLaunchFailureRetryStore,
    external_provider_sessions: ExternalProviderSessionIndexStore,
    attached_provider_transcript_cursors: AttachedProviderTranscriptCursorStore,
    slice_store: crate::slice::SliceStore,
    kernel_browser_host: crate::runtime::kernel_browser_host::KernelBrowserHost,
    dictation: Arc<crate::dictation::DictationService>,
    leased_computer_changes: Arc<std::sync::Mutex<()>>,
    leased_computer_grants: Arc<
        std::sync::Mutex<BTreeMap<String, crate::transport::relay_peer::LeasedComputerGrant>>,
    >,
    app_grant_epochs: Arc<capability_grant_runtime::AppGrantEpochs>,
    browser_controller_processes:
        crate::runtime::browser_controller_process::BrowserControllerProcessStore,
    browser_import_admission: crate::runtime::browser_import_admission::BrowserImportAdmission,
    human_control_task_waits: Arc<std::sync::Mutex<BTreeMap<String, u64>>>,
    uncertain_leased_tasks: Arc<std::sync::Mutex<BTreeSet<(String, String, String)>>>,
    room_computer_revoked: Arc<std::sync::Mutex<BTreeSet<String>>>,
    room_display_leases: room_shared_display::RoomDisplayLeases,
    room_secret_observations: room_secret_observation::RoomSecretObservations,
    kernel_browser_secret_observations: room_secret_observation::RoomSecretObservations,
    room_terminals: crate::room_terminal::RoomTerminalService,
    room_execs: crate::room_exec::RoomExecService,
    environment_execution_gates: environment_execution_gate::EnvironmentExecutionGates,
    computer_input_executions:
        crate::runtime::computer_input_execution::ComputerInputExecutionStore,
    room_browser_health_inflight: Arc<std::sync::Mutex<BTreeSet<String>>>,
    next_room_browser_health_at_ms: Arc<AtomicU64>,
    room_environment_health_probes: Arc<room_environment_health::RoomEnvironmentHealthProbes>,
    browser_controller_generations:
        Arc<std::sync::Mutex<BTreeMap<String, browser_controller_runtime_state::ControllerGenerationObservation>>>,
    session_projection: crate::runtime::projection::SessionStateProjectionStore,
    agent_runtime_projection: crate::runtime::projection::AgentRuntimeProjectionStore,
    provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
    provider_process_projection: crate::runtime::projection::ProviderProcessProjectionStore,
    operational_history_store: OperationalHistoryStore,
    transcript_history_append_lock: Arc<std::sync::Mutex<()>>,
    durable_state_store: DurableKernelStateStore,
    managed_activity_transitions:
        managed_activity_persistence::ManagedActivityTransitionState,
    // Serializes local activity-bearing mutations with their durable observation. Callers must
    // release this boundary before async or provider I/O.
    managed_activity_mutation_lock: Arc<std::sync::Mutex<()>>,
    managed_kernel_quiescence: Option<Arc<ManagedKernelQuiescenceGate>>,
    managed_kernel_quiescence_client:
        Option<crate::runtime::managed_kernel_quiescence::ManagedKernelQuiescenceClient>,
    #[cfg(test)]
    managed_activity_before_record_pause: Arc<
        std::sync::Mutex<Option<managed_activity_runtime_state::ManagedActivityRecordPause>>,
    >,
    #[cfg(test)]
    managed_activity_next_lock_probe: Arc<
        std::sync::Mutex<Option<managed_activity_runtime_state::ManagedActivityLockProbe>>,
    >,
    #[cfg(test)]
    managed_activity_record_calls: Arc<std::sync::atomic::AtomicU64>,
    legacy_workflow_history: crate::app::LegacyWorkflowHistoryStore,
    provider_account_profiles: crate::account_profile::ProviderAccountProfileRegistry,
    provider_login_processes: ProviderLoginProcessStore,
    provider_auth_recovery_runs: Arc<std::sync::Mutex<BTreeSet<String>>>,
    claude_token_checks: Arc<claude_token_checks::ClaudeTokenChecks>,
    event_connection_registry: crate::event_connection::EventConnectionRegistry,
    prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
    active_turns: ActiveTurnStore,
    prompt_activity: PromptActivityStore,
    provider_output_deadlines: ProviderOutputDeadlineStore,
    prompt_workspace_claims: PromptWorkspaceClaimStore,
    structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
    pty_output_signal: crate::pty::PtyOutputSignal,
    terminal_stream: crate::terminal::TerminalStreamStore,
    runtime_projection_changes: Arc<RuntimeChangeSignal>,
    workflow_design_events: WorkflowDesignEventStore,
    workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    workspace_live_sync_coordinator: Arc<Mutex<crate::io::ArtifactEditCoordinator>>,
    workspace_live_sync_external_changes: crate::io::ArtifactExternalChangeMonitor,
    workspace_identity_monitor:
        crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor,
    pending_agent_context_handoffs: PendingAgentContextHandoffStore,
    pending_mcp_continuations: PendingMcpContinuationStore,
    private_assessment_results: agent_assessment_result::AssessmentResults,
    private_work_results: agent_assessment_result::AssessmentResults,
    /// The turn each native provider TUI run was last seen on, to settle it once it ends.
    native_tui_turns: std::sync::Arc<
        std::sync::Mutex<std::collections::BTreeMap<String, crate::session::PromptQueueItem>>,
    >,
    private_assessment_dispatch: agent_assessment_result::PrivateAssessmentDispatch,
    metaagent_events: crate::runtime::metaagent_event::MetaagentEventStore,
    metaagent_trace_subscriptions: crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore,
    connector_adapter_processes: crate::connector::ConnectorAdapterProcessPool,
    pending_provider_reloads: PendingProviderReloadStore,
    pending_interactions: PendingInteractionStore,
    handoff_claims: Arc<std::sync::Mutex<BTreeSet<String>>>,
    git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore,
    completed_git_turn_snapshots: crate::git_observer::CompletedGitTurnSnapshotStore,
    workspace_live_sync_journal: crate::git_observer::WorkspaceLiveSyncJournal,
    remote_workspace_live_sync_invocations:
        Arc<Mutex<BTreeMap<String, RemoteWorkspaceLiveSyncInvocationState>>>,
    remote_extension_invocations: Arc<Mutex<BTreeMap<String, RemoteExtensionInvocationState>>>,
    remote_extension_cancellations: Arc<Mutex<std::collections::BTreeSet<String>>>,
    remote_home_extension_inflight:
        Arc<Mutex<BTreeMap<String, Vec<RemoteHomeExtensionInflightInvocation>>>>,
    remote_extension_manifest_retry_counts: Arc<Mutex<BTreeMap<String, u32>>>,
    agent_message_idempotency: Arc<Mutex<AgentMessageIdempotencyStore>>,
    runtime_tool_call_activity: RuntimeToolCallActivity,
    next_provider_process_gc_at_ms: Arc<AtomicU64>,
    next_agent_lifecycle_sweep_ms: Arc<AtomicU64>,
    agent_wakes: Arc<agent_wake_scheduler::AgentWakeMonitor>,
    relay_state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    notification_inventory_projection: crate::runtime::projection::RemoteRelayInventoryProjectionStore,
    remote_prompt_projection_drains:
        remote_prompt_claim_runtime::RemotePromptProjectionDrainClaimStore,
    remote_prompt_projection_serial: Arc<remote_prompt_claim_runtime::RemotePromptProjectionSerialStore>,
    remote_prompt_recoveries: remote_prompt_claim_runtime::RemotePromptRecoveryClaimStore,
    remote_steer_receipt_reconciliations:
        Arc<std::sync::Mutex<BTreeSet<(String, String, String)>>>,
    slice_private_relay_connectors: Arc<Mutex<BTreeMap<String, SlicePrivateRelayConnector>>>,
    workflow_publication_runtimes:
        crate::runtime::state::workflow_publication_runtime_lifecycle::WorkflowPublicationRuntimeProcessStore,
    project_environment_placements: project_environment_placement::ProjectEnvironmentPlacements,
    project_environment_setups: project_environment_setup::ProjectEnvironmentSetupStore,
}

#[derive(Debug, Clone)]
struct RemoteHomeExtensionInflightInvocation {
    context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
    metadata: crate::extension::RemoteExtensionInvocationMetadata,
}

#[derive(Debug, Clone)]
struct RemoteExtensionInvocationState {
    invocation_id: String,
    result: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct RemoteWorkspaceLiveSyncInvocationState {
    request_fingerprint: String,
    result: Option<RemoteWorkspaceLiveSyncInvocationResult>,
    completion_tx: tokio::sync::watch::Sender<Option<RemoteWorkspaceLiveSyncInvocationResult>>,
    finalized: bool,
}

#[derive(Debug, Default)]
struct AgentMessageIdempotencyStore {
    entries: BTreeMap<String, AgentMessageIdempotencyEntry>,
    order: VecDeque<String>,
}

#[derive(Debug, Clone)]
struct AgentMessageIdempotencyEntry {
    fingerprint: String,
    result: crate::transport::runtime_tools::RuntimeToolResult,
}

impl AgentMessageIdempotencyStore {
    const LIMIT: usize = 1_024;

    fn record(
        &mut self,
        operation_id: String,
        fingerprint: String,
        result: crate::transport::runtime_tools::RuntimeToolResult,
    ) {
        if !self.entries.contains_key(&operation_id) {
            self.order.push_back(operation_id.clone());
        }
        self.entries.insert(
            operation_id,
            AgentMessageIdempotencyEntry {
                fingerprint,
                result,
            },
        );
        while self.entries.len() > Self::LIMIT {
            if let Some(expired) = self.order.pop_front() {
                self.entries.remove(&expired);
            }
        }
    }
}

type RemoteWorkspaceLiveSyncInvocationResult = (
    crate::transport::runtime_tools::RuntimeToolResult,
    Vec<crate::transport::relay_peer::RemoteWorkspaceLiveSyncArtifactState>,
);

struct SlicePrivateRelayConnector {
    relay_url: String,
    state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    task: std::thread::JoinHandle<()>,
}

mod agent_archive_runtime_state;
mod agent_config_owned_state;
mod agent_config_runtime_state;
mod agent_lifecycle_owned_state;
mod agent_profile_compaction;
mod agent_profile_owned_state;
mod agent_prompt_schedule_runtime_state;
mod agent_turn_actions_runtime_state;
mod agent_utility_runtime_state;
mod attachment_owned_state;
mod browser_controller_action_cancellation_runtime_state;
mod browser_controller_action_execution_runtime_state;
mod browser_controller_compatibility_runtime_state;
mod wake_driver;
pub(crate) use browser_controller_action_execution_runtime_state::BrowserControllerActionExecution;
mod browser_configuration_runtime_state;
mod browser_controller_runtime_state;
mod browser_download_cancellation_runtime_state;
mod browser_snapshot_observation_runtime_state;
mod browser_upload_runtime_state;
mod capability_owned_state;
mod detached_provider_run_owned_state;
mod owned;
mod pending_runtime_state;
mod remote_agent_profile_runtime;
mod remote_profile_account_runtime;
mod room_browser_health;
pub(in crate::runtime) use pending_runtime_state::PendingInteractionResolution;
use pending_runtime_state::*;
mod local_prompt_dispatch_runtime;
mod local_prompt_submission_owned_state;
mod managed_activity_persistence;
mod structured_prompt_dispatch;
pub(crate) use managed_activity_persistence::ManagedActivityObservation;
mod metaagent_event_owned_state;
mod metaagent_task_runtime_state;
pub(crate) use metaagent_task_runtime_state::parse_meta_slash_command;
mod active_prompt_steering;
mod external_command_authority;
mod project_runtime_state;
mod prompt;
mod prompt_activity_owned_state;
mod prompt_cancellation_owned_state;
mod prompt_dispatch;
mod room_pin;
mod session_activity_owned_state;
use external_command_authority::ExternalCommandAuthority;
mod prompt_git_observer_runtime;
mod prompt_queue_owned_state;
mod prompt_skill_context_state;
mod prompt_transcript_owned_state;
mod provider;
mod provider_account_admission;
mod provider_account_authority;
mod provider_account_copy;
mod provider_account_copy_offer;
mod provider_focus_owned_state;
mod provider_launch_failure_runtime;
mod provider_launch_owned_state;
mod provider_launch_runtime;
pub(crate) use provider_launch_runtime::ProviderLaunchStartOutcome;
mod claude_setup_token_capture;
mod provider_liveness_runtime;
pub(in crate::runtime) use claude_setup_token_capture::{
    SetupTokenScan, CLAUDE_SETUP_TOKEN_COLUMNS, CLAUDE_SETUP_TOKEN_ROWS,
};
mod claude_setup_token_vault;
pub(in crate::runtime) use claude_setup_token_vault::ClaudeSetupTokenStoreOutcome;
mod provider_login_state;
pub(in crate::runtime) use provider_login_state::{
    ClaudeSetupTokenLogin, ClaudeSetupTokenVaultPrompt, ProviderAuthProcessOperation,
    ProviderLoginProcessBackend, ProviderLoginProcessRecord, ProviderLoginProcessStore,
    PROVIDER_LOGIN_TIMEOUT_MS,
};
mod provider_history_runtime;
mod provider_mcp_continuation_runtime;
mod provider_mcp_task_continuation_runtime;
mod provider_output_pump_runtime;
mod provider_output_runtime;
mod provider_process_runtime_state;
pub(crate) use provider_process_runtime_state::*;
mod forwarded_peer_authority;
mod leased_profile_transition_runtime;
mod project_environment_setup;
pub(crate) use project_environment_setup::restore_agent_move_preparation_environment;
mod provider_auth_recovery;
#[cfg(test)]
mod provider_output_runtime_tests;
mod provider_prompt_failure_runtime;
mod provider_prompt_settlement_runtime;
mod provider_substitute_runtime;
mod relay_peer_runtime_state;
mod remote_native_provider_launch;
mod remote_prompt_claim_runtime;
mod remote_prompt_dispatch_execution_runtime;
mod remote_prompt_dispatch_runtime;
mod remote_prompt_dispatch_settlement_runtime;
mod remote_prompt_lifecycle_runtime;
mod remote_prompt_owned_state;
mod remote_prompt_projection_drain_runtime;
mod remote_prompt_receipt_reconciliation_runtime;
mod remote_prompt_worker_submission_runtime;
mod remote_provider_failure_runtime;
#[cfg(test)]
mod remote_queue_advance_tests;
mod restart_recovery_runtime;
pub(crate) use restart_recovery_runtime::is_internal_recovery_prompt_attachment;
mod agent_batch_runtime_state;
mod claude_token_checks;
mod claude_token_first_use;
mod provider_login_workflow;
mod runtime_interaction_owned_state;
pub(crate) use runtime_interaction_owned_state::interaction_waits;
mod runtime_interaction_state;
mod runtime_notice_owned_state;
mod runtime_state_views;
mod runtime_vault_unlock_state;
mod session;
mod session_collaboration_state;
mod session_lifecycle_runtime_state;
mod session_lookup_state;
mod session_process_control;
mod slice_development_runtime_state;
mod slice_project_source;
mod slice_runtime_state;
mod vault_owner_gate;
mod vault_unlock_request;
mod vault_unlock_requests;
pub(crate) use slice_runtime_state::SliceAgentRelaunchManifest;
mod kernel_browser_command_error;
mod kernel_browser_mirror;
mod kernel_browser_receipts;
mod kernel_browser_runtime;
mod kernel_browser_secret_runtime;
mod kernel_browser_state_wait;
mod owner_handoff_runtime;
pub(crate) use owner_handoff_runtime::{is_handoff_interaction_id, HANDOFF_OBLIGATION_KIND};
mod dictation_runtime;
mod kernel_computer_runtime;
mod leased_computer_runtime;
#[cfg(test)]
mod leased_computer_runtime_tests;
mod notes_runtime;
mod structured_provider_output_runtime;
mod terminal_runtime_state;
mod terminal_subscription_runtime;
mod tool_dispatch;
mod transport_runtime_state;
mod user_domain_access_runtime;

mod capability_grant_runtime;
mod workflow;
mod workflow_access_owned_state;
mod workflow_admin;
mod workflow_agent_owned_state;
mod workflow_artifact_request_runtime_state;
mod workflow_blocked_claim_retry;
mod workflow_claim_release;
mod workflow_code_request_runtime_state;
mod workflow_code_request_support;
mod workflow_completion_owned_state;
mod workflow_completion_snapshot_owned_state;
mod workflow_console_tool;
mod workflow_definition_owned_state;
mod workflow_definition_settings_owned_state;
mod workflow_dispatch;
mod workflow_endpoint_owned_state;
mod workflow_history_admission;
mod workflow_launch_owned_state;
mod workflow_node_owned_state;
mod workflow_output_tool;
mod workflow_prompt_dispatches;
mod workflow_prompt_queue_owned_state;
mod workflow_queue_durable;
mod workflow_source_request_runtime_state;
use workflow_prompt_dispatches::*;
pub(crate) mod notification_delivery;
mod room_workflow_control_runtime_state;
mod workflow_notification_grants;
mod workflow_notification_peers;
mod workflow_notification_router;
mod workflow_prompt_failure_owned_state;
pub(crate) mod workflow_publication_endpoint_runtime;
mod workflow_publication_owned_state;
pub(crate) mod workflow_publication_runtime_lifecycle;
mod workflow_query_owned_state;
mod workflow_registry_request_runtime_state;
mod workflow_request_runtime_state;
mod workflow_resume_owned_state;
mod workflow_run_request_runtime_state;
mod workflow_scheduling_owned_state;
mod workflow_tool;
mod workflow_turn_admin_owned_state;
mod workflow_turn_prompt_owned_state;

impl KernelRuntimeState {
    pub(crate) fn event_connection_registry(
        &self,
    ) -> &crate::event_connection::EventConnectionRegistry {
        &self.owned.event_connection_registry
    }

    #[allow(dead_code)]
    #[allow(
        clippy::too_many_arguments,
        reason = "Preserve the existing new_with_owned_state operation signature and explicit context arguments"
    )]
    pub(crate) fn new_with_owned_state(
        app: Arc<Mutex<DaemonApp>>,
        config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
        session_store: SessionStateStore,
        agent_store: AgentServiceStore,
        attachment_store: AttachmentServiceStore,
        provider_store: ProviderProcessServiceStore,
        provider_process_tracking: ProviderProcessTrackingStore,
        slice_store: crate::slice::SliceStore,
        session_projection: crate::runtime::projection::SessionStateProjectionStore,
        provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
        operational_history_store: OperationalHistoryStore,
        durable_state_store: DurableKernelStateStore,
        prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
        active_turns: ActiveTurnStore,
        prompt_activity: PromptActivityStore,
        prompt_workspace_claims: PromptWorkspaceClaimStore,
        structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workflow_design_events: WorkflowDesignEventStore,
        metaagent_events: MetaagentEventStore,
        workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
        let (
            external_provider_sessions,
            attached_provider_transcript_cursors,
            pty_output_signal,
            provider_account_profiles,
        ) = {
            let started = Instant::now();
            loop {
                if let Ok(app) = app.try_lock() {
                    break (
                        app.external_provider_session_index_store(),
                        app.attached_provider_transcript_cursor_store(),
                        app.pty_output_signal(),
                        app.provider_account_profile_registry(),
                    );
                }
                if started.elapsed() >= Duration::from_secs(5) {
                    panic!(
                        "KernelRuntimeState could not acquire the app lock for external provider session state during bootstrap"
                    );
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        Self::new_with_owned_state_and_lanes(
            app,
            ProviderRunOperationLanes::default(),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            external_provider_sessions,
            attached_provider_transcript_cursors,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            provider_account_profiles,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            pty_output_signal,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore::default(),
            workspace_coordinator,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Preserve the existing new_with_owned_state_and_lanes operation signature and explicit context arguments"
    )]
    pub(crate) fn new_with_owned_state_and_lanes(
        app: Arc<Mutex<DaemonApp>>,
        provider_runtime_lanes: ProviderRunOperationLanes,
        config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
        session_store: SessionStateStore,
        agent_store: AgentServiceStore,
        attachment_store: AttachmentServiceStore,
        provider_store: ProviderProcessServiceStore,
        provider_process_tracking: ProviderProcessTrackingStore,
        external_provider_sessions: ExternalProviderSessionIndexStore,
        attached_provider_transcript_cursors: AttachedProviderTranscriptCursorStore,
        slice_store: crate::slice::SliceStore,
        session_projection: crate::runtime::projection::SessionStateProjectionStore,
        provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
        operational_history_store: OperationalHistoryStore,
        durable_state_store: DurableKernelStateStore,
        provider_account_profiles: crate::account_profile::ProviderAccountProfileRegistry,
        prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
        active_turns: ActiveTurnStore,
        prompt_activity: PromptActivityStore,
        prompt_workspace_claims: PromptWorkspaceClaimStore,
        structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
        pty_output_signal: crate::pty::PtyOutputSignal,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workflow_design_events: WorkflowDesignEventStore,
        metaagent_events: MetaagentEventStore,
        metaagent_trace_subscriptions:
            crate::runtime::metaagent_trace::MetaagentTraceSubscriptionStore,
        workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
        let (
            completed_git_turn_snapshots,
            provider_process_projection,
            provider_launch_failure_retries,
            relay_state,
            notification_inventory_projection,
            legacy_workflow_history,
            agent_runtime_projection,
            app_control,
            managed_kernel_registration,
            runtime_tool_call_activity,
            kernel_browser_host,
            dictation,
            room_computer_revoked,
            sudo_windows,
            pending_agent_context_handoffs,
            room_terminals,
            agent_moves,
        ) = {
            let started = Instant::now();
            loop {
                if let Ok(app) = app.try_lock() {
                    break (
                        app.completed_git_turn_snapshot_store(),
                        app.provider_process_projection_store(),
                        app.provider_launch_failure_retry_store(),
                        app.relay_client_state(),
                        app.remote_relay_inventory_projection_store(),
                        app.legacy_workflow_history_store(),
                        app.agent_runtime_projection_store(),
                        app.app_control_service(),
                        app.managed_kernel_registration(),
                        app.runtime_tool_call_activity.clone(),
                        app.kernel_browser_host.clone(),
                        app.dictation.clone(),
                        app.room_computer_revoked.clone(),
                        app.sudo_window_projection(),
                        app.pending_agent_context_handoff_store(),
                        app.room_terminals(),
                        app.agent_moves.clone(),
                    );
                }
                if started.elapsed() >= Duration::from_secs(5) {
                    panic!("KernelRuntimeState could not acquire the app lock during bootstrap");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        let event_config = config_projection.snapshot();
        durable_state_store
            .outside_mutate(
                crate::durable_state::agent_lifecycle::outside::Mutation::Identity {
                    kernel: event_config.daemon_id.clone(),
                    machine: event_config.host_machine_id.clone(),
                },
            )
            .expect("outside event identity must be durable before runtime admission");
        durable_state_store
            .outside_mutate(
                crate::durable_state::agent_lifecycle::outside::Mutation::Restart {
                    keep: kernel_access::grant_inboxes_to_keep(&durable_state_store)
                        .expect("access grants must be readable before runtime admission"),
                },
            )
            .expect("outside subscriptions must end when their home kernel restarts");
        agent_store
            .write()
            .bind_outside_events(durable_state_store.clone());
        room_terminals.bind_outside_events(durable_state_store.clone());
        let workspace_live_sync_journal =
            match crate::git_observer::WorkspaceLiveSyncJournal::restore_from_durable_state(
                &durable_state_store,
            ) {
                Ok(journal) => journal,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.workspace_live_sync",
                        "failed to restore workspace live sync journal",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                    crate::git_observer::WorkspaceLiveSyncJournal::default()
                }
            };
        let publication_activation = Arc::new(publication_activation::PublicationActivation::new(
            config_projection
                .snapshot()
                .publication_control_state_root
                .is_some(),
        ));
        let project_environment_setups =
            project_environment_setup::ProjectEnvironmentSetupStore::restore_from_durable_state(
                &durable_state_store,
            );
        let config = config_projection.snapshot();
        let managed_activity_kernel_id = (managed_kernel_registration.is_some()
            || (config.kernel_runtime_role == crate::config::KernelRuntimeRole::RemoteLeaseWorker
                && std::env::var_os(crate::managed_bootstrap::worker::ACTIVITY_RECEIPT_ENV)
                    .filter(|value| !value.is_empty())
                    .map(std::path::PathBuf::from)
                    .is_some_and(|path| path.exists())))
        .then(|| config.daemon_id.clone());
        let managed_activity_transitions =
            managed_activity_persistence::ManagedActivityTransitionState::new(
                durable_state_store.clone(),
                managed_activity_kernel_id.clone(),
            );
        let managed_activity_mutation_lock = Arc::new(std::sync::Mutex::new(()));
        let managed_kernel_quiescence = managed_kernel_registration.as_ref().map(|_| {
            let kernel_id = config.daemon_id.clone();
            match ManagedKernelQuiescenceGate::restore(
                durable_state_store.clone(),
                kernel_id.clone(),
                Arc::clone(&managed_activity_mutation_lock),
                ) {
                Ok(gate) => gate,
                Err(error) => {
                    crate::logging::error_with_fields(
                        "managed_kernel.auto_stop_quiescence",
                        "durable provider admission fence could not be restored; admissions are closed",
                        serde_json::json!({
                            "kernel_id": kernel_id,
                            "error": error.to_string(),
                        }),
                    );
                    ManagedKernelQuiescenceGate::unavailable(
                        durable_state_store.clone(),
                        kernel_id,
                        Arc::clone(&managed_activity_mutation_lock),
                        error.to_string(),
                    )
                }
            }
        });
        let managed_kernel_quiescence_client =
            match crate::runtime::managed_kernel_quiescence::ManagedKernelQuiescenceClient::from_runtime(
                &config,
                managed_kernel_registration.as_ref(),
            ) {
                Ok(client) => client,
                Err(error) => {
                    crate::logging::error_with_fields(
                        "managed_kernel.auto_stop_quiescence",
                        "managed kernel quiescence transport is unavailable",
                        serde_json::json!({
                            "kernel_id": config.daemon_id,
                            "error": error.to_string(),
                        }),
                    );
                    None
                }
            };
        provider_store.set_managed_kernel_admission_gate(managed_kernel_quiescence.clone());
        let room_secret_observations = room_secret_observation::RoomSecretObservations::new(
            config
                .private_runtime_state_root()
                .join("room-observation-quarantine"),
            session_store
                .list_all_sessions()
                .into_iter()
                .filter(|session| {
                    config.room_environment_worker_binding.is_some()
                        || session_store
                            .room_environment_snapshot(session.id())
                            .is_ok()
                })
                .map(|session| {
                    config
                        .room_environment_worker_binding
                        .as_ref()
                        .map(|binding| binding.session_id.clone())
                        .unwrap_or_else(|| session.id().to_string())
                })
                .collect(),
        )
        .with_identity(&config.relay_private_key)
        .with_worker_room(
            config
                .room_environment_worker_binding
                .as_ref()
                .map(|binding| binding.session_id.clone()),
        );
        // MP-08 / MP-11 A09: capture only newly admitted, sanitized public rows.
        let projection_protection = room_secret_observations.clone();
        operational_history_store.set_public_history_projector(Arc::new(move |event| {
            projection_protection
                .public_history_document(event, event.public_history_owner_user_id.as_deref()?)
        }));
        let room_secret_observations =
            room_secret_observations.with_public_history(operational_history_store.clone());
        let private_assessment_dispatch = {
            let started = Instant::now();
            loop {
                if let Ok(mut app) = app.try_lock() {
                    break app
                        .private_assessment_dispatch
                        .get_or_insert_with(|| {
                            PrivateAssessmentDispatch::new(
                                config_projection.clone(),
                                durable_state_store.clone(),
                                room_secret_observations.clone(),
                            )
                        })
                        .clone();
                }
                if started.elapsed() >= Duration::from_secs(5) {
                    panic!("KernelRuntimeState could not bind private assessment dispatch");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        let mut runtime = Self {
            external_command_authority: None,
            room_provider_origin: None,
            room_request_origin: None,
            terminal_caller: false,
            relay_peer_authority: None,
            forwarded_peer_binding: None,
            #[cfg(test)]
            app_lock_wait_probe: None,
            #[cfg(test)]
            queue_promotion_exit_probe: None,
            app,
            provider_runtime_lanes,
            provider_history_appends: Default::default(),
            leased_agent_operations: leased_agent_operations::LeasedAgentOperations::default(),
            detached_workflow_provider_launches: Arc::new(std::sync::Mutex::new(BTreeSet::new())),
            restart_recovery_dispatched: Arc::new(std::sync::Mutex::new(BTreeSet::new())),
            claude_session_grants: claude_permission_rules::StandingGrants::default(),
            terminal_wake_driver: None,
            managed_terminal_activity: None,
            owned: KernelRuntimeOwnedState {
                app_control,
                critical_approval_passkeys:
                    critical_approval_passkey::CriticalApprovalPasskeys::new(
                        &config_projection.snapshot().user_config.credential_vault,
                    ),
                passkey_prompts: Arc::default(),
                kernel_access: Default::default(),
                sudo_turns: sudo_windows.store(),
                sudo_timers: Default::default(),
                sudo_timer_changes: Default::default(),
                sudo_verified_at: Default::default(),
                sudo_end_wakes: Default::default(),
                leased_sudo_grants: Default::default(),
                leased_sudo_fenced: Default::default(),
                config_projection,
                session_store,
                agent_store,
                attachment_store,
                provider_store,
                agent_moves,
                pending_provider_launch_credentials: Arc::new(std::sync::Mutex::new(
                    BTreeMap::new(),
                )),
                workflow_provider_launch_lock: Arc::new(std::sync::Mutex::new(())),
                workflow_instance_provision_lock: Arc::new(std::sync::Mutex::new(())),
                workflow_entry_claims: workflow_queue_durable::WorkflowEntryClaims::default(),
                publication_activation,
                provider_process_tracking,
                provider_launch_failure_retries,
                external_provider_sessions,
                attached_provider_transcript_cursors,
                slice_store,
                kernel_browser_host,
                dictation,
                leased_computer_changes: Default::default(),
                leased_computer_grants: Default::default(),
                app_grant_epochs: Arc::default(),
                browser_controller_processes:
                    crate::runtime::browser_controller_process::BrowserControllerProcessStore::from_environment(),
                browser_import_admission:
                    crate::runtime::browser_import_admission::BrowserImportAdmission::default(),
                environment_execution_gates: Default::default(),
                human_control_task_waits: Default::default(),
                uncertain_leased_tasks: Default::default(),
                room_computer_revoked,
                room_secret_observations,
                room_display_leases: Default::default(),
                kernel_browser_secret_observations: room_secret_observation::RoomSecretObservations::new(
                    config.private_runtime_state_root().join("kernel-browser/observations"),
                    BTreeSet::new(),
                ).with_identity(&config.relay_private_key),
                room_terminals,
                room_execs: Default::default(),
                computer_input_executions:
                    crate::runtime::computer_input_execution::ComputerInputExecutionStore::default(),
                room_browser_health_inflight: Arc::new(std::sync::Mutex::new(BTreeSet::new())),
                next_room_browser_health_at_ms: Arc::new(AtomicU64::new(0)),
                room_environment_health_probes: Arc::new(room_environment_health::RoomEnvironmentHealthProbes::default()),
                browser_controller_generations: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
                session_projection,
                agent_runtime_projection,
                provider_run_projection,
                provider_process_projection,
                operational_history_store,
                transcript_history_append_lock: Arc::new(std::sync::Mutex::new(())),
                event_connection_registry:
                    crate::event_connection::EventConnectionRegistry::new(
                        durable_state_store.clone(),
                    ),
                durable_state_store,
                managed_activity_transitions,
                managed_activity_mutation_lock,
                managed_kernel_quiescence,
                managed_kernel_quiescence_client,
                #[cfg(test)]
                managed_activity_before_record_pause: Arc::new(std::sync::Mutex::new(None)),
                #[cfg(test)]
                managed_activity_next_lock_probe: Arc::new(std::sync::Mutex::new(None)),
                #[cfg(test)]
                managed_activity_record_calls: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                legacy_workflow_history,
                provider_account_profiles,
                provider_login_processes: ProviderLoginProcessStore::default(),
                provider_auth_recovery_runs: Arc::new(std::sync::Mutex::new(BTreeSet::new())),
                claude_token_checks: Arc::default(),
                prompt_state_owner,
                active_turns,
                prompt_activity,
                provider_output_deadlines: ProviderOutputDeadlineStore::default(),
                prompt_workspace_claims,
                structured_output_records,
                pty_output_signal,
                terminal_stream,
                runtime_projection_changes: Arc::new(RuntimeChangeSignal::default()),
                workflow_design_events,
                workspace_coordinator,
                workspace_live_sync_coordinator: Arc::new(Mutex::new(
                    crate::io::ArtifactEditCoordinator::new(),
                )),
                workspace_live_sync_external_changes:
                    crate::io::ArtifactExternalChangeMonitor::default(),
                workspace_identity_monitor:
                    crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor::default(),
                pending_agent_context_handoffs,
                pending_mcp_continuations: PendingMcpContinuationStore::shared(),
                private_assessment_results: Default::default(),
                private_work_results: private_assessment_dispatch.work_results.clone(),
                private_assessment_dispatch,
                native_tui_turns: Default::default(),
                metaagent_events,
                metaagent_trace_subscriptions,
                connector_adapter_processes: crate::connector::ConnectorAdapterProcessPool::default(
                ),
                pending_provider_reloads: PendingProviderReloadStore::default(),
                pending_interactions: PendingInteractionStore::shared(),
                handoff_claims: Arc::default(),
                git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore::default(),
                completed_git_turn_snapshots,
                workspace_live_sync_journal,
                remote_workspace_live_sync_invocations: Arc::new(Mutex::new(BTreeMap::new())),
                remote_extension_invocations: Arc::new(Mutex::new(BTreeMap::new())),
                remote_extension_cancellations: Arc::new(Mutex::new(
                    std::collections::BTreeSet::new(),
                )),
                remote_home_extension_inflight: Arc::new(Mutex::new(BTreeMap::new())),
                remote_extension_manifest_retry_counts: Arc::new(Mutex::new(BTreeMap::new())),
                agent_message_idempotency: Arc::new(Mutex::new(
                    AgentMessageIdempotencyStore::default(),
                )),
                runtime_tool_call_activity,
                next_provider_process_gc_at_ms: Arc::new(AtomicU64::new(0)),
                next_agent_lifecycle_sweep_ms: Arc::new(AtomicU64::new(0)),
                agent_wakes: Arc::new(agent_wake_scheduler::AgentWakeMonitor::new()),
                relay_state,
                notification_inventory_projection,
                remote_prompt_projection_drains: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
                remote_prompt_projection_serial: Arc::default(),
                remote_prompt_recoveries: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
                remote_steer_receipt_reconciliations: Arc::new(std::sync::Mutex::new(
                    BTreeSet::new(),
                )),
                slice_private_relay_connectors: Arc::new(Mutex::new(BTreeMap::new())),
                workflow_publication_runtimes:
                    crate::runtime::state::workflow_publication_runtime_lifecycle::WorkflowPublicationRuntimeProcessStore::default(),
                project_environment_placements: Default::default(),
                project_environment_setups,
            },
        };
        runtime.link_room_terminals_to_managed_kernel();
        runtime.register_terminal_wake_driver();
        runtime.owned.record_managed_activity_transition();
        runtime.recover_sudo_notices();
        // Access grants are process-bound and never survive a kernel restart.
        runtime.retire_notification_grants(None);
        runtime.recover_app_grant_expiries();
        runtime
    }

    #[cfg(test)]
    pub(crate) fn pending_provider_reload_for_test(&self, agent_id: &str) -> bool {
        self.owned
            .pending_provider_reloads
            .write()
            .contains_key(agent_id)
    }

    #[cfg(test)]
    pub(crate) fn observe_app_lock_wait_for_test(&mut self, probe: Arc<tokio::sync::Notify>) {
        self.app_lock_wait_probe = Some(probe);
    }

    pub(crate) async fn with_app_side_effect<R>(
        &self,
        operation: impl FnOnce(&mut DaemonApp) -> R,
    ) -> R {
        #[cfg(test)]
        if let Some(probe) = &self.app_lock_wait_probe {
            probe.notify_one();
        }
        let (result, dispatches) = {
            let mut app =
                crate::runtime::app_lock::lock_app_instrumented(&self.app, "kernel_runtime_state")
                    .await;
            let result = operation(&mut app);
            let dispatches = app.take_deferred_workflow_remote_prompt_dispatches();
            (result, dispatches)
        };
        self.spawn_deferred_workflow_remote_prompt_dispatches(dispatches);
        result
    }

    pub(crate) async fn with_app_side_effect_blocking<R, F>(
        &self,
        operation: F,
    ) -> Result<R, DaemonError>
    where
        F: FnOnce(&mut DaemonApp) -> Result<R, DaemonError> + Send + 'static,
        R: Send + 'static,
    {
        let app = Arc::clone(&self.app);
        #[cfg(test)]
        let probe = self.app_lock_wait_probe.clone();
        let (result, dispatches) = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            if let Some(probe) = probe {
                probe.notify_one();
            }
            let mut app = app.blocking_lock();
            let result = operation(&mut app);
            let dispatches = app.take_deferred_workflow_remote_prompt_dispatches();
            (result, dispatches)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "run blocking kernel app side effect",
            message: error.to_string(),
        })?;
        self.spawn_deferred_workflow_remote_prompt_dispatches(dispatches);
        result
    }

    pub(crate) fn provider_account_profile_registry(
        &self,
    ) -> &crate::account_profile::ProviderAccountProfileRegistry {
        &self.owned.provider_account_profiles
    }

    pub(crate) fn provider_account_authority_owner_user_id(
        &self,
        runtime_owner_user_id: &str,
    ) -> String {
        crate::account_profile::provider_account_authority_owner_user_id(
            &self.owned.config_projection.snapshot(),
            runtime_owner_user_id,
        )
    }

    pub(in crate::runtime) fn provider_login_kernel_id(&self) -> String {
        self.owned.config_projection.snapshot().daemon_id
    }

    pub(in crate::runtime) fn provider_login_process_store(&self) -> &ProviderLoginProcessStore {
        &self.owned.provider_login_processes
    }

    fn try_with_app_side_effect<R>(
        &self,
        operation: impl FnOnce(&mut DaemonApp) -> R,
    ) -> Option<R> {
        let can_spawn_dispatches = tokio::runtime::Handle::try_current().is_ok();
        let (result, dispatches) = {
            let mut app = self.app.try_lock().ok()?;
            let result = operation(&mut app);
            let dispatches = if can_spawn_dispatches {
                app.take_deferred_workflow_remote_prompt_dispatches()
            } else {
                Vec::new()
            };
            (result, dispatches)
        };
        self.spawn_deferred_workflow_remote_prompt_dispatches(dispatches);
        Some(result)
    }

    fn spawn_deferred_workflow_remote_prompt_dispatches(
        &self,
        dispatches: Vec<crate::app::KernelRemotePromptDispatch>,
    ) {
        for dispatch in dispatches {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
    }

    async fn append_agent_durable_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        capability_name: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.record_agent_durable_event(kind, agent, capability_name)
    }

    fn record_agent_durable_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        capability_name: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            kind,
            Some(agent.id().to_string()),
            serde_json::json!({
                "agent": agent,
                "capability_name": capability_name,
            }),
        )?;
        Ok(())
    }

    async fn append_session_durable_event(
        &self,
        kind: &'static str,
        session: &crate::session::RuntimeSession,
        reason: &'static str,
    ) -> Result<(), DaemonError> {
        if kind == "session.deleted" {
            self.owned
                .durable_state_store
                .persist_session_deleted(session, reason)?;
            return Ok(());
        }
        if kind == "session.updated" && reason == "workflow" {
            self.owned
                .persist_workflow_runtime_session(session.id(), reason)?;
            return Ok(());
        }
        let session = session.durable_runtime_snapshot();
        self.owned.durable_state_store.append_event(
            kind,
            Some(session.id().to_string()),
            serde_json::json!({
                "session": &session,
                "reason": reason,
            }),
        )?;
        Ok(())
    }

    pub(crate) async fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        self.owned
            .capability_context(session_id, attachment_id, capability)
    }
}

#[derive(Debug, Default)]
pub(crate) struct RuntimeChangeSignal {
    sequence: AtomicU64,
    notify: Notify,
}

impl RuntimeChangeSignal {
    pub(crate) fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }

    pub(crate) fn record_change(&self) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    pub(crate) async fn wait_for_change_after(&self, sequence: u64) {
        if self.sequence() != sequence {
            return;
        }
        let notified = self.notify.notified();
        if self.sequence() != sequence {
            return;
        }
        notified.await;
    }
}

pub(crate) struct CapabilityRuntimeSnapshot {
    pub(crate) workspace_id: String,
    pub(crate) worktree_root: std::path::PathBuf,
    pub(crate) workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    pub(crate) operational_history_store: crate::history::OperationalHistoryStore,
    pub(crate) operational_artifact_root: std::path::PathBuf,
    pub(crate) operational_artifact_index_path: std::path::PathBuf,
    pub(crate) history_archive_enabled: bool,
}

#[cfg(test)]
mod runtime_change_signal_tests {
    use super::RuntimeChangeSignal;
    use std::time::Duration;

    #[tokio::test]
    async fn runtime_change_signal_wakes_waiters() {
        let signal = std::sync::Arc::new(RuntimeChangeSignal::default());
        let sequence = signal.sequence();
        let waiter = {
            let signal = std::sync::Arc::clone(&signal);
            tokio::spawn(async move {
                signal.wait_for_change_after(sequence).await;
            })
        };

        signal.record_change();

        tokio::time::timeout(Duration::from_millis(100), waiter)
            .await
            .expect("change waiter should wake")
            .expect("wait task should complete");
    }

    #[tokio::test]
    async fn runtime_change_signal_returns_when_sequence_already_changed() {
        let signal = RuntimeChangeSignal::default();
        let sequence = signal.sequence();
        signal.record_change();

        tokio::time::timeout(
            Duration::from_millis(100),
            signal.wait_for_change_after(sequence),
        )
        .await
        .expect("changed sequence should not wait");
    }
}

#[cfg(test)]
mod workspace_live_sync_external_change_notice_tests;

// Exercise the actual post-lock worker submission seam from caller contract tests.
#[cfg(test)]
impl KernelRuntimeState {
    pub(crate) async fn submit_remote_prompt_to_worker_for_test(
        &self,
        dispatch: &mut crate::app::KernelRemotePromptDispatch,
    ) -> Result<String, DaemonError> {
        self.owned.mark_active_prompt_delivery(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            None,
            None,
        )?;
        remote_prompt_worker_submission_runtime::submit_remote_prompt_to_worker_with_binding_refresh(
            self, dispatch, dispatch.prompt.clone(), Vec::new(),
        ).await
    }
}

mod room_agent_admission;

mod agent_assessment_result;
mod agent_delegation_runtime;
pub(crate) use agent_assessment_result::PrivateAssessmentDispatch;
mod agent_deadline_runtime;
mod agent_goal_context;
mod agent_goal_migration;
mod agent_goal_native_approval;
mod agent_goal_runtime;
mod agent_inbox_delivery;
mod agent_inbox_leased_steer;
mod agent_leased_process_home;
mod agent_leased_process_receipts;
mod agent_leased_process_worker;
mod agent_lifecycle_runtime;
mod agent_process_group;
mod agent_process_output;
mod agent_process_watch;
#[cfg(test)]
mod agent_reply_tests;
mod agent_task_cancellation;
#[cfg(test)]
mod agent_task_cancellation_sweep_tests;
mod agent_task_failure;
mod agent_task_human_control_wait;
mod agent_task_owner_interactions;
mod agent_task_owner_resolution;
mod agent_task_projection;
#[cfg(test)]
mod agent_turn_task_tests;
mod agent_wake_cancellation;
mod agent_wake_scheduler;
mod agent_workflow_tasks;
mod leased_task_connectivity;
mod room_dispatch_obligation;
mod terminal_wake;
mod terminal_wake_worker;
// MD-3: typed internal seam for display integration; public protocol remains coordinator-owned.
pub(crate) use kernel_browser_runtime::{KernelBrowserDisplayRequest, KernelBrowserPushCredit};
