//! Shared runtime-state facade and async orchestration wiring.
//!
//! Domain modules own the concrete session/provider/prompt/workflow and managed-I/O mutations.
//! This root keeps the public `KernelRuntimeState` entry points, shared fields, and cross-domain
//! plumbing that would otherwise create cycles between those modules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine;
use tokio::sync::{oneshot, Mutex};

use crate::agent::AgentServiceStore;
use crate::app::{
    ActiveTurnState, ActiveTurnStore, DaemonApp, PromptActivityStore, PromptWorkspaceClaimStore,
    ProviderProcessTrackingStore,
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

mod managed_io;
use managed_io::*;
mod context_handoff;
use context_handoff::*;
mod provider_reload;
pub(crate) use provider_reload::*;

#[derive(Clone)]
pub(crate) struct KernelRuntimeState {
    app: Arc<Mutex<DaemonApp>>,
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
    workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    managed_io_coordinator: Arc<Mutex<crate::io::ArtifactEditCoordinator>>,
    managed_io_external_changes: crate::io::ArtifactExternalChangeMonitor,
    workspace_identity_monitor:
        crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor,
    pending_agent_context_handoffs: PendingAgentContextHandoffStore,
    pending_mcp_continuations: PendingMcpContinuationStore,
    pending_provider_reloads: PendingProviderReloadStore,
    pending_interactions: PendingInteractionStore,
    git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore,
}

#[derive(Default)]
struct WorkflowPromptDispatches {
    local: Vec<crate::app::KernelPromptDispatch>,
    remote: Vec<crate::app::KernelRemotePromptDispatch>,
}

impl WorkflowPromptDispatches {
    fn extend(&mut self, other: Self) {
        self.local.extend(other.local);
        self.remote.extend(other.remote);
    }
}

#[derive(Debug, Clone)]
struct PendingMcpContinuation {
    session_id: String,
    agent_id: String,
    source_attachment_id: String,
    mcp_name: String,
    previous_prompt: String,
}

#[derive(Debug, Clone, Default)]
struct PendingMcpContinuationStore {
    inner: Arc<StdMutex<BTreeMap<String, PendingMcpContinuation>>>,
}

#[derive(Debug, Clone)]
struct PendingProviderReload {
    session_id: String,
    agent_id: String,
    reason: String,
}

#[derive(Debug, Clone, Default)]
struct PendingProviderReloadStore {
    inner: Arc<StdMutex<BTreeMap<String, PendingProviderReload>>>,
}

#[derive(Debug, Clone)]
struct PendingInteraction {
    session_id: String,
    responder: Arc<StdMutex<Option<oneshot::Sender<PendingInteractionResolution>>>>,
}

#[derive(Debug, Clone)]
pub(super) struct PendingInteractionResolution {
    pub(crate) status: &'static str,
    pub(crate) choice_id: Option<String>,
    pub(crate) reply: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct PendingInteractionStore {
    inner: Arc<StdMutex<BTreeMap<String, PendingInteraction>>>,
}

impl PendingMcpContinuationStore {
    fn shared() -> Self {
        static STORE: OnceLock<PendingMcpContinuationStore> = OnceLock::new();
        STORE
            .get_or_init(PendingMcpContinuationStore::default)
            .clone()
    }
}

impl PendingInteractionStore {
    fn shared() -> Self {
        static STORE: OnceLock<PendingInteractionStore> = OnceLock::new();
        STORE.get_or_init(PendingInteractionStore::default).clone()
    }

    fn write(&self) -> StdMutexGuard<'_, BTreeMap<String, PendingInteraction>> {
        self.inner
            .lock()
            .expect("pending interaction mutex poisoned")
    }
}

impl PendingMcpContinuationStore {
    fn write(&self) -> StdMutexGuard<'_, BTreeMap<String, PendingMcpContinuation>> {
        self.inner
            .lock()
            .expect("pending MCP continuation mutex poisoned")
    }
}

impl PendingProviderReloadStore {
    fn write(&self) -> StdMutexGuard<'_, BTreeMap<String, PendingProviderReload>> {
        self.inner
            .lock()
            .expect("pending provider reload mutex poisoned")
    }
}

struct ManagedIoWorkspaceContext {
    root: PathBuf,
    identity: crate::io::WorkspaceIdentity,
    generation: u64,
    identity_changed: bool,
    valid: bool,
}

mod owned;
mod prompt;
mod prompt_dispatch;
mod provider;
mod provider_runtime;
mod session;
mod tool_dispatch;
mod workflow;
mod workflow_admin;
mod workflow_dispatch;
mod workflow_tool;

impl KernelRuntimeState {
    pub(crate) fn new_with_owned_state(
        app: Arc<Mutex<DaemonApp>>,
        config_projection: crate::runtime::projection::DaemonConfigProjectionStore,
        session_store: SessionStateStore,
        agent_store: AgentServiceStore,
        attachment_store: AttachmentServiceStore,
        provider_store: ProviderProcessServiceStore,
        provider_process_tracking: ProviderProcessTrackingStore,
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
        workspace_coordinator: crate::runtime::workspace_coordinator::WorkspaceCoordinator,
    ) -> Self {
        Self {
            app,
            owned: KernelRuntimeOwnedState {
                config_projection,
                session_store,
                agent_store,
                attachment_store,
                provider_store,
                provider_process_tracking,
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
                workspace_coordinator,
                managed_io_coordinator: Arc::new(Mutex::new(
                    crate::io::ArtifactEditCoordinator::new(),
                )),
                managed_io_external_changes: crate::io::ArtifactExternalChangeMonitor::default(),
                workspace_identity_monitor:
                    crate::runtime::workspace_identity_monitor::WorkspaceIdentityMonitor::default(),
                pending_agent_context_handoffs: PendingAgentContextHandoffStore::default(),
                pending_mcp_continuations: PendingMcpContinuationStore::shared(),
                pending_provider_reloads: PendingProviderReloadStore::default(),
                pending_interactions: PendingInteractionStore::shared(),
                git_turn_snapshots: crate::git_observer::GitTurnSnapshotStore::default(),
            },
        }
    }

    pub(crate) fn start_active_turn(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        provider_run_id: &str,
    ) {
        self.owned.active_turns.start(ActiveTurnState::new(
            session_id.to_string(),
            agent_id.to_string(),
            prompt_id.to_string(),
            provider_run_id.to_string(),
        ));
    }

    pub(crate) fn agent_activity_for_session(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity> {
        let prompt_activity = self.owned.prompt_activity.read();
        let active_turns = self.owned.active_turns.snapshot();
        crate::runtime::projection::agent_activity_for_session_projection(
            session,
            |agent_id| {
                self.owned
                    .provider_run_projection
                    .get_for_agent(session.id(), agent_id)
                    .or_else(|| {
                        self.owned
                            .provider_store
                            .get_run_for_agent(session.id(), agent_id)
                    })
            },
            &prompt_activity,
            &active_turns,
        )
    }

    pub(crate) async fn config_snapshot(&self) -> crate::config::DaemonConfig {
        self.owned.config_projection.snapshot()
    }

    pub(crate) async fn managed_io_health_snapshot(
        &self,
    ) -> crate::runtime::projection::ManagedIoHealthSnapshot {
        let reservations = self
            .owned
            .managed_io_coordinator
            .lock()
            .await
            .active_reservation_snapshots();
        let active_reservation_artifacts = reservations
            .iter()
            .map(|reservation| reservation.artifact_id.clone())
            .collect::<BTreeSet<_>>()
            .len();
        crate::runtime::projection::ManagedIoHealthSnapshot {
            active_reservations: reservations.len(),
            active_reservation_artifacts,
            workspace_identity: self.owned.workspace_identity_monitor.health_snapshot(),
            external_changes: self.owned.managed_io_external_changes.health_snapshot(),
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

    pub(crate) async fn active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(self
            .owned
            .prompt_state_owner
            .active_prompt_agent_id(&session))
    }

    pub(crate) async fn focused_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        Ok(session.focused_agent_id().map(str::to_string))
    }

    pub(crate) async fn resolve_session_ref_id(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        Ok(self
            .owned
            .session_store
            .read()
            .resolve_session_ref(session_ref, workspace_id)?
            .id()
            .to_string())
    }

    pub(crate) async fn attachment_session_id(
        &self,
        attachment_id: &str,
    ) -> Result<String, DaemonError> {
        Ok(self
            .owned
            .attachment_store
            .get_attachment(attachment_id)?
            .session_id()
            .to_string())
    }

    pub(crate) async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.session_snapshot(session_id)
    }

    pub(crate) async fn create_session_response(
        &self,
        request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let slice_ref = request.slice_ref.clone();
        let response = if let Some(slice_ref) = slice_ref {
            let worker_kernel_ref = self.resolve_slice_worker_kernel_ref(&slice_ref).await?;
            self.create_sliced_session_response(request, worker_kernel_ref)
                .await?
        } else {
            self.owned.create_session_response(request)?
        };
        if let LocalDaemonResponse::SessionCreated { session, agent } = &response {
            self.owned.durable_state_store.append_event(
                "session.created",
                Some(session.id().to_string()),
                serde_json::json!({
                    "session": session,
                    "default_agent": agent,
                }),
            )?;
        }
        Ok(response)
    }

    async fn create_sliced_session_response(
        &self,
        mut request: crate::session::CreateSessionRequest,
        worker_kernel_ref: String,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create sliced session started",
            serde_json::json!({
                "workspace_id": request.workspace_id,
                "worktree_id": request.worktree_id,
                "slice_ref": request.slice_ref,
                "worker_kernel_ref": worker_kernel_ref,
                "has_alias": request.alias.as_ref().is_some_and(|alias| !alias.trim().is_empty()),
                "has_agent_defaults": request.agent_defaults.is_some(),
            }),
        );
        request.slice_ref = None;
        let mut session =
            SessionStateOwner::new(self.owned.session_store.clone()).create_session(request)?;
        let agent_request =
            session::agent_request_from_session_defaults(&session, Some(session.owner_user_id()))
                .with_worktree(session.worktree_id())
                .with_kernel(worker_kernel_ref);
        let agent = self.spawn_agent(agent_request).await?;
        self.owned
            .session_store
            .write()
            .set_focused_agent(session.id(), Some(agent.id().to_string()))?;
        session = self.owned.session_snapshot(session.id())?;
        crate::logging::info_with_fields(
            "daemon.kernel_session",
            "create sliced session completed",
            serde_json::json!({
                "session_id": session.id(),
                "agent_id": agent.id(),
                "agent_count": session.agents().len(),
            }),
        );
        Ok(LocalDaemonResponse::SessionCreated { session, agent })
    }

    pub(crate) async fn attach(
        &self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        self.owned.attach(request)
    }

    pub(crate) async fn detach(
        &self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        self.owned.detach(attachment_id)
    }

    pub(crate) async fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned.focus_agent(session_id, agent_id, caller_user_id)
    }

    pub(crate) async fn cycle_agent_focus(
        &self,
        session_id: &str,
        caller_user_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        self.owned.cycle_agent_focus(session_id, caller_user_id)
    }

    pub(crate) async fn grant_agent_mcp(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let existing = self
            .owned
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.owned.agent_store.get_agent_by_ref(agent_ref))?;
        self.owned
            .ensure_agent_owner(existing.id(), caller_user_id, "grant agent capability")?;
        if existing.remote_execution().is_some() && !existing.mcp_grants().contains(&name) {
            let mut checked = existing.clone();
            checked.grant_mcp(name.clone());
            self.ensure_remote_mcp_availability_for_agent(&checked)
                .await?;
        }
        let agent = self
            .owned
            .grant_agent_mcp(agent_ref, name.clone(), caller_user_id)?;
        self.append_agent_durable_event("agent.mcp_granted", &agent, Some(&name))
            .await?;
        let _ = self
            .apply_provider_reload_policy(ProviderReloadTrigger::AgentMcpGrant {
                session_id: agent.session_id().to_string(),
                agent_id: agent.id().to_string(),
                name,
            })
            .await?;
        Ok(agent)
    }

    pub(crate) async fn revoke_agent_mcp(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .revoke_agent_mcp(agent_ref, name, caller_user_id)?;
        self.append_agent_durable_event("agent.mcp_revoked", &agent, Some(name))
            .await?;
        Ok(agent)
    }

    pub(crate) async fn grant_agent_skill(
        &self,
        agent_ref: &str,
        name: String,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .grant_agent_skill(agent_ref, name.clone(), caller_user_id)?;
        self.append_agent_durable_event("agent.skill_granted", &agent, Some(&name))
            .await?;
        self.ensure_remote_skill_packages_for_agent(&agent).await?;
        Ok(agent)
    }

    pub(crate) async fn revoke_agent_skill(
        &self,
        agent_ref: &str,
        name: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .revoke_agent_skill(agent_ref, name, caller_user_id)?;
        self.append_agent_durable_event("agent.skill_revoked", &agent, Some(name))
            .await?;
        Ok(agent)
    }

    pub(crate) async fn resize_terminal(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        if let Some(provider_run_id) = self.owned.resize_terminal(session_id)? {
            self.with_app_side_effect(|app| app.pty_mut().resize(&provider_run_id, cols, rows))
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(), DaemonError> {
        let _ = self
            .owned
            .ensure_attachment_in_session(session_id, attachment_id)?;
        Ok(())
    }

    pub(crate) async fn drain_notice_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<crate::terminal::RuntimeNoticeRecord> {
        self.owned
            .terminal_stream
            .drain_notice_records(session_id, attachment_id)
    }

    pub(crate) async fn update_session_config(
        &self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        self.owned
            .update_session_config(session_id, attachment_id, values, requires_idle)
    }

    pub(crate) async fn update_agent_config(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        execution_mode_override: Option<Option<crate::provider::AgentExecutionMode>>,
        permission_level_override: Option<Option<crate::provider::AgentPermissionLevel>>,
        workspace_id: Option<Option<String>>,
        worktree_id: Option<Option<String>>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let update = self.owned.update_agent_config(
            session_id,
            agent_id,
            caller_user_id,
            execution_mode_override.clone(),
            permission_level_override.clone(),
            workspace_id,
            worktree_id,
        )?;
        for provider_run_id in update.terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            self.owned
                .remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        let mut agent = update.agent;
        if let Some(remote_update) = update.remote_update {
            let mut config = self.config_snapshot().await;
            if let (Some(relay_url), Some(relay_token)) = (
                remote_update.relay_url.clone(),
                remote_update.relay_token.clone(),
            ) {
                config.relay_url = Some(relay_url);
                config.relay_token = Some(relay_token);
                config.cloud_relay = None;
            }
            match tokio::time::timeout(
                Duration::from_secs(5),
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config,
                    ClientTarget {
                        daemon_id: Some(remote_update.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::UpdateLeasedAgentConfig {
                        leased_agent_id: remote_update.leased_agent_id,
                        execution_mode: remote_update.execution_mode,
                        permission_level: remote_update.permission_level,
                    },
                ),
            )
            .await
            {
                Ok(Ok(RelayPeerResponse::LeasedAgentConfigUpdated { .. })) => {}
                Ok(Ok(other)) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "update remote leased agent config",
                        message: format!("unexpected remote config response: {other:?}"),
                    });
                }
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "update remote leased agent config",
                        message: "timed out waiting for remote worker config update".to_string(),
                    });
                }
            }
            agent = self.owned.commit_remote_agent_config_update(
                session_id,
                agent_id,
                execution_mode_override,
                permission_level_override,
            )?;
        }
        Ok(agent)
    }

    pub(crate) async fn update_agent_profile(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        provider: Option<String>,
        model: Option<String>,
        effort: Option<Option<String>>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let (agent, terminated_run_ids) = self.owned.update_agent_profile(
            session_id,
            agent_id,
            caller_user_id,
            provider,
            model,
            effort,
        )?;
        for provider_run_id in terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            self.owned
                .remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        Ok(agent)
    }

    pub(crate) async fn alias_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        alias: Option<String>,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .alias_agent(session_id, agent_id, caller_user_id, alias)
    }

    pub(crate) async fn update_agent_substitutes(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
        action: crate::local::AgentSubstituteAction,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .update_agent_substitutes(session_id, agent_id, caller_user_id, action)
    }

    pub(super) async fn create_runtime_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        let (tx, rx) = oneshot::channel();
        self.owned
            .register_runtime_interaction(session_id, interaction, tx)?;
        Ok(rx)
    }

    pub(crate) async fn resolve_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        custom_reply: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned
            .resolve_runtime_interaction(session_id, interaction_id, choice_id, custom_reply)
    }

    pub(crate) async fn timeout_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
    ) -> Result<(), DaemonError> {
        self.owned
            .timeout_runtime_interaction(session_id, interaction_id)
    }

    pub(crate) async fn alias_session(
        &self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.alias_session(session_id, alias)
    }

    pub(crate) async fn ensure_agent_owner(
        &self,
        agent_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .ensure_agent_owner(agent_id, caller_user_id, operation)
    }

    pub(crate) async fn spawn_agent(
        &self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if request.kernel_ref.is_none() {
            return self.owned.spawn_agent(request);
        }
        self.with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).spawn_agent(request)
        })
        .await
    }

    pub(crate) async fn resolve_slice_worker_kernel_ref(
        &self,
        slice_ref: &str,
    ) -> Result<String, DaemonError> {
        let slice_ref = slice_ref.to_string();
        self.with_app_side_effect(move |app| app.slices().resolve_worker_kernel_ref(&slice_ref))
            .await
    }

    pub(crate) async fn move_agent_to_remote(
        &self,
        session_id: &str,
        agent_ref: &str,
        machine_ref: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned
            .ensure_agent_ref_owner(agent_ref, caller_user_id, "move agent to remote")?;
        self.with_app_side_effect(|app| {
            app.move_agent_to_remote(session_id, agent_ref, machine_ref)
        })
        .await
    }

    pub(crate) async fn destroy_agent(
        &self,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        self.owned
            .ensure_agent_owner(agent.id(), caller_user_id, "destroy agent")?;
        if agent.remote_execution().is_none() {
            return self.owned.destroy_agent(agent_id, caller_user_id);
        }
        self.with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).destroy_agent(agent_id)
        })
        .await
    }

    pub(crate) async fn end_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let owned = &self.owned;
        let (session, terminated_run_ids) = owned.end_session(session_id)?;
        for provider_run_id in terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        self.append_session_durable_event("session.ended", &session, "runtime_end_session")
            .await?;
        Ok(session)
    }

    pub(crate) async fn delete_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let owned = &self.owned;
        let (session, terminated_run_ids) = owned.delete_session_ref(session_ref, workspace_id)?;
        for provider_run_id in terminated_run_ids {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(&provider_run_id, process_key);
        }
        self.append_session_durable_event("session.deleted", &session, "runtime_delete_session")
            .await?;
        Ok(session)
    }

    pub(crate) async fn delete_current_kernel_sessions(
        &self,
    ) -> Result<Vec<crate::session::RuntimeSession>, DaemonError> {
        let session_ids: Vec<String> = self
            .owned
            .session_store
            .list_all_sessions()
            .into_iter()
            .map(|session| session.id().to_string())
            .collect();
        let mut deleted_sessions = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            let session = self.delete_session_ref(&session_id, None).await?;
            deleted_sessions.push(session);
        }
        Ok(deleted_sessions)
    }

    pub(crate) async fn execute_workflow_request(
        &self,
        request: LocalDaemonRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let owned = &self.owned;

        let outcome = match request {
            LocalDaemonRequest::CreateWorkflow(request) => {
                let result = owned.workflow_create_workflow(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ApplyWorkflowDesignOp(request) => {
                let origin_client_id = request.origin_client_id.clone();
                let op_id = request.op_id.clone();
                let session_id = request.session_id.clone();
                let op = request.op.clone();
                let event_store = {
                    let app = self.app.lock().await;
                    app.workflow_design_event_store()
                };
                let result = owned
                    .workflow_apply_design_op(request, &caller_user_id)
                    .map(|session| {
                        let event = event_store.append(session_id, origin_client_id, op_id, op);
                        LocalDaemonResponse::WorkflowDesignOpAccepted { session, event }
                    });
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AliasWorkflow(request) => {
                let result = owned.workflow_alias_workflow(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflows(request) => {
                (owned.workflow_list_workflows(request), None)
            }
            LocalDaemonRequest::ResolveWorkflow(request) => {
                (owned.workflow_resolve_workflow(request), None)
            }
            LocalDaemonRequest::CreateWorkflowPublication(request) => {
                let result = owned.workflow_create_publication(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowPublications(request) => {
                (owned.workflow_list_publications(request), None)
            }
            LocalDaemonRequest::GetWorkflowPublication(request) => {
                (owned.workflow_get_publication(request), None)
            }
            LocalDaemonRequest::DisableWorkflowPublication(request) => {
                let result = owned.workflow_disable_publication(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::CreateWorkflowPublicationPairCode(request) => {
                let result = owned.workflow_create_publication_pair_code(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RedeemWorkflowPublicationPairCode(request) => {
                let result = owned.workflow_redeem_publication_pair_code(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowPublicationSenders(request) => (
                owned.workflow_list_publication_senders(request, &caller_user_id),
                None,
            ),
            LocalDaemonRequest::RevokeWorkflowPublicationSender(request) => {
                let result = owned.workflow_revoke_publication_sender(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AuthenticateWorkflowPublicationSender(request) => (
                owned.workflow_authenticate_publication_sender(request),
                None,
            ),
            LocalDaemonRequest::CreateWorkflowEndpoint(request) => {
                let result = owned.workflow_create_endpoint(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AliasWorkflowEndpoint(request) => {
                let result = owned.workflow_alias_endpoint(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::BindWorkflowEndpoint(request) => {
                let result = owned.workflow_bind_endpoint(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AddWorkflowNode(request) => {
                let result = owned.workflow_add_node(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowNode(request) => {
                let result = owned.workflow_remove_node(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => {
                let result = owned.workflow_update_node_instructions(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => {
                let result = owned.workflow_set_node_can_complete_run(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
                let result =
                    owned.workflow_set_node_can_emit_intermediate_output(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
                let result =
                    owned.workflow_set_node_intermediate_output_schema(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => {
                let result = owned.workflow_set_node_max_turns(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AddWorkflowEdge(request) => {
                let result = owned.workflow_add_edge(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowEdge(request) => {
                let result = owned.workflow_remove_edge(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::UpdateWorkflowCanvasLayout(request) => {
                let result = owned.workflow_update_canvas_layout(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowFlushContext(request) => {
                let result = owned.workflow_set_flush_context(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => {
                let result = owned.workflow_set_run_output_schema(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
                let result = owned.workflow_set_intermediate_output_schema(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => {
                let result = owned.workflow_set_launch_policy(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowRuns(request) => {
                (owned.workflow_list_runs(request), None)
            }
            LocalDaemonRequest::GetWorkflowRun(request) => (owned.workflow_get_run(request), None),
            LocalDaemonRequest::CreateWorkflowWatchdog(request) => {
                let result = owned.workflow_create_watchdog(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
                (owned.workflow_list_watchdogs(request), None)
            }
            LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => {
                let result = owned.workflow_set_watchdog_enabled(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowWatchdog(request) => {
                let result = owned.workflow_remove_watchdog(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => {
                (owned.workflow_list_queued_launches(request), None)
            }
            LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => {
                let result = owned.workflow_remove_queued_launch(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => {
                let result = owned.workflow_clear_queued_launches(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::InvokeWorkflowEndpoint(request) => {
                let session_id = request.session_id.clone();
                let result = match owned
                    .ensure_workflow_endpoint_owner(
                        &request.session_id,
                        &request.workflow_ref,
                        &request.endpoint_ref,
                        &caller_user_id,
                        "invoke workflow endpoint",
                    )
                    .and_then(|()| {
                        owned.workflow_invoke_endpoint_with_admission(
                            &request.session_id,
                            &request.workflow_ref,
                            &request.endpoint_ref,
                            request.prompt,
                        )
                    }) {
                    Ok((outcome, dispatches)) => {
                        self.spawn_workflow_prompt_dispatches(dispatches);
                        let session = match owned.session_snapshot(&request.session_id) {
                            Ok(session) => session,
                            Err(error) => return (Err(error), None),
                        };
                        match outcome {
                            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                                workflow_run,
                                workflow,
                                endpoint,
                            } => Ok(LocalDaemonResponse::WorkflowRunInvoked {
                                workflow_run,
                                workflow,
                                endpoint,
                                session,
                            }),
                            crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued {
                                queued_launch,
                                workflow,
                                endpoint,
                            } => Ok(LocalDaemonResponse::WorkflowRunQueued {
                                queued_launch,
                                workflow,
                                endpoint,
                                session,
                            }),
                        }
                    }
                    Err(error) => Err(error),
                };
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
                (result, session)
            }
            LocalDaemonRequest::CancelWorkflowRun(request) => {
                let session_id = request.session_id.clone();
                let result = (|| {
                    let workflow_run_id = owned
                        .session_store
                        .read()
                        .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                        .id()
                        .to_string();
                    let session = owned.session_store.get_session(&request.session_id)?;
                    for agent in owned.agent_store.get_session_agents(&request.session_id) {
                        if owned
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, agent.id())
                            .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
                            .as_deref()
                            == Some(workflow_run_id.as_str())
                        {
                            let _ = owned
                                .prompt_state_owner
                                .begin_cancelling_active_prompt(&session, agent.id())
                                .ok_or_else(|| DaemonError::NoActivePrompt {
                                    session_id: request.session_id.clone(),
                                })?;
                            let (active_prompt, queued_prompts) =
                                owned.prompt_state_owner.state_parts(&session, agent.id());
                            owned.session_store.mirror_agent_prompt_state(
                                &request.session_id,
                                agent.id(),
                                active_prompt,
                                queued_prompts,
                            )?;
                        }
                    }
                    let workflow_run = owned
                        .session_store
                        .write()
                        .cancel_workflow_run(&request.session_id, &request.workflow_run_ref)?;
                    let _ = owned.prompt_workspace_claims.remove_matching(|claim| {
                        claim.session_id == request.session_id
                            && claim.operation == "workflow_node_dispatch"
                    });
                    let workflow = owned
                        .session_store
                        .read()
                        .resolve_workflow_ref(&request.session_id, workflow_run.workflow_id())?;
                    for node in workflow.nodes() {
                        if let Some(run) = owned
                            .provider_store
                            .get_run_for_agent(&request.session_id, node.agent_id())
                        {
                            let _ = owned.clear_prompt_activity(run.id());
                        }
                    }
                    let session = owned.session_store.get_session(&request.session_id)?;
                    let _ = owned
                        .prompt_state_owner
                        .remove_queued_prompts_by_workflow_run(&session, &workflow_run_id);
                    for agent in owned.agent_store.get_session_agents(&request.session_id) {
                        let (active_prompt, queued_prompts) =
                            owned.prompt_state_owner.state_parts(&session, agent.id());
                        let _ = owned.session_store.mirror_agent_prompt_state(
                            &request.session_id,
                            agent.id(),
                            active_prompt,
                            queued_prompts,
                        );
                    }
                    owned.workflow_maybe_start_next_queued_launch(&request.session_id);
                    let session = owned.session_snapshot(&request.session_id)?;
                    Ok(LocalDaemonResponse::WorkflowRunCancelled {
                        workflow_run,
                        session,
                    })
                })();
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
                (result, session)
            }
            LocalDaemonRequest::ResumeWorkflowRun(request) => {
                let session_id = request.session_id.clone();
                let result = match owned
                    .workflow_resume_run(&request.session_id, &request.workflow_run_ref)
                {
                    Ok((workflow_run, dispatches)) => {
                        self.spawn_workflow_prompt_dispatches(dispatches);
                        owned.workflow_session(&request.session_id).map(|session| {
                            LocalDaemonResponse::WorkflowRunResumed {
                                workflow_run,
                                session,
                            }
                        })
                    }
                    Err(error) => Err(error),
                };
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
                (result, session)
            }
            LocalDaemonRequest::ValidateWorkflowOutput(request) => {
                let result = owned.workflow_validate_output(request);
                (result, None)
            }
            LocalDaemonRequest::AckWorkflowTurn(request) => {
                let result = owned.workflow_ack_turn(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            _ => (
                Err(DaemonError::LocalTransport {
                    operation: "execute workflow request",
                    message: "request is not handled by the workflow runtime".to_string(),
                }),
                None,
            ),
        };
        if outcome.0.is_ok() {
            if let Some(session) = outcome.1.as_ref() {
                if let Err(error) = self
                    .append_session_durable_event("session.updated", session, "workflow")
                    .await
                {
                    return (Err(error), outcome.1);
                }
            }
        }
        outcome
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

fn workflow_response_session(
    response: &LocalDaemonResponse,
) -> Option<crate::session::RuntimeSession> {
    match response {
        LocalDaemonResponse::WorkflowCreated { session, .. }
        | LocalDaemonResponse::WorkflowDesignOpAccepted { session, .. }
        | LocalDaemonResponse::WorkflowAliased { session, .. }
        | LocalDaemonResponse::WorkflowPublicationCreated { session, .. }
        | LocalDaemonResponse::WorkflowPublicationDisabled { session, .. }
        | LocalDaemonResponse::WorkflowPublicationPairCodeCreated { session, .. }
        | LocalDaemonResponse::WorkflowPublicationSenderPaired { session, .. }
        | LocalDaemonResponse::WorkflowPublicationSenderRevoked { session, .. }
        | LocalDaemonResponse::WorkflowEndpointCreated { session, .. }
        | LocalDaemonResponse::WorkflowEndpointAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointBound { session, .. }
        | LocalDaemonResponse::WorkflowNodeAdded { session, .. }
        | LocalDaemonResponse::WorkflowNodeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowNodeInstructionsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowEdgeAdded { session, .. }
        | LocalDaemonResponse::WorkflowEdgeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowCanvasLayoutUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunInvoked { session, .. }
        | LocalDaemonResponse::WorkflowRunQueued { session, .. }
        | LocalDaemonResponse::WorkflowRunCancelled { session, .. }
        | LocalDaemonResponse::WorkflowRunResumed { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogCreated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogUpdated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogRemoved { session, .. }
        | LocalDaemonResponse::WorkflowFlushContextUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchRemoved { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchesCleared { session, .. }
        | LocalDaemonResponse::WorkflowTurnAcknowledged { session, .. } => Some(session.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod managed_io_external_change_notice_tests;
