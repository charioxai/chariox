//! Shared runtime-state facade and async orchestration wiring.
//!
//! Domain modules own the concrete session/provider/prompt/workflow and workspace live sync mutations.
//! This root keeps the public `KernelRuntimeState` entry points, shared fields, and cross-domain
//! plumbing that would otherwise create cycles between those modules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use tokio::sync::Mutex;

use crate::agent::AgentServiceStore;
use crate::app::{
    ActiveTurnStore, DaemonApp, PromptActivityStore, PromptWorkspaceClaimStore,
    ProviderProcessTrackingStore, WorkflowDesignEventStore,
};
use crate::attachment::AttachmentServiceStore;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::history::{OperationalHistoryStore, SessionHistoryEntry, SessionHistoryStore};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::{ProviderProcessServiceStore, ProviderRunOperationLanes};
use crate::session::{SessionStateOwner, SessionStateStore};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

mod workspace_live_sync;
use workspace_live_sync::*;
mod workspace_live_sync_workspace_context;
use workspace_live_sync_workspace_context::*;
mod context_handoff;
use context_handoff::*;
mod config_runtime_state;
mod provider_reload;
pub(crate) use provider_reload::*;
mod provider_relaunch_runtime;
mod provider_reload_pending_runtime;
mod provider_run_read_state;

#[derive(Clone)]
pub(crate) struct KernelRuntimeState {
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
    owned: KernelRuntimeOwnedState,
}

#[derive(Clone)]
struct KernelRuntimeOwnedState {
    config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
    session_store: SessionStateStore,
    agent_store: AgentServiceStore,
    attachment_store: AttachmentServiceStore,
    provider_store: ProviderProcessServiceStore,
    provider_process_tracking: ProviderProcessTrackingStore,
    slice_store: crate::slice::SliceStore,
    session_projection: crate::runtime::projection::SessionStateProjectionStore,
    provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
    history_store: SessionHistoryStore,
    operational_history_store: OperationalHistoryStore,
    durable_state_store: DurableKernelStateStore,
    history_projection: crate::runtime::projection::SessionHistoryProjectionStore,
    prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
    active_turns: ActiveTurnStore,
    prompt_activity: PromptActivityStore,
    prompt_workspace_claims: PromptWorkspaceClaimStore,
    structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
    terminal_stream: crate::terminal::TerminalStreamStore,
    workflow_design_events: WorkflowDesignEventStore,
    workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    workspace_live_sync_coordinator: Arc<Mutex<crate::io::ArtifactEditCoordinator>>,
    workspace_live_sync_external_changes: crate::io::ArtifactExternalChangeMonitor,
    workspace_identity_monitor:
        crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor,
    pending_agent_context_handoffs: PendingAgentContextHandoffStore,
    pending_mcp_continuations: PendingMcpContinuationStore,
    metaagent_events: crate::runtime::metaagent_event::MetaagentEventStore,
    connector_adapter_processes: crate::connector::ConnectorAdapterProcessPool,
    pending_provider_reloads: PendingProviderReloadStore,
    pending_interactions: PendingInteractionStore,
    git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore,
    workspace_live_sync_journal: crate::git_observer::WorkspaceLiveSyncJournal,
    remote_extension_invocations: Arc<Mutex<BTreeMap<String, RemoteExtensionInvocationState>>>,
    remote_extension_cancellations: Arc<Mutex<std::collections::BTreeSet<String>>>,
    remote_home_extension_inflight:
        Arc<Mutex<BTreeMap<String, Vec<RemoteHomeExtensionInflightInvocation>>>>,
    remote_extension_manifest_retry_counts: Arc<Mutex<BTreeMap<String, u32>>>,
    slice_private_relay_connectors: Arc<Mutex<BTreeMap<String, SlicePrivateRelayConnector>>>,
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

struct SlicePrivateRelayConnector {
    relay_url: String,
    state: Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    task: std::thread::JoinHandle<()>,
}

mod agent_config_owned_state;
mod agent_config_runtime_state;
mod agent_lifecycle_owned_state;
mod agent_profile_owned_state;
mod agent_utility_runtime_state;
mod attachment_owned_state;
mod capability_owned_state;
mod owned;
mod pending_runtime_state;
use pending_runtime_state::*;
mod local_prompt_dispatch_runtime;
mod local_prompt_submission_owned_state;
mod prompt;
mod prompt_activity_owned_state;
mod prompt_cancellation_owned_state;
mod prompt_dispatch;
mod prompt_git_observer_runtime;
mod prompt_queue_owned_state;
mod prompt_skill_context_state;
mod prompt_transcript_owned_state;
mod provider;
mod provider_focus_owned_state;
mod provider_launch_failure_runtime;
mod provider_launch_owned_state;
mod provider_launch_runtime;
mod provider_liveness_runtime;
mod provider_mcp_continuation_runtime;
mod provider_output_runtime;
mod provider_process_runtime_state;
pub(crate) use provider_process_runtime_state::*;
#[cfg(test)]
mod provider_output_runtime_tests;
mod provider_prompt_failure_runtime;
mod provider_prompt_settlement_runtime;
mod provider_substitute_runtime;
mod relay_peer_runtime_state;
mod remote_prompt_dispatch_runtime;
mod remote_prompt_lifecycle_runtime;
mod remote_prompt_owned_state;
mod remote_prompt_worker_submission_runtime;
mod runtime_interaction_owned_state;
mod runtime_interaction_state;
mod runtime_notice_owned_state;
mod runtime_state_views;
mod session;
mod session_collaboration_state;
mod session_lifecycle_runtime_state;
mod session_lookup_state;
mod slice_runtime_state;
mod structured_provider_output_runtime;
mod terminal_runtime_state;
mod tool_dispatch;
mod transport_runtime_state;
mod workflow;
mod workflow_access_owned_state;
mod workflow_admin;
mod workflow_blocked_claim_retry;
mod workflow_completion_owned_state;
mod workflow_completion_snapshot_owned_state;
mod workflow_console_tool;
mod workflow_definition_owned_state;
mod workflow_definition_settings_owned_state;
mod workflow_dispatch;
mod workflow_endpoint_owned_state;
mod workflow_launch_owned_state;
mod workflow_node_owned_state;
mod workflow_output_tool;
mod workflow_prompt_dispatches;
mod workflow_prompt_queue_owned_state;
use workflow_prompt_dispatches::*;
mod workflow_prompt_failure_owned_state;
pub(crate) mod workflow_publication_endpoint_runtime;
mod workflow_publication_owned_state;
mod workflow_query_owned_state;
mod workflow_request_runtime_state;
mod workflow_resume_owned_state;
mod workflow_run_request_runtime_state;
mod workflow_scheduling_owned_state;
mod workflow_tool;
mod workflow_turn_admin_owned_state;
mod workflow_turn_prompt_owned_state;

impl KernelRuntimeState {
    #[allow(dead_code)]
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
        history_store: SessionHistoryStore,
        operational_history_store: OperationalHistoryStore,
        durable_state_store: DurableKernelStateStore,
        history_projection: crate::runtime::projection::SessionHistoryProjectionStore,
        prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
        active_turns: ActiveTurnStore,
        prompt_activity: PromptActivityStore,
        prompt_workspace_claims: PromptWorkspaceClaimStore,
        structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workflow_design_events: WorkflowDesignEventStore,
        workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
        Self::new_with_owned_state_and_lanes(
            app,
            ProviderRunOperationLanes::default(),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            history_projection,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            workspace_coordinator,
        )
    }

    pub(crate) fn new_with_owned_state_and_lanes(
        app: Arc<Mutex<DaemonApp>>,
        provider_runtime_lanes: ProviderRunOperationLanes,
        config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
        session_store: SessionStateStore,
        agent_store: AgentServiceStore,
        attachment_store: AttachmentServiceStore,
        provider_store: ProviderProcessServiceStore,
        provider_process_tracking: ProviderProcessTrackingStore,
        slice_store: crate::slice::SliceStore,
        session_projection: crate::runtime::projection::SessionStateProjectionStore,
        provider_run_projection: crate::runtime::projection::ProviderRunProjectionStore,
        history_store: SessionHistoryStore,
        operational_history_store: OperationalHistoryStore,
        durable_state_store: DurableKernelStateStore,
        history_projection: crate::runtime::projection::SessionHistoryProjectionStore,
        prompt_state_owner: crate::runtime::prompt_state::PromptStateOwner,
        active_turns: ActiveTurnStore,
        prompt_activity: PromptActivityStore,
        prompt_workspace_claims: PromptWorkspaceClaimStore,
        structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workflow_design_events: WorkflowDesignEventStore,
        workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
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
        Self {
            app,
            provider_runtime_lanes,
            owned: KernelRuntimeOwnedState {
                config_projection,
                session_store,
                agent_store,
                attachment_store,
                provider_store,
                provider_process_tracking,
                slice_store,
                session_projection,
                provider_run_projection,
                history_store,
                operational_history_store,
                durable_state_store,
                history_projection,
                prompt_state_owner,
                active_turns,
                prompt_activity,
                prompt_workspace_claims,
                structured_output_records,
                terminal_stream,
                workflow_design_events,
                workspace_coordinator,
                workspace_live_sync_coordinator: Arc::new(Mutex::new(
                    crate::io::ArtifactEditCoordinator::new(),
                )),
                workspace_live_sync_external_changes:
                    crate::io::ArtifactExternalChangeMonitor::default(),
                workspace_identity_monitor:
                    crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor::default(),
                pending_agent_context_handoffs: PendingAgentContextHandoffStore::default(),
                pending_mcp_continuations: PendingMcpContinuationStore::shared(),
                metaagent_events: crate::runtime::metaagent_event::MetaagentEventStore::default(),
                connector_adapter_processes: crate::connector::ConnectorAdapterProcessPool::default(
                ),
                pending_provider_reloads: PendingProviderReloadStore::default(),
                pending_interactions: PendingInteractionStore::shared(),
                git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore::default(),
                workspace_live_sync_journal,
                remote_extension_invocations: Arc::new(Mutex::new(BTreeMap::new())),
                remote_extension_cancellations: Arc::new(Mutex::new(
                    std::collections::BTreeSet::new(),
                )),
                remote_home_extension_inflight: Arc::new(Mutex::new(BTreeMap::new())),
                remote_extension_manifest_retry_counts: Arc::new(Mutex::new(BTreeMap::new())),
                slice_private_relay_connectors: Arc::new(Mutex::new(BTreeMap::new())),
            },
        }
    }

    async fn with_app_side_effect<R>(&self, operation: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.app.lock().await;
        operation(&mut app)
    }

    async fn append_agent_durable_event(
        &self,
        kind: &'static str,
        agent: &crate::agent::AgentInstance,
        capability_name: Option<&str>,
    ) -> Result<(), DaemonError> {
        let agent = agent.clone();
        let capability_name = capability_name.map(str::to_string);
        self.owned.durable_state_store.append_event(
            kind,
            Some(agent.id().to_string()),
            serde_json::json!({
                "agent": &agent,
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
        let session = session.clone();
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
mod workspace_live_sync_external_change_notice_tests;
