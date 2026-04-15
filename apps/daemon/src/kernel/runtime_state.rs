use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::agent::AgentServiceStore;
use crate::app::{
    DaemonApp, PromptActivityStore, PromptWorkspaceClaimStore, ProviderProcessTrackingStore,
};
use crate::attachment::AttachmentServiceStore;
use crate::error::DaemonError;
use crate::history::{SessionHistoryEntry, SessionHistoryStore};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::{ProviderProcessServiceStore, ProviderRunOperationLanes};
use crate::session::{SessionStateOwner, SessionStateStore};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

#[derive(Clone)]
pub(crate) struct KernelRuntimeState {
    app: Arc<Mutex<DaemonApp>>,
    owned: KernelRuntimeOwnedState,
}

#[derive(Clone)]
pub(crate) struct KernelRuntimeOwnedState {
    config_projection: crate::kernel::projection::DaemonConfigProjectionStore,
    session_store: SessionStateStore,
    agent_store: AgentServiceStore,
    attachment_store: AttachmentServiceStore,
    provider_store: ProviderProcessServiceStore,
    provider_process_tracking: ProviderProcessTrackingStore,
    session_projection: crate::kernel::projection::SessionStateProjectionStore,
    provider_run_projection: crate::kernel::projection::ProviderRunProjectionStore,
    history_store: SessionHistoryStore,
    history_projection: crate::kernel::projection::SessionHistoryProjectionStore,
    prompt_state_owner: crate::kernel::prompt_state::PromptStateOwner,
    prompt_activity: PromptActivityStore,
    prompt_idle_timeout: Duration,
    prompt_workspace_claims: PromptWorkspaceClaimStore,
    structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
    terminal_stream: crate::terminal::TerminalStreamStore,
    workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
    managed_io_coordinator: Arc<Mutex<crate::io::ArtifactEditCoordinator>>,
    managed_io_workspace_identities: Arc<
        std::sync::Mutex<BTreeMap<String, crate::io::WorkspaceIdentity>>,
    >,
}

struct ManagedIoWorkspaceContext {
    root: PathBuf,
    identity: crate::io::WorkspaceIdentity,
    identity_changed: bool,
}

impl KernelRuntimeOwnedState {
    fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let mut session = self.session_store.get_session(session_id)?;
        let agents = self.agent_store.get_session_agents(session_id);
        session.set_agents(agents);
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        Ok(session)
    }

    fn project_session_runtime_view(&self, session: &mut crate::session::RuntimeSession) {
        if let Some(active_provider_run_id) = session.active_provider_run_id() {
            if let Ok(active_run) = self.provider_store.get_run(active_provider_run_id) {
                let active_run_agent_id = active_run.agent_instance_id();
                let active_prompt_is_running = active_run_agent_id
                    .and_then(|agent_id| {
                        self.prompt_state_owner
                            .active_prompt_for_agent_snapshot(session, agent_id)
                    })
                    .is_some();
                if active_run.state() == crate::provider::ProviderRunState::Running
                    && active_prompt_is_running
                {
                    return;
                }
            }
        }

        let projected_run_id = session.focused_agent_id().and_then(|agent_id| {
            self.provider_store
                .get_run_for_agent(session.id(), agent_id)
                .map(|run| run.id().to_string())
        });
        session.set_active_provider_run(projected_run_id);
    }

    fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let attachment = self.attachment_store.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        Ok(attachment)
    }

    fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let attachment = self.ensure_attachment_in_session(session_id, attachment_id)?;
        if !matches!(
            attachment.capability_level(),
            crate::attachment::ClientCapabilityLevel::FullTerminal
                | crate::attachment::ClientCapabilityLevel::InteractiveStructured
        ) {
            return Err(DaemonError::AttachmentCapabilityDenied {
                session_id: session_id.to_string(),
                attachment_id: attachment.id().to_string(),
                capability,
            });
        }
        Ok(CapabilityRuntimeSnapshot {
            workspace_id: session.workspace_id().to_string(),
            worktree_root: std::path::PathBuf::from(session.worktree_id()),
            workspace_coordinator: self.workspace_coordinator.clone(),
        })
    }

    fn managed_io_workspace_for_provider_run(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Result<ManagedIoWorkspaceContext, DaemonError> {
        let session = self.session_store.get_session(provider_run.session_id())?;
        let workspace_root = provider_run
            .working_directory()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(session.worktree_id()));
        let identity = workspace_identity_for_root(&workspace_root);
        let identity_changed = {
            let mut identities = self.managed_io_workspace_identities.lock().map_err(|_| {
                DaemonError::LocalTransport {
                    operation: "managed_io_workspace_identity",
                    message: "managed I/O workspace identity tracker is poisoned".to_string(),
                }
            })?;
            match identities.insert(provider_run.id().to_string(), identity.clone()) {
                Some(previous) => previous != identity,
                None => false,
            }
        };
        Ok(ManagedIoWorkspaceContext {
            root: workspace_root,
            identity,
            identity_changed,
        })
    }

    fn managed_io_domain_from_arg(
        domain: Option<&str>,
    ) -> Result<crate::io::ArtifactDomainKind, DaemonError> {
        match domain.unwrap_or("text") {
            "text" => Ok(crate::io::ArtifactDomainKind::TextDocument),
            "structured" => Ok(crate::io::ArtifactDomainKind::StructuredDocument),
            "opaque" => Ok(crate::io::ArtifactDomainKind::OpaqueBlob),
            other => Err(DaemonError::LocalTransport {
                operation: "runtime_tool_managed_io",
                message: format!("unsupported artifact domain `{other}`"),
            }),
        }
    }

    fn prepare_provider_launch_request(
        &self,
        mut request: crate::provider::LaunchProviderRequest,
        runtime_mcp_url: String,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        if request.agent_id.is_none() {
            request.agent_id = self
                .session_store
                .get_session(&request.session_id)?
                .focused_agent_id()
                .map(str::to_string)
                .or_else(|| {
                    self.agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                });
        }
        if request.resume_state.is_none() {
            if let Some(agent_id) = request.agent_id.as_deref() {
                if let Ok(agent) = self.agent_store.get_agent(agent_id) {
                    let resume_state =
                        crate::app::sanitize_resume_state_for_launch(&request, &agent);
                    if !resume_state.is_empty() {
                        request = request.with_resume_state(resume_state);
                    }
                }
            }
        }
        if (request.adapter_key == "opencode" || request.adapter_key == "codex")
            && request.working_directory.is_none()
        {
            let agent_worktree = request.agent_id.as_deref().and_then(|agent_id| {
                self.agent_store
                    .get_agent(agent_id)
                    .ok()
                    .and_then(|agent| agent.worktree_id().map(std::path::PathBuf::from))
            });
            request.working_directory = Some(agent_worktree.unwrap_or_else(|| {
                std::path::PathBuf::from(
                    self.session_store
                        .get_session(&request.session_id)
                        .map(|session| session.worktree_id().to_string())
                        .unwrap_or_default(),
                )
            }));
        }
        if request.runtime_mcp_binding.is_none() {
            let shared_auth_token = self
                .provider_store
                .get_session_run_for_provider(&request.session_id, &request.provider)
                .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string));
            request = request.with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                runtime_mcp_url,
                shared_auth_token.unwrap_or_else(crate::app::generate_runtime_mcp_auth_token),
            ));
        }
        Ok(request)
    }

    fn create_session_response(
        &self,
        request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut session =
            SessionStateOwner::new(self.session_store.clone()).create_session(request)?;
        let agent_request = crate::agent::CreateAgentRequest::new(session.id(), "default")
            .with_worktree(session.worktree_id());
        let mut sessions = self.session_store.write();
        let agent = self
            .agent_store
            .create_agent(agent_request, &mut sessions)?;
        drop(sessions);
        session = self.session_store.get_session(session.id())?;
        let agents = self.agent_store.get_session_agents(session.id());
        session.set_agents(agents);
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::SessionCreated { session, agent })
    }

    fn update_session_config(
        &self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let (_session, config) = SessionStateOwner::new(self.session_store.clone()).update_config(
            session_id,
            attachment_id,
            values,
            requires_idle,
        )?;
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|id| id != attachment_id)
            .collect::<Vec<_>>();
        if !recipient_attachment_ids.is_empty() {
            let active_provider_run_id = self
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                .map(str::to_string);
            self.record_notice(
                session_id,
                active_provider_run_id.as_deref(),
                recipient_attachment_ids,
                format!(
                    "Attachment `{attachment_id}` updated configuration for session `{session_id}`."
                ),
            );
        }
        Ok(config)
    }

    fn alias_session(
        &self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        SessionStateOwner::new(self.session_store.clone())
            .assign_session_alias(session_id, alias)?;
        self.session_snapshot(session_id)
    }

    fn spawn_agent(
        &self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let mut sessions = self.session_store.write();
        self.agent_store.create_agent(request, &mut sessions)
    }

    fn destroy_agent(&self, agent_id: &str) -> Result<crate::agent::AgentInstance, DaemonError> {
        let mut sessions = self.session_store.write();
        self.agent_store.destroy_agent(agent_id, &mut sessions)
    }

    fn attach(
        &self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let session_id = request.session_id.clone();
        let client_id = request.client_id.clone();
        let capability_level = format!("{:?}", request.capability_level);
        let replaced_attachment_ids = self
            .attachment_store
            .list_client_attachments(&client_id)
            .into_iter()
            .map(|attachment| attachment.id().to_string())
            .collect::<Vec<_>>();
        for attachment_id in &replaced_attachment_ids {
            let _ = self.detach(attachment_id)?;
        }

        let mut sessions = self.session_store.write();
        let attachment = self.attachment_store.attach(&mut sessions, request)?;
        drop(sessions);

        if self.agent_store.get_session_agents(&session_id).is_empty() {
            let worktree_id = self
                .session_store
                .get_session(&session_id)?
                .worktree_id()
                .to_string();
            let agent_request = crate::agent::CreateAgentRequest::new(&session_id, "default")
                .with_worktree(worktree_id);
            let mut sessions = self.session_store.write();
            let _ = self
                .agent_store
                .create_agent(agent_request, &mut sessions)?;
            drop(sessions);
            crate::logging::info_with_fields(
                "daemon.app",
                "created default agent for session",
                serde_json::json!({
                    "session_id": session_id,
                    "reason": "session had no agents (possibly after being ended and reattached)",
                }),
            );
        }

        self.sync_focused_provider_run_if_idle(&session_id)?;

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment joined session",
            serde_json::json!({
                "session_id": session_id,
                "attachment_id": attachment.id(),
                "client_id": client_id,
                "capability_level": capability_level,
                "replaced_attachment_ids": replaced_attachment_ids,
            }),
        );
        Ok(attachment)
    }

    fn detach(
        &self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        let mut sessions = self.session_store.write();
        let (attachment, effect) = self
            .attachment_store
            .detach_with_effect(&mut sessions, attachment_id)?;
        drop(sessions);

        let session = self.session_store.get_session(attachment.session_id())?;
        let owner_removed_queued_prompt_count = self
            .prompt_state_owner
            .remove_queued_prompts_by_attachment(&session, attachment_id);
        self.mirror_prompt_owner_session_state(attachment.session_id())?;
        let removed_queued_prompt_count = effect
            .removed_queued_prompt_count
            .max(owner_removed_queued_prompt_count);
        let session_after_detach = self.session_store.get_session(attachment.session_id())?;

        if removed_queued_prompt_count > 0 {
            self.record_notice(
                attachment.session_id(),
                None,
                self.attachment_store
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed {} queued prompt(s) from detached attachment `{}`.",
                    removed_queued_prompt_count, attachment_id
                ),
            );
        }

        if effect.removed_active_prompt {
            self.record_notice(
                attachment.session_id(),
                None,
                self.attachment_store
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed the active prompt from detached attachment `{}` and advanced the queue.",
                    attachment_id
                ),
            );
            if let Some(agent_id) = session_after_detach.focused_agent_id() {
                let _ = self.activate_next_queued_prompt_for_agent(
                    attachment.session_id(),
                    agent_id,
                    None,
                )?;
            }
        }

        let remaining_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(attachment.session_id());
        let active_prompt_agent_id = self
            .prompt_state_owner
            .active_prompt_agent_id(&self.session_snapshot(attachment.session_id())?);
        if remaining_attachment_ids.is_empty() && active_prompt_agent_id.is_none() {
            if let Some(active_provider_run_id) = session_after_detach
                .active_provider_run_id()
                .map(str::to_string)
            {
                let run = self.provider_store.get_run(&active_provider_run_id)?;
                if run.state() != crate::provider::ProviderRunState::Ended {
                    let outcome = self
                        .provider_store
                        .park_run_provider_only(attachment.session_id(), &active_provider_run_id)?;
                    if self
                        .session_store
                        .get_session(attachment.session_id())?
                        .active_provider_run_id()
                        == Some(outcome.run().id())
                    {
                        self.session_store
                            .set_active_provider_run(attachment.session_id(), None)?;
                    }
                    self.provider_run_projection.update(outcome.into_run());
                }
            }
            for run in self.provider_store.list_runs() {
                if run.session_id() == attachment.session_id() {
                    self.clear_prompt_activity(run.id());
                }
            }
        }

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment left session",
            serde_json::json!({
                "session_id": attachment.session_id(),
                "attachment_id": attachment.id(),
                "removed_queued_prompts": removed_queued_prompt_count,
                "removed_active_prompt": effect.removed_active_prompt,
                "remaining_attachment_ids": remaining_attachment_ids,
            }),
        );
        self.session_snapshot(attachment.session_id())?;

        Ok(attachment)
    }

    fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let mut sessions = self.session_store.write();
        let agent = self
            .agent_store
            .focus_agent(session_id, agent_id, &mut sessions)?;
        drop(sessions);
        if !self.should_defer_provider_run_sync_for_focus_change(session_id, agent_id)? {
            self.sync_active_provider_run_for_agent(session_id, agent_id)?;
        }
        Ok(agent)
    }

    fn cycle_agent_focus(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        let mut sessions = self.session_store.write();
        let agent = self.agent_store.cycle_focus(session_id, &mut sessions)?;
        drop(sessions);
        if let Some(focused) = agent.as_ref() {
            if !self.should_defer_provider_run_sync_for_focus_change(session_id, focused.id())? {
                self.sync_active_provider_run_for_agent(session_id, focused.id())?;
            }
        }
        Ok(agent)
    }

    fn resize_terminal(&self, session_id: &str) -> Result<Option<String>, DaemonError> {
        let provider_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        let _ = self.reconcile_provider_run_liveness_provider_phase(
            session_id,
            &provider_run_id,
            None,
        )?;
        let provider_run = self.ensure_provider_run_in_session(session_id, &provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id,
                state: provider_run.state(),
                operation: "resize terminal",
            });
        }
        if provider_run.endpoint_mode() == crate::provider::AgentEndpointMode::External {
            return Ok(None);
        }
        Ok(Some(provider_run.id().to_string()))
    }

    fn end_session(
        &self,
        session_id: &str,
    ) -> Result<(crate::session::RuntimeSession, Vec<String>), DaemonError> {
        let session = self.session_store.get_session(session_id)?;

        if session.status() == crate::session::SessionStatus::Ended {
            self.prompt_state_owner.remove_session(session_id);
            let ended = self.session_store.end_session(session_id)?;
            return Ok((ended, Vec::new()));
        }

        let removed_attachments = self.attachment_store.remove_session_attachments(session_id);
        let terminated_runs = self
            .provider_store
            .terminate_session_runs_provider_only(session_id)?;
        let terminated_run_ids = terminated_runs
            .runs()
            .iter()
            .map(|outcome| outcome.run().id().to_string())
            .collect::<Vec<_>>();
        for outcome in terminated_runs.into_runs() {
            if self
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(outcome.run().id())
            {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            self.provider_run_projection.update(outcome.into_run());
        }

        let removed_agents = self.agent_store.remove_session_agents(session_id);
        let removed_agent_ids: Vec<_> = removed_agents
            .iter()
            .map(|agent| format!("{} ({})", agent.agent_ref(), agent.id()))
            .collect();

        for run in self.provider_store.list_runs() {
            if run.session_id() == session_id {
                self.clear_prompt_activity(run.id());
            }
        }
        self.prompt_state_owner.remove_session(session_id);
        let mut ended = self.session_store.end_session(session_id)?;
        ended.set_agents(removed_agents);
        crate::logging::info_with_fields(
            "daemon.session",
            "session ended",
            serde_json::json!({
                "session_id": session_id,
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        );
        Ok((ended, terminated_run_ids))
    }

    fn delete_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<(crate::session::RuntimeSession, Vec<String>), DaemonError> {
        let session = self
            .session_store
            .read()
            .resolve_session_ref(session_ref, workspace_id)?;
        let session_id = session.id().to_string();
        let (ended, terminated_run_ids) = self.end_session(&session_id)?;
        let mut deleted = self.session_store.delete_session(ended.id())?;
        deleted.set_agents(ended.agents().to_vec());
        self.history_projection.remove(deleted.id());
        self.session_projection.remove(deleted.id());
        crate::logging::info_with_fields(
            "daemon.session",
            "session deleted",
            serde_json::json!({
                "session_id": deleted.id(),
                "session_alias": deleted.alias(),
            }),
        );
        Ok((deleted, terminated_run_ids))
    }

    fn should_defer_provider_run_sync_for_focus_change(
        &self,
        session_id: &str,
        target_agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.session_snapshot(session_id)?;
        let Some(active_provider_run_id) = session.active_provider_run_id().map(str::to_string)
        else {
            return Ok(false);
        };
        let active_run = self.provider_store.get_run(&active_provider_run_id)?;
        if active_run.agent_instance_id() == Some(target_agent_id)
            || active_run.state() != crate::provider::ProviderRunState::Running
        {
            return Ok(false);
        }

        Ok(self
            .prompt_state_owner
            .active_prompt_agent_id(&session)
            .is_some()
            || session.agents().iter().any(|agent| agent.is_processing()))
    }

    fn sync_active_provider_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let current_active_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);

        if let Some(current_active_run_id) = current_active_run_id.as_deref() {
            let active_run = self.provider_store.get_run(current_active_run_id)?;
            if active_run.agent_instance_id() != Some(agent_id)
                && active_run.state() == crate::provider::ProviderRunState::Running
            {
                let outcome = self
                    .provider_store
                    .park_run_provider_only(session_id, current_active_run_id)?;
                self.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
                self.provider_run_projection.update(outcome.into_run());
            }
        }

        if let Some(agent_run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
            match agent_run.state() {
                crate::provider::ProviderRunState::Running
                | crate::provider::ProviderRunState::Starting => {
                    self.session_store
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                crate::provider::ProviderRunState::Parked => {
                    let _ = self.resume_provider_run_for_session(session_id, agent_run.id())?;
                }
                crate::provider::ProviderRunState::Ended => {
                    self.session_store
                        .set_active_provider_run(session_id, None)?;
                }
            }
        } else {
            self.session_store
                .set_active_provider_run(session_id, None)?;
        }

        Ok(())
    }

    fn sync_focused_provider_run_if_idle(&self, session_id: &str) -> Result<(), DaemonError> {
        let session = self.session_snapshot(session_id)?;
        if session.agents().len() > 1 {
            let focused_agent_id = session.focused_agent_id().map(str::to_string);
            if let Some(focused_agent_id) = focused_agent_id {
                if self
                    .prompt_state_owner
                    .active_prompt_agent_id(&session)
                    .is_none()
                {
                    let current_active_run_id =
                        session.active_provider_run_id().map(str::to_string);
                    if let Some(current_active_run_id) = current_active_run_id.as_deref() {
                        let active_run = self.provider_store.get_run(current_active_run_id)?;
                        if active_run.agent_instance_id() != Some(focused_agent_id.as_str())
                            && active_run.state() == crate::provider::ProviderRunState::Running
                        {
                            let outcome = self
                                .provider_store
                                .park_run_provider_only(session_id, current_active_run_id)?;
                            self.clear_active_provider_run_session_pointer(
                                session_id,
                                outcome.run().id(),
                            )?;
                            self.provider_run_projection.update(outcome.into_run());
                        }
                    }
                }
                self.project_active_provider_run_for_agent(session_id, &focused_agent_id)?;
            } else {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            return Ok(());
        }

        if self
            .prompt_state_owner
            .active_prompt_agent_id(&session)
            .is_some()
            || session.agents().iter().any(|agent| agent.is_processing())
        {
            return Ok(());
        }

        if let Some(focused_agent_id) = session.focused_agent_id() {
            self.sync_active_provider_run_for_agent(session_id, focused_agent_id)?;
        } else {
            self.session_store
                .set_active_provider_run(session_id, None)?;
        }
        Ok(())
    }

    fn project_active_provider_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let projected_run_id = self
            .provider_store
            .get_run_for_agent(session_id, agent_id)
            .map(|run| run.id().to_string());
        self.session_store
            .set_active_provider_run(session_id, projected_run_id)?;
        Ok(())
    }

    fn mirror_prompt_owner_session_state(&self, session_id: &str) -> Result<(), DaemonError> {
        let mut agent_ids = self
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        let session = self.session_store.get_session(session_id)?;
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.sort();
        agent_ids.dedup();
        for agent_id in agent_ids {
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, &agent_id);
            self.session_store.mirror_agent_prompt_state(
                session_id,
                &agent_id,
                active_prompt,
                queued_prompts,
            )?;
        }
        Ok(())
    }

    fn activate_next_queued_prompt_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self.prompt_state_owner.activate_next_queued_prompt(
            &session,
            agent_id,
            expected_prompt_id,
        )?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        Ok(prompt)
    }

    fn advance_next_queued_prompt_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<crate::app::KernelPromptDispatch>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let Some(next_prompt) = self
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id)
        else {
            return Ok(None);
        };
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "advance queued prompt",
            });
        }
        self.acquire_provider_prompt_claim(
            session_id,
            provider_run_id,
            agent_id,
            Some(next_prompt.source_attachment_id()),
        )?;
        let started_next = self
            .prompt_state_owner
            .activate_next_queued_prompt(&session, agent_id, Some(next_prompt.id()))?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "advance queued prompt",
                message: format!(
                    "expected queued prompt `{}` but no queued prompt was available",
                    next_prompt.id()
                ),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
            started_next.workflow_run_id(),
            started_next.workflow_node_run_id(),
        ) {
            let _ = self.session_store.write().mark_workflow_turn_dispatched(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            let _ = self.workflow_start_prompt(session_id, &started_next)?;
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(Some(crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.to_string(),
            source_attachment_id: started_next.source_attachment_id().to_string(),
            prompt: started_next.prompt().to_string(),
            attachments: started_next.attachments().to_vec(),
        }))
    }

    fn start_provider_launch(
        &self,
        request: crate::provider::LaunchProviderRequest,
    ) -> Result<crate::app::StartedProviderLaunch, DaemonError> {
        let session_id = request.session_id.clone();
        let previous_active_run_id = self
            .session_store
            .get_session(&session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = previous_active_run_id.as_deref() {
            let active_run = self.provider_store.get_run(active_run_id)?;
            match active_run.state() {
                crate::provider::ProviderRunState::Ended => {
                    self.session_store
                        .set_active_provider_run(&session_id, None)?;
                    self.provider_store.clear_runtime(active_run_id);
                }
                crate::provider::ProviderRunState::Starting => {
                    let outcome = self
                        .provider_store
                        .terminate_run_provider_only(&session_id, active_run_id)?;
                    self.clear_active_provider_run_session_pointer(
                        &session_id,
                        outcome.run().id(),
                    )?;
                    self.provider_run_projection.update(outcome.into_run());
                }
                crate::provider::ProviderRunState::Running => {
                    let outcome = self
                        .provider_store
                        .park_run_provider_only(&session_id, active_run_id)?;
                    self.clear_active_provider_run_session_pointer(
                        &session_id,
                        outcome.run().id(),
                    )?;
                    self.provider_run_projection.update(outcome.into_run());
                }
                crate::provider::ProviderRunState::Parked => {
                    self.session_store
                        .set_active_provider_run(&session_id, None)?;
                }
            }
        }

        let outcome = self.provider_store.start_run_provider_only(request)?;
        self.session_store
            .set_active_provider_run(&session_id, Some(outcome.run().id().to_string()))?;
        Ok(crate::app::StartedProviderLaunch {
            run: outcome.into_run(),
            previous_active_run_id,
        })
    }

    fn resume_provider_run_for_session(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let active_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_owned);

        if let Some(active_run_id) = active_run_id.as_deref() {
            if active_run_id != run_id {
                let outcome = self
                    .provider_store
                    .park_run_provider_only(session_id, active_run_id)?;
                self.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
                self.provider_run_projection.update(outcome.into_run());
            }
        }

        let outcome = self
            .provider_store
            .resume_run_provider_only(session_id, run_id)?;
        self.session_store
            .set_active_provider_run(session_id, Some(outcome.run().id().to_string()))?;
        let run = outcome.into_run();
        self.provider_run_projection.update(run.clone());
        Ok(run)
    }

    fn clear_active_provider_run_session_pointer(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        if self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            == Some(provider_run_id)
        {
            self.session_store
                .set_active_provider_run(session_id, None)?;
        }
        Ok(())
    }

    fn finish_provider_launch_success(
        &self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        if let Some(binding) = binding {
            self.provider_store
                .apply_runtime_binding(started.run.id(), binding)?;
        }
        let run = self.provider_store.mark_run_running(started.run.id())?;
        self.session_store
            .set_active_provider_run(run.session_id(), Some(run.id().to_string()))?;
        let _ = self.session_snapshot(run.session_id())?;
        crate::logging::info_with_fields(
            "daemon.app",
            "initializing provider runtime",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        crate::logging::info_with_fields(
            "daemon.app",
            "provider runtime initialized successfully",
            serde_json::json!({
                "provider_run_id": run.id(),
                "session_id": run.session_id(),
            }),
        );
        let _ = self.provider_store.record_run_activity(run.id());
        if let Some(agent_id) = run.agent_instance_id() {
            let _ = self.agent_store.set_agent_runtime_profile(
                agent_id,
                run.provider(),
                Some(run.model().to_string()),
                run.variant().map(str::to_string),
                run.resume_state().clone(),
            )?;
        }
        self.provider_run_projection.update(run.clone());
        Ok(run)
    }

    fn cancel_active_prompt_only(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let cancelled = self
            .prompt_state_owner
            .cancel_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        Ok(cancelled)
    }

    fn complete_local_prompt_without_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<Option<OwnedPromptCompletion>, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        let _active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;

        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;

        let completion_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        let completion_record_key = provider_run_id.unwrap_or(agent_id);
        if !self.prompt_completion_recorded(completion_record_key) {
            let provider_run_id = completion_provider_run_id
                .as_deref()
                .unwrap_or("provider-run-completed");
            let recipient_attachment_ids = self
                .attachment_store
                .list_session_attachment_ids(session_id);
            self.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completed.id()),
                crate::session::unix_epoch_ms(),
            );
            self.mark_prompt_completion_recorded(provider_run_id);
        }
        let released_claim = completion_provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let _ = self.session_snapshot(session_id)?;

        Ok(Some(OwnedPromptCompletion {
            completion: crate::session::PromptCompletion {
                completed,
                started_next: None,
            },
            released_claim,
            dispatch: None,
        }))
    }

    fn submit_local_prepared_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let session_id = prepared.session_id.clone();
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
            let _ = self.ensure_attachment_in_session(&session_id, &attachment_id)?;
        }
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        let target_agent = self.agent_store.get_agent(&target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(&session_id)?;
        let queued_while_active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &target_agent_id)
            .is_some();
        let provider_run_id = self
            .provider_store
            .get_run_for_agent(&session_id, &target_agent_id)
            .map(|run| run.id().to_string());
        if !queued_while_active && provider_run_id.is_none() {
            return Ok(None);
        }
        if !queued_while_active {
            if let Some(provider_run_id) = provider_run_id.as_deref() {
                let provider_run =
                    self.ensure_provider_run_in_session(&session_id, provider_run_id)?;
                if provider_run.state() == crate::provider::ProviderRunState::Parked {
                    let _ = self.resume_provider_run_for_session(&session_id, provider_run_id)?;
                }
            }
        }
        let provider_run_is_starting = provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.provider_store.get_run(provider_run_id).ok())
            .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Starting);

        self.append_user_prompt_history(
            &session_id,
            &attachment_id,
            &target_agent_id,
            prepared.prompt.prompt(),
            prepared.prompt.attachments(),
        )?;
        let force_queue = prepared.force_queue || provider_run_is_starting;
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prepared.prompt.clone(),
            force_queue,
        );
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.session_store.mirror_agent_prompt_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;

        let mut dispatch = None;
        match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => {
                let provider_run_id =
                    provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: session_id.clone(),
                        })?;
                self.echo_prompt_to_other_attachments(
                    &session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                if let Err(error) = self.acquire_provider_prompt_claim(
                    &session_id,
                    provider_run_id,
                    &target_agent_id,
                    Some(prompt.source_attachment_id()),
                ) {
                    let _ = self.cancel_active_prompt_only(&session_id, &target_agent_id);
                    let _ = self.clear_prompt_activity(provider_run_id);
                    return Err(error);
                }
                dispatch = Some(crate::app::KernelPromptDispatch {
                    session_id: session_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: target_agent_id.clone(),
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                });
            }
            crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                let queue_depth = self
                    .prompt_state_owner
                    .queued_prompt_count_for_agent(&session, &target_agent_id);
                if let Some(provider_run_id) = provider_run_id.as_deref() {
                    self.echo_prompt_to_other_attachments(
                        &session_id,
                        provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                }
                self.record_notice(
                    &session_id,
                    provider_run_id.as_deref(),
                    self.other_attachment_ids(&session_id, &attachment_id),
                    format!(
                        "A queued message from attachment `{}` was added to agent `{}` in session `{}` as `{}`. Queue depth is now {}.",
                        attachment_id,
                        target_agent_id,
                        session_id,
                        prompt.id(),
                        queue_depth
                    ),
                );
            }
        }
        let session = self.session_snapshot(&session_id)?;
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome,
            session,
            dispatch,
            remote_dispatch: None,
        }))
    }

    fn complete_local_prompt_with_queued_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
        next_queued_prompt: &crate::session::PromptQueueItem,
    ) -> Result<Option<OwnedPromptCompletion>, DaemonError> {
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(
            next_queued_prompt.source_attachment_id(),
        ) {
            let _ = self.ensure_attachment_in_session(
                session_id,
                next_queued_prompt.source_attachment_id(),
            )?;
        }
        let provider_run_id = provider_run_id
            .map(str::to_string)
            .or_else(|| {
                self.provider_store
                    .get_run_for_agent(session_id, agent_id)
                    .map(|run| run.id().to_string())
            })
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self.ensure_provider_run_in_session(session_id, &provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Ok(None);
        }
        self.acquire_provider_prompt_claim(
            session_id,
            &provider_run_id,
            agent_id,
            Some(next_queued_prompt.source_attachment_id()),
        )?;

        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let started_next = self
            .prompt_state_owner
            .activate_next_queued_prompt(&session, agent_id, Some(next_queued_prompt.id()))?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "advance queued prompt",
                message: format!(
                    "expected queued prompt `{}` but no queued prompt was available",
                    next_queued_prompt.id()
                ),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        if self
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            if let Err(error) = self.provider_store.enqueue_structured_prompt_submit(
                session_id.to_string(),
                provider_run_id.clone(),
                agent_id.to_string(),
                &provider_run,
                started_next.prompt(),
                started_next.attachments(),
            ) {
                let _ = self.cancel_active_prompt_only(session_id, agent_id);
                let _ = self.clear_prompt_activity(&provider_run_id);
                return Err(error);
            }
            self.note_prompt_started(&provider_run_id);
            let _ = self.session_snapshot(session_id)?;
            return Ok(Some(OwnedPromptCompletion {
                completion: crate::session::PromptCompletion {
                    completed,
                    started_next: Some(started_next),
                },
                released_claim: false,
                dispatch: None,
            }));
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(Some(OwnedPromptCompletion {
            completion: crate::session::PromptCompletion {
                completed,
                started_next: Some(started_next.clone()),
            },
            released_claim: false,
            dispatch: Some(crate::app::KernelPromptDispatch {
                session_id: session_id.to_string(),
                provider_run_id,
                agent_id: agent_id.to_string(),
                source_attachment_id: started_next.source_attachment_id().to_string(),
                prompt: started_next.prompt().to_string(),
                attachments: started_next.attachments().to_vec(),
            }),
        }))
    }

    fn finalize_local_prompt_cancellation_with_queued_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<OwnedPromptCancellation, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .finalize_active_prompt_cancellation(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        let released_claim = provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let started_next = if self
            .prompt_state_owner
            .active_prompt_for_agent(&self.session_store.get_session(session_id)?, agent_id)
            .is_none()
        {
            let next_prompt = self
                .prompt_state_owner
                .peek_next_queued_prompt(&self.session_store.get_session(session_id)?, agent_id);
            if let (Some(provider_run_id), Some(next_prompt)) =
                (provider_run_id.as_deref(), next_prompt.as_ref())
            {
                let provider_run =
                    self.ensure_provider_run_in_session(session_id, provider_run_id)?;
                if provider_run.state() == crate::provider::ProviderRunState::Running {
                    self.acquire_provider_prompt_claim(
                        session_id,
                        provider_run_id,
                        agent_id,
                        Some(next_prompt.source_attachment_id()),
                    )?;
                    self.prompt_state_owner.activate_next_queued_prompt(
                        &self.session_store.get_session(session_id)?,
                        agent_id,
                        Some(next_prompt.id()),
                    )?
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&self.session_store.get_session(session_id)?, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        if started_next.is_none() {
            self.sync_focused_provider_run_if_idle(session_id)?;
        }
        let dispatch = if let (Some(provider_run_id), Some(started_next)) =
            (provider_run_id.as_deref(), started_next.as_ref())
        {
            let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
            if self
                .provider_store
                .run_uses_structured_prompt_io(&provider_run)
            {
                self.provider_store.enqueue_structured_prompt_submit(
                    session_id.to_string(),
                    provider_run_id.to_string(),
                    agent_id.to_string(),
                    &provider_run,
                    started_next.prompt(),
                    started_next.attachments(),
                )?;
                self.note_prompt_started(provider_run_id);
                None
            } else {
                Some(crate::app::KernelPromptDispatch {
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: agent_id.to_string(),
                    source_attachment_id: started_next.source_attachment_id().to_string(),
                    prompt: started_next.prompt().to_string(),
                    attachments: started_next.attachments().to_vec(),
                })
            }
        } else {
            None
        };
        let _ = self.session_snapshot(session_id)?;
        Ok(OwnedPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next,
            },
            released_claim,
            dispatch,
        })
    }

    fn cancel_local_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<Option<crate::app::KernelPromptCancellation>, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        let active_prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            let session = self.session_snapshot(session_id)?;
            return Ok(Some(crate::app::KernelPromptCancellation {
                cancellation: crate::session::PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
                dispatch: None,
            }));
        }

        let provider_run = self
            .provider_run_projection
            .get_for_agent(session_id, target_agent_id)
            .or_else(|| {
                self.provider_store
                    .get_run_for_agent(session_id, target_agent_id)
            })
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run.id())?;

        let prompt = self
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, target_agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            target_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        self.note_prompt_settlement_requested(provider_run.id());
        let recipients = self.other_attachment_ids(session_id, attachment_id);
        self.record_notice(
            session_id,
            Some(provider_run.id()),
            recipients,
            format!(
                "Attachment `{}` requested cancellation of active prompt `{}` on provider run `{}`.",
                attachment_id,
                prompt.id(),
                provider_run.id()
            ),
        );
        let session = self.session_snapshot(session_id)?;

        Ok(Some(crate::app::KernelPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch: Some(crate::app::KernelPromptAbortDispatch {
                session_id: session_id.to_string(),
                provider_run_id: provider_run.id().to_string(),
                source_attachment_id: attachment_id.to_string(),
            }),
        }))
    }

    fn submit_remote_prepared_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let session_id = prepared.session_id.clone();
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
            let _ = self.ensure_attachment_in_session(&session_id, &attachment_id)?;
        }
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        let target_agent = self.agent_store.get_agent(&target_agent_id)?;
        let Some(remote_execution) = target_agent.remote_execution().cloned() else {
            return Ok(None);
        };
        self.append_user_prompt_history(
            &session_id,
            &attachment_id,
            &target_agent_id,
            prepared.prompt.prompt(),
            prepared.prompt.attachments(),
        )?;
        let session = self.session_store.get_session(&session_id)?;
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prepared.prompt.clone(),
            prepared.force_queue,
        );
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.session_store.mirror_agent_prompt_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let remote_dispatch =
            if let crate::session::PromptSubmissionOutcome::Started { prompt } = &outcome {
                Some(crate::app::KernelRemotePromptDispatch {
                    session_id: session_id.clone(),
                    agent_id: target_agent_id,
                    worker_kernel_id: remote_execution.worker_kernel_id,
                    leased_agent_id: remote_execution.leased_agent_id,
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    workflow_context: None,
                })
            } else {
                None
            };
        let session = self.session_snapshot(&session_id)?;
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome,
            session,
            dispatch: None,
            remote_dispatch,
        }))
    }

    fn complete_remote_prompt_owner(
        &self,
        session_id: &str,
        agent_id: &str,
        remote_provider_run_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        self.record_assistant_message_completion(
            session_id,
            remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completed.id()),
            crate::session::unix_epoch_ms(),
        );
        let started_next = if self
            .prompt_state_owner
            .active_prompt_for_agent(&self.session_store.get_session(session_id)?, agent_id)
            .is_none()
        {
            if let Some(expected_next) = next_queued_prompt {
                let session = self.session_store.get_session(session_id)?;
                let active = self.prompt_state_owner.activate_next_queued_prompt(
                    &session,
                    agent_id,
                    Some(expected_next.id()),
                )?;
                let (active_prompt, queued_prompts) =
                    self.prompt_state_owner.state_parts(&session, agent_id);
                self.session_store.mirror_agent_prompt_state(
                    session_id,
                    agent_id,
                    active_prompt,
                    queued_prompts,
                )?;
                active
            } else {
                None
            }
        } else {
            None
        };
        let _ = self.session_snapshot(session_id)?;
        Ok(crate::session::PromptCompletion {
            completed,
            started_next,
        })
    }

    fn begin_remote_prompt_cancellation(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let session = self.session_store.get_session(session_id)?;
        let active_prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            let session = self.session_snapshot(session_id)?;
            return Ok(crate::app::KernelPromptCancellation {
                cancellation: crate::session::PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
                dispatch: None,
            });
        }
        let prompt = self
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, target_agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            target_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let worker_kernel_id = self
            .agent_store
            .get_agent(target_agent_id)?
            .remote_execution()
            .map(|remote| remote.worker_kernel_id.clone())
            .unwrap_or_else(|| "remote".to_string());
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                prompt.id(),
                worker_kernel_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch: None,
        })
    }

    fn other_attachment_ids(&self, session_id: &str, attachment_id: &str) -> Vec<String> {
        self.attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|id| id != attachment_id)
            .collect()
    }

    fn prompt_completion_recorded(&self, provider_run_id: &str) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .map(|state| state.completion_recorded)
            .unwrap_or(false)
    }

    fn mark_prompt_completion_recorded(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.completion_recorded = true;
        }
    }

    fn record_assistant_message_completion(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        self.terminal_stream.record_assistant_message_completion(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
    }

    fn reap_structured_prompt_jobs(&self) {
        self.provider_store
            .apply_finished_provider_run_selection_sync_jobs();
        for finished in self
            .provider_store
            .drain_finished_structured_prompt_submit_jobs()
        {
            if let Err(error) = finished.result {
                let _ = self.cancel_active_prompt_only(&finished.session_id, &finished.agent_id);
                let _ = self.session_snapshot(&finished.session_id);
                let recipients = self
                    .attachment_store
                    .list_session_attachment_ids(&finished.session_id);
                self.record_notice(
                    &finished.session_id,
                    Some(&finished.provider_run_id),
                    recipients,
                    format!("Prompt dispatch failed after acknowledgement: {error}"),
                );
            }
        }
        for finished in self
            .provider_store
            .drain_finished_structured_prompt_abort_jobs()
        {
            if let Err(error) = finished.result {
                let recipients = self
                    .attachment_store
                    .list_session_attachment_ids(&finished.session_id);
                self.record_notice(
                    &finished.session_id,
                    Some(&finished.provider_run_id),
                    recipients,
                    format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
                );
            }
        }
    }

    fn fan_out_terminal_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        kind: crate::terminal::TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> crate::terminal::TerminalOutputRecord {
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        let record = self.terminal_stream.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            kind.clone(),
            merge_key.clone(),
            recipient_attachment_ids,
            bytes,
        );
        if kind != crate::terminal::TerminalOutputKind::PromptEcho {
            self.append_history_entry(
                session_id,
                SessionHistoryEntry::provider_output(
                    session_id,
                    provider_run_id,
                    agent_id.as_deref(),
                    kind,
                    merge_key,
                    String::from_utf8_lossy(bytes).into_owned(),
                ),
            );
        }
        record
    }

    fn append_history_entry(&self, session_id: &str, entry: SessionHistoryEntry) {
        let session = match self.session_store.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping provider-output history append because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if let Err(error) = self.history_store.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append provider-output session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        } else {
            self.history_projection.append(entry);
        }
    }

    fn clear_prompt_activity(&self, provider_run_id: &str) -> bool {
        self.prompt_activity.write().remove(provider_run_id);
        self.prompt_workspace_claims.remove(provider_run_id)
    }

    fn release_workflow_node_workspace_claim(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> bool {
        let owner = format!("{workflow_run_id}:{workflow_node_run_id}");
        self.prompt_workspace_claims.remove_matching(|claim| {
            claim.session_id == session_id
                && claim.attachment_id.as_deref() == Some(owner.as_str())
                && claim.operation == "workflow_node_dispatch"
        }) > 0
    }

    fn note_prompt_started(&self, provider_run_id: &str) {
        self.prompt_activity.write().insert(
            provider_run_id.to_string(),
            crate::app::ActivePromptState {
                last_output_at: None,
                saw_response_content: false,
                completion_recorded: false,
            },
        );
    }

    fn note_prompt_output(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
        }
    }

    fn note_prompt_response_content(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
            state.saw_response_content = true;
        }
    }

    fn note_prompt_settlement_requested(&self, provider_run_id: &str) {
        self.prompt_activity
            .write()
            .entry(provider_run_id.to_string())
            .and_modify(|state| {
                state.last_output_at = Some(Instant::now());
                state.saw_response_content = true;
            })
            .or_insert(crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: false,
            });
    }

    fn prompt_should_settle(&self, provider_run_id: &str) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .map(|state| {
                (state.saw_response_content || state.completion_recorded)
                    && state
                        .last_output_at
                        .map(|last_output_at| last_output_at.elapsed() >= self.prompt_idle_timeout)
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    fn acquire_provider_prompt_claim(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains(provider_run_id) {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_provider_prompt_claim(
            workspace_id,
            worktree_id,
            session_id,
            attachment_id.map(str::to_string),
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    fn acquire_workflow_node_workspace_claim(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains(provider_run_id) {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_worktree_write_claim(
            workspace_id,
            worktree_id,
            session_id,
            Some(format!("{workflow_run_id}:{workflow_node_run_id}")),
            "workflow_node_dispatch",
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    fn append_user_prompt_history(
        &self,
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
    ) -> Result<(), DaemonError> {
        let session = self.session_snapshot(session_id)?;
        let entry = crate::history::SessionHistoryEntry::user_prompt(
            session_id,
            source_attachment_id,
            agent_id,
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments),
        );
        self.history_store.append(&session, &entry)?;
        self.history_projection.append(entry);
        Ok(())
    }

    fn echo_prompt_to_other_attachments(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
    ) {
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|attachment_id| attachment_id != source_attachment_id)
            .collect::<Vec<_>>();
        if recipient_attachment_ids.is_empty() {
            return;
        }
        let mut bytes =
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments).into_bytes();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        self.terminal_stream.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            crate::terminal::TerminalOutputKind::PromptEcho,
            None,
            recipient_attachment_ids,
            &bytes,
        );
    }

    fn ensure_provider_run_in_session(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let provider_run = self.provider_store.get_run(provider_run_id)?;
        if provider_run.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.to_string(),
            });
        }
        Ok(provider_run)
    }

    fn remove_provider_process_tracking_for_run(
        &self,
        provider_run_id: &str,
        pty_process_key: Option<String>,
    ) {
        let process_key = self
            .provider_process_tracking
            .read()
            .run_processes
            .get(provider_run_id)
            .cloned()
            .or(pty_process_key);
        let Some(process_key) = process_key else {
            return;
        };
        let mut tracking = self.provider_process_tracking.write();
        tracking.run_processes.remove(provider_run_id);
        let should_remove_entry = if let Some(entry) = tracking.processes.get_mut(&process_key) {
            entry
                .owner_provider_run_ids
                .retain(|id| id != provider_run_id);
            entry.owner_provider_run_ids.is_empty()
        } else {
            false
        };
        if should_remove_entry {
            tracking.processes.remove(&process_key);
        }
    }

    fn reconcile_provider_run_liveness_provider_phase(
        &self,
        session_id: &str,
        provider_run_id: &str,
        process_running: Option<bool>,
    ) -> Result<Option<OwnedProviderRunExit>, DaemonError> {
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let _ = provider_run
            .agent_instance_id()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        let reconciliation = self.provider_store.reconcile_run_liveness_provider_only(
            session_id,
            provider_run_id,
            process_running,
        )?;
        match reconciliation {
            crate::provider::ProviderRunLivenessReconciliation::AlreadyEnded(run) => {
                self.clear_active_provider_run_session_pointer(session_id, provider_run_id)?;
                self.provider_run_projection.update(run.clone());
                Ok(Some(OwnedProviderRunExit {
                    ended_run: run,
                    already_ended: true,
                }))
            }
            crate::provider::ProviderRunLivenessReconciliation::NewlyEnded(run) => {
                self.clear_active_provider_run_session_pointer(session_id, provider_run_id)?;
                self.provider_run_projection.update(run.clone());
                Ok(Some(OwnedProviderRunExit {
                    ended_run: run,
                    already_ended: false,
                }))
            }
            crate::provider::ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | crate::provider::ProviderRunLivenessReconciliation::StillRunning(_) => Ok(None),
        }
    }

    fn record_notice(
        &self,
        session_id: &str,
        provider_run_id: Option<&str>,
        recipient_attachment_ids: Vec<String>,
        message: impl Into<String>,
    ) {
        let message = message.into();
        let agent_id = provider_run_id.and_then(|run_id| {
            self.provider_store
                .get_run(run_id)
                .ok()
                .and_then(|run| run.agent_instance_id().map(str::to_string))
        });
        self.terminal_stream.record_notice(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message.clone(),
        );
        let session = match self.session_store.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping history append because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        let entry =
            SessionHistoryEntry::notice(session_id, provider_run_id, agent_id.as_deref(), message);
        if let Err(error) = self.history_store.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        } else {
            self.history_projection.append(entry);
        }
    }

    fn workflow_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.session_snapshot(session_id)
    }

    fn workflow_create_workflow(
        &self,
        request: crate::local::CreateWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .create_workflow(&request.session_id, request.alias)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
    }

    fn workflow_alias_workflow(
        &self,
        request: crate::local::AliasWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self.session_store.write().assign_workflow_alias(
            &request.session_id,
            &request.workflow_ref,
            request.alias,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
    }

    fn workflow_list_workflows(
        &self,
        request: crate::local::ListWorkflowsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowsListed {
            workflows: self
                .session_store
                .read()
                .list_workflows(&request.session_id)?,
        })
    }

    fn workflow_resolve_workflow(
        &self,
        request: crate::local::ResolveWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowResolved {
            workflow: self
                .session_store
                .read()
                .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
        })
    }

    fn workflow_create_endpoint(
        &self,
        request: crate::local::CreateWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.session_store.write().create_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.entry_node_id,
            request.alias,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointCreated {
            endpoint,
            workflow,
            session,
        })
    }

    fn workflow_alias_endpoint(
        &self,
        request: crate::local::AliasWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.session_store.write().assign_workflow_endpoint_alias(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.alias,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointAliased {
            endpoint,
            workflow,
            session,
        })
    }

    fn workflow_bind_endpoint(
        &self,
        request: crate::local::BindWorkflowEndpointRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = self.session_store.write().bind_workflow_endpoint(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            &request.entry_node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEndpointBound {
            endpoint,
            workflow,
            session,
        })
    }

    fn workflow_add_node(
        &self,
        request: crate::local::AddWorkflowNodeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if self
            .agent_store
            .get_session_agents(&request.session_id)
            .into_iter()
            .all(|agent| agent.id() != request.agent_id)
        {
            return Err(DaemonError::AgentNotFound {
                agent_id: request.agent_id,
            });
        }
        let node = self.session_store.write().add_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.agent_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeAdded {
            node,
            workflow,
            session,
        })
    }

    fn workflow_remove_node(
        &self,
        request: crate::local::RemoveWorkflowNodeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.session_store.write().remove_workflow_node(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeRemoved {
            node,
            workflow,
            session,
        })
    }

    fn workflow_update_node_instructions(
        &self,
        request: crate::local::UpdateWorkflowNodeInstructionsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .session_store
            .write()
            .update_workflow_node_instructions(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.instructions,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
            node,
            workflow,
            session,
        })
    }

    fn workflow_set_node_can_complete_run(
        &self,
        request: crate::local::SetWorkflowNodeCanCompleteRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_complete_run(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_complete_workflow_run,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
            node,
            workflow,
            session,
        })
    }

    fn workflow_set_node_can_emit_intermediate_output(
        &self,
        request: crate::local::SetWorkflowNodeCanEmitIntermediateOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .session_store
            .write()
            .set_workflow_node_can_emit_intermediate_output(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.can_emit_intermediate_workflow_run_output,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    fn workflow_set_node_intermediate_output_schema(
        &self,
        request: crate::local::SetWorkflowNodeIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self
            .session_store
            .write()
            .set_workflow_node_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                &request.node_id,
                request.intermediate_output_schema_ref,
            )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(
            LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                node,
                workflow,
                session,
            },
        )
    }

    fn workflow_set_node_max_turns(
        &self,
        request: crate::local::SetWorkflowNodeMaxTurnsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let node = self.session_store.write().set_workflow_node_max_turns(
            &request.session_id,
            &request.workflow_ref,
            &request.node_id,
            request.max_turns,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
            node,
            workflow,
            session,
        })
    }

    fn workflow_add_edge(
        &self,
        request: crate::local::AddWorkflowEdgeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let edge = self.session_store.write().add_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.from_node_id,
            &request.to_node_id,
            request.output_schema_ref,
            request.validation_policy,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeAdded {
            edge,
            workflow,
            session,
        })
    }

    fn workflow_remove_edge(
        &self,
        request: crate::local::RemoveWorkflowEdgeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let edge = self.session_store.write().remove_workflow_edge(
            &request.session_id,
            &request.workflow_ref,
            &request.edge_id,
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
            edge,
            workflow,
            session,
        })
    }

    fn workflow_set_flush_context(
        &self,
        request: crate::local::SetWorkflowFlushContextRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .set_workflow_flush_agent_context_before_run(
                &request.session_id,
                &request.workflow_ref,
                request.flush_agent_context_before_run,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
    }

    fn workflow_set_run_output_schema(
        &self,
        request: crate::local::SetWorkflowRunOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .set_workflow_run_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.run_output_schema_ref,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
    }

    fn workflow_set_intermediate_output_schema(
        &self,
        request: crate::local::SetWorkflowIntermediateOutputSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow = self
            .session_store
            .write()
            .set_workflow_intermediate_output_schema_ref(
                &request.session_id,
                &request.workflow_ref,
                request.intermediate_output_schema_ref,
            )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { workflow, session })
    }

    fn workflow_set_launch_policy(
        &self,
        request: crate::local::SetWorkflowLaunchPolicyRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self
            .session_store
            .write()
            .set_workflow_launch_policy(&request.session_id, request.policy)?;
        let mut session = session;
        session.set_agents(self.agent_store.get_session_agents(&request.session_id));
        self.project_session_runtime_view(&mut session);
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
    }

    fn workflow_list_runs(
        &self,
        request: crate::local::ListWorkflowRunsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRunsListed {
            workflow_runs: self
                .session_store
                .read()
                .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    fn workflow_get_run(
        &self,
        request: crate::local::GetWorkflowRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowRun {
            workflow_run: self
                .session_store
                .read()
                .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
        })
    }

    fn workflow_create_watchdog(
        &self,
        request: crate::local::CreateWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().create_workflow_watchdog(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
            request.interval_seconds,
            request.invocation_prompt,
            request.policy,
            if request.max_wakeups_configured {
                Some(request.max_wakeups)
            } else {
                None
            },
        )?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            &request.session_id,
            &request.workflow_ref,
            &request.endpoint_ref,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
            watchdog,
            workflow,
            endpoint,
            session,
        })
    }

    fn workflow_list_watchdogs(
        &self,
        request: crate::local::ListWorkflowWatchdogsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
            watchdogs: self
                .session_store
                .read()
                .list_workflow_watchdogs(&request.session_id, request.workflow_ref.as_deref())?,
        })
    }

    fn workflow_set_watchdog_enabled(
        &self,
        request: crate::local::SetWorkflowWatchdogEnabledRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self.session_store.write().set_workflow_watchdog_enabled(
            &request.session_id,
            &request.watchdog_ref,
            request.enabled,
        )?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
    }

    fn workflow_remove_watchdog(
        &self,
        request: crate::local::RemoveWorkflowWatchdogRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let watchdog = self
            .session_store
            .write()
            .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
    }

    fn workflow_list_queued_launches(
        &self,
        request: crate::local::ListQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
            queued_launches: self
                .session_store
                .read()
                .list_queued_workflow_launches(&request.session_id)?,
        })
    }

    fn workflow_remove_queued_launch(
        &self,
        request: crate::local::RemoveQueuedWorkflowLaunchRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launch = self
            .session_store
            .write()
            .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
            queued_launch,
            session,
        })
    }

    fn workflow_clear_queued_launches(
        &self,
        request: crate::local::ClearQueuedWorkflowLaunchesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let queued_launches = self
            .session_store
            .write()
            .clear_queued_workflow_launches(&request.session_id)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
            queued_launches,
            session,
        })
    }

    fn workflow_validate_output(
        &self,
        request: crate::local::ValidateWorkflowOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let warning = crate::transport::runtime_tools::validate_workflow_output_schema(
            &request.output_schema_ref,
            &request.output_json,
        )
        .err();
        Ok(LocalDaemonResponse::WorkflowOutputValidated {
            valid: warning.is_none(),
            warning,
        })
    }

    fn workflow_ack_turn(
        &self,
        request: crate::local::AckWorkflowTurnRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workflow_run_id = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
            .id()
            .to_string();
        let workflow_run = self.session_store.write().ack_workflow_turn(
            &request.session_id,
            &workflow_run_id,
            &request.workflow_node_run_id,
            &request.delivery_token,
        )?;
        let event = crate::session::WorkflowRuntimeToolCallEvent::new(
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL.to_string(),
            serde_json::json!({"delivery_token": request.delivery_token}).to_string(),
            Some(
                serde_json::json!({
                    "workflow_run_id": workflow_run.id(),
                    "workflow_node_run_id": request.workflow_node_run_id,
                    "state": "acknowledged",
                    "next_action": "Continue this same workflow turn. This acknowledgement is not the final answer; emit the required final fenced json block before stopping.",
                })
                .to_string(),
            ),
            true,
        );
        let _ = self
            .session_store
            .write()
            .record_workflow_runtime_tool_call(
                &request.session_id,
                &request.workflow_node_run_id,
                event,
            );
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&request.session_id, &workflow_run_id)?;
        let session = self.workflow_session(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkflowTurnAcknowledged {
            workflow_run,
            session,
        })
    }

    fn workflow_start_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.session_store.write().start_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let recipients = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        let active_provider_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        self.record_notice(
            session_id,
            active_provider_run_id.as_deref(),
            recipients,
            format!(
                "Workflow run `{}` started on agent `{}`.",
                workflow_run.id(),
                prompt.target_agent_id()
            ),
        );
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    fn workflow_ensure_provider_run(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
            if run.state() == crate::provider::ProviderRunState::Parked {
                let resumed = self.resume_provider_run_for_session(session_id, run.id())?;
                self.session_store
                    .set_active_provider_run(session_id, Some(resumed.id().to_string()))?;
                return Ok(resumed.id().to_string());
            }
            if run.state() != crate::provider::ProviderRunState::Ended {
                self.session_store
                    .set_active_provider_run(session_id, Some(run.id().to_string()))?;
                return Ok(run.id().to_string());
            }
        }
        let agent = self.agent_store.get_agent(agent_id)?;
        let adapter_key = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let provider = match agent.provider() {
            "default" => "opencode",
            value => value,
        };
        let mut request = crate::provider::LaunchProviderRequest::new(
            session_id,
            adapter_key,
            provider,
            "default",
            agent.model().unwrap_or("default"),
        )
        .with_agent_id(agent.id().to_string())
        .with_variant(agent.effort().map(str::to_string));
        if let Some(worktree_id) = agent.worktree_id() {
            request = request.with_working_directory(std::path::PathBuf::from(worktree_id));
        }
        let run = self.provider_store.launch_run_detached(request)?;
        self.session_store
            .set_active_provider_run(session_id, Some(run.id().to_string()))?;
        self.provider_run_projection.update(run.clone());
        Ok(run.id().to_string())
    }

    fn workflow_validate_agents(
        &self,
        session_id: &str,
        workflow: &crate::session::WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        let agents = self.agent_store.get_session_agents(session_id);
        let agent_ids = agents
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for node in workflow.nodes() {
            if !agent_ids.contains(node.agent_id()) {
                return Err(DaemonError::WorkflowNodeAgentMissing {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                });
            }
            let Some(agent) = agents.iter().find(|agent| agent.id() == node.agent_id()) else {
                continue;
            };
            let capabilities = self
                .provider_store
                .get_run_for_agent(session_id, node.agent_id())
                .unwrap_or_else(|| {
                    crate::provider::RuntimeProviderRun::from_control_capability_inference(
                        format!("inferred-{session_id}-{}", node.agent_id()),
                        session_id.to_string(),
                        Some(node.agent_id().to_string()),
                        agent.provider().to_string(),
                    )
                });
            if !capabilities
                .supports_control_operation(crate::provider::ControlOperation::AckWorkflowTurn)
            {
                return Err(DaemonError::WorkflowNodeControlUnsupported {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    operation: "ack_workflow_turn",
                });
            }
            let requires_validation = workflow
                .edges()
                .iter()
                .any(|edge| edge.from_node_id() == node.id() && edge.output_schema_ref().is_some());
            if requires_validation
                && !capabilities.supports_control_operation(
                    crate::provider::ControlOperation::ValidateWorkflowOutput,
                )
            {
                return Err(DaemonError::WorkflowNodeControlUnsupported {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    node_id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    operation: "validate_workflow_output",
                });
            }
        }
        Ok(())
    }

    fn workflow_flush_agent_context_if_needed(
        &self,
        session_id: &str,
        workflow: &crate::session::WorkflowDefinition,
    ) -> Result<(), DaemonError> {
        if !workflow.flush_agent_context_before_run() {
            return Ok(());
        }
        let workflow_agent_ids = workflow
            .nodes()
            .iter()
            .map(|node| node.agent_id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        if workflow_agent_ids.is_empty() {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        for agent_id in &workflow_agent_ids {
            if self
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
                .is_some()
            {
                let _ = self.cancel_active_prompt_only(session_id, agent_id);
            }
        }
        for agent_id in workflow_agent_ids {
            let Some(run) = self.provider_store.get_run_for_agent(session_id, &agent_id) else {
                continue;
            };
            if run.state() == crate::provider::ProviderRunState::Ended {
                continue;
            }
            let ended = self
                .provider_store
                .terminate_run_provider_only(session_id, run.id())?
                .into_run();
            if self
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(ended.id())
            {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            self.provider_run_projection.update(ended.clone());
            self.remove_provider_process_tracking_for_run(ended.id(), None);
        }
        Ok(())
    }

    fn workflow_schedule_entry_node(
        &self,
        session_id: &str,
        workflow_run: &crate::session::WorkflowRun,
    ) -> Result<Vec<crate::app::KernelPromptDispatch>, DaemonError> {
        let endpoint_prompt = workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        let node_run = workflow_run.node_runs().first().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
                reference: workflow_run.id().to_string(),
                message: "workflow run has no entry node run",
            }
        })?;
        let prompt_text = self.workflow_turn_prompt_text(
            session_id,
            workflow_run.id(),
            node_run.id(),
            node_run.node_id(),
            endpoint_prompt,
            None,
            None,
        )?;
        let _ = self.session_store.write().prepare_workflow_turn(
            session_id,
            workflow_run.id(),
            node_run.id(),
            format!("workflow-ack:{}", node_run.id()),
            prompt_text.clone(),
            None,
            None,
        )?;
        let provider_run_id = self.workflow_ensure_provider_run(session_id, node_run.agent_id())?;
        match self.acquire_workflow_node_workspace_claim(
            session_id,
            &provider_run_id,
            node_run.agent_id(),
            workflow_run.id(),
            node_run.id(),
        ) {
            Ok(()) => {}
            Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                let _ = self
                    .session_store
                    .write()
                    .block_workflow_node_on_workspace_claim(
                        session_id,
                        workflow_run.id(),
                        node_run.id(),
                    );
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{}` blocked node `{}` on a workspace claim: {error}",
                        workflow_run.id(),
                        node_run.node_id()
                    ),
                );
                let _ = self.session_snapshot(session_id)?;
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        }
        let _ = self
            .session_store
            .write()
            .ready_workflow_node_after_workspace_claim(
                session_id,
                workflow_run.id(),
                node_run.id(),
            );
        let prompt = crate::session::PromptQueueItem::new(
            self.session_store.reserve_prompt_id(),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            node_run.agent_id(),
            prompt_text,
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), node_run.id());
        let mut dispatches = Vec::new();
        if let Some(mut submission) =
            self.submit_local_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            })?
        {
            if let crate::session::PromptSubmissionOutcome::Started { prompt } = &submission.outcome
            {
                let _ = self.session_store.write().mark_workflow_turn_dispatched(
                    session_id,
                    workflow_run.id(),
                    node_run.id(),
                );
                let _ = self.workflow_start_prompt(session_id, prompt);
            }
            if let Some(dispatch) = submission.dispatch.take() {
                dispatches.push(dispatch);
            }
        }
        Ok(dispatches)
    }

    fn workflow_invoke_queued_launch(
        &self,
        session_id: &str,
        queued_launch: crate::session::QueuedWorkflowLaunch,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            Vec<crate::app::KernelPromptDispatch>,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, queued_launch.workflow_id())?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            queued_launch.workflow_id(),
            queued_launch.endpoint_id(),
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
        let workflow_run = self.session_store.write().invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            queued_launch.invocation_prompt().map(str::to_string),
        )?;
        let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self.session_store.write().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        Ok((
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            },
            dispatches,
        ))
    }

    fn workflow_invoke_endpoint_with_admission(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            Vec<crate::app::KernelPromptDispatch>,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        let admission = self.session_store.write().admit_manual_workflow_launch(
            session_id,
            workflow.id(),
            endpoint.id(),
            prompt.clone(),
        )?;
        match admission {
            crate::session::WorkflowLaunchAdmission::StartNow => {
                self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
                let workflow_run = self.session_store.write().invoke_workflow_endpoint(
                    session_id,
                    workflow.id(),
                    endpoint.id(),
                    prompt,
                )?;
                let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(session_id, workflow_run.id())?;
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    },
                    dispatches,
                ))
            }
            crate::session::WorkflowLaunchAdmission::Queued(queued_launch) => Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued {
                    queued_launch,
                    workflow,
                    endpoint,
                },
                Vec::new(),
            )),
        }
    }

    fn workflow_cancel_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.session_store.write().stop_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let _ = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::RunStopped,
                workflow_node_run_id,
                Vec::new(),
                "workflow node run was stopped before validated completion",
            ),
        );
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!("Workflow run `{}` was stopped.", workflow_run.id()),
        );
        self.workflow_maybe_start_next_queued_launch(session_id);
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    #[allow(dead_code)]
    fn workflow_complete_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
    ) -> Result<Vec<crate::app::KernelPromptDispatch>, DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(Vec::new());
        };
        let completion_snapshot = self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        );
        let max_turns = self.workflow_max_turns(session_id);
        let completion_result = self.session_store.write().complete_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            completion_snapshot.clone(),
            max_turns,
        );
        let update = match completion_result {
            Ok(update) => update,
            Err(crate::error::DaemonError::WorkflowOutputValidationFailed {
                edge_id,
                message,
                ..
            }) => {
                self.workflow_record_failure(
                    session_id,
                    workflow_run_id,
                    &crate::session::WorkflowFailureEvent::new(
                        crate::session::WorkflowFailureKind::OutputValidationFailed,
                        workflow_node_run_id,
                        vec![edge_id.clone()],
                        message.clone(),
                    ),
                );
                self.session_store.write().stop_workflow_node_run(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
                let _ = self.release_workflow_node_workspace_claim(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                );
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store.list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` stopped after validation failed on edge `{edge_id}`: {message}"
                    ),
                );
                self.workflow_maybe_start_next_queued_launch(session_id);
                let _ = self.session_snapshot(session_id)?;
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        for warning in &update.validation_warnings {
            let failure = crate::session::WorkflowFailureEvent::new(
                crate::session::classify_workflow_failure_kind(
                    &completion_snapshot,
                    &warning.message,
                ),
                workflow_node_run_id,
                vec![warning.edge_id.clone()],
                warning.message.clone(),
            );
            self.workflow_record_failure(session_id, workflow_run_id, &failure);
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow output validation warning on edge `{}`: {}",
                    warning.edge_id, warning.message
                ),
            );
        }
        if update.workflow_run.status() == crate::session::WorkflowRunStatus::Stopped
            && update.workflow_run.final_output().is_none()
            && update.workflow_run.failure_events().iter().all(|event| {
                event.kind() != crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted
            })
        {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted,
                    workflow_node_run_id,
                    Vec::new(),
                    "workflow run stopped after a node exhausted its turn budget",
                ),
            );
        }
        if update.workflow_run.final_output_valid() == Some(false) {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                    workflow_node_run_id,
                    Vec::new(),
                    update
                        .workflow_run
                        .final_output_warning()
                        .unwrap_or("workflow run output validation failed"),
                ),
            );
        }
        if update.validation_warnings.is_empty() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_turn_validated_completed(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
        }
        let claim_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, prompt.target_agent_id())
                .map(|run| run.id().to_string())
        });
        let released_claim = claim_provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let released_workflow_claim = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        let mut dispatches =
            self.workflow_prepare_dispatches(session_id, workflow_run_id, &update.dispatches);
        if released_claim || released_workflow_claim {
            dispatches.extend(self.workflow_retry_blocked_claims());
        }
        let state_suffix = match update.workflow_run.status() {
            crate::session::WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
            crate::session::WorkflowRunStatus::Completing => "is completing",
            crate::session::WorkflowRunStatus::Completed => "completed",
            crate::session::WorkflowRunStatus::Stopped => "stopped",
            _ => "updated",
        };
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{}` {state_suffix}.",
                update.workflow_run.id()
            ),
        );
        if matches!(
            update.workflow_run.status(),
            crate::session::WorkflowRunStatus::Completed
                | crate::session::WorkflowRunStatus::Failed
                | crate::session::WorkflowRunStatus::Stopped
        ) {
            self.workflow_maybe_start_next_queued_launch(session_id);
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(dispatches)
    }

    #[allow(dead_code)]
    fn workflow_completion_snapshot(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: Option<&str>,
    ) -> Option<crate::session::WorkflowCompletionSnapshot> {
        let provider_run_id = provider_run_id
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let session = self.session_store.get_session(session_id).ok()?;
        let history = match self.history_store.load(&session) {
            Ok(history) => history,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.workflow",
                    "failed to load session history for workflow completion snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "workflow_run_id": workflow_run_id,
                        "workflow_node_run_id": workflow_node_run_id,
                        "provider_run_id": provider_run_id,
                        "error": error.to_string(),
                    }),
                );
                return None;
            }
        };
        self.history_projection
            .update_entries(session_id, history.clone());
        crate::scheduler::runtime::build_workflow_completion_snapshot_from_history(
            &session,
            history,
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        )
    }

    fn workflow_prompt_has_completion_output(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        provider_run_id: &str,
    ) -> bool {
        self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            Some(provider_run_id),
        )
        .and_then(|snapshot| snapshot.output().cloned())
        .is_some()
    }

    #[allow(dead_code)]
    fn workflow_max_turns(&self, session_id: &str) -> Option<usize> {
        self.session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                session
                    .config_state()
                    .values()
                    .get("workflow.max_turns")
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .filter(|value| *value > 0)
            })
            .or(Some(
                crate::session::DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT,
            ))
    }

    fn workflow_record_failure(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        failure: &crate::session::WorkflowFailureEvent,
    ) {
        let _ = self.session_store.write().record_workflow_failure_event(
            session_id,
            workflow_run_id,
            failure.clone(),
        );
    }

    #[allow(dead_code)]
    fn workflow_control_mailbox_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        _workflow_node_run_id: &str,
    ) -> Option<String> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
            .ok()?;
        let lines = workflow_run
            .failure_events()
            .iter()
            .map(|failure| format!("- {:?}: {}", failure.kind(), failure.message()))
            .collect::<Vec<_>>();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    #[allow(dead_code)]
    fn workflow_outgoing_edge_contracts_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        node_id: &str,
    ) -> String {
        let workflow_id = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run.workflow_id().to_string(),
            Err(_) => return String::new(),
        };
        let Ok(workflow) = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, &workflow_id)
        else {
            return String::new();
        };
        let lines = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == node_id)
            .map(|edge| {
                let mut line = format!("- edge {} -> {}", edge.id(), edge.to_node_id());
                if let Some(schema_ref) = edge.output_schema_ref() {
                    line.push_str(&format!(", output_schema_ref: {schema_ref}"));
                }
                if let Some(validation_policy) = edge.validation_policy() {
                    line.push_str(&format!(", validation_policy: {validation_policy:?}"));
                }
                line
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            String::new()
        } else {
            format!("Outgoing edge contracts:\n{}\n\n", lines.join("\n"))
        }
    }

    #[allow(dead_code)]
    fn workflow_prepare_dispatches(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        dispatches: &[crate::session::WorkflowDispatch],
    ) -> Vec<crate::app::KernelPromptDispatch> {
        let mut prepared = Vec::new();
        for dispatch in dispatches {
            if !self.workflow_dispatch_has_all_inputs(session_id, workflow_run_id, &dispatch) {
                continue;
            }
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` routed {} upstream message(s) to node `{}`.",
                    dispatch.messages.len(),
                    dispatch.node_run.node_id()
                ),
            );
            let handoff_payloads_json =
                serde_json::to_string(&dispatch.messages).unwrap_or_else(|_| "[]".to_string());
            let control_mailbox = self.workflow_control_mailbox_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
            );
            let prompt_text = match self.workflow_turn_prompt_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                dispatch.node_run.node_id(),
                "",
                Some(&handoff_payloads_json),
                control_mailbox.as_deref(),
            ) {
                Ok(prompt_text) => prompt_text,
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not prepare downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    continue;
                }
            };
            let _ = self.session_store.write().prepare_workflow_turn(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                format!("workflow-ack:{}", dispatch.node_run.id()),
                prompt_text.clone(),
                control_mailbox,
                Some(handoff_payloads_json),
            );
            let provider_run_id = match self
                .workflow_ensure_provider_run(session_id, dispatch.node_run.agent_id())
            {
                Ok(provider_run_id) => provider_run_id,
                Err(error) => {
                    self.record_notice(
                            session_id,
                            None,
                            self.attachment_store.list_session_attachment_ids(session_id),
                            format!(
                                "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                                dispatch.node_run.node_id(),
                                error
                            ),
                        );
                    continue;
                }
            };
            match self.acquire_workflow_node_workspace_claim(
                session_id,
                &provider_run_id,
                dispatch.node_run.agent_id(),
                workflow_run_id,
                dispatch.node_run.id(),
            ) {
                Ok(()) => {
                    let _ = self
                        .session_store
                        .write()
                        .ready_workflow_node_after_workspace_claim(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                }
                Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                    let _ = self
                        .session_store
                        .write()
                        .block_workflow_node_on_workspace_claim(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` blocked node `{}` on a workspace claim: {error}",
                            dispatch.node_run.node_id()
                        ),
                    );
                    continue;
                }
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    continue;
                }
            }
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run_id),
                dispatch.node_run.agent_id(),
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run_id, dispatch.node_run.id());
            match self.submit_local_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            }) {
                Ok(Some(mut submission)) => {
                    if let crate::session::PromptSubmissionOutcome::Started { prompt } =
                        &submission.outcome
                    {
                        let _ = self.session_store.write().mark_workflow_turn_dispatched(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                        let _ = self.workflow_start_prompt(session_id, prompt);
                    }
                    if let Some(dispatch) = submission.dispatch.take() {
                        prepared.push(dispatch);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                }
            }
        }
        prepared
    }

    fn workflow_turn_prompt_text(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        node_id: &str,
        endpoint_prompt: &str,
        handoff_payloads_json: Option<&str>,
        control_mailbox: Option<&str>,
    ) -> Result<String, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_run.workflow_id())?;
        let node_instructions = workflow
            .node(node_id)
            .and_then(|node| node.instructions())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("No node-specific instructions were configured.");
        let endpoint_block = if endpoint_prompt.trim().is_empty() {
            String::new()
        } else {
            format!("Endpoint prompt:\n{}\n\n", endpoint_prompt.trim())
        };
        let handoff_block = handoff_payloads_json
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "[]")
            .map(|value| format!("Workflow handoff payloads (JSON array):\n{value}\n\n"))
            .unwrap_or_default();
        let edge_contracts =
            self.workflow_outgoing_edge_contracts_text(session_id, workflow_run_id, node_id);
        let control_block = control_mailbox
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|content| {
                format!(
                    "Control mailbox:\n{content}\nTreat the control mailbox as authoritative runtime feedback for this node. Fix every listed issue in this turn before you finalize the workflow output.\n\n"
                )
            })
            .unwrap_or_default();
        Ok(format!(
            "{endpoint_block}{handoff_block}Workflow-level prompt:\n{}\n\nNode-level instructions:\n{node_instructions}\n\n{edge_contracts}{control_block}For the proper behavior of the workflow, you MUST acknowledge that you have successfully read the current input from the queue by calling the Arroba runtime MCP tool `ack_workflow_turn` exactly once with this JSON argument object:\n{{\"delivery_token\":\"workflow-ack:{workflow_node_run_id}\"}}\n\nIf an outgoing edge contract for this turn includes an `output_schema_ref`, you MUST validate your proposed `output.message` before finalizing by calling the Arroba runtime MCP tool `validate_workflow_output` with the delivery token above, that `output_schema_ref`, and your proposed `output.message` JSON. If no `output_schema_ref` is present for this turn, do not call `validate_workflow_output`.\n\nIf your node-level instructions require shared console output or inspection, use the Arroba runtime MCP tools `workflow_console_read`, `workflow_console_write`, and `workflow_console_clear` for that work.\n\nAt the end of this workflow turn, return exactly one fenced ```json block with this shape:\n{{\"summary\":\"human-facing summary\",\"output\":{{\"message\":\"explicit downstream output message\"}}}}\nDo not output any prose before or after that fenced block. Do not mention acknowledgments, tool calls, or workflow mechanics in the summary unless the task explicitly requires it.\n\nIf a Control mailbox is present, resolve every listed issue before finalizing and do not repeat the invalid payload. When this turn includes an `output_schema_ref`, validation is a gate, not a suggestion. If `validate_workflow_output` returns `valid: false` or any warning, revise the proposed output, call `validate_workflow_output` again, and only finalize once the tool returns `valid: true` with no warning.",
            workflow_run.invocation_prompt().unwrap_or_default()
        ))
    }

    fn workflow_retry_blocked_claims(&self) -> Vec<crate::app::KernelPromptDispatch> {
        let mut blocked = Vec::new();
        for session in self.session_store.read().list_sessions() {
            for workflow_run in session.workflow_runs() {
                for node_run in workflow_run.node_runs() {
                    if node_run.status()
                        != crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
                    {
                        continue;
                    }
                    let Some(prompt) = node_run
                        .turn_envelope()
                        .and_then(|envelope| envelope.rendered_prompt())
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    blocked.push((
                        session.id().to_string(),
                        workflow_run.id().to_string(),
                        node_run.id().to_string(),
                        node_run.agent_id().to_string(),
                        node_run.node_id().to_string(),
                        prompt,
                    ));
                }
            }
        }
        let mut dispatches = Vec::new();
        for (session_id, workflow_run_id, workflow_node_run_id, agent_id, node_id, prompt_text) in
            blocked
        {
            let provider_run_id = match self.workflow_ensure_provider_run(&session_id, &agent_id) {
                Ok(provider_run_id) => provider_run_id,
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                    continue;
                }
            };
            match self.acquire_workflow_node_workspace_claim(
                &session_id,
                &provider_run_id,
                &agent_id,
                &workflow_run_id,
                &workflow_node_run_id,
            ) {
                Ok(()) => {
                    let _ = self
                        .session_store
                        .write()
                        .ready_workflow_node_after_workspace_claim(
                            &session_id,
                            &workflow_run_id,
                            &workflow_node_run_id,
                        );
                }
                Err(DaemonError::WorkspaceClaimConflict { .. }) => continue,
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                    continue;
                }
            }
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(&workflow_run_id),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(&workflow_run_id, &workflow_node_run_id);
            match self.submit_local_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt,
                force_queue: false,
            }) {
                Ok(Some(mut submission)) => {
                    if let crate::session::PromptSubmissionOutcome::Started { prompt } =
                        &submission.outcome
                    {
                        let _ = self.session_store.write().mark_workflow_turn_dispatched(
                            &session_id,
                            &workflow_run_id,
                            &workflow_node_run_id,
                        );
                        let _ = self.workflow_start_prompt(&session_id, prompt);
                    }
                    if let Some(dispatch) = submission.dispatch.take() {
                        dispatches.push(dispatch);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(&session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                        ),
                    );
                }
            }
        }
        dispatches
    }

    fn workflow_dispatch_has_all_inputs(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        dispatch: &crate::session::WorkflowDispatch,
    ) -> bool {
        let workflow_id = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run.workflow_id().to_string(),
            Err(_) => return true,
        };
        let workflow = match self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, &workflow_id)
        {
            Ok(workflow) => workflow,
            Err(_) => return true,
        };
        let expected = workflow
            .edges()
            .iter()
            .filter(|edge| edge.to_node_id() == dispatch.node_run.node_id())
            .map(|edge| edge.from_node_id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        if expected.len() <= 1 {
            return true;
        }
        let run = match self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_id)
        {
            Ok(run) => run,
            Err(_) => return true,
        };
        let run_node_by_id = run
            .node_runs()
            .iter()
            .map(|node_run| (node_run.id().to_string(), node_run.node_id().to_string()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let delivered = dispatch
            .messages
            .iter()
            .filter_map(|message| message.source_node_run_id())
            .filter_map(|node_run_id| run_node_by_id.get(node_run_id).cloned())
            .collect::<std::collections::BTreeSet<_>>();
        expected.is_subset(&delivered)
    }

    fn workflow_maybe_start_next_queued_launch(&self, session_id: &str) {
        let queued_launch = match self
            .session_store
            .write()
            .dequeue_next_workflow_launch(session_id)
        {
            Ok(Some(queued_launch)) => queued_launch,
            Ok(None) => return,
            Err(error) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!("Failed to start queued workflow launch: {error}"),
                );
                return;
            }
        };
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
        }
        match self.workflow_invoke_queued_launch(session_id, queued_launch.clone()) {
            Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                    workflow_run,
                    workflow,
                    endpoint,
                },
                _dispatches,
            )) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Started queued workflow run `{}` for workflow `{}` endpoint `{}`.",
                        workflow_run.id(),
                        workflow.id(),
                        endpoint.id()
                    ),
                );
            }
            Ok((crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued { .. }, _)) => {}
            Err(error) => {
                if let Some(watchdog_id) = queued_launch.watchdog_id() {
                    let _ = self.session_store.write().mark_workflow_watchdog_failed(
                        session_id,
                        watchdog_id,
                        error.to_string(),
                    );
                }
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Queued workflow launch `{}` failed: {error}",
                        queued_launch.id()
                    ),
                );
            }
        }
    }

    fn workflow_resume_run(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<
        (
            crate::session::WorkflowRun,
            Vec<crate::app::KernelPromptDispatch>,
        ),
        DaemonError,
    > {
        let workflow_run = self
            .session_store
            .write()
            .resume_workflow_run(session_id, workflow_run_ref)?;
        let resumable = workflow_run
            .node_runs()
            .iter()
            .filter_map(|node_run| {
                let prompt = node_run.turn_envelope()?.rendered_prompt()?.to_string();
                Some((
                    node_run.id().to_string(),
                    node_run.agent_id().to_string(),
                    prompt,
                ))
            })
            .collect::<Vec<_>>();
        let mut dispatches = Vec::new();
        for (workflow_node_run_id, agent_id, prompt_text) in resumable {
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run.id(), workflow_node_run_id);
            match self.submit_local_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            }) {
                Ok(Some(mut submission)) => {
                    if let crate::session::PromptSubmissionOutcome::Started { prompt } =
                        &submission.outcome
                    {
                        let _ = self.workflow_start_prompt(session_id, prompt);
                    }
                    if let Some(dispatch) = submission.dispatch.take() {
                        dispatches.push(dispatch);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{}` could not resume node prompt: {}",
                            workflow_run.id(),
                            error
                        ),
                    );
                }
            }
        }
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, dispatches))
    }

    fn dispatch_workflow_runtime_tool_call(
        &self,
        tool_name: String,
        arguments: serde_json::Value,
        context: crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let canonical_tool_name = tool_name
            .strip_prefix("arroba_")
            .unwrap_or(tool_name.as_str())
            .to_string();
        let arguments_json = serde_json::to_string(&arguments)
            .unwrap_or_else(|_| String::from("<unserializable runtime tool arguments>"));
        let result = match canonical_tool_name.as_str() {
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::AckWorkflowTurnArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_ack_workflow_turn",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run_id = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?
                    .id()
                    .to_string();
                let workflow_run = self.session_store.write().ack_workflow_turn(
                    &context.session_id,
                    &workflow_run_id,
                    &context.workflow_node_run_id,
                    &args.delivery_token,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "workflow_run_id": workflow_run.id(),
                        "workflow_node_run_id": context.workflow_node_run_id,
                        "state": "acknowledged",
                        "next_action": "Continue this same workflow turn. This acknowledgement is not the final answer; emit the required final fenced json block before stopping.",
                    }),
                })
            }
            crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ValidateWorkflowOutputArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_validate_workflow_output",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                if !context.allowed_output_schema_refs.is_empty()
                    && !context
                        .allowed_output_schema_refs
                        .iter()
                        .any(|schema_ref| schema_ref == &args.output_schema_ref)
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_workflow_output",
                        message: format!(
                            "schema ref `{}` is not allowed for workflow node run `{}`",
                            args.output_schema_ref, context.workflow_node_run_id
                        ),
                    });
                }
                let warning = crate::transport::runtime_tools::validate_workflow_output_schema(
                    &args.output_schema_ref,
                    &args.output_json,
                )
                .err();
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "valid": warning.is_none(),
                        "warning": warning,
                    }),
                })
            }
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
            | crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL =>
            {
                let is_final = canonical_tool_name
                    == crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL;
                if is_final && !context.can_complete_workflow_run {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_and_submit_workflow_run_output",
                        message:
                            "current workflow node run is not allowed to complete the workflow run"
                                .to_string(),
                    });
                }
                if !is_final && !context.can_emit_intermediate_workflow_run_output {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_and_submit_intermediate_workflow_run_output",
                        message:
                            "current workflow node run is not allowed to emit intermediate workflow run output"
                                .to_string(),
                    });
                }
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ValidateAndSubmitWorkflowRunOutputArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: if is_final {
                        "runtime_tool_validate_and_submit_workflow_run_output"
                    } else {
                        "runtime_tool_validate_and_submit_intermediate_workflow_run_output"
                    },
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run_id = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?
                    .id()
                    .to_string();
                let schema_ref = if is_final {
                    context.workflow_run_output_schema_ref.as_deref()
                } else {
                    context.workflow_intermediate_output_schema_ref.as_deref()
                };
                let warning = schema_ref.and_then(|schema_ref| {
                    crate::transport::runtime_tools::validate_workflow_output_schema(
                        schema_ref,
                        &args.workflow_output_json,
                    )
                    .err()
                });
                let output = crate::session::WorkflowOutputPayload::new(
                    args.workflow_output_json,
                    Vec::<crate::session::WorkflowArtifactRef>::new(),
                );
                let workflow_run = if is_final {
                    self.session_store.write().submit_workflow_run_final_output(
                        &context.session_id,
                        &workflow_run_id,
                        &context.workflow_node_run_id,
                        output,
                        warning.is_none(),
                        warning.clone(),
                    )?
                } else {
                    self.session_store.write().submit_workflow_run_intermediate_output(
                        &context.session_id,
                        &workflow_run_id,
                        &context.workflow_node_run_id,
                        output,
                        warning.is_none(),
                        warning.clone(),
                    )?
                };
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "submitted": true,
                        "valid": warning.is_none(),
                        "warning": warning,
                        "workflow_run_id": workflow_run.id(),
                        "workflow_node_run_id": context.workflow_node_run_id,
                    }),
                })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_READ_TOOL => {
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
                let console = self
                    .session_store
                    .read()
                    .read_workflow_console(&context.session_id, workflow_run.workflow_id())?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "workflow_id": console.workflow_id(),
                        "entries": console.entries().iter().map(|entry| serde_json::json!({
                            "timestamp_ms": entry.timestamp_ms(),
                            "source_node_run_id": entry.source_node_run_id(),
                            "source_agent_id": entry.source_agent_id(),
                            "text": entry.text(),
                        })).collect::<Vec<_>>(),
                    }),
                })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_WRITE_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::WorkflowConsoleWriteArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_workflow_console_write",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
                let source_agent_id = self.workflow_node_agent_id(
                    &context.session_id,
                    &context.workflow_run_ref,
                    &context.workflow_node_run_id,
                );
                let entry = self.session_store.write().append_workflow_console_entry(
                    &context.session_id,
                    workflow_run.workflow_id(),
                    Some(context.workflow_node_run_id.clone()),
                    source_agent_id,
                    &args.text,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "timestamp_ms": entry.timestamp_ms(),
                        "source_node_run_id": entry.source_node_run_id(),
                        "source_agent_id": entry.source_agent_id(),
                        "text": entry.text(),
                    }),
                })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_CLEAR_TOOL => {
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
                let console = self
                    .session_store
                    .write()
                    .clear_workflow_console(&context.session_id, workflow_run.workflow_id())?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "cleared": true,
                        "workflow_id": console.workflow_id(),
                    }),
                })
            }
            other => Err(DaemonError::LocalTransport {
                operation: "dispatch_runtime_tool_call",
                message: format!("unsupported runtime tool `{other}`"),
            }),
        };
        let result_json = match &result {
            Ok(result) => Some(
                serde_json::to_string(&result.payload)
                    .unwrap_or_else(|_| String::from("<unserializable runtime tool result>")),
            ),
            Err(error) => Some(serde_json::json!({"error": error.to_string()}).to_string()),
        };
        let ok = result.as_ref().map(|entry| entry.ok).unwrap_or(false);
        let _ = self
            .session_store
            .write()
            .record_workflow_runtime_tool_call(
                &context.session_id,
                &context.workflow_node_run_id,
                crate::session::WorkflowRuntimeToolCallEvent::new(
                    canonical_tool_name,
                    arguments_json,
                    result_json,
                    ok,
                ),
            );
        let _ = self.session_snapshot(&context.session_id);
        result
    }

    fn workflow_node_agent_id(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
        workflow_node_run_id: &str,
    ) -> Option<String> {
        self.session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run_ref)
            .ok()
            .and_then(|workflow_run| {
                workflow_run
                    .node_runs()
                    .iter()
                    .find(|node_run| node_run.id() == workflow_node_run_id)
                    .map(|node_run| node_run.agent_id().to_string())
            })
    }

    fn workflow_tool_context(
        &self,
        session_id: String,
        workflow_run_ref: String,
        workflow_node_run_id: String,
        delivery_token: Option<String>,
    ) -> Result<crate::transport::runtime_tools::WorkflowRuntimeToolContext, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&session_id, &workflow_run_ref)?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&session_id, workflow_run.workflow_id())?;
        let node_id = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .map(|node_run| node_run.node_id().to_string())
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.clone(),
                workflow_id: workflow.id().to_string(),
                reference: workflow_node_run_id.clone(),
                message: "workflow node run was not found while resolving runtime tool scope",
            })?;
        let allowed_output_schema_refs = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == node_id)
            .filter_map(|edge| edge.output_schema_ref().map(str::to_string))
            .collect();
        let node = workflow.node(&node_id);
        let can_complete_workflow_run = node.is_some_and(|node| node.can_complete_workflow_run());
        let can_emit_intermediate_workflow_run_output =
            node.is_some_and(|node| node.can_emit_intermediate_run_output());
        let workflow_intermediate_output_schema_ref = node
            .and_then(|node| node.intermediate_output_schema_ref())
            .map(str::to_string)
            .or_else(|| {
                workflow
                    .intermediate_output_schema_ref()
                    .map(str::to_string)
            });
        Ok(
            crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                session_id,
                workflow_run_ref,
                workflow_node_run_id,
                delivery_token,
                allowed_output_schema_refs,
                workflow_run_output_schema_ref: workflow
                    .run_output_schema_ref()
                    .map(str::to_string),
                workflow_intermediate_output_schema_ref,
                can_complete_workflow_run,
                can_emit_intermediate_workflow_run_output,
            },
        )
    }

    fn resolve_owned_authenticated_workflow_turn(
        &self,
        session_id: &str,
        candidate_agent_ids: &[String],
        delivery_token: Option<&str>,
    ) -> Result<(String, String), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let agent_matches = |agent_id: &str| {
            candidate_agent_ids.is_empty()
                || candidate_agent_ids
                    .iter()
                    .any(|candidate| candidate == agent_id)
        };
        for agent_id in candidate_agent_ids {
            if let Some(prompt) = self
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
            {
                let (Some(workflow_run_ref), Some(workflow_node_run_id)) =
                    (prompt.workflow_run_id(), prompt.workflow_node_run_id())
                else {
                    continue;
                };
                let matches_token = delivery_token.is_none_or(|requested| {
                    session
                        .workflow_runs()
                        .iter()
                        .find(|workflow_run| workflow_run.id() == workflow_run_ref)
                        .and_then(|workflow_run| {
                            workflow_run
                                .node_runs()
                                .iter()
                                .find(|node_run| node_run.id() == workflow_node_run_id)
                        })
                        .and_then(|node_run| node_run.turn_envelope())
                        .is_some_and(|envelope| envelope.delivery_token() == requested)
                });
                if matches_token {
                    return Ok((
                        workflow_run_ref.to_string(),
                        workflow_node_run_id.to_string(),
                    ));
                }
            }
        }
        let mut running_turns = session
            .workflow_runs()
            .iter()
            .flat_map(|workflow_run| {
                workflow_run.node_runs().iter().filter_map(|node_run| {
                    let envelope = node_run.turn_envelope()?;
                    if node_run.status() != crate::session::WorkflowNodeRunStatus::Running
                        || !matches!(
                            envelope.state(),
                            crate::session::WorkflowTurnRuntimeState::Prepared
                                | crate::session::WorkflowTurnRuntimeState::Dispatched
                                | crate::session::WorkflowTurnRuntimeState::Acknowledged
                        )
                    {
                        return None;
                    }
                    if !agent_matches(node_run.agent_id()) {
                        return None;
                    }
                    if delivery_token
                        .is_some_and(|requested| envelope.delivery_token() != requested)
                    {
                        return None;
                    }
                    Some((workflow_run.id().to_string(), node_run.id().to_string()))
                })
            })
            .collect::<Vec<_>>();
        match running_turns.len() {
            1 => Ok(running_turns.remove(0)),
            0 => Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: "no active workflow turn for authenticated provider run".to_string(),
            }),
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: "multiple workflow turns matched the authenticated provider run"
                    .to_string(),
            }),
        }
    }
}

struct OwnedProviderRunExit {
    ended_run: crate::provider::RuntimeProviderRun,
    already_ended: bool,
}

struct OwnedPromptCompletion {
    completion: crate::session::PromptCompletion,
    released_claim: bool,
    dispatch: Option<crate::app::KernelPromptDispatch>,
}

struct OwnedPromptCancellation {
    cancellation: crate::session::PromptCancellation,
    released_claim: bool,
    dispatch: Option<crate::app::KernelPromptDispatch>,
}

impl KernelRuntimeState {
    pub(crate) fn new_with_owned_state(
        app: Arc<Mutex<DaemonApp>>,
        config_projection: crate::kernel::projection::DaemonConfigProjectionStore,
        session_store: SessionStateStore,
        agent_store: AgentServiceStore,
        attachment_store: AttachmentServiceStore,
        provider_store: ProviderProcessServiceStore,
        provider_process_tracking: ProviderProcessTrackingStore,
        session_projection: crate::kernel::projection::SessionStateProjectionStore,
        provider_run_projection: crate::kernel::projection::ProviderRunProjectionStore,
        history_store: SessionHistoryStore,
        history_projection: crate::kernel::projection::SessionHistoryProjectionStore,
        prompt_state_owner: crate::kernel::prompt_state::PromptStateOwner,
        prompt_activity: PromptActivityStore,
        prompt_idle_timeout: Duration,
        prompt_workspace_claims: PromptWorkspaceClaimStore,
        structured_output_records: crate::app::provider_output::StructuredOutputRecordStore,
        terminal_stream: crate::terminal::TerminalStreamStore,
        workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
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
                history_projection,
                prompt_state_owner,
                prompt_activity,
                prompt_idle_timeout,
                prompt_workspace_claims,
                structured_output_records,
                terminal_stream,
                workspace_coordinator,
                managed_io_coordinator: Arc::new(Mutex::new(
                    crate::io::ArtifactEditCoordinator::new(),
                )),
                managed_io_workspace_identities: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            },
        }
    }

    pub(crate) async fn config_snapshot(&self) -> crate::config::DaemonConfig {
        self.owned.config_projection.snapshot()
    }

    async fn with_app_side_effect<R>(&self, operation: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.app.lock().await;
        operation(&mut app)
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
        self.owned.create_session_response(request)
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
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.owned.focus_agent(session_id, agent_id)
    }

    pub(crate) async fn cycle_agent_focus(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        self.owned.cycle_agent_focus(session_id)
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

    pub(crate) async fn alias_session(
        &self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.owned.alias_session(session_id, alias)
    }

    pub(crate) async fn spawn_agent(
        &self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if request.machine_ref.is_none() {
            return self.owned.spawn_agent(request);
        }
        self.with_app_side_effect(|app| {
            crate::app::KernelSessionService::new(app).spawn_agent(request)
        })
        .await
    }

    pub(crate) async fn destroy_agent(
        &self,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_none() {
            return self.owned.destroy_agent(agent_id);
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
        Ok(session)
    }

    pub(crate) async fn submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
    ) -> Result<crate::app::KernelPromptSubmission, DaemonError> {
        {
            let owned = &self.owned;
            if let Some(mut submission) = owned.submit_local_prepared_prompt(&prepared)? {
                self.finish_owned_prompt_submission_workflow_start(&mut submission)
                    .await?;
                return Ok(submission);
            }
            if let Some(mut submission) = owned.submit_remote_prepared_prompt(&prepared)? {
                self.finish_owned_prompt_submission_workflow_start(&mut submission)
                    .await?;
                return Ok(submission);
            }
            let session_id = prepared.session_id.clone();
            let target_agent_id = prepared.prompt.target_agent_id().to_string();
            let attachment_id = prepared.prompt.source_attachment_id().to_string();
            let has_active = owned
                .prompt_state_owner
                .active_prompt_for_agent(
                    &owned.session_store.get_session(&session_id)?,
                    &target_agent_id,
                )
                .is_some();
            let has_run = owned
                .provider_store
                .get_run_for_agent(&session_id, &target_agent_id)
                .is_some();
            if !has_active && !has_run {
                if crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
                    owned.workflow_ensure_provider_run(&session_id, &target_agent_id)?;
                } else {
                    self.with_app_side_effect(|app| {
                        app.ensure_prompt_provider_run_for_agent(&session_id, &target_agent_id)
                    })
                    .await?;
                };
                if let Some(mut submission) = owned.submit_local_prepared_prompt(&prepared)? {
                    self.finish_owned_prompt_submission_workflow_start(&mut submission)
                        .await?;
                    return Ok(submission);
                }
            }
            Err(DaemonError::LocalTransport {
                operation: "submit prepared prompt",
                message:
                    "owned prompt runtime could not admit prompt without side-effect completion"
                        .to_string(),
            })
        }
    }

    async fn finish_owned_prompt_submission_workflow_start(
        &self,
        submission: &mut crate::app::KernelPromptSubmission,
    ) -> Result<(), DaemonError> {
        let crate::session::PromptSubmissionOutcome::Started { prompt } = &submission.outcome
        else {
            return Ok(());
        };
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(prompt.source_attachment_id())
        {
            return Ok(());
        }
        let session_id = submission.session.id().to_string();
        let prompt = prompt.clone();
        if let Some(remote_dispatch) = submission.remote_dispatch.as_mut() {
            remote_dispatch.workflow_context = Some(
                self.with_app_side_effect(|app| {
                    crate::app::RemoteWorkflowTurnContextResolver::new(app)
                        .remote_workflow_turn_context_for_prompt(
                            &session_id,
                            prompt.target_agent_id(),
                            &prompt,
                        )
                })
                .await?,
            );
        }
        self.owned.workflow_start_prompt(&session_id, &prompt)
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        {
            let owned = &self.owned;
            if owned
                .agent_store
                .get_agent(target_agent_id)?
                .remote_execution()
                .is_some()
            {
                let remote_execution = owned
                    .agent_store
                    .get_agent(target_agent_id)?
                    .remote_execution()
                    .cloned()
                    .expect("remote execution checked above");
                match self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::CancelLeasedPrompt {
                                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                                },
                            ),
                        )
                    })
                    .await?
                {
                    RelayPeerResponse::LeasedPromptCancelled { .. } => {
                        return owned.begin_remote_prompt_cancellation(
                            session_id,
                            target_agent_id,
                            attachment_id,
                        );
                    }
                    other => {
                        return Err(DaemonError::LocalTransport {
                            operation: "cancel remote prompt",
                            message: format!(
                                "unexpected remote prompt cancellation response: {other:?}"
                            ),
                        });
                    }
                }
            }
            if let Some(cancellation) =
                owned.cancel_local_prompt(session_id, target_agent_id, attachment_id)?
            {
                return Ok(cancellation);
            }
            Err(DaemonError::LocalTransport {
                operation: "cancel prompt",
                message:
                    "owned prompt runtime could not cancel prompt without side-effect completion"
                        .to_string(),
            })
        }
    }

    pub(crate) async fn complete_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let owned = &self.owned;
        let owned_provider_run_id = owned
            .provider_run_projection
            .get_for_agent(session_id, target_agent_id)
            .or_else(|| {
                owned
                    .provider_store
                    .get_run_for_agent(session_id, target_agent_id)
            })
            .map(|run| run.id().to_string());
        {
            if let Some(remote_execution) = owned
                .agent_store
                .get_agent(target_agent_id)?
                .remote_execution()
                .cloned()
            {
                let remote_provider_run_id = match self
                    .with_app_side_effect(|app| {
                        app.block_on_relay_future(
                            crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                app.config(),
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::CompleteLeasedPrompt {
                                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                                },
                            ),
                        )
                    })
                    .await?
                {
                    RelayPeerResponse::LeasedPromptCompleted {
                        provider_run_id, ..
                    } => provider_run_id
                        .unwrap_or_else(|| "remote-provider-run-completed".to_string()),
                    other => {
                        return Err(DaemonError::LocalTransport {
                            operation: "complete remote prompt",
                            message: format!(
                                "unexpected remote prompt completion response: {other:?}"
                            ),
                        });
                    }
                };
                let completion = owned.complete_remote_prompt_owner(
                    session_id,
                    target_agent_id,
                    &remote_provider_run_id,
                    next_queued_prompt,
                )?;
                if let Some(started_next) = completion.started_next.as_ref() {
                    let attachments = self
                        .with_app_side_effect(|app| {
                            app.serialize_remote_prompt_attachments(started_next.attachments())
                        })
                        .await?;
                    let workflow_context =
                        if crate::scheduler::runtime::is_workflow_prompt_attachment(
                            started_next.source_attachment_id(),
                        ) {
                            Some(
                                self.with_app_side_effect(|app| {
                                    crate::app::RemoteWorkflowTurnContextResolver::new(app)
                                        .remote_workflow_turn_context_for_prompt(
                                            session_id,
                                            target_agent_id,
                                            started_next,
                                        )
                                })
                                .await?,
                            )
                        } else {
                            None
                        };
                    let submit_result = self
                        .with_app_side_effect(|app| {
                            app.block_on_relay_future(
                                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                                    app.config(),
                                    ClientTarget {
                                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                        daemon_alias: None,
                                    },
                                    RelayPeerRequest::SubmitLeasedPrompt {
                                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                                        prompt: started_next.prompt().to_string(),
                                        attachments,
                                        workflow_context,
                                    },
                                ),
                            )
                        })
                        .await?;
                    if let RelayPeerResponse::LeasedPromptSubmitted {
                        provider_run_id, ..
                    } = submit_result
                    {
                        owned.echo_prompt_to_other_attachments(
                            session_id,
                            &provider_run_id,
                            started_next.source_attachment_id(),
                            started_next.prompt(),
                            started_next.attachments(),
                        );
                    }
                }
                return Ok(completion);
            }
        }
        if next_queued_prompt.is_none() {
            {
                let owned = &self.owned;
                if let Some(completion) = owned.complete_local_prompt_without_advance(
                    session_id,
                    target_agent_id,
                    owned_provider_run_id.as_deref(),
                )? {
                    if completion.completion.completed.workflow_run_id().is_some() {
                        let dispatches = owned.workflow_complete_prompt(
                            session_id,
                            &completion.completion.completed,
                            owned_provider_run_id.as_deref(),
                        )?;
                        for dispatch in dispatches {
                            if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                                let _ = self.fail_prompt_dispatch(dispatch, error).await;
                            }
                        }
                    }
                    if completion.released_claim
                        && completion.completion.completed.workflow_run_id().is_none()
                    {
                        for dispatch in owned.workflow_retry_blocked_claims() {
                            if let Err(error) = self
                                .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                                .await
                            {
                                owned.record_notice(
                                    &dispatch.session_id,
                                    Some(&dispatch.provider_run_id),
                                    owned
                                        .attachment_store
                                        .list_session_attachment_ids(&dispatch.session_id),
                                    format!("Blocked workflow retry dispatch failed: {error}"),
                                );
                            }
                        }
                    }
                    return Ok(completion.completion);
                }
            }
        } else if let Some(next_queued_prompt) = next_queued_prompt {
            if let Some(completion) = owned.complete_local_prompt_with_queued_advance(
                session_id,
                target_agent_id,
                owned_provider_run_id.as_deref(),
                next_queued_prompt,
            )? {
                let completion_result = completion.completion;
                if completion_result.completed.workflow_run_id().is_some() {
                    let dispatches = owned.workflow_complete_prompt(
                        session_id,
                        &completion_result.completed,
                        owned_provider_run_id.as_deref(),
                    )?;
                    for dispatch in dispatches {
                        if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                            let _ = self.fail_prompt_dispatch(dispatch, error).await;
                        }
                    }
                }
                if let Some(started_next) = completion_result.started_next.as_ref() {
                    if crate::scheduler::runtime::is_workflow_prompt_attachment(
                        started_next.source_attachment_id(),
                    ) {
                        owned.workflow_start_prompt(session_id, started_next)?;
                    }
                }
                if let Some(dispatch) = completion.dispatch {
                    if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                        let _ = self.fail_prompt_dispatch(dispatch, error).await;
                    }
                }
                return Ok(completion_result);
            }
        }
        Err(DaemonError::LocalTransport {
            operation: "complete prompt",
            message:
                "owned prompt runtime could not complete prompt without side-effect completion"
                    .to_string(),
        })
    }

    async fn reconcile_provider_run_exit(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let owned = &self.owned;

        if let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            None,
        )? {
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
            return Ok(exit.already_ended);
        }

        let process_running = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).poll_running(provider_run_id)
            })
            .await?;
        let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            Some(process_running),
        )?
        else {
            return Ok(false);
        };
        let (_, process_key) = self
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
            })
            .await
            .unwrap_or((false, None));
        owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
        if exit.already_ended {
            return Ok(true);
        }

        let session_outcome = self
            .settle_owned_provider_prompt(session_id, provider_run_id, false, true)
            .await?;
        let recipients = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        owned.record_notice(
            session_id,
            Some(provider_run_id),
            recipients,
            format!(
                "Provider run `{}` for `{}` ended unexpectedly. {}",
                provider_run_id,
                exit.ended_run.provider(),
                if session_outcome.had_active_prompt {
                    if session_outcome.started_next_prompt {
                        "The active prompt was closed and Arroba advanced the queued backlog onto the next available provider run."
                    } else {
                        "The active prompt was closed without starting the queued backlog."
                    }
                } else {
                    "No active prompt was running."
                }
            ),
        );
        Ok(true)
    }

    async fn enqueue_prompt_dispatch(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            let has_managed_process = owned
                .provider_process_tracking
                .read()
                .run_processes
                .contains_key(&dispatch.provider_run_id);
            if has_managed_process {
                let _ = self
                    .reconcile_provider_run_exit(&dispatch.session_id, &dispatch.provider_run_id)
                    .await?;
            }
            self.enqueue_prompt_dispatch_after_liveness(dispatch, owned)
                .await
        }
    }

    async fn enqueue_prompt_dispatch_after_liveness(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        owned: &KernelRuntimeOwnedState,
    ) -> Result<(), DaemonError> {
        owned.echo_prompt_to_other_attachments(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.source_attachment_id,
            &dispatch.prompt,
            &dispatch.attachments,
        );
        let provider_run = owned
            .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: dispatch.provider_run_id.clone(),
                state: provider_run.state(),
                operation: "submit prompt",
            });
        }
        if owned
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            owned.note_prompt_started(&dispatch.provider_run_id);
            return owned.provider_store.enqueue_structured_prompt_submit(
                dispatch.session_id.clone(),
                dispatch.provider_run_id.clone(),
                dispatch.agent_id.clone(),
                &provider_run,
                &dispatch.prompt,
                &dispatch.attachments,
            );
        }
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&dispatch.source_attachment_id)
        {
            let attachment = owned
                .attachment_store
                .get_attachment(&dispatch.source_attachment_id)?;
            if attachment.session_id() != dispatch.session_id {
                return Err(DaemonError::AttachmentNotInSession {
                    session_id: dispatch.session_id.clone(),
                    attachment_id: dispatch.source_attachment_id.clone(),
                });
            }
        }
        owned.terminal_stream.record_input(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.source_attachment_id,
            dispatch.prompt.as_bytes(),
        );
        let has_managed_process = owned
            .provider_process_tracking
            .read()
            .run_processes
            .contains_key(&dispatch.provider_run_id);
        if !has_managed_process {
            owned.note_prompt_started(&dispatch.provider_run_id);
            return Ok(());
        }
        self.with_app_side_effect(|app| {
            app.write_provider_pty_input_for_runtime(
                &dispatch.provider_run_id,
                dispatch.prompt.as_bytes(),
            )
        })
        .await?;
        owned.note_prompt_started(&dispatch.provider_run_id);
        return Ok(());
    }

    async fn fail_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            let _ = owned.cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id);
            let released_claim = owned.clear_prompt_activity(&dispatch.provider_run_id);
            let _ = owned.session_snapshot(&dispatch.session_id);
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id);
            owned.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                recipients,
                format!("Prompt dispatch failed after acknowledgement: {error}"),
            );
            if released_claim {
                for dispatch in owned.workflow_retry_blocked_claims() {
                    if let Err(error) = self
                        .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                        .await
                    {
                        owned.record_notice(
                            &dispatch.session_id,
                            Some(&dispatch.provider_run_id),
                            owned
                                .attachment_store
                                .list_session_attachment_ids(&dispatch.session_id),
                            format!("Blocked workflow retry dispatch failed: {error}"),
                        );
                    }
                }
            }
            Err(error)
        }
    }

    async fn finish_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            match result {
                Ok(remote_provider_run_id) => {
                    owned.echo_prompt_to_other_attachments(
                        &dispatch.session_id,
                        &remote_provider_run_id,
                        &dispatch.source_attachment_id,
                        &dispatch.prompt,
                        &dispatch.attachments,
                    );
                    Ok(())
                }
                Err(error) => {
                    let _ =
                        owned.cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id);
                    let _ = owned.session_snapshot(&dispatch.session_id);
                    let recipients = owned
                        .attachment_store
                        .list_session_attachment_ids(&dispatch.session_id);
                    owned.record_notice(
                        &dispatch.session_id,
                        None,
                        recipients,
                        format!("Remote prompt dispatch failed after acknowledgement: {error}"),
                    );
                    Err(error)
                }
            }
        }
    }

    async fn enqueue_prompt_abort(
        &self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            owned.reap_structured_prompt_jobs();
            self.reconcile_provider_run_exit(&dispatch.session_id, &dispatch.provider_run_id)
                .await?;
            let provider_run = owned
                .ensure_provider_run_in_session(&dispatch.session_id, &dispatch.provider_run_id)?;
            if provider_run.state() != crate::provider::ProviderRunState::Running {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: dispatch.provider_run_id.clone(),
                    state: provider_run.state(),
                    operation: "submit prompt",
                });
            }
            if owned
                .provider_store
                .run_uses_structured_prompt_io(&provider_run)
            {
                return owned.provider_store.enqueue_structured_prompt_abort(
                    dispatch.session_id.clone(),
                    dispatch.provider_run_id.clone(),
                );
            }
            owned.terminal_stream.record_input(
                &dispatch.session_id,
                &dispatch.provider_run_id,
                &dispatch.source_attachment_id,
                b"\x03",
            );
            self.with_app_side_effect(|app| {
                app.write_provider_pty_input_for_runtime(&dispatch.provider_run_id, b"\x03")
            })
            .await?;
            Ok(())
        }
    }

    async fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        {
            let owned = &self.owned;
            owned
                .provider_store
                .structured_prompt_io_in_flight(provider_run_id)
        }
    }

    async fn fail_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        {
            let owned = &self.owned;
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id);
            owned.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                recipients,
                format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
            );
            Err(error)
        }
    }

    pub(crate) fn spawn_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelPromptDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            if let Err(error) = state.enqueue_prompt_dispatch(&dispatch).await {
                let _ = state.fail_prompt_dispatch(dispatch, error).await;
            }
        });
    }

    pub(crate) fn spawn_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let config = state.config_snapshot().await;
            let attachments = dispatch.attachments.clone();
            let serialized_attachments = match tokio::task::spawn_blocking(move || {
                crate::app::serialize_remote_prompt_attachments(&attachments)
            })
            .await
            {
                Ok(result) => result,
                Err(error) => Err(DaemonError::LocalTransport {
                    operation: "serialize remote prompt attachments",
                    message: error.to_string(),
                }),
            };
            let result = match serialized_attachments {
                Ok(attachments) => {
                    match crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &config,
                        ClientTarget {
                            daemon_id: Some(dispatch.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::SubmitLeasedPrompt {
                            leased_agent_id: dispatch.leased_agent_id.clone(),
                            prompt: dispatch.prompt.clone(),
                            attachments,
                            workflow_context: dispatch.workflow_context.clone(),
                        },
                    )
                    .await
                    {
                        Ok(RelayPeerResponse::LeasedPromptSubmitted {
                            provider_run_id, ..
                        }) => Ok(provider_run_id),
                        Ok(other) => Err(DaemonError::LocalTransport {
                            operation: "submit remote prepared prompt",
                            message: format!("unexpected remote prompt response: {other:?}"),
                        }),
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            };
            let _ = state.finish_remote_prompt_dispatch(dispatch, result).await;
        });
    }

    pub(crate) fn spawn_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            loop {
                let outcome = match state.enqueue_prompt_abort(&dispatch).await {
                    Ok(()) => PromptAbortDispatchOutcome::Done,
                    Err(_)
                        if state
                            .structured_prompt_io_in_flight(&dispatch.provider_run_id)
                            .await =>
                    {
                        PromptAbortDispatchOutcome::Retry
                    }
                    Err(error) => {
                        let _ = state.fail_prompt_abort(dispatch.clone(), error).await;
                        PromptAbortDispatchOutcome::Done
                    }
                };
                match outcome {
                    PromptAbortDispatchOutcome::Done => break,
                    PromptAbortDispatchOutcome::Retry => {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                }
            }
        });
    }

    pub(crate) async fn execute_workflow_request(
        &self,
        request: LocalDaemonRequest,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let owned = &self.owned;

        match request {
            LocalDaemonRequest::CreateWorkflow(request) => {
                let result = owned.workflow_create_workflow(request);
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
            LocalDaemonRequest::CreateWorkflowEndpoint(request) => {
                let result = owned.workflow_create_endpoint(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AliasWorkflowEndpoint(request) => {
                let result = owned.workflow_alias_endpoint(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::BindWorkflowEndpoint(request) => {
                let result = owned.workflow_bind_endpoint(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AddWorkflowNode(request) => {
                let result = owned.workflow_add_node(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowNode(request) => {
                let result = owned.workflow_remove_node(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => {
                let result = owned.workflow_update_node_instructions(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => {
                let result = owned.workflow_set_node_can_complete_run(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
                let result = owned.workflow_set_node_can_emit_intermediate_output(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
                let result = owned.workflow_set_node_intermediate_output_schema(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => {
                let result = owned.workflow_set_node_max_turns(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AddWorkflowEdge(request) => {
                let result = owned.workflow_add_edge(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowEdge(request) => {
                let result = owned.workflow_remove_edge(request);
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
                let result = match owned.workflow_invoke_endpoint_with_admission(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.prompt,
                ) {
                    Ok((outcome, dispatches)) => {
                        for dispatch in dispatches {
                            if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                                let _ = self.fail_prompt_dispatch(dispatch, error).await;
                            }
                        }
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
                        for dispatch in dispatches {
                            if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                                let _ = self.fail_prompt_dispatch(dispatch, error).await;
                            }
                        }
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
        }
    }

    pub(crate) async fn start_provider_launch(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> Result<(crate::app::StartedProviderLaunch, u64), DaemonError> {
        let launch_request = self.launch_provider_request_from_owned_state(request);
        {
            let owned = &self.owned;
            let config = owned.config_projection.snapshot();
            let launch_request =
                owned.prepare_provider_launch_request(launch_request, config.runtime_mcp_url())?;
            crate::logging::info_with_fields(
                "daemon.app",
                "launching provider run",
                serde_json::json!({
                    "adapter_key": launch_request.adapter_key.clone(),
                    "agent_id": launch_request.agent_id.clone(),
                    "provider": launch_request.provider.clone(),
                    "session_id": launch_request.session_id.clone(),
                }),
            );
            let started = owned.start_provider_launch(launch_request)?;
            let run = started.run.clone();
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                if let Ok(previous_run) = owned.provider_store.get_run(previous_active_run_id) {
                    owned.provider_run_projection.update(previous_run);
                }
            }
            crate::logging::info_with_fields(
                "daemon.app",
                "prepared provider run endpoint metadata",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "endpoint_mode": run.endpoint_mode().to_string(),
                    "session_id": run.session_id(),
                    "provider": run.provider(),
                }),
            );
            if let Err(error) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).spawn_for_launch(&run)
                })
                .await
            {
                crate::logging::error_with_fields(
                    "daemon.app",
                    "PTY spawn failed for provider run",
                    serde_json::json!({
                        "provider_run_id": run.id(),
                        "session_id": run.session_id(),
                        "error": error.to_string(),
                    }),
                );
                if let Ok(outcome) = owned
                    .provider_store
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    let _ = owned.clear_active_provider_run_session_pointer(
                        run.session_id(),
                        outcome.run().id(),
                    );
                    owned.provider_run_projection.update(outcome.into_run());
                }
                if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                    let recipients = owned
                        .attachment_store
                        .list_session_attachment_ids(run.session_id());
                    match owned
                        .resume_provider_run_for_session(run.session_id(), previous_active_run_id)
                    {
                        Ok(resumed_run) => {
                            owned.record_notice(
                                run.session_id(),
                                Some(resumed_run.id()),
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}`. Arroba resumed the previous provider run `{}` automatically.",
                                    run.session_id(),
                                    resumed_run.id()
                                ),
                            );
                        }
                        Err(resume_error) => {
                            owned.record_notice(
                                run.session_id(),
                                None,
                                recipients,
                                format!(
                                    "Provider switch failed for session `{}` and Arroba could not resume the previous provider run: {}",
                                    run.session_id(),
                                    resume_error
                                ),
                            );
                        }
                    }
                }
                return Err(error);
            }
            owned.provider_run_projection.update(run);
            Ok((started, config.provider_runtime_init_delay_ms))
        }
    }

    fn launch_provider_request_from_owned_state(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> crate::provider::LaunchProviderRequest {
        let mut launch_request = crate::provider::LaunchProviderRequest::new(
            request.session_id.clone(),
            request.adapter_key,
            request.provider,
            request.account_profile,
            request.model,
        )
        .with_variant(request.variant);
        if let Some(agent_id) = request.agent_id.clone().or_else(|| {
            self.owned
                .session_store
                .get_session(&request.session_id)
                .ok()
                .and_then(|session| session.focused_agent_id().map(str::to_string))
                .or_else(|| {
                    self.owned
                        .agent_store
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                })
        }) {
            launch_request = launch_request.with_agent_id(agent_id);
        }
        launch_request
    }

    pub(crate) async fn finish_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        binding: Option<crate::provider::ProviderRuntimeBinding>,
    ) {
        {
            let owned = &self.owned;
            let result = owned.finish_provider_launch_success(started, binding);
            match result {
                Ok(run) => {
                    if let Some(agent_id) = run.agent_instance_id() {
                        match owned.advance_next_queued_prompt_dispatch(
                            run.session_id(),
                            agent_id,
                            run.id(),
                        ) {
                            Ok(Some(dispatch)) => {
                                if let Err(error) = self.enqueue_prompt_dispatch(&dispatch).await {
                                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                self.fail_provider_launch(started, &error).await;
                                return;
                            }
                        }
                        let _ = owned.session_snapshot(run.session_id());
                    }
                }
                Err(error) => {
                    self.fail_provider_launch(started, &error).await;
                }
            }
        }
    }

    pub(crate) async fn fail_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
    ) {
        {
            let owned = &self.owned;
            crate::logging::error_with_fields(
                "daemon.app",
                "provider runtime initialization failed",
                serde_json::json!({
                    "provider_run_id": started.run.id(),
                    "session_id": started.run.session_id(),
                    "error": error.to_string(),
                }),
            );
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(started.run.session_id());
            owned.record_notice(
                started.run.session_id(),
                Some(started.run.id()),
                recipients,
                format!(
                    "Provider launch `{}` failed before it became ready: {}",
                    started.run.id(),
                    error
                ),
            );
            let (_, process_key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(started.run.id())
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(started.run.id(), process_key);
            owned.provider_store.clear_runtime(started.run.id());
            if let Ok(outcome) = owned
                .provider_store
                .terminate_run_provider_only(started.run.session_id(), started.run.id())
            {
                let _ = owned.clear_active_provider_run_session_pointer(
                    started.run.session_id(),
                    outcome.run().id(),
                );
                owned.provider_run_projection.update(outcome.into_run());
            }
            if let Some(previous_active_run_id) = started.previous_active_run_id.as_deref() {
                let _ = owned.resume_provider_run_for_session(
                    started.run.session_id(),
                    previous_active_run_id,
                );
            }
            let _ = owned.session_snapshot(started.run.session_id());
        }
    }

    async fn settle_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        force: bool,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
        let active_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&owned.session_store.get_session(session_id)?, &agent_id);
        let Some(active_prompt) = active_prompt else {
            if owned.clear_prompt_activity(provider_run_id) {
                for dispatch in owned.workflow_retry_blocked_claims() {
                    if let Err(error) = self
                        .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                        .await
                    {
                        owned.record_notice(
                            &dispatch.session_id,
                            Some(&dispatch.provider_run_id),
                            owned
                                .attachment_store
                                .list_session_attachment_ids(&dispatch.session_id),
                            format!("Blocked workflow retry dispatch failed: {error}"),
                        );
                    }
                }
            }
            let _ = owned.sync_focused_provider_run_if_idle(session_id);
            let _ = owned.session_snapshot(session_id);
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        };

        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            if !force && !prompt_completed && !owned.prompt_should_settle(provider_run_id) {
                return Ok(crate::app::ProviderRunExitSessionSummary {
                    had_active_prompt: true,
                    started_next_prompt: false,
                });
            }
            let cancellation = owned.finalize_local_prompt_cancellation_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?;
            owned.workflow_cancel_prompt(session_id, &cancellation.cancellation.prompt)?;
            if cancellation.released_claim {
                for dispatch in owned.workflow_retry_blocked_claims() {
                    if let Err(error) = self
                        .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                        .await
                    {
                        owned.record_notice(
                            &dispatch.session_id,
                            Some(&dispatch.provider_run_id),
                            owned
                                .attachment_store
                                .list_session_attachment_ids(&dispatch.session_id),
                            format!("Blocked workflow retry dispatch failed: {error}"),
                        );
                    }
                }
            }
            if let Some(dispatch) = cancellation.dispatch {
                if let Err(error) = self
                    .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                    .await
                {
                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                }
            }
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: cancellation.cancellation.started_next.is_some(),
            });
        }

        if !force && !prompt_completed && !owned.prompt_should_settle(provider_run_id) {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: true,
                started_next_prompt: false,
            });
        }
        if !force {
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                active_prompt.workflow_run_id(),
                active_prompt.workflow_node_run_id(),
            ) {
                if !owned.workflow_prompt_has_completion_output(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                    provider_run_id,
                ) {
                    return Ok(crate::app::ProviderRunExitSessionSummary {
                        had_active_prompt: true,
                        started_next_prompt: false,
                    });
                }
            }
        }
        let provider_run_state = provider_run.state();
        let next_queued_prompt = if provider_run_state == crate::provider::ProviderRunState::Running
        {
            owned
                .prompt_state_owner
                .peek_next_queued_prompt(&owned.session_store.get_session(session_id)?, &agent_id)
        } else {
            None
        };
        let completion = if let Some(next_queued_prompt) = next_queued_prompt.as_ref() {
            owned.complete_local_prompt_with_queued_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
                next_queued_prompt,
            )?
        } else {
            owned.complete_local_prompt_without_advance(
                session_id,
                &agent_id,
                Some(provider_run_id),
            )?
        }
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "settle provider prompt",
            message: "owned prompt runtime could not settle provider prompt".to_string(),
        })?;
        if completion.completion.completed.workflow_run_id().is_some() {
            let dispatches = owned.workflow_complete_prompt(
                session_id,
                &completion.completion.completed,
                Some(provider_run_id),
            )?;
            for dispatch in dispatches {
                if let Err(error) = self
                    .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                    .await
                {
                    owned.record_notice(
                        &dispatch.session_id,
                        Some(&dispatch.provider_run_id),
                        owned
                            .attachment_store
                            .list_session_attachment_ids(&dispatch.session_id),
                        format!("Workflow dispatch failed after completion: {error}"),
                    );
                }
            }
        }
        if let Some(started_next) = completion.completion.started_next.as_ref() {
            if crate::scheduler::runtime::is_workflow_prompt_attachment(
                started_next.source_attachment_id(),
            ) {
                owned.workflow_start_prompt(session_id, started_next)?;
            }
        }
        if completion.released_claim && completion.completion.completed.workflow_run_id().is_none()
        {
            for dispatch in owned.workflow_retry_blocked_claims() {
                if let Err(error) = self
                    .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                    .await
                {
                    let _ = self.fail_prompt_dispatch(dispatch, error).await;
                }
            }
        }
        if let Some(dispatch) = completion.dispatch {
            if let Err(error) = self
                .enqueue_prompt_dispatch_after_liveness(&dispatch, owned)
                .await
            {
                let _ = self.fail_prompt_dispatch(dispatch, error).await;
            }
        }
        Ok(crate::app::ProviderRunExitSessionSummary {
            had_active_prompt: true,
            started_next_prompt: completion.completion.started_next.is_some(),
        })
    }

    async fn pump_owned_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        initial_liveness_already_checked: bool,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        owned.reap_structured_prompt_jobs();
        if !initial_liveness_already_checked
            && self
                .reconcile_provider_run_exit(session_id, provider_run_id)
                .await?
        {
            return Ok(Vec::new());
        }
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if matches!(
            provider_run.state(),
            crate::provider::ProviderRunState::Ended | crate::provider::ProviderRunState::Parked
        ) {
            return Ok(Vec::new());
        }

        if owned
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            return self
                .pump_owned_structured_provider_output(
                    session_id,
                    provider_run_id,
                    recipient_attachment_ids,
                )
                .await;
        }

        let chunks = match self
            .with_app_side_effect(|app| app.drain_provider_pty_output_for_runtime(provider_run_id))
            .await
        {
            Ok(chunks) => chunks,
            Err(error) => {
                if self
                    .reconcile_provider_run_exit(session_id, provider_run_id)
                    .await?
                {
                    return Ok(Vec::new());
                }
                return Err(error);
            }
        };
        if !chunks.is_empty() {
            owned.note_prompt_response_content(provider_run_id);
        }
        if !self
            .reconcile_provider_run_exit(session_id, provider_run_id)
            .await?
        {
            let _ = self
                .settle_owned_provider_prompt(session_id, provider_run_id, false, false)
                .await?;
        }
        Ok(chunks
            .into_iter()
            .map(|chunk| {
                owned.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    crate::terminal::TerminalOutputKind::ProviderOutput,
                    None,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect())
    }

    async fn pump_owned_structured_provider_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            return Ok(Vec::new());
        }
        if provider_run.endpoint_mode() != crate::provider::AgentEndpointMode::External {
            if let Err(error) = self
                .with_app_side_effect(|app| {
                    app.drain_provider_pty_output_for_runtime(provider_run_id)
                })
                .await
            {
                if self
                    .reconcile_provider_run_exit(session_id, provider_run_id)
                    .await?
                {
                    return Ok(Vec::new());
                }
                if !matches!(error, DaemonError::PtyProcessNotFound { .. }) {
                    return Err(error);
                }
            }
        }
        let mut records = owned.structured_output_records.take(provider_run_id);
        for finished in owned
            .provider_store
            .drain_finished_structured_output_poll_jobs()
        {
            let finished_run_id = finished.provider_run_id.clone();
            let is_requested_run = finished_run_id == provider_run_id;
            let poll_result = match finished.result {
                Ok(Some(poll_result)) => poll_result,
                Ok(None) => continue,
                Err(error) => {
                    let reconcile_result = if is_requested_run {
                        self.reconcile_provider_run_exit(session_id, provider_run_id)
                            .await
                    } else {
                        match owned.provider_store.get_run(&finished_run_id) {
                            Ok(run) => {
                                self.reconcile_provider_run_exit(run.session_id(), &finished_run_id)
                                    .await
                            }
                            Err(run_error) => Err(run_error),
                        }
                    };
                    match reconcile_result {
                        Ok(true) => continue,
                        Ok(false) if is_requested_run => return Err(error),
                        Ok(false) => {
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll failed",
                                serde_json::json!({
                                    "provider_run_id": finished_run_id,
                                    "error": error.to_string(),
                                }),
                            );
                            continue;
                        }
                        Err(reconcile_error) if is_requested_run => return Err(reconcile_error),
                        Err(reconcile_error) => {
                            crate::logging::error_with_fields(
                                "daemon.app",
                                "background structured output poll reconciliation failed",
                                serde_json::json!({
                                    "provider_run_id": finished_run_id,
                                    "error": reconcile_error.to_string(),
                                }),
                            );
                            continue;
                        }
                    }
                }
            };
            let run = match owned.provider_store.get_run(&finished_run_id) {
                Ok(run) => run,
                Err(_) => continue,
            };
            let run_session_id = run.session_id().to_string();
            let recipients = if is_requested_run {
                recipient_attachment_ids.clone()
            } else {
                owned
                    .attachment_store
                    .list_session_attachment_ids(&run_session_id)
            };
            let applied = self
                .apply_owned_structured_output_batch(
                    &run_session_id,
                    &finished_run_id,
                    recipients,
                    poll_result,
                )
                .await?;
            if is_requested_run {
                records.extend(applied);
            } else {
                owned
                    .structured_output_records
                    .append(finished_run_id, applied);
            }
        }
        owned
            .provider_store
            .enqueue_structured_output_poll(provider_run_id)?;
        Ok(records)
    }

    async fn apply_owned_structured_output_batch(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        poll_result: crate::provider::ProviderPromptSignalBatch,
    ) -> Result<Vec<crate::terminal::TerminalOutputRecord>, DaemonError> {
        let owned = &self.owned;
        owned
            .provider_store
            .apply_structured_output_metadata(provider_run_id, &poll_result)?;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        owned.provider_run_projection.update(provider_run);
        for notice in &poll_result.notices {
            owned.record_notice(
                session_id,
                Some(provider_run_id),
                recipient_attachment_ids.clone(),
                notice.to_string(),
            );
        }
        let saw_response_content = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                crate::terminal::TerminalOutputKind::ProviderOutput
                    | crate::terminal::TerminalOutputKind::ProviderReasoning
            )
        });
        let saw_runtime_activity = poll_result.chunks.iter().any(|chunk| {
            matches!(
                chunk.kind,
                crate::terminal::TerminalOutputKind::ProviderOutput
                    | crate::terminal::TerminalOutputKind::ProviderReasoning
                    | crate::terminal::TerminalOutputKind::ProviderTool
                    | crate::terminal::TerminalOutputKind::ProviderStatus
            )
        });
        if saw_response_content {
            owned.note_prompt_response_content(provider_run_id);
        } else if saw_runtime_activity {
            owned.note_prompt_output(provider_run_id);
        }
        for completion in &poll_result.completions {
            owned.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &completion.message_id,
                completion.completed_at_ms,
            );
            owned.mark_prompt_completion_recorded(provider_run_id);
        }
        let prompt_completed = poll_result.prompt_completed;
        let records = poll_result
            .chunks
            .into_iter()
            .map(|chunk| {
                owned.fan_out_terminal_output(
                    session_id,
                    provider_run_id,
                    chunk.kind,
                    chunk.merge_key,
                    recipient_attachment_ids.clone(),
                    &chunk.bytes,
                )
            })
            .collect::<Vec<_>>();
        if !self
            .reconcile_provider_run_exit(session_id, provider_run_id)
            .await?
        {
            let _ = self
                .settle_owned_provider_prompt(session_id, provider_run_id, prompt_completed, false)
                .await?;
        }
        Ok(records)
    }

    pub(crate) async fn pump_terminal_output_with_snapshot(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<
        (
            Vec<crate::terminal::TerminalOutputRecord>,
            Option<crate::session::RuntimeSession>,
        ),
        DaemonError,
    > {
        let owned = &self.owned;
        owned.reap_structured_prompt_jobs();
        owned.ensure_attachment_in_session(session_id, attachment_id)?;
        let active_provider_run_id = owned
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        let mut provider_run_ids = BTreeSet::new();
        if let Some(provider_run_id) = active_provider_run_id {
            provider_run_ids.insert(provider_run_id);
        }
        provider_run_ids.extend(
            owned
                .provider_store
                .list_runs()
                .into_iter()
                .filter(|run| run.session_id() == session_id)
                .filter(|run| {
                    matches!(
                        run.state(),
                        crate::provider::ProviderRunState::Starting
                            | crate::provider::ProviderRunState::Running
                    )
                })
                .map(|run| run.id().to_string()),
        );
        let recipient_attachment_ids = owned
            .attachment_store
            .list_session_attachment_ids(session_id);
        for provider_run_id in provider_run_ids {
            let _ = self
                .pump_owned_provider_output(
                    session_id,
                    &provider_run_id,
                    recipient_attachment_ids.clone(),
                    false,
                )
                .await?;
        }
        let records = owned
            .terminal_stream
            .drain_output_records(session_id, attachment_id);
        let session = owned.session_snapshot(session_id).ok();
        Ok((records, session))
    }

    pub(crate) async fn pump_active_provider_output_with_snapshot(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Option<crate::session::RuntimeSession>, DaemonError> {
        let _ = self
            .pump_owned_provider_output(
                session_id,
                provider_run_id,
                recipient_attachment_ids,
                false,
            )
            .await?;
        let session = self.owned.session_snapshot(session_id).ok();
        Ok(session)
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

    pub(crate) async fn dispatch_authenticated_runtime_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        {
            let owned = &self.owned;
            let canonical_tool_name = tool_name.strip_prefix("arroba_").unwrap_or(tool_name);
            let provider_runs = owned
                .provider_store
                .get_runs_by_runtime_mcp_auth_token(auth_token);
            if provider_runs.is_empty() {
                return Err(DaemonError::LocalTransport {
                    operation: "dispatch_authenticated_runtime_tool_call",
                    message: "invalid runtime MCP auth token".to_string(),
                });
            }
            if matches!(
                canonical_tool_name,
                crate::transport::runtime_tools::READ_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL
                    | crate::transport::runtime_tools::APPLY_PATCH_TOOL
                    | crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL
            ) {
                return self
                    .dispatch_managed_io_runtime_tool_call(
                        &provider_runs[0],
                        canonical_tool_name,
                        arguments,
                    )
                    .await;
            }
            let requested_delivery_token = match canonical_tool_name {
                crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL => {
                    serde_json::from_value::<crate::transport::runtime_tools::AckWorkflowTurnArgs>(
                        arguments.clone(),
                    )
                    .ok()
                    .map(|args| args.delivery_token)
                }
                crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL => {
                    serde_json::from_value::<
                        crate::transport::runtime_tools::ValidateWorkflowOutputArgs,
                    >(arguments.clone())
                    .ok()
                    .and_then(|args| args.delivery_token)
                }
                crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
                | crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL => {
                    serde_json::from_value::<
                        crate::transport::runtime_tools::ValidateAndSubmitWorkflowRunOutputArgs,
                    >(arguments.clone())
                    .ok()
                    .and_then(|args| args.delivery_token)
                }
                _ => None,
            };
            let session_id = provider_runs[0].session_id().to_string();
            let candidate_agent_ids = provider_runs
                .iter()
                .filter_map(|run| run.agent_instance_id().map(str::to_string))
                .collect::<Vec<_>>();
            let (workflow_run_ref, workflow_node_run_id) = owned
                .resolve_owned_authenticated_workflow_turn(
                    &session_id,
                    &candidate_agent_ids,
                    requested_delivery_token.as_deref(),
                )?;
            let context = owned.workflow_tool_context(
                session_id,
                workflow_run_ref,
                workflow_node_run_id,
                None,
            )?;
            owned.dispatch_workflow_runtime_tool_call(
                canonical_tool_name.to_string(),
                arguments,
                context,
            )
        }
    }

    async fn dispatch_managed_io_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let workspace_context = self
            .owned
            .managed_io_workspace_for_provider_run(provider_run)?;
        let workspace_root = workspace_context.root.clone();
        let workspace_identity = workspace_context.identity.clone();
        let mut coordinator = self.owned.managed_io_coordinator.lock().await;
        match tool_name {
            crate::transport::runtime_tools::READ_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedReadArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_read_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                let read = crate::io::ManagedFileIo::read_artifact(
                    &mut coordinator,
                    crate::io::ManagedFileReadRequest {
                        workspace_identity,
                        workspace_root: workspace_root.clone(),
                        path: PathBuf::from(args.path),
                        domain,
                    },
                )
                .map_err(managed_io_daemon_error)?;
                let mut payload = managed_io_read_payload(read);
                add_managed_io_workspace_payload(&mut payload, &workspace_context);
                Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
            }
            crate::transport::runtime_tools::EDIT_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedEditArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_edit_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_edit_artifact",
                        message: "managed edit currently supports only text artifacts".to_string(),
                    });
                }
                let operation = match (args.range, args.old_text) {
                    (Some(range), Some(old_text)) => crate::io::AgentEditOperation::ReplaceRange {
                        range: crate::io::TextRange::new(range.start, range.end),
                        old_text,
                        new_text: args.new_text,
                    },
                    (None, Some(old_text)) => crate::io::AgentEditOperation::ReplaceText {
                        old_text,
                        new_text: args.new_text,
                    },
                    (Some(_), None) => {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_edit_artifact",
                            message: "range edits require old_text".to_string(),
                        });
                    }
                    (None, None) => {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_edit_artifact",
                            message: "managed text edits require old_text or range+old_text"
                                .to_string(),
                        });
                    }
                };
                let path = PathBuf::from(args.path.clone());
                let before = managed_io_text_for_diff(&workspace_root, &path, false);
                let result = crate::io::ManagedFileIo::apply_edit(
                    &mut coordinator,
                    crate::io::ManagedFileWriteRequest {
                        workspace_identity,
                        workspace_root: workspace_root.clone(),
                        domain,
                        intent: crate::io::AgentEditIntent {
                            path: path.clone(),
                            snapshot_id: args.snapshot_id.map(crate::io::ArtifactSnapshotId::new),
                            operation,
                        },
                    },
                );
                let after = managed_io_result_applied(&result)
                    .then(|| managed_io_text_for_diff(&workspace_root, &path, true))
                    .flatten();
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::APPLY_PATCH_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedApplyPatchArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_apply_patch",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_apply_patch",
                        message: "managed apply_patch currently supports only text artifacts"
                            .to_string(),
                    });
                }
                let operations = parse_managed_apply_patch(&args.patch_text)?;
                let mut output = apply_managed_patch_operations(
                    &mut coordinator,
                    workspace_identity,
                    workspace_root.clone(),
                    domain,
                    operations,
                )?;
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            crate::transport::runtime_tools::WRITE_ARTIFACT_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ManagedWriteArtifactArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_write_artifact",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let domain =
                    KernelRuntimeOwnedState::managed_io_domain_from_arg(args.domain.as_deref())?;
                if domain != crate::io::ArtifactDomainKind::TextDocument {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_write_artifact",
                        message: "managed write currently supports only text artifacts".to_string(),
                    });
                }
                let path = PathBuf::from(args.path.clone());
                let before = managed_io_text_for_diff(&workspace_root, &path, true);
                let result = crate::io::ManagedFileIo::apply_edit(
                    &mut coordinator,
                    crate::io::ManagedFileWriteRequest {
                        workspace_identity,
                        workspace_root: workspace_root.clone(),
                        domain,
                        intent: crate::io::AgentEditIntent {
                            path: path.clone(),
                            snapshot_id: args.snapshot_id.map(crate::io::ArtifactSnapshotId::new),
                            operation: crate::io::AgentEditOperation::WriteArtifact {
                                content: crate::io::ArtifactContent::Text(args.content_text),
                            },
                        },
                    },
                );
                let after = managed_io_result_applied(&result)
                    .then(|| managed_io_text_for_diff(&workspace_root, &path, true))
                    .flatten();
                let mut output = managed_io_edit_result(
                    result,
                    ManagedIoChangeContext {
                        path,
                        before,
                        after,
                    },
                );
                add_managed_io_workspace_payload(&mut output.payload, &workspace_context);
                Ok(output)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "dispatch_managed_io_runtime_tool_call",
                message: format!("unsupported managed I/O tool `{other}`"),
            }),
        }
    }

    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        {
            let owned = &self.owned;
            let context = owned.workflow_tool_context(
                context.home_session_id,
                context.workflow_run_id,
                context.workflow_node_run_id,
                Some(context.delivery_token),
            )?;
            owned.dispatch_workflow_runtime_tool_call(tool_name, arguments, context)
        }
    }
}

enum PromptAbortDispatchOutcome {
    Done,
    Retry,
}

pub(crate) struct CapabilityRuntimeSnapshot {
    pub(crate) workspace_id: String,
    pub(crate) worktree_root: std::path::PathBuf,
    pub(crate) workspace_coordinator: crate::kernel::workspace_coordinator::WorkspaceCoordinator,
}

fn managed_io_read_payload(read: crate::io::ArtifactReadResult) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "artifact_id": read.artifact_id.as_str(),
        "path": read.path.to_string_lossy(),
        "domain": managed_io_domain_name(read.domain),
        "version": read.version.value(),
        "snapshot_id": read.snapshot_id.as_str(),
    });
    match read.content {
        crate::io::ArtifactContent::Text(text) => {
            payload["content_text"] = serde_json::Value::String(text);
        }
        crate::io::ArtifactContent::Bytes(bytes) => {
            payload["byte_count"] = serde_json::json!(bytes.len());
        }
    }
    payload
}

fn add_managed_io_workspace_payload(
    payload: &mut serde_json::Value,
    workspace: &ManagedIoWorkspaceContext,
) {
    payload["workspace"] = serde_json::json!({
        "identity_changed": workspace.identity_changed,
        "vcs_provider": workspace.identity.vcs_provider.clone(),
        "repo_id": workspace.identity.repo_id.clone(),
        "repo_url": workspace.identity.repo_url.clone(),
        "branch": workspace.identity.branch.clone(),
        "head_commit": workspace.identity.head_commit.clone(),
        "worktree_root_fingerprint": workspace.identity.worktree_root_fingerprint.clone(),
    });
}

struct ManagedIoChangeContext {
    path: PathBuf,
    before: Option<ManagedIoTextSnapshot>,
    after: Option<ManagedIoTextSnapshot>,
}

struct ManagedIoTextSnapshot {
    existed: bool,
    text: String,
}

#[derive(Debug, Clone)]
enum ManagedPatchOperation {
    Add {
        path: PathBuf,
        content: String,
    },
    Update {
        path: PathBuf,
        old_text: String,
        new_text: String,
    },
    Delete {
        path: PathBuf,
    },
    Move {
        from_path: PathBuf,
        to_path: PathBuf,
        old_text: Option<String>,
        new_text: Option<String>,
    },
}

fn managed_io_edit_result(
    result: crate::io::EditResult,
    change: ManagedIoChangeContext,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    match result {
        crate::io::EditResult::Applied { new_version } => {
            let mut payload = serde_json::json!({
                "applied": true,
                "new_version": new_version.value(),
            });
            add_managed_io_change_payload(&mut payload, change);
            crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
        }
        crate::io::EditResult::AppliedWithWarning {
            new_version,
            warning,
        } => {
            let mut payload = serde_json::json!({
                "applied": true,
                "new_version": new_version.value(),
                "warning": managed_io_warning_payload(warning),
            });
            add_managed_io_change_payload(&mut payload, change);
            crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
        }
        crate::io::EditResult::Rejected { reason } => {
            crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "applied": false,
                    "reason": managed_io_error_payload(reason),
                    "next_action": "Reread the artifact with arroba.read_artifact, reconcile with the current content, and retry through arroba.edit_artifact.",
                }),
            }
        }
    }
}

fn managed_io_result_applied(result: &crate::io::EditResult) -> bool {
    matches!(
        result,
        crate::io::EditResult::Applied { .. } | crate::io::EditResult::AppliedWithWarning { .. }
    )
}

fn parse_managed_apply_patch(patch_text: &str) -> Result<Vec<ManagedPatchOperation>, DaemonError> {
    let lines = patch_text.lines().collect::<Vec<_>>();
    if lines.first().map(|line| line.trim()) != Some("*** Begin Patch")
        || lines.last().map(|line| line.trim()) != Some("*** End Patch")
    {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "patch_text must use the apply_patch envelope".to_string(),
        });
    }
    let mut operations = Vec::new();
    let mut index = 1usize;
    while index + 1 < lines.len() {
        let line = lines[index];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            index += 1;
            let mut body = Vec::new();
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let line = lines[index];
                body.push(line.strip_prefix('+').unwrap_or(line).to_string());
                index += 1;
            }
            operations.push(ManagedPatchOperation::Add {
                path: PathBuf::from(path.trim()),
                content: join_patch_lines(&body),
            });
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(ManagedPatchOperation::Delete {
                path: PathBuf::from(path.trim()),
            });
            index += 1;
            continue;
        }
        if line.starts_with("*** Move to: ") {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_apply_patch",
                message: "move hunks must follow an update file header".to_string(),
            });
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            index += 1;
            let mut move_to = None;
            if index < lines.len() {
                if let Some(target) = lines[index].strip_prefix("*** Move to: ") {
                    move_to = Some(PathBuf::from(target.trim()));
                    index += 1;
                }
            }
            let mut old_lines = Vec::new();
            let mut new_lines = Vec::new();
            while index < lines.len() && !lines[index].starts_with("*** ") {
                let line = lines[index];
                if line.starts_with("@@") || line.starts_with('\\') {
                    index += 1;
                    continue;
                }
                if let Some(rest) = line.strip_prefix('-') {
                    old_lines.push(rest.to_string());
                } else if let Some(rest) = line.strip_prefix('+') {
                    new_lines.push(rest.to_string());
                } else {
                    let rest = line.strip_prefix(' ').unwrap_or(line);
                    old_lines.push(rest.to_string());
                    new_lines.push(rest.to_string());
                }
                index += 1;
            }
            match move_to {
                Some(to_path) => operations.push(ManagedPatchOperation::Move {
                    from_path: PathBuf::from(path.trim()),
                    to_path,
                    old_text: (!old_lines.is_empty()).then(|| join_patch_lines(&old_lines)),
                    new_text: (!new_lines.is_empty()).then(|| join_patch_lines(&new_lines)),
                }),
                None => {
                    if old_lines.is_empty() {
                        return Err(DaemonError::LocalTransport {
                            operation: "runtime_tool_apply_patch",
                            message: format!("update hunk for `{}` has no old text", path.trim()),
                        });
                    }
                    operations.push(ManagedPatchOperation::Update {
                        path: PathBuf::from(path.trim()),
                        old_text: join_patch_lines(&old_lines),
                        new_text: join_patch_lines(&new_lines),
                    });
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!("unsupported patch line `{line}`"),
        });
    }
    if operations.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "patch_text did not contain any supported file operations".to_string(),
        });
    }
    Ok(operations)
}

fn join_patch_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn apply_managed_patch_operations(
    coordinator: &mut crate::io::ArtifactEditCoordinator,
    workspace_identity: crate::io::WorkspaceIdentity,
    workspace_root: PathBuf,
    domain: crate::io::ArtifactDomainKind,
    operations: Vec<ManagedPatchOperation>,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    let mut before_states: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut final_states: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();

    for operation in operations {
        match operation {
            ManagedPatchOperation::Add { path, content } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_patch_state(
                    &workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_some() {
                    return Ok(managed_patch_rejected(
                        path,
                        "add file target already exists; reread and retry with an update",
                    ));
                }
                final_states.insert(path, Some(content));
            }
            ManagedPatchOperation::Update {
                path,
                old_text,
                new_text,
            } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_patch_state(
                    &workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(current) = current else {
                    return Ok(managed_patch_rejected(
                        path,
                        "update file target does not exist",
                    ));
                };
                let Some(updated) = replace_unique_text(&current, &old_text, &new_text) else {
                    return Ok(managed_patch_rejected(
                        path,
                        "patch old text was not found exactly once in the current artifact",
                    ));
                };
                final_states.insert(path, Some(updated));
            }
            ManagedPatchOperation::Delete { path } => {
                managed_io_validate_patch_path(&workspace_root, &path)?;
                let current = managed_patch_state(
                    &workspace_root,
                    &path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if current.is_none() {
                    return Ok(managed_patch_rejected(
                        path,
                        "delete file target does not exist",
                    ));
                }
                final_states.insert(path, None);
            }
            ManagedPatchOperation::Move {
                from_path,
                to_path,
                old_text,
                new_text,
            } => {
                managed_io_validate_patch_path(&workspace_root, &from_path)?;
                managed_io_validate_patch_path(&workspace_root, &to_path)?;
                if from_path == to_path {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source and target are identical",
                    ));
                }
                let source = managed_patch_state(
                    &workspace_root,
                    &from_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                let Some(mut source) = source else {
                    return Ok(managed_patch_rejected(
                        from_path,
                        "move source does not exist",
                    ));
                };
                let target = managed_patch_state(
                    &workspace_root,
                    &to_path,
                    &mut before_states,
                    &mut final_states,
                )?;
                if target.is_some() {
                    return Ok(managed_patch_rejected(
                        to_path,
                        "move target already exists",
                    ));
                }
                if let (Some(old_text), Some(new_text)) = (old_text, new_text) {
                    let Some(updated) = replace_unique_text(&source, &old_text, &new_text) else {
                        return Ok(managed_patch_rejected(
                            from_path,
                            "move patch old text was not found exactly once in the current artifact",
                        ));
                    };
                    source = updated;
                }
                final_states.insert(from_path, None);
                final_states.insert(to_path, Some(source));
            }
        }
    }

    for (path, before) in &before_states {
        let latest = managed_io_read_optional_text(&workspace_root, path)?;
        if &latest != before {
            return Ok(managed_patch_rejected(
                path.clone(),
                "artifact changed while the managed patch was being prepared; reread and retry",
            ));
        }
    }

    if let Err(error) = managed_io_write_final_states(&workspace_root, &final_states) {
        let _ = managed_io_write_final_states(&workspace_root, &before_states);
        return Err(error);
    }

    for (path, after) in &final_states {
        match after {
            Some(text) => {
                coordinator.read_artifact(crate::io::ArtifactReadRequest {
                    workspace_identity: workspace_identity.clone(),
                    path: path.clone(),
                    domain,
                    content: crate::io::ArtifactContent::Text(text.clone()),
                });
            }
            None => coordinator.forget_artifact(&workspace_identity, path),
        }
    }

    let mut changes = Vec::new();
    for (path, after) in final_states {
        let before =
            before_states
                .get(&path)
                .cloned()
                .flatten()
                .map(|text| ManagedIoTextSnapshot {
                    existed: true,
                    text,
                });
        let after = after.map(|text| ManagedIoTextSnapshot {
            existed: true,
            text,
        });
        let mut change_payload = serde_json::json!({});
        add_managed_io_change_payload(
            &mut change_payload,
            ManagedIoChangeContext {
                path,
                before,
                after,
            },
        );
        if let Some(change) = change_payload.get("change") {
            changes.push(change.clone());
        }
    }

    let mut payload = serde_json::json!({
        "applied": true,
        "atomic": true,
        "changes": changes,
    });
    if changes.len() == 1 {
        payload["change"] = changes[0].clone();
        if let Some(path) = changes[0].get("path").cloned() {
            payload["path"] = path;
        }
    }
    Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
}

fn managed_patch_state(
    workspace_root: &PathBuf,
    path: &PathBuf,
    before_states: &mut BTreeMap<PathBuf, Option<String>>,
    final_states: &mut BTreeMap<PathBuf, Option<String>>,
) -> Result<Option<String>, DaemonError> {
    if let Some(current) = final_states.get(path) {
        return Ok(current.clone());
    }
    let current = managed_io_read_optional_text(workspace_root, path)?;
    before_states
        .entry(path.clone())
        .or_insert_with(|| current.clone());
    final_states.insert(path.clone(), current.clone());
    Ok(current)
}

fn replace_unique_text(current: &str, old_text: &str, new_text: &str) -> Option<String> {
    let start = current.find(old_text)?;
    if current[start + old_text.len()..].contains(old_text) {
        return None;
    }
    let mut updated = String::with_capacity(current.len() - old_text.len() + new_text.len());
    updated.push_str(&current[..start]);
    updated.push_str(new_text);
    updated.push_str(&current[start + old_text.len()..]);
    Some(updated)
}

fn managed_patch_rejected(
    path: PathBuf,
    message: impl Into<String>,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({
            "applied": false,
            "reason": {
                "kind": "invalid_operation",
                "path": path.to_string_lossy(),
                "message": message.into(),
            },
            "next_action": "Reread the affected artifact with arroba.read_artifact, reconcile with the current content, and retry through Arroba managed I/O.",
        }),
    }
}

fn managed_io_validate_patch_path(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<(), DaemonError> {
    let _ = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| DaemonError::LocalTransport {
        operation: "runtime_tool_apply_patch",
        message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
    })?;
    if path == std::path::Path::new(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH)
        && managed_io_is_arroba_source_workspace(workspace_root)
    {
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!(
                "the Arroba managed-I/O instruction policy `{}` is owned by Arroba and cannot be edited through managed artifact I/O",
                crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH
            ),
        });
    }
    Ok(())
}

fn managed_io_read_optional_text(
    workspace_root: &PathBuf,
    path: &PathBuf,
) -> Result<Option<String>, DaemonError> {
    let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
        DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
        }
    })?;
    match std::fs::read_to_string(&full_path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DaemonError::LocalTransport {
            operation: "runtime_tool_apply_patch",
            message: format!("failed to read `{}`: {error}", path.to_string_lossy()),
        }),
    }
}

fn managed_io_write_final_states(
    workspace_root: &PathBuf,
    states: &BTreeMap<PathBuf, Option<String>>,
) -> Result<(), DaemonError> {
    for (path, text) in states {
        let full_path = managed_io_diff_workspace_path(workspace_root, path).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "runtime_tool_apply_patch",
                message: "managed patch paths must be workspace-relative and cannot escape the workspace root".to_string(),
            }
        })?;
        match text {
            Some(text) => {
                if let Some(parent) = full_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_apply_patch",
                            message: format!(
                                "failed to create `{}`: {error}",
                                parent.to_string_lossy()
                            ),
                        }
                    })?;
                }
                std::fs::write(&full_path, text).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_apply_patch",
                    message: format!("failed to write `{}`: {error}", path.to_string_lossy()),
                })?;
            }
            None => match std::fs::remove_file(&full_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "runtime_tool_apply_patch",
                        message: format!("failed to delete `{}`: {error}", path.to_string_lossy()),
                    });
                }
            },
        }
    }
    Ok(())
}

fn managed_io_is_arroba_source_workspace(root: &PathBuf) -> bool {
    root.join("apps/daemon/Cargo.toml").is_file()
        && root
            .join(crate::provider::MANAGED_IO_INSTRUCTIONS_SOURCE_PATH)
            .is_file()
}

fn add_managed_io_change_payload(payload: &mut serde_json::Value, change: ManagedIoChangeContext) {
    if change.before.is_none() && change.after.is_none() {
        return;
    }
    let before = change.before.unwrap_or(ManagedIoTextSnapshot {
        existed: false,
        text: String::new(),
    });
    let after = change.after.unwrap_or(ManagedIoTextSnapshot {
        existed: false,
        text: String::new(),
    });
    let diff = managed_io_unified_diff(&change.path, &before, &after);
    payload["path"] = serde_json::Value::String(change.path.to_string_lossy().to_string());
    payload["change"] = serde_json::json!({
        "path": change.path.to_string_lossy(),
        "kind": if !before.existed {
            "add"
        } else if !after.existed {
            "delete"
        } else {
            "update"
        },
        "diff": diff.text,
        "diff_truncated": diff.truncated,
    });
}

struct ManagedIoDiff {
    text: String,
    truncated: bool,
}

const MANAGED_IO_MAX_DIFF_BYTES: usize = 80_000;

fn managed_io_unified_diff(
    path: &PathBuf,
    before: &ManagedIoTextSnapshot,
    after: &ManagedIoTextSnapshot,
) -> ManagedIoDiff {
    let normalized_path = path.to_string_lossy();
    let mut lines = Vec::new();
    lines.push(format!(
        "diff --git a/{normalized_path} b/{normalized_path}"
    ));
    if !before.existed {
        lines.push("new file mode 100644".to_string());
        lines.push("--- /dev/null".to_string());
    } else {
        if !after.existed {
            lines.push("deleted file mode 100644".to_string());
        }
        lines.push(format!("--- a/{normalized_path}"));
    }
    if after.existed {
        lines.push(format!("+++ b/{normalized_path}"));
    } else {
        lines.push("+++ /dev/null".to_string());
    }
    let before_lines = diff_lines(&before.text);
    let after_lines = diff_lines(&after.text);
    lines.extend(managed_io_diff_hunks(&before_lines, &after_lines));
    let mut text = lines.join("\n");
    let mut truncated = false;
    if text.len() > MANAGED_IO_MAX_DIFF_BYTES {
        text.truncate(MANAGED_IO_MAX_DIFF_BYTES);
        text.push_str("\n... diff truncated ...");
        truncated = true;
    }
    ManagedIoDiff { text, truncated }
}

fn diff_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n')
        .map(|line| line.strip_suffix('\n').unwrap_or(line))
        .collect()
}

#[derive(Clone, Copy)]
enum ManagedIoDiffOp<'a> {
    Context(&'a str),
    Remove(&'a str),
    Add(&'a str),
}

fn managed_io_diff_ops<'a>(
    before: &'a [&'a str],
    after: &'a [&'a str],
) -> Vec<ManagedIoDiffOp<'a>> {
    let lcs = managed_io_lcs_table(before, after);
    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < before.len() && j < after.len() {
        if before[i] == after[j] {
            ops.push(ManagedIoDiffOp::Context(before[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            ops.push(ManagedIoDiffOp::Remove(before[i]));
            i += 1;
        } else {
            ops.push(ManagedIoDiffOp::Add(after[j]));
            j += 1;
        }
    }
    while i < before.len() {
        ops.push(ManagedIoDiffOp::Remove(before[i]));
        i += 1;
    }
    while j < after.len() {
        ops.push(ManagedIoDiffOp::Add(after[j]));
        j += 1;
    }
    ops
}

fn managed_io_diff_hunks(before: &[&str], after: &[&str]) -> Vec<String> {
    const CONTEXT: usize = 3;
    let ops = managed_io_diff_ops(before, after);
    if !ops
        .iter()
        .any(|op| matches!(op, ManagedIoDiffOp::Remove(_) | ManagedIoDiffOp::Add(_)))
    {
        return vec![format!("@@ -1,{} +1,{} @@", before.len(), after.len())];
    }

    let mut old_positions = Vec::with_capacity(ops.len());
    let mut new_positions = Vec::with_capacity(ops.len());
    let (mut old_line, mut new_line) = (1usize, 1usize);
    for op in &ops {
        old_positions.push(old_line);
        new_positions.push(new_line);
        match op {
            ManagedIoDiffOp::Context(_) => {
                old_line += 1;
                new_line += 1;
            }
            ManagedIoDiffOp::Remove(_) => old_line += 1,
            ManagedIoDiffOp::Add(_) => new_line += 1,
        }
    }

    let changed_indices = ops
        .iter()
        .enumerate()
        .filter_map(|(idx, op)| {
            matches!(op, ManagedIoDiffOp::Remove(_) | ManagedIoDiffOp::Add(_)).then_some(idx)
        })
        .collect::<Vec<_>>();
    let mut groups = Vec::new();
    for idx in changed_indices {
        let start = idx.saturating_sub(CONTEXT);
        let end = (idx + CONTEXT + 1).min(ops.len());
        if let Some((_, current_end)) = groups.last_mut() {
            if start <= *current_end {
                *current_end = (*current_end).max(end);
                continue;
            }
        }
        groups.push((start, end));
    }

    let mut lines = Vec::new();
    for (start, end) in groups {
        let hunk_ops = &ops[start..end];
        let old_start = old_positions[start];
        let new_start = new_positions[start];
        let old_count = hunk_ops
            .iter()
            .filter(|op| !matches!(op, ManagedIoDiffOp::Add(_)))
            .count();
        let new_count = hunk_ops
            .iter()
            .filter(|op| !matches!(op, ManagedIoDiffOp::Remove(_)))
            .count();
        lines.push(format!(
            "@@ -{},{} +{},{} @@",
            old_start, old_count, new_start, new_count
        ));
        lines.extend(hunk_ops.iter().map(|op| match op {
            ManagedIoDiffOp::Context(line) => format!(" {line}"),
            ManagedIoDiffOp::Remove(line) => format!("-{line}"),
            ManagedIoDiffOp::Add(line) => format!("+{line}"),
        }));
    }
    lines
}

fn managed_io_lcs_table(before: &[&str], after: &[&str]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0; after.len() + 1]; before.len() + 1];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            table[i][j] = if before[i] == after[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    table
}

fn managed_io_text_for_diff(
    workspace_root: &PathBuf,
    path: &PathBuf,
    allow_missing: bool,
) -> Option<ManagedIoTextSnapshot> {
    let full_path = managed_io_diff_workspace_path(workspace_root, path)?;
    match std::fs::read_to_string(full_path) {
        Ok(text) => Some(ManagedIoTextSnapshot {
            existed: true,
            text,
        }),
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            Some(ManagedIoTextSnapshot {
                existed: false,
                text: String::new(),
            })
        }
        Err(_) => None,
    }
}

fn managed_io_diff_workspace_path(workspace_root: &PathBuf, path: &PathBuf) -> Option<PathBuf> {
    if path.is_absolute() {
        return None;
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => relative.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    Some(workspace_root.join(relative))
}

fn workspace_identity_for_root(workspace_root: &PathBuf) -> crate::io::WorkspaceIdentity {
    let fingerprint = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone())
        .to_string_lossy()
        .to_string();
    let git_root = git_output(workspace_root, &["rev-parse", "--show-toplevel"]);
    let Some(git_root) = git_root else {
        return crate::io::WorkspaceIdentity::local(fingerprint);
    };
    let normalized_git_root = PathBuf::from(git_root.trim())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(git_root.trim()))
        .to_string_lossy()
        .to_string();
    crate::io::WorkspaceIdentity {
        vcs_provider: Some("git".to_string()),
        repo_id: None,
        repo_url: git_output(workspace_root, &["config", "--get", "remote.origin.url"])
            .and_then(|value| non_empty_owned(value.trim())),
        branch: git_output(workspace_root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .and_then(|value| non_empty_owned(value.trim()))
            .map(|branch| {
                if branch == "HEAD" {
                    "detached".to_string()
                } else {
                    branch
                }
            }),
        head_commit: git_output(workspace_root, &["rev-parse", "HEAD"])
            .and_then(|value| non_empty_owned(value.trim())),
        worktree_root_fingerprint: normalized_git_root,
    }
}

fn git_output(workspace_root: &PathBuf, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn non_empty_owned(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

fn managed_io_warning_payload(warning: crate::io::ArtifactEditWarning) -> serde_json::Value {
    match warning {
        crate::io::ArtifactEditWarning::RebasedOverNonOverlappingChange {
            base_version,
            applied_version,
        } => serde_json::json!({
            "kind": "rebased_over_non_overlapping_change",
            "base_version": base_version.value(),
            "applied_version": applied_version.value(),
        }),
    }
}

fn managed_io_error_payload(error: crate::io::ArtifactEditError) -> serde_json::Value {
    match error {
        crate::io::ArtifactEditError::ArtifactNotTracked { path } => serde_json::json!({
            "kind": "artifact_not_tracked",
            "path": path.to_string_lossy(),
        }),
        crate::io::ArtifactEditError::SnapshotNotFound { snapshot_id } => serde_json::json!({
            "kind": "snapshot_not_found",
            "snapshot_id": snapshot_id.as_str(),
        }),
        crate::io::ArtifactEditError::UnsupportedDomain { domain } => serde_json::json!({
            "kind": "unsupported_domain",
            "domain": managed_io_domain_name(domain),
        }),
        crate::io::ArtifactEditError::InvalidOperation { message } => serde_json::json!({
            "kind": "invalid_operation",
            "message": message,
        }),
        crate::io::ArtifactEditError::Filesystem { path, message } => serde_json::json!({
            "kind": "filesystem",
            "path": path.to_string_lossy(),
            "message": message,
        }),
        crate::io::ArtifactEditError::ExternalChangeDuringApply { path } => serde_json::json!({
            "kind": "external_change_during_apply",
            "path": path.to_string_lossy(),
        }),
        crate::io::ArtifactEditError::Conflict {
            path,
            base_version,
            current_version,
            requested_ranges,
            changed_ranges,
            message,
        } => serde_json::json!({
            "kind": "conflict",
            "path": path.to_string_lossy(),
            "base_version": base_version.value(),
            "current_version": current_version.value(),
            "requested_ranges": requested_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "changed_ranges": changed_ranges.into_iter().map(managed_io_range_payload).collect::<Vec<_>>(),
            "message": message,
        }),
    }
}

fn managed_io_range_payload(range: crate::io::TextRange) -> serde_json::Value {
    serde_json::json!({
        "start": range.start,
        "end": range.end,
    })
}

fn managed_io_domain_name(domain: crate::io::ArtifactDomainKind) -> &'static str {
    match domain {
        crate::io::ArtifactDomainKind::TextDocument => "text",
        crate::io::ArtifactDomainKind::StructuredDocument => "structured",
        crate::io::ArtifactDomainKind::OpaqueBlob => "opaque",
    }
}

fn managed_io_daemon_error(error: crate::io::ArtifactEditError) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool_managed_io",
        message: managed_io_error_payload(error).to_string(),
    }
}

fn workflow_response_session(
    response: &LocalDaemonResponse,
) -> Option<crate::session::RuntimeSession> {
    match response {
        LocalDaemonResponse::WorkflowCreated { session, .. }
        | LocalDaemonResponse::WorkflowAliased { session, .. }
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
