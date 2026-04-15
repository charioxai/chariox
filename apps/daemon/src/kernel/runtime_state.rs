use std::sync::Arc;

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
    prompt_workspace_claims: PromptWorkspaceClaimStore,
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
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
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
                    agent_id,
                    already_ended: true,
                }))
            }
            crate::provider::ProviderRunLivenessReconciliation::NewlyEnded(run) => {
                self.clear_active_provider_run_session_pointer(session_id, provider_run_id)?;
                self.provider_run_projection.update(run.clone());
                Ok(Some(OwnedProviderRunExit {
                    ended_run: run,
                    agent_id,
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
    agent_id: String,
    already_ended: bool,
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
        prompt_workspace_claims: PromptWorkspaceClaimStore,
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
                prompt_workspace_claims,
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
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).attach(request))
            .await
    }

    pub(crate) async fn detach(
        &self,
        attachment_id: &str,
    ) -> Result<crate::attachment::RuntimeAttachment, DaemonError> {
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).detach(attachment_id))
            .await
    }

    pub(crate) async fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).focus_agent(session_id, agent_id)
        })
        .await
    }

    pub(crate) async fn cycle_agent_focus(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
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
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).spawn_agent(request))
            .await
    }

    pub(crate) async fn destroy_agent(
        &self,
        agent_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).destroy_agent(agent_id))
            .await
    }

    pub(crate) async fn end_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.with_app_mut(|app| crate::app::KernelSessionService::new(app).end_session(session_id))
            .await
    }

    pub(crate) async fn delete_session_ref(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app).delete_session_ref(session_ref, workspace_id)
        })
        .await
    }

    pub(crate) async fn submit_prepared_prompt(
        &self,
        prepared: crate::app::KernelPreparedPromptSubmission,
    ) -> Result<crate::app::KernelPromptSubmission, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelAgentService::new(app).submit_prepared_prompt_for_kernel(prepared)
        })
        .await
    }

    pub(crate) async fn cancel_agent_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        self.with_app_mut(|app| {
            crate::app::KernelAgentService::new(app).cancel_agent_prompt_for_kernel(
                session_id,
                target_agent_id,
                attachment_id,
            )
        })
        .await
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
        self.with_app_mut(|app| {
            let provider_run_id = owned_provider_run_id.unwrap_or_else(|| {
                app.providers()
                    .get_run_for_agent(session_id, target_agent_id)
                    .map(|run| run.id().to_string())
            });
            crate::app::KernelAgentService::new(app).complete_active_prompt_for_kernel(
                session_id,
                target_agent_id,
                provider_run_id.as_deref(),
                next_queued_prompt,
            )
        })
        .await
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
            .with_app_mut(|app| {
                app.settle_provider_run_exit_for_runtime(
                    session_id,
                    provider_run_id,
                    &exit.agent_id,
                )
            })
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
            if !crate::scheduler::runtime::is_workflow_prompt_attachment(
                &dispatch.source_attachment_id,
            ) {
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
        self.with_app_mut(|app| app.enqueue_kernel_prompt_dispatch(dispatch))
            .await
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
            self.with_app_mut(crate::app::provider_output::reap_structured_prompt_jobs)
                .await;
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
            return owned.provider_store.enqueue_structured_prompt_abort(
                dispatch.session_id.clone(),
                dispatch.provider_run_id.clone(),
            );
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
                        if let Err(error) = self
                            .with_app_mut(|app| {
                                app.advance_next_queued_prompt(run.session_id(), agent_id)
                            })
                            .await
                        {
                            self.fail_provider_launch(started, &error).await;
                            return;
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
        let records = self
            .with_app_mut(|app| {
                crate::app::provider_output::pump_terminal_output_for_attachment(
                    app,
                    session_id,
                    attachment_id,
                )
            })
            .await?;
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
        if !self
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
