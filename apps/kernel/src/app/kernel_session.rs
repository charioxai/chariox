use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, RuntimeAttachment};
use crate::error::DaemonError;
use crate::history::SessionHistoryEntry;
use crate::local::{
    GetSessionStateRequest, ListAgentsRequest, LocalDaemonResponse, ResolveSessionRequest,
};
use crate::provider::{AgentEndpointMode, ProviderRunState};
use crate::session::{
    CreateSessionRequest, RuntimeSession, SessionStateOwner, SessionStateReader, SessionStatus,
};

pub(crate) struct KernelSessionService<'a> {
    app: &'a mut DaemonApp,
}

#[cfg(test)]
mod tests {
    use crate::agent::CreateAgentRequest;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn create_session_writes_durable_state_event() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");

        let events = app
            .durable_state_store()
            .load_events_after(0)
            .expect("durable state events should load");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "session.created");
        assert_eq!(events[0].subject_id.as_deref(), Some(session.id()));
        assert_eq!(events[0].payload["session"]["id"], session.id());
        assert_eq!(events[0].payload["default_agent"]["id"], agent.id());
    }

    #[test]
    fn spawn_agent_and_end_session_write_durable_state_events() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let spawned = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
            .expect("agent should spawn");
        crate::app::KernelSessionService::new(&mut app)
            .end_session(session.id())
            .expect("session should end");

        let events = app
            .durable_state_store()
            .load_events_after(0)
            .expect("durable state events should load");
        assert_eq!(
            events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["session.created", "agent.created", "session.ended"]
        );
        assert_eq!(events[1].subject_id.as_deref(), Some(spawned.id()));
        assert_eq!(events[2].subject_id.as_deref(), Some(session.id()));
    }
}

pub(crate) struct KernelSessionReadService<'a> {
    app: &'a DaemonApp,
}

impl<'a> KernelSessionReadService<'a> {
    pub(crate) fn new(app: &'a DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn session_snapshot(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let mut session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;
        let agents = self.app.agents().get_session_agents(session_id);
        session.set_agents(agents);
        self.app.project_session_runtime_view(&mut session);
        self.app.update_session_projection(session.clone());
        Ok(session)
    }

    pub(crate) fn ensure_attachment_in_session(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<RuntimeAttachment, DaemonError> {
        let attachment = self.app.attachments.get_attachment(attachment_id)?;
        if attachment.session_id() != session_id {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        Ok(attachment)
    }

    pub(crate) fn list_sessions_response(&self) -> Result<LocalDaemonResponse, DaemonError> {
        let sessions = SessionStateReader::new(self.app.session_state_store()).list_sessions();
        let sessions_with_agents: Vec<_> = sessions
            .into_iter()
            .map(|mut session| {
                let agents = self.app.agents().get_session_agents(session.id());
                session.set_agents(agents);
                session
            })
            .collect();
        Ok(LocalDaemonResponse::SessionsListed {
            sessions: sessions_with_agents,
        })
    }

    pub(crate) fn resolve_session_response(
        &self,
        request: ResolveSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut session = SessionStateReader::new(self.app.session_state_store())
            .resolve_session_ref(&request.session_ref, request.workspace_id.as_deref())?;
        let agents = self.app.agents().get_session_agents(session.id());
        session.set_agents(agents);
        Ok(LocalDaemonResponse::SessionResolved { session })
    }

    pub(crate) fn get_session_state_response(
        &self,
        request: GetSessionStateRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self.session_snapshot(&request.session_id)?;
        Ok(LocalDaemonResponse::SessionState { session })
    }

    pub(crate) fn list_agents_response(
        &self,
        request: ListAgentsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let agents = self.app.agents().get_session_agents(&request.session_id);
        Ok(LocalDaemonResponse::AgentsListed { agents })
    }

    pub(crate) fn session_history(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionHistoryEntry>, DaemonError> {
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;
        let entries = self.app.load_session_history_entries(&session, None)?;
        self.app
            .session_history_projection_store()
            .update_entries(session.id(), entries.clone());
        Ok(entries)
    }
}

impl<'a> KernelSessionService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn create_session(
        &mut self,
        request: CreateSessionRequest,
    ) -> Result<(RuntimeSession, AgentInstance), DaemonError> {
        let session =
            SessionStateOwner::new(self.app.session_state_store()).create_session(request)?;
        let agent_request =
            CreateAgentRequest::new(session.id(), "default").with_worktree(session.worktree_id());
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let agent = self.app.agents.create_agent(agent_request, &mut sessions)?;
        drop(sessions);
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session.id())?;
        self.app.durable_state_store().append_event(
            "session.created",
            Some(session.id().to_string()),
            serde_json::json!({
                "session": &session,
                "default_agent": &agent,
            }),
        )?;

        crate::logging::info_with_fields(
            "daemon.session",
            "session created with default agent",
            serde_json::json!({
                "session_id": session.id(),
                "agent_id": agent.id(),
                "agent_ref": agent.agent_ref(),
            }),
        );

        Ok((session, agent))
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
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let attachment = self.app.attachments.attach(&mut sessions, request)?;
        drop(sessions);

        // Create default agent if session has no agents (e.g., after session was ended and reattached).
        // Parked/active sessions that were never ended will retain their existing agents.
        let session_agents = self.app.agents.get_session_agents(&session_id);
        if session_agents.is_empty() {
            let worktree_id = self
                .app
                .sessions()
                .get_session(&session_id)?
                .worktree_id()
                .to_string();
            let agent_request =
                CreateAgentRequest::new(&session_id, "default").with_worktree(worktree_id);
            let session_store = self.app.session_state_store();
            let mut sessions = session_store.write();
            let _agent = self.app.agents.create_agent(agent_request, &mut sessions)?;
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

    pub(crate) fn spawn_agent(
        &mut self,
        request: CreateAgentRequest,
    ) -> Result<AgentInstance, DaemonError> {
        if let Some(machine_ref) = request.machine_ref.clone() {
            return self.app.spawn_remote_agent(request, &machine_ref);
        }
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let agent = self.app.agents.create_agent(request, &mut sessions)?;
        drop(sessions);
        self.app.durable_state_store().append_event(
            "agent.created",
            Some(agent.id().to_string()),
            serde_json::json!({
                "agent": &agent,
            }),
        )?;
        Ok(agent)
    }

    pub(crate) fn destroy_agent(&mut self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        let agent = self.app.agents.get_agent(agent_id)?;
        if let Some(remote) = agent.remote_execution().cloned() {
            let target = arroba_relay::protocol::ClientTarget {
                daemon_id: Some(remote.worker_kernel_id.clone()),
                daemon_alias: None,
            };
            self.app.block_on_relay_future(
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &self.app.config,
                    target.clone(),
                    crate::transport::relay_peer::RelayPeerRequest::DestroyLeasedAgent {
                        leased_agent_id: remote.leased_agent_id.clone(),
                    },
                ),
            )?;
            self.app.block_on_relay_future(
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &self.app.config,
                    target,
                    crate::transport::relay_peer::RelayPeerRequest::DestroyExecutionLease {
                        lease_id: remote.execution_lease_id.clone(),
                    },
                ),
            )?;
        }
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        self.app.agents.destroy_agent(agent_id, &mut sessions)
    }

    pub(crate) fn detach(&mut self, attachment_id: &str) -> Result<RuntimeAttachment, DaemonError> {
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let (attachment, effect) = self
            .app
            .attachments
            .detach_with_effect(&mut sessions, attachment_id)?;
        drop(sessions);
        let owner_removed_queued_prompt_count =
            self.app.prompt_owner_remove_queued_prompts_by_attachment(
                attachment.session_id(),
                attachment_id,
            )?;
        let removed_queued_prompt_count = effect
            .removed_queued_prompt_count
            .max(owner_removed_queued_prompt_count);
        let session_after_detach = SessionStateReader::new(self.app.session_state_store())
            .get_session(attachment.session_id())?;

        if removed_queued_prompt_count > 0 {
            self.app.record_notice(
                attachment.session_id(),
                None,
                self.app
                    .attachments
                    .list_session_attachment_ids(attachment.session_id()),
                format!(
                    "Removed {} queued prompt(s) from detached attachment `{}`.",
                    removed_queued_prompt_count, attachment_id
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
                    let outcome = self
                        .app
                        .providers
                        .park_run_provider_only(attachment.session_id(), &active_provider_run_id)?;
                    if SessionStateReader::new(self.app.session_state_store())
                        .get_session(attachment.session_id())?
                        .active_provider_run_id()
                        == Some(outcome.run().id())
                    {
                        SessionStateOwner::new(self.app.session_state_store())
                            .set_active_provider_run(attachment.session_id(), None)?;
                    }
                    self.app.update_provider_run_projection(outcome.into_run());
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
        crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(attachment.session_id())?;

        Ok(attachment)
    }

    pub(crate) fn end_session(&mut self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let session =
            SessionStateReader::new(self.app.session_state_store()).get_session(session_id)?;

        if session.status() == SessionStatus::Ended {
            self.app.prompt_owner_remove_session(session_id);
            let ended =
                SessionStateOwner::new(self.app.session_state_store()).end_session(session_id)?;
            self.app.durable_state_store().append_event(
                "session.ended",
                Some(ended.id().to_string()),
                serde_json::json!({
                    "session": &ended,
                    "already_ended": true,
                }),
            )?;
            return Ok(ended);
        }

        let removed_attachments = self.app.attachments.remove_session_attachments(session_id);
        let terminated_runs = self
            .app
            .providers
            .terminate_session_runs_provider_only(session_id)?;
        let terminated_run_ids = terminated_runs
            .runs()
            .iter()
            .map(|outcome| outcome.run().id().to_string())
            .collect::<Vec<_>>();
        for outcome in terminated_runs.into_runs() {
            if SessionStateReader::new(self.app.session_state_store())
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(outcome.run().id())
            {
                SessionStateOwner::new(self.app.session_state_store())
                    .set_active_provider_run(session_id, None)?;
            }
            let run = outcome.into_run();
            super::provider_runtime::ProviderProcessTracker::new(self.app).remove_run(run.id())?;
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
        self.app.prompt_owner_remove_session(session_id);
        let mut ended =
            SessionStateOwner::new(self.app.session_state_store()).end_session(session_id)?;
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
        self.app.durable_state_store().append_event(
            "session.ended",
            Some(ended.id().to_string()),
            serde_json::json!({
                "session": &ended,
                "removed_attachment_ids": removed_attachments
                    .iter()
                    .map(|attachment| attachment.id().to_string())
                    .collect::<Vec<_>>(),
                "terminated_provider_run_ids": terminated_run_ids,
                "removed_agents": removed_agent_ids,
            }),
        )?;
        Ok(ended)
    }

    pub(crate) fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentInstance, DaemonError> {
        let session_store = self.app.session_state_store();
        let mut sessions = session_store.write();
        let agent = self
            .app
            .agents
            .focus_agent(session_id, agent_id, &mut sessions)?;
        drop(sessions);
        if !self
            .app
            .should_defer_provider_run_sync_for_focus_change(session_id, agent_id)?
        {
            self.app
                .sync_active_provider_run_for_agent(session_id, agent_id)?;
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
            .sessions()
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
        let _ = super::provider_runtime::ProviderRunLivenessRuntime::new(self.app)
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = crate::app::ProviderRunReadService::new(self.app)
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
