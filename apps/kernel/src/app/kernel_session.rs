use crate::agent::{AgentInstance, CreateAgentRequest};
use crate::app::DaemonApp;
use crate::attachment::{AttachRequest, RuntimeAttachment};
use crate::error::DaemonError;
use crate::history::SessionHistoryEntry;
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
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::session::{
        CreateSessionRequest, SchedulerState, SessionStatus, WorkflowNodeRun,
        WorkflowNodeRunStatus, WorkflowRun, WorkflowRunStatus,
    };
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

    #[test]
    fn bootstrap_restores_created_session_and_agents_from_durable_state() {
        let config = DaemonConfig::for_tests();
        let (session_id, default_agent_id, reviewer_agent_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            let reviewer = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
                .expect("agent should spawn");
            (
                session.id().to_string(),
                default_agent.id().to_string(),
                reviewer.id().to_string(),
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_session = app
            .sessions()
            .get_session(&session_id)
            .expect("session should restore");
        assert_eq!(restored_session.id(), session_id);
        assert_eq!(
            app.agents
                .get_agent(&default_agent_id)
                .expect("default agent should restore")
                .session_id(),
            session_id
        );
        assert_eq!(
            app.agents
                .get_agent(&reviewer_agent_id)
                .expect("spawned agent should restore")
                .session_id(),
            session_id
        );
    }

    #[test]
    fn bootstrap_restores_ended_session_without_live_agents() {
        let config = DaemonConfig::for_tests();
        let session_id = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
                .expect("agent should spawn");
            crate::app::KernelSessionService::new(&mut app)
                .end_session(session.id())
                .expect("session should end");
            session.id().to_string()
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_session = app
            .sessions()
            .get_session(&session_id)
            .expect("ended session should restore");
        assert_eq!(restored_session.status(), SessionStatus::Ended);
        assert!(
            app.agents.get_session_agents(&session_id).is_empty(),
            "ended sessions should not restore live agents"
        );
    }

    #[test]
    fn bootstrap_restores_snapshot_then_replays_later_events() {
        let config = DaemonConfig::for_tests();
        let (session_id, reviewer_agent_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            app.save_durable_state_snapshot()
                .expect("snapshot should save");
            let reviewer = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
                .expect("post-snapshot agent should spawn");
            (session.id().to_string(), reviewer.id().to_string())
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        app.sessions()
            .get_session(&session_id)
            .expect("snapshot session should restore");
        assert_eq!(
            app.agents
                .get_agent(&reviewer_agent_id)
                .expect("post-snapshot event should replay")
                .session_id(),
            session_id
        );
    }

    #[test]
    fn bootstrap_restores_metaagent_events_from_snapshot_then_replays_state() {
        let config = DaemonConfig::for_tests();
        let (metaagent_id, event_id, subscription_id, deleted_subscription_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, metaagent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(
                    CreateSessionRequest::new("workspace", "worktree").with_metaagent(true),
                )
                .expect("metaagent session should create");
            let metaagent_id = metaagent.id().to_string();
            let event = app.metaagent_event_store().record(
                crate::runtime::metaagent_event::NewMetaagentEvent {
                    session_id: session.id().to_string(),
                    metaagent_id: metaagent_id.clone(),
                    owner_user_id: metaagent.owner_user_id().to_string(),
                    kind: "agent.turn.completed".to_string(),
                    source_agent_id: None,
                    title: "Worker completed".to_string(),
                    summary: "Worker completed a turn".to_string(),
                    detail: serde_json::json!({ "prompt_id": "prompt-1" }),
                    injected_prompt_id: Some("prompt-meta-1".to_string()),
                },
            );
            app.durable_state_store()
                .append_event(
                    "metaagent.event.recorded",
                    Some(event.event_id.clone()),
                    serde_json::json!({ "record": &event }),
                )
                .expect("event record should persist");
            app.save_durable_state_snapshot()
                .expect("snapshot should save recorded event");

            let delivered = app
                .metaagent_event_store()
                .update_prompt_delivery_status(
                    &event.event_id,
                    crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Queued,
                    None,
                )
                .expect("event delivery status should update");
            app.durable_state_store()
                .append_event(
                    "metaagent.event.delivery_updated",
                    Some(delivered.event_id.clone()),
                    serde_json::json!({ "record": &delivered }),
                )
                .expect("event delivery update should persist");

            let read = app
                .metaagent_event_store()
                .read(&metaagent_id, &event.event_id)
                .expect("event should read");
            app.durable_state_store()
                .append_event(
                    "metaagent.event.read",
                    Some(read.event_id.clone()),
                    serde_json::json!({ "record": &read }),
                )
                .expect("event read should persist");

            let acked =
                app.metaagent_event_store()
                    .ack(&metaagent_id, &[event.event_id.clone()], None);
            let acked_event = acked.first().expect("event should ack");
            app.durable_state_store()
                .append_event(
                    "metaagent.event.acked",
                    Some(acked_event.event_id.clone()),
                    serde_json::json!({ "record": acked_event }),
                )
                .expect("event ack should persist");

            let subscription = app.metaagent_event_store().subscribe(
                &metaagent_id,
                "workflow.run.completed".to_string(),
                None,
            );
            app.durable_state_store()
                .append_event(
                    "metaagent.subscription.created",
                    Some(subscription.subscription_id.clone()),
                    serde_json::json!({ "subscription": &subscription }),
                )
                .expect("subscription should persist");

            let deleted_subscription = app.metaagent_event_store().subscribe(
                &metaagent_id,
                "workflow.run.failed".to_string(),
                None,
            );
            app.durable_state_store()
                .append_event(
                    "metaagent.subscription.created",
                    Some(deleted_subscription.subscription_id.clone()),
                    serde_json::json!({ "subscription": &deleted_subscription }),
                )
                .expect("deleted subscription create should persist");
            let deleted_subscription = app
                .metaagent_event_store()
                .unsubscribe(&metaagent_id, &deleted_subscription.subscription_id)
                .expect("subscription should remove");
            app.durable_state_store()
                .append_event(
                    "metaagent.subscription.deleted",
                    Some(deleted_subscription.subscription_id.clone()),
                    serde_json::json!({ "subscription": &deleted_subscription }),
                )
                .expect("subscription deletion should persist");

            (
                metaagent_id,
                event.event_id,
                subscription.subscription_id,
                deleted_subscription.subscription_id,
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_events = app.metaagent_event_store().list(
            &metaagent_id,
            Some("agent.turn.completed"),
            Some("acked"),
            10,
        );
        assert_eq!(restored_events.len(), 1);
        assert_eq!(restored_events[0].event_id, event_id);
        assert!(restored_events[0].read_at_ms.is_some());
        assert!(restored_events[0].ack_at_ms.is_some());
        assert_eq!(
            restored_events[0].injected_prompt_id.as_deref(),
            Some("prompt-meta-1")
        );
        assert_eq!(
            restored_events[0].prompt_delivery_status,
            crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Queued
        );
        assert!(restored_events[0].prompt_delivery_updated_at_ms.is_some());

        let restored_subscriptions = app
            .metaagent_event_store()
            .list_subscriptions(&metaagent_id);
        assert_eq!(restored_subscriptions.len(), 1);
        assert_eq!(restored_subscriptions[0].subscription_id, subscription_id);
        assert_ne!(
            restored_subscriptions[0].subscription_id,
            deleted_subscription_id
        );
    }

    #[test]
    fn bootstrap_reconciles_stale_runtime_work_after_restart() {
        let config = DaemonConfig::for_tests();
        let (session_id, workflow_run_id, workflow_node_run_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should create");
            let attachment = crate::app::KernelSessionService::new(&mut app)
                .attach(AttachRequest::new(
                    session.id(),
                    "client-a",
                    ClientCapabilityLevel::FullTerminal,
                ))
                .expect("attachment should attach");
            app.sessions_mut()
                .submit_prompt(
                    session.id(),
                    attachment.id(),
                    agent.id(),
                    "still running when the kernel stops",
                    Vec::new(),
                )
                .expect("prompt should start");

            let mut session = app
                .sessions()
                .get_session(session.id())
                .expect("session should still exist");
            let session_id = session.id().to_string();
            session.set_active_provider_run(Some("provider-run-stale".to_string()));
            let node_run = WorkflowNodeRun::new(
                "node-run-stale",
                "node-1",
                agent.id(),
                1,
                WorkflowNodeRunStatus::Running,
            );
            let mut workflow_run = WorkflowRun::new(
                "workflow-run-stale",
                "workflow-1",
                "endpoint-1",
                "node-1",
                Some("invoke".to_string()),
                None,
                vec![node_run],
                Vec::new(),
            );
            workflow_run.set_active_node_run("node-run-stale");
            workflow_run.set_status(WorkflowRunStatus::Running);
            session.create_workflow_run(workflow_run);
            app.sessions.restore_session(session);
            app.save_durable_state_snapshot()
                .expect("snapshot should save stale runtime state");
            (
                session_id,
                "workflow-run-stale".to_string(),
                "node-run-stale".to_string(),
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored = app
            .sessions()
            .get_session(&session_id)
            .expect("session should restore");
        assert_eq!(restored.active_provider_run_id(), None);
        assert!(restored.active_prompt().is_none());
        assert_eq!(restored.scheduler_state(), SchedulerState::Idle);
        let workflow_run = restored
            .workflow_run(&workflow_run_id)
            .expect("workflow run should restore");
        assert_eq!(workflow_run.status(), WorkflowRunStatus::Stopped);
        assert_eq!(workflow_run.active_node_run_id(), None);
        assert_eq!(
            workflow_run.node_runs()[0].status(),
            WorkflowNodeRunStatus::Stopped
        );
        assert_eq!(workflow_run.node_runs()[0].id(), workflow_node_run_id);
        assert!(workflow_run
            .failure_events()
            .iter()
            .any(|event| { event.message().contains("interrupted by kernel restart") }));
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
        let defaults = session.agent_defaults();
        let mut agent_request = CreateAgentRequest::new(session.id(), &defaults.provider)
            .with_owner_user_id(session.owner_user_id().to_string())
            .with_worktree(session.worktree_id());
        if let Some(model) = defaults.model.as_deref() {
            agent_request = agent_request.with_model(model.to_string());
        }
        if let Some(effort) = defaults.effort.as_deref() {
            agent_request = agent_request.with_effort(effort.to_string());
        }
        if let Some(execution_mode) = defaults.execution_mode {
            agent_request = agent_request.with_execution_mode_override(execution_mode);
        }
        if let Some(permission_level) = defaults.permission_level {
            agent_request = agent_request.with_permission_level_override(permission_level);
        }
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
        mut request: CreateAgentRequest,
    ) -> Result<AgentInstance, DaemonError> {
        if let Some(kernel_ref) = request.kernel_ref.clone() {
            if self.app.kernel_ref_is_local(&kernel_ref) {
                request.kernel_ref = None;
            } else {
                let agent = self.app.spawn_worker_agent(request, &kernel_ref)?;
                self.app.durable_state_store().append_event(
                    "agent.created",
                    Some(agent.id().to_string()),
                    serde_json::json!({
                        "agent": &agent,
                    }),
                )?;
                return Ok(agent);
            }
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
                    crate::transport::flow_control::clear_active_turn(self.app, run.id());
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
            self.app
                .external_provider_session_index_store()
                .detach_session(session_id);
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
                crate::transport::flow_control::clear_active_turn(self.app, run.id());
            }
        }
        self.app.prompt_owner_remove_session(session_id);
        self.app
            .external_provider_session_index_store()
            .detach_session(session_id);
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
