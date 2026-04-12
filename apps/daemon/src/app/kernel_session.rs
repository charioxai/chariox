use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, RuntimeAttachment};
use crate::error::DaemonError;
use crate::provider::{AgentEndpointMode, ProviderRunState};
use crate::session::{RuntimeSession, SessionStatus};

pub(crate) struct KernelSessionService<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelSessionService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn attach(
        &mut self,
        request: AttachRequest,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let session_id = request.session_id.clone();
        let client_id = request.client_id.clone();
        let capability_level = format!("{:?}", request.capability_level);
        let replaced_attachment_ids = self
            .app
            .attachments
            .list_client_attachments(&client_id)
            .into_iter()
            .map(|attachment| attachment.id().to_string())
            .collect::<Vec<_>>();
        for attachment_id in &replaced_attachment_ids {
            let _ = self.detach(attachment_id)?;
        }
        let attachment = self
            .app
            .attachments
            .attach(&mut self.app.sessions, request)?;

        // Create default agent if session has no agents (e.g., after session was ended and reattached).
        // Parked/active sessions that were never ended will retain their existing agents.
        let session_agents = self.app.agents.get_session_agents(&session_id);
        if session_agents.is_empty() {
            let worktree_id = self
                .app
                .sessions
                .get_session(&session_id)?
                .worktree_id()
                .to_string();
            let agent_request =
                CreateAgentRequest::new(&session_id, "default").with_worktree(worktree_id);
            let _agent = self
                .app
                .agents
                .create_agent(agent_request, &mut self.app.sessions)?;
            crate::logging::info_with_fields(
                "daemon.app",
                "created default agent for session",
                serde_json::json!({
                    "session_id": session_id,
                    "reason": "session had no agents (possibly after being ended and reattached)",
                }),
            );
        }

        self.app.sync_focused_provider_run_if_idle(&session_id)?;

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

    pub(crate) fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        let (attachment, effect) = self
            .app
            .attachments
            .detach_with_effect(&mut self.app.sessions, attachment_id)?;
        let session_after_detach = self.app.sessions.get_session(attachment.session_id())?;

        if effect.removed_queued_prompt_count > 0 {
            self.app.record_notice(
                attachment.session_id(),
                None,
                self.app
                    .attachments
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed {} queued prompt(s) from detached attachment `{}`.",
                    effect.removed_queued_prompt_count, attachment_id
                ),
            );
        }

        if effect.removed_active_prompt {
            self.app.record_notice(
                attachment.session_id(),
                None,
                self.app
                    .attachments
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed the active prompt from detached attachment `{}` and advanced the queue.",
                    attachment_id
                ),
            );
            if let Some(agent_id) = session_after_detach.focused_agent_id() {
                let _ = self
                    .app
                    .advance_next_queued_prompt(attachment.session_id(), agent_id)?;
            }
        }

        let remaining_attachment_ids = self
            .app
            .attachments
            .list_session_attachment_ids(attachment.session_id());
        if remaining_attachment_ids.is_empty() && session_after_detach.active_prompt().is_none() {
            if let Some(active_provider_run_id) = session_after_detach
                .active_provider_run_id()
                .map(str::to_string)
            {
                let run = self.app.providers.get_run(&active_provider_run_id)?;
                if run.state() != ProviderRunState::Ended {
                    self.app.providers.park_run(
                        &mut self.app.sessions,
                        attachment.session_id(),
                        &active_provider_run_id,
                    )?;
                }
            }
            for run in self.app.providers.list_runs() {
                if run.session_id() == attachment.session_id() {
                    crate::transport::flow_control::clear_prompt_activity(self.app, run.id());
                }
            }
        }

        crate::logging::info_with_fields(
            "daemon.session",
            "attachment left session",
            serde_json::json!({
                "session_id": attachment.session_id(),
                "attachment_id": attachment.id(),
                "removed_queued_prompts": effect.removed_queued_prompt_count,
                "removed_active_prompt": effect.removed_active_prompt,
                "remaining_attachment_ids": remaining_attachment_ids,
            }),
        );

        Ok(attachment)
    }

    pub(crate) fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let session = self.app.sessions.get_session(session_id)?;

        if session.status() == SessionStatus::Ended {
            return self.app.sessions.end_session(session_id);
        }

        let removed_attachments = self.app.attachments.remove_session_attachments(session_id);
        let terminated_runs = self
            .app
            .providers
            .terminate_session_runs(&mut self.app.sessions, session_id)?;
        let terminated_run_ids = terminated_runs
            .iter()
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        for run in terminated_runs {
            self.app.remove_tracked_provider_process_for_run(run.id())?;
        }

        let removed_agents = self.app.agents.remove_session_agents(session_id);
        let removed_agent_ids: Vec<_> = removed_agents
            .iter()
            .map(|agent| format!("{} ({})", agent.agent_ref(), agent.id()))
            .collect();

        for run in self.app.providers.list_runs() {
            if run.session_id() == session_id {
                crate::transport::flow_control::clear_prompt_activity(self.app, run.id());
            }
        }
        let ended = self.app.sessions.end_session(session_id)?;
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
        Ok(ended)
    }

    pub(crate) fn delete_session_ref(
        &mut self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<RuntimeSession, DaemonError> {
        let session = self
            .app
            .sessions
            .resolve_session_ref(session_ref, workspace_id)?;
        let session_id = session.id().to_string();
        let ended = self.end_session(&session_id)?;
        let deleted = self.app.sessions.delete_session(ended.id())?;
        crate::logging::info_with_fields(
            "daemon.session",
            "session deleted",
            serde_json::json!({
                "session_id": deleted.id(),
                "session_alias": deleted.alias(),
            }),
        );
        Ok(deleted)
    }

    pub(crate) fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .app
            .agents
            .focus_agent(session_id, agent_id, &mut self.app.sessions)?;
        if !self
            .app
            .should_defer_provider_run_sync_for_focus_change(session_id, agent_id)?
        {
            self.app
                .sync_active_provider_run_for_agent(session_id, agent_id)?;
        }
        Ok(agent)
    }

    pub(crate) fn cycle_agent_focus(
        &mut self,
        session_id: &str,
    ) -> Result<Option<AgentInstance>, DaemonError> {
        let agent = self
            .app
            .agents
            .cycle_focus(session_id, &mut self.app.sessions)?;
        if let Some(focused) = agent.as_ref() {
            if !self
                .app
                .should_defer_provider_run_sync_for_focus_change(session_id, focused.id())?
            {
                self.app
                    .sync_active_provider_run_for_agent(session_id, focused.id())?;
            }
        }
        Ok(agent)
    }

    pub(crate) fn resize_terminal(
        &mut self,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let provider_run_id = self
            .app
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();

        self.resize_provider_terminal(session_id, &provider_run_id, cols, rows)
    }

    pub(crate) fn resize_provider_terminal(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), DaemonError> {
        let _ = self
            .app
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = self
            .app
            .ensure_provider_run_in_session(session_id, provider_run_id)?;

        if provider_run.state() == ProviderRunState::Ended {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "resize terminal",
            });
        }

        if provider_run.endpoint_mode() == AgentEndpointMode::External {
            return Ok(());
        }

        self.app.pty.resize(provider_run_id, cols, rows)
    }
}
