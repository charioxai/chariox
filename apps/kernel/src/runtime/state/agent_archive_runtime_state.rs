//! MP-08 / MP-09 / MP-10 / MP-11: suspend one agent without removing its identity.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};

impl KernelRuntimeState {
    pub(crate) async fn archive_agent_in_session(
        &self,
        request: &crate::local::ArchiveAgentRequest,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.authorize_current_external_command()?;
        let agent =
            self.owned
                .ensure_agent_owner(&request.agent_id, caller_user_id, "archive agent")?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id.clone(),
                agent_id: request.agent_id.clone(),
            });
        }
        if !request.archived && !agent.archived() {
            return Ok(agent);
        }
        let session = self.owned.session_store.get_session(&request.session_id)?;
        if !request.archived
            && session
                .agents()
                .iter()
                .filter(|agent| !agent.archived())
                .count()
                >= session.max_agents() as usize
        {
            return Err(DaemonError::AgentLimitReached {
                session_id: request.session_id.clone(),
                max_agents: session.max_agents(),
            });
        }
        if !agent.archived() {
            self.commit_agent_archive(&session, agent.id(), true)?;
        }
        // The durable archived flag fences new work even if a cleanup fails or
        // the kernel stops here. Unarchive always finishes cleanup before opening.
        self.finish_agent_archive(&request.session_id, agent.id()).await.map_err(|error| DaemonError::LocalTransport {
            operation: "archive agent",
            message: format!("Agent is archived, but cleanup is pending: {error}. Retry archive to finish cleanup."),
        })?;
        if !request.archived {
            if agent.remote_execution().is_some() {
                self.refresh_remote_agent_binding_authorized(agent.id())
                    .await?;
            }
            let session = self.owned.session_store.get_session(&request.session_id)?;
            if let Err(error) = self.commit_agent_archive(&session, agent.id(), false) {
                // A newly reacquired lease must not outlive a failed reopen.
                if let Err(cleanup) = self
                    .finish_agent_archive(&request.session_id, agent.id())
                    .await
                {
                    return Err(DaemonError::LocalTransport {
                        operation: "unarchive agent",
                        message: format!("Agent remains archived: {error}. Lease cleanup is pending: {cleanup}. Retry archive to finish cleanup."),
                    });
                }
                return Err(error);
            }
        }
        if request.archived {
            self.owned.record_notice_for_agent(&request.session_id, None, Some(agent.id()), self.owned.attachment_store.list_session_attachment_ids(&request.session_id), "Agent archived. Its running and queued work was cancelled; unarchive and send a new prompt to continue.");
        }
        self.owned
            .sync_focused_provider_run_if_idle(&request.session_id)?;
        self.owned.agent_store.get_agent(agent.id())
    }

    fn commit_agent_archive(
        &self,
        session: &crate::session::RuntimeSession,
        agent_id: &str,
        archived: bool,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        // Fence admission first, without holding the owner lock across agent
        // locks or durable writer I/O. An App call may need both to settle.
        if archived {
            self.owned
                .prompt_state_owner
                .change_agent_archive(session, agent_id, true, || Ok(()))?;
        }
        let previous = self.owned.agent_store.get_session_agents(session.id());
        let focused = self
            .owned
            .session_store
            .get_session(session.id())?
            .focused_agent_id()
            .map(str::to_string);
        let result = (|| {
            let agent = self.owned.agent_store.archive_agent(
                agent_id,
                archived,
                &mut self.owned.session_store.write(),
            )?;
            self.record_agent_durable_event("agent.updated", &agent, None)?;
            Ok(agent)
        })();
        match result {
            Ok(agent) => {
                if !archived {
                    self.owned.prompt_state_owner.change_agent_archive(
                        session,
                        agent_id,
                        false,
                        || Ok(()),
                    )?;
                }
                Ok(agent)
            }
            Err(error) => {
                let was_archived = previous
                    .iter()
                    .find(|agent| agent.id() == agent_id)
                    .is_some_and(|agent| agent.archived());
                for agent in previous {
                    self.owned.agent_store.restore_agent(agent);
                }
                self.owned
                    .session_store
                    .write()
                    .set_focused_agent(session.id(), focused)?;
                self.owned.prompt_state_owner.change_agent_archive(
                    session,
                    agent_id,
                    was_archived,
                    || Ok(()),
                )?;
                Err(error)
            }
        }
    }

    async fn finish_agent_archive(&self, room: &str, agent_id: &str) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let session = owned.session_store.get_session(room)?;
        for task in owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent_id))?
        {
            if !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled) {
                owned
                    .durable_state_store
                    .agent_lifecycle(Operation::ArchiveTask {
                        task: task.task_id,
                        owner: if task.owner_user_id.is_empty() {
                            session.owner_user_id().into()
                        } else {
                            task.owner_user_id
                        },
                        revision: task.revision,
                    })?;
            }
        }
        let (_, queued) = owned.prompt_state_owner.state_parts(&session, agent_id);
        for prompt in queued {
            if let Some(mut cancelled) =
                owned
                    .prompt_state_owner
                    .remove_queued_prompt(&session, agent_id, prompt.id())
            {
                cancelled.set_status(crate::session::PromptStatus::Cancelled);
                // Archive is kernel-owned cleanup, independent of whether the
                // attachment that submitted the queued prompt still exists.
                owned.record_cancelled_prompt_settlement(room, agent_id, &cancelled, None);
                owned.record_notice_for_agent(
                    room,
                    None,
                    Some(agent_id),
                    owned.attachment_store.list_session_attachment_ids(room),
                    format!(
                        "Queued prompt `{}` cancelled: agent archived.",
                        cancelled.id()
                    ),
                );
            }
        }
        let (active, queued) = owned.prompt_state_owner.state_parts(&session, agent_id);
        owned.mirror_prompt_owner_agent_state(room, agent_id, active, queued)?;
        for wake in owned
            .durable_state_store
            .agent_wakes(Some(room), Some(agent_id))?
        {
            if matches!(
                wake.state.as_str(),
                "scheduled" | "starting" | "running" | "cancelling"
            ) {
                self.cancel_agent_wake(&wake, &wake.task_id, None).await?;
            }
        }
        let schedules = session
            .agent_prompt_schedules()
            .iter()
            .filter(|s| s.agent_id() == agent_id)
            .map(|s| s.id().to_string())
            .collect::<Vec<_>>();
        if !schedules.is_empty() {
            for schedule in schedules {
                owned
                    .session_store
                    .write()
                    .cancel_agent_prompt_schedule(room, &schedule)?;
            }
            owned.persist_agent_prompt_schedule_session(
                room,
                "agent_archived_schedules_cancelled",
            )?;
        }
        let agent = owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some() {
            // Acknowledge worker cleanup while the home binding is retained.
            self.with_app_side_effect(|app| {
                crate::app::KernelSessionService::new(app)
                    .destroy_agent_worker_execution_authorized(&agent, &|| {
                        self.authorize_current_external_command()
                    })
            })
            .await?;
            // Like destroy-agent cleanup, retire home-only worker runs after
            // the worker acknowledges release. Ended fences delayed snapshots.
            for mut run in owned.provider_run_projection.list_for_session(room) {
                if run.agent_instance_id() == Some(agent_id) {
                    run.mark_ended();
                    owned.clear_active_provider_run_session_pointer(room, run.id())?;
                    owned.clear_prompt_activity(run.id());
                    owned.provider_run_projection.update(run);
                }
            }
        }
        let runs = owned
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|run| run.session_id() == room && run.agent_instance_id() == Some(agent_id))
            .collect::<Vec<_>>();
        for run in &runs {
            // Termination and provider spawn share the live-run store fence.
            // Publish Ended before removing handles so a cold spawn cannot land
            // behind cleanup after it inspected an empty process map.
            let ended = owned
                .provider_store
                .terminate_run_provider_only(room, run.id())?;
            let (_, key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(run.id())
                })
                .await?;
            owned.remove_provider_process_tracking_for_run(run.id(), key);
            owned.clear_active_provider_run_session_pointer(room, run.id())?;
            owned.provider_run_projection.update(ended.into_run());
        }
        if owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            match owned.cancel_active_prompt_only(room, agent_id) {
                Ok(_) | Err(DaemonError::NoActivePrompt { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        let snapshot = owned.session_snapshot_without_projection_update(room)?;
        owned.persist_prompt_session_state(&snapshot, agent_id)?;
        for run in &runs {
            owned.clear_prompt_activity(run.id());
        }
        owned.withdraw_agent_interactions(room, Some(agent_id))?;
        owned.room_terminals.stop_agent(room, agent_id)?;
        owned.await_owned_wakes_settled(room, agent_id).await?;
        owned.agent_store.set_agent_processing(agent_id, false)?;
        let agent = owned
            .agent_store
            .set_agent_state(agent_id, crate::agent::AgentState::Idle)?;
        self.record_agent_durable_event("agent.updated", &agent, None)?;
        Ok(())
    }
}
