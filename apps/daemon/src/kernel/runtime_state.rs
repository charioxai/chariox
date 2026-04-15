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
use crate::local::LocalDaemonResponse;
use crate::provider::{ProviderProcessServiceStore, ProviderRunOperationLanes};
use crate::session::{SessionStateOwner, SessionStateStore};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

#[derive(Clone)]
pub(crate) struct CompatibilityRuntimeState {
    app: Arc<Mutex<DaemonApp>>,
    owned: Option<CompatibilityRuntimeOwnedState>,
}

#[derive(Clone)]
pub(crate) struct CompatibilityRuntimeOwnedState {
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
}

impl CompatibilityRuntimeOwnedState {
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

impl CompatibilityRuntimeState {
    #[cfg(test)]
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self { app, owned: None }
    }

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
            owned: Some(CompatibilityRuntimeOwnedState {
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
            }),
        }
    }

    pub(crate) async fn config_snapshot(&self) -> crate::config::DaemonConfig {
        if let Some(owned) = &self.owned {
            return owned.config_projection.snapshot();
        }
        self.with_app_mut(|app| app.config().clone()).await
    }

    async fn with_app_mut<R>(&self, operation: impl FnOnce(&mut DaemonApp) -> R) -> R {
        let mut app = self.app.lock().await;
        operation(&mut app)
    }

    pub(crate) async fn active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        if let Some(owned) = &self.owned {
            let session = owned.session_store.get_session(session_id)?;
            return Ok(owned.prompt_state_owner.active_prompt_agent_id(&session));
        }
        self.with_app_mut(|app| app.prompt_owner_active_prompt_agent_id(session_id))
            .await
    }

    pub(crate) async fn focused_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        if let Some(owned) = &self.owned {
            let session = owned.session_store.get_session(session_id)?;
            return Ok(session.focused_agent_id().map(str::to_string));
        }
        self.with_app_mut(|app| {
            Ok(app
                .sessions()
                .get_session(session_id)?
                .focused_agent_id()
                .map(str::to_string))
        })
        .await
    }

    pub(crate) async fn resolve_session_ref_id(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        if let Some(owned) = &self.owned {
            return Ok(owned
                .session_store
                .read()
                .resolve_session_ref(session_ref, workspace_id)?
                .id()
                .to_string());
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app)
                .resolve_session_ref_id(session_ref, workspace_id)
        })
        .await
    }

    pub(crate) async fn attachment_session_id(
        &self,
        attachment_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(owned) = &self.owned {
            return Ok(owned
                .attachment_store
                .get_attachment(attachment_id)?
                .session_id()
                .to_string());
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).attachment_session_id(attachment_id)
        })
        .await
    }

    pub(crate) async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.session_snapshot(session_id);
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).session_snapshot(session_id)
        })
        .await
    }

    pub(crate) async fn create_session_response(
        &self,
        request: crate::session::CreateSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.create_session_response(request);
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).create_session_response(request)
        })
        .await
    }

    pub(crate) async fn attach(
        &self,
        request: crate::attachment::AttachRequest,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.attach(request);
        }
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).attach(request))
            .await
    }

    pub(crate) async fn detach(
        &self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.detach(attachment_id);
        }
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).detach(attachment_id))
            .await
    }

    pub(crate) async fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.focus_agent(session_id, agent_id);
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).focus_agent(session_id, agent_id)
        })
        .await
    }

    pub(crate) async fn cycle_agent_focus(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.cycle_agent_focus(session_id);
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).cycle_agent_focus(session_id)
        })
        .await
    }

    pub(crate) async fn resize_terminal(
        &self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        if let Some(owned) = &self.owned {
            if let Some(provider_run_id) = owned.resize_terminal(session_id)? {
                self.with_app_mut(|app| app.pty_mut().resize(&provider_run_id, cols, rows))
                    .await?;
            }
            return Ok(());
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).resize_terminal(session_id, cols, rows)
        })
        .await
    }

    pub(crate) async fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(), DaemonError> {
        if let Some(owned) = &self.owned {
            let _ = owned.ensure_attachment_in_session(session_id, attachment_id)?;
            return Ok(());
        }
        self.with_app_mut(|app| {
            let _ = crate::app::KernelSessionService::new(app)
                .ensure_attachment_in_session(session_id, attachment_id)?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn drain_notice_records(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<crate::terminal::RuntimeNoticeRecord> {
        if let Some(owned) = &self.owned {
            return owned
                .terminal_stream
                .drain_notice_records(session_id, attachment_id);
        }
        self.with_app_mut(|app| {
            app.terminal()
                .drain_notice_records(session_id, attachment_id)
        })
        .await
    }

    pub(crate) async fn update_session_config(
        &self,
        session_id: &str,
        attachment_id: &str,
        values: std::collections::BTreeMap<String, String>,
        requires_idle: bool,
    ) -> Result<crate::session::SessionConfigState, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.update_session_config(session_id, attachment_id, values, requires_idle);
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).update_session_config(
                session_id,
                attachment_id,
                values,
                requires_idle,
            )
        })
        .await
    }

    pub(crate) async fn alias_session(
        &self,
        session_id: &str,
        alias: String,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.alias_session(session_id, alias);
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).alias_session(session_id, alias)
        })
        .await
    }

    pub(crate) async fn spawn_agent(
        &self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if request.machine_ref.is_none() {
            if let Some(owned) = &self.owned {
                return owned.spawn_agent(request);
            }
        }
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).spawn_agent(request))
            .await
    }

    pub(crate) async fn destroy_agent(
        &self,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        if let Some(owned) = &self.owned {
            let agent = owned.agent_store.get_agent(agent_id)?;
            if agent.remote_execution().is_none() {
                return owned.destroy_agent(agent_id);
            }
        }
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).destroy_agent(agent_id))
            .await
    }

    pub(crate) async fn end_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        if let Some(owned) = &self.owned {
            let (session, terminated_run_ids) = owned.end_session(session_id)?;
            for provider_run_id in terminated_run_ids {
                let (_, process_key) = self
                    .with_app_mut(|app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(&provider_run_id)
                    })
                    .await
                    .unwrap_or((false, None));
                owned.remove_provider_process_tracking_for_run(&provider_run_id, process_key);
            }
            return Ok(session);
        }
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).end_session(session_id))
            .await
    }

    pub(crate) async fn delete_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        if let Some(owned) = &self.owned {
            let (session, terminated_run_ids) =
                owned.delete_session_ref(session_ref, workspace_id)?;
            for provider_run_id in terminated_run_ids {
                let (_, process_key) = self
                    .with_app_mut(|app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(&provider_run_id)
                    })
                    .await
                    .unwrap_or((false, None));
                owned.remove_provider_process_tracking_for_run(&provider_run_id, process_key);
            }
            return Ok(session);
        }
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).delete_session_ref(session_ref, workspace_id)
        })
        .await
    }

    pub(crate) async fn submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
    ) -> Result<crate::app::KernelPromptSubmission, DaemonError> {
        if let Some(owned) = &self.owned {
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
                let ensure_result =
                    if crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
                        self.with_app_mut(|app| {
                            crate::app::workflow_runtime::ensure_workflow_provider_run_from_runtime(
                                app,
                                &session_id,
                                &target_agent_id,
                            )
                        })
                        .await
                    } else {
                        self.with_app_mut(|app| {
                            app.ensure_prompt_provider_run_for_agent(&session_id, &target_agent_id)
                        })
                        .await
                    };
                ensure_result?;
                if let Some(mut submission) = owned.submit_local_prepared_prompt(&prepared)? {
                    self.finish_owned_prompt_submission_workflow_start(&mut submission)
                        .await?;
                    return Ok(submission);
                }
            }
            return Err(DaemonError::LocalTransport {
                operation: "submit prepared prompt",
                message:
                    "owned prompt runtime could not admit prompt without app-backed agent service"
                        .to_string(),
            });
        }
        Err(DaemonError::LocalTransport {
            operation: "submit prepared prompt",
            message: "owned prompt runtime is not available".to_string(),
        })
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
                self.with_app_mut(|app| {
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
        self.with_app_mut(|app| {
            crate::app::workflow_runtime::start_workflow_prompt_from_runtime(
                app,
                &session_id,
                &prompt,
            )
        })
        .await
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        if let Some(owned) = &self.owned {
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
                    .with_app_mut(|app| {
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
            return Err(DaemonError::LocalTransport {
                operation: "cancel prompt",
                message:
                    "owned prompt runtime could not cancel prompt without app-backed agent service"
                        .to_string(),
            });
        }
        Err(DaemonError::LocalTransport {
            operation: "cancel prompt",
            message: "owned prompt runtime is not available".to_string(),
        })
    }

    pub(crate) async fn complete_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let owned_provider_run_id = self.owned.as_ref().map(|owned| {
            owned
                .provider_run_projection
                .get_for_agent(session_id, target_agent_id)
                .or_else(|| {
                    owned
                        .provider_store
                        .get_run_for_agent(session_id, target_agent_id)
                })
                .map(|run| run.id().to_string())
        });
        if let Some(owned) = &self.owned {
            if let Some(remote_execution) = owned
                .agent_store
                .get_agent(target_agent_id)?
                .remote_execution()
                .cloned()
            {
                let remote_provider_run_id = match self
                    .with_app_mut(|app| {
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
                        .with_app_mut(|app| {
                            app.serialize_remote_prompt_attachments(started_next.attachments())
                        })
                        .await?;
                    let workflow_context =
                        if crate::scheduler::runtime::is_workflow_prompt_attachment(
                            started_next.source_attachment_id(),
                        ) {
                            Some(
                                self.with_app_mut(|app| {
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
                        .with_app_mut(|app| {
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
            if let Some(owned) = &self.owned {
                if let Some(completion) = owned.complete_local_prompt_without_advance(
                    session_id,
                    target_agent_id,
                    owned_provider_run_id
                        .as_ref()
                        .and_then(|run_id| run_id.as_deref()),
                )? {
                    if completion.completion.completed.workflow_run_id().is_some() {
                        self.with_app_mut(|app| {
                            crate::app::workflow_runtime::complete_workflow_prompt_from_runtime(
                                app,
                                session_id,
                                &completion.completion.completed,
                                owned_provider_run_id
                                    .as_ref()
                                    .and_then(|run_id| run_id.as_deref()),
                            )
                        })
                        .await?;
                        let _ = owned.session_snapshot(session_id)?;
                    }
                    if completion.released_claim {
                        self.with_app_mut(|app| {
                            crate::app::workflow_runtime::retry_blocked_workflow_claims_from_runtime(
                                app,
                            )
                        })
                        .await;
                    }
                    return Ok(completion.completion);
                }
            }
        } else if let (Some(owned), Some(next_queued_prompt)) = (&self.owned, next_queued_prompt) {
            if let Some(completion) = owned.complete_local_prompt_with_queued_advance(
                session_id,
                target_agent_id,
                owned_provider_run_id
                    .as_ref()
                    .and_then(|run_id| run_id.as_deref()),
                next_queued_prompt,
            )? {
                let completion_result = completion.completion;
                if completion_result.completed.workflow_run_id().is_some() {
                    self.with_app_mut(|app| {
                        crate::app::workflow_runtime::complete_workflow_prompt_from_runtime(
                            app,
                            session_id,
                            &completion_result.completed,
                            owned_provider_run_id
                                .as_ref()
                                .and_then(|run_id| run_id.as_deref()),
                        )
                    })
                    .await?;
                    let _ = owned.session_snapshot(session_id)?;
                }
                if let Some(started_next) = completion_result.started_next.as_ref() {
                    if crate::scheduler::runtime::is_workflow_prompt_attachment(
                        started_next.source_attachment_id(),
                    ) {
                        self.with_app_mut(|app| {
                            crate::app::workflow_runtime::start_workflow_prompt_from_runtime(
                                app,
                                session_id,
                                started_next,
                            )
                        })
                        .await?;
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
                "owned prompt runtime could not complete prompt without app-backed agent service"
                    .to_string(),
        })
    }

    async fn reconcile_provider_run_exit(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some(owned) = &self.owned else {
            return self
                .with_app_mut(|app| {
                    crate::app::ProviderRunLivenessRuntime::new(app)
                        .reconcile_provider_run_exit(session_id, provider_run_id)
                })
                .await;
        };

        if let Some(exit) = owned.reconcile_provider_run_liveness_provider_phase(
            session_id,
            provider_run_id,
            None,
        )? {
            let (_, process_key) = self
                .with_app_mut(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(provider_run_id)
                })
                .await
                .unwrap_or((false, None));
            owned.remove_provider_process_tracking_for_run(provider_run_id, process_key);
            return Ok(exit.already_ended);
        }

        let process_running = self
            .with_app_mut(|app| {
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
            .with_app_mut(|app| {
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
        if let Some(owned) = &self.owned {
            let _ = self
                .reconcile_provider_run_exit(&dispatch.session_id, &dispatch.provider_run_id)
                .await?;
            return self
                .enqueue_prompt_dispatch_after_liveness(dispatch, owned)
                .await;
        }
        self.with_app_mut(|app| app.enqueue_kernel_prompt_dispatch(dispatch))
            .await
    }

    async fn enqueue_prompt_dispatch_after_liveness(
        &self,
        dispatch: &crate::app::KernelPromptDispatch,
        owned: &CompatibilityRuntimeOwnedState,
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
        self.with_app_mut(|app| {
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
        if let Some(owned) = &self.owned {
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
                self.with_app_mut(|app| {
                    crate::app::workflow_runtime::retry_blocked_workflow_claims_from_runtime(app)
                })
                .await;
            }
            return Err(error);
        }
        self.with_app_mut(|app| app.fail_kernel_prompt_dispatch(dispatch, error))
            .await
    }

    async fn finish_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        if let Some(owned) = &self.owned {
            match result {
                Ok(remote_provider_run_id) => {
                    owned.echo_prompt_to_other_attachments(
                        &dispatch.session_id,
                        &remote_provider_run_id,
                        &dispatch.source_attachment_id,
                        &dispatch.prompt,
                        &dispatch.attachments,
                    );
                    return Ok(());
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
                    return Err(error);
                }
            }
        }
        self.with_app_mut(|app| app.finish_kernel_remote_prompt_dispatch(dispatch, result))
            .await
    }

    async fn enqueue_prompt_abort(
        &self,
        dispatch: &crate::app::KernelPromptAbortDispatch,
    ) -> Result<(), DaemonError> {
        if let Some(owned) = &self.owned {
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
            self.with_app_mut(|app| {
                app.write_provider_pty_input_for_runtime(&dispatch.provider_run_id, b"\x03")
            })
            .await?;
            return Ok(());
        }
        self.with_app_mut(|app| app.enqueue_kernel_prompt_abort(dispatch))
            .await
    }

    async fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        if let Some(owned) = &self.owned {
            return owned
                .provider_store
                .structured_prompt_io_in_flight(provider_run_id);
        }
        self.with_app_mut(|app| {
            crate::app::KernelPromptDispatchRuntime::new(app)
                .structured_prompt_io_in_flight(provider_run_id)
        })
        .await
    }

    async fn fail_prompt_abort(
        &self,
        dispatch: crate::app::KernelPromptAbortDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        if let Some(owned) = &self.owned {
            let recipients = owned
                .attachment_store
                .list_session_attachment_ids(&dispatch.session_id);
            owned.record_notice(
                &dispatch.session_id,
                Some(&dispatch.provider_run_id),
                recipients,
                format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
            );
            return Err(error);
        }
        self.with_app_mut(|app| app.fail_kernel_prompt_abort(dispatch, error))
            .await
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

    pub(crate) async fn execute_workflow_service_operation(
        &self,
        session_id: &str,
        operation: impl FnOnce(
            &mut crate::app::KernelWorkflowService<'_>,
        ) -> Result<LocalDaemonResponse, DaemonError>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let (result, response_session) = self
            .with_app_mut(|app| {
                let result = {
                    let mut workflows = crate::app::KernelWorkflowService::new(app);
                    operation(&mut workflows)
                };
                let response_session = result.as_ref().ok().and_then(workflow_response_session);
                (result, response_session)
            })
            .await;
        let projected_session = if response_session.is_some() || result.is_err() {
            response_session
        } else if let Some(owned) = &self.owned {
            owned.session_snapshot(session_id).ok()
        } else {
            self.with_app_mut(|app| {
                crate::app::KernelSessionReadService::new(app)
                    .session_snapshot(session_id)
                    .ok()
            })
            .await
        };
        (result, projected_session)
    }

    pub(crate) async fn start_provider_launch(
        &self,
        request: crate::local::LaunchProviderRunRequest,
    ) -> Result<(crate::app::StartedProviderLaunch, u64), DaemonError> {
        let launch_request = self.launch_provider_request_from_owned_state(request);
        if let Some(owned) = &self.owned {
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
                .with_app_mut(|app| {
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
            return Ok((started, config.provider_runtime_init_delay_ms));
        }
        self.with_app_mut(|app| {
            Ok((
                app.start_provider_launch(launch_request)?,
                app.config().provider_runtime_init_delay_ms,
            ))
        })
        .await
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
            self.owned.as_ref().and_then(|owned| {
                owned
                    .session_store
                    .get_session(&request.session_id)
                    .ok()
                    .and_then(|session| session.focused_agent_id().map(str::to_string))
                    .or_else(|| {
                        owned
                            .agent_store
                            .get_focused_agent(&request.session_id)
                            .map(|agent| agent.id().to_string())
                    })
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
        if let Some(owned) = &self.owned {
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
            return;
        }
        self.with_app_mut(|app| {
            if let Err(error) = app.finish_provider_launch(started, binding) {
                app.fail_provider_launch(started, &error);
            }
        })
        .await;
    }

    pub(crate) async fn fail_provider_launch(
        &self,
        started: &crate::app::StartedProviderLaunch,
        error: &DaemonError,
    ) {
        if let Some(owned) = &self.owned {
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
                .with_app_mut(|app| {
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
            return;
        }
        self.with_app_mut(|app| app.fail_provider_launch(started, error))
            .await;
    }

    async fn settle_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        prompt_completed: bool,
        force: bool,
    ) -> Result<crate::app::ProviderRunExitSessionSummary, DaemonError> {
        let Some(owned) = &self.owned else {
            return Ok(crate::app::ProviderRunExitSessionSummary {
                had_active_prompt: false,
                started_next_prompt: false,
            });
        };
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
            let released_claim = owned.clear_prompt_activity(provider_run_id);
            if released_claim {
                self.with_app_mut(|app| {
                    crate::app::workflow_runtime::retry_blocked_workflow_claims_from_runtime(app)
                })
                .await;
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
            self.with_app_mut(|app| {
                crate::app::workflow_runtime::cancel_workflow_prompt_from_runtime(
                    app,
                    session_id,
                    &cancellation.cancellation.prompt,
                )
            })
            .await?;
            if cancellation.released_claim {
                self.with_app_mut(|app| {
                    crate::app::workflow_runtime::retry_blocked_workflow_claims_from_runtime(app)
                })
                .await;
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
            self.with_app_mut(|app| {
                crate::app::workflow_runtime::complete_workflow_prompt_from_runtime(
                    app,
                    session_id,
                    &completion.completion.completed,
                    Some(provider_run_id),
                )
            })
            .await?;
            let _ = owned.session_snapshot(session_id)?;
        }
        if let Some(started_next) = completion.completion.started_next.as_ref() {
            if crate::scheduler::runtime::is_workflow_prompt_attachment(
                started_next.source_attachment_id(),
            ) {
                self.with_app_mut(|app| {
                    crate::app::workflow_runtime::start_workflow_prompt_from_runtime(
                        app,
                        session_id,
                        started_next,
                    )
                })
                .await?;
            }
        }
        if completion.released_claim {
            self.with_app_mut(|app| {
                crate::app::workflow_runtime::retry_blocked_workflow_claims_from_runtime(app)
            })
            .await;
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
        let Some(owned) = &self.owned else {
            return self
                .with_app_mut(|app| {
                    crate::app::provider_output::ProviderOutputPump::new(app).pump_provider_output(
                        crate::app::provider_output::ProviderOutputPumpRequest {
                            session_id,
                            provider_run_id,
                            recipient_attachment_ids,
                            initial_liveness_already_checked,
                        },
                    )
                })
                .await;
        };
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
            .with_app_mut(|app| app.drain_provider_pty_output_for_runtime(provider_run_id))
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
        let Some(owned) = &self.owned else {
            return Ok(Vec::new());
        };
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Parked {
            return Ok(Vec::new());
        }
        if provider_run.endpoint_mode() != crate::provider::AgentEndpointMode::External {
            if let Err(error) = self
                .with_app_mut(|app| app.drain_provider_pty_output_for_runtime(provider_run_id))
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
        let Some(owned) = &self.owned else {
            return Ok(Vec::new());
        };
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
        let records = if let Some(owned) = &self.owned {
            owned.reap_structured_prompt_jobs();
            owned.ensure_attachment_in_session(session_id, attachment_id)?;
            let provider_run_id = owned
                .session_store
                .get_session(session_id)?
                .active_provider_run_id()
                .map(str::to_string);
            if let Some(provider_run_id) = provider_run_id {
                let recipient_attachment_ids = owned
                    .attachment_store
                    .list_session_attachment_ids(session_id);
                let _ = self
                    .pump_owned_provider_output(
                        session_id,
                        &provider_run_id,
                        recipient_attachment_ids,
                        false,
                    )
                    .await?;
            }
            owned
                .terminal_stream
                .drain_output_records(session_id, attachment_id)
        } else {
            self.with_app_mut(|app| {
                crate::app::provider_output::pump_terminal_output_for_attachment(
                    app,
                    session_id,
                    attachment_id,
                )
            })
            .await?
        };
        let session = if let Some(owned) = &self.owned {
            owned.session_snapshot(session_id).ok()
        } else {
            self.with_app_mut(|app| {
                crate::app::KernelSessionReadService::new(app)
                    .session_snapshot(session_id)
                    .ok()
            })
            .await
        };
        Ok((records, session))
    }

    pub(crate) async fn pump_active_provider_output_with_snapshot(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Option<crate::session::RuntimeSession>, DaemonError> {
        if self.owned.is_some() {
            let _ = self
                .pump_owned_provider_output(
                    session_id,
                    provider_run_id,
                    recipient_attachment_ids,
                    false,
                )
                .await?;
        } else if !self
            .reconcile_provider_run_exit(session_id, provider_run_id)
            .await?
        {
            self.with_app_mut(|app| {
                crate::app::provider_output::ProviderOutputPump::new(app).pump_provider_output(
                    crate::app::provider_output::ProviderOutputPumpRequest {
                        session_id,
                        provider_run_id,
                        recipient_attachment_ids,
                        initial_liveness_already_checked: true,
                    },
                )
            })
            .await?;
        }
        let session = if let Some(owned) = &self.owned {
            owned.session_snapshot(session_id).ok()
        } else {
            self.with_app_mut(|app| {
                crate::app::KernelSessionReadService::new(app)
                    .session_snapshot(session_id)
                    .ok()
            })
            .await
        };
        Ok(session)
    }

    pub(crate) async fn capability_context(
        &self,
        session_id: &str,
        attachment_id: &str,
        capability: &'static str,
    ) -> Result<CapabilityRuntimeSnapshot, DaemonError> {
        if let Some(owned) = &self.owned {
            return owned.capability_context(session_id, attachment_id, capability);
        }
        let app = self.app.lock().await;
        let context = crate::app::KernelSessionReadService::new(&app).capability_context(
            session_id,
            attachment_id,
            capability,
        )?;
        Ok(CapabilityRuntimeSnapshot {
            workspace_id: context.workspace_id,
            worktree_root: context.worktree_root,
            workspace_coordinator: self
                .owned
                .as_ref()
                .map(|owned| owned.workspace_coordinator.clone())
                .unwrap_or_else(|| app.workspace_coordinator()),
        })
    }

    pub(crate) async fn dispatch_authenticated_runtime_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.with_app_mut(|app| {
            crate::transport::runtime_tools::dispatch_authenticated_runtime_tool_call(
                app, auth_token, tool_name, arguments,
            )
        })
        .await
    }

    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.with_app_mut(|app| {
            crate::transport::runtime_tools::dispatch_forwarded_workflow_runtime_tool_call(
                app, context, tool_name, arguments,
            )
        })
        .await
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
