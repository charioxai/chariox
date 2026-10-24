//! MP-08 / MP-09 / MP-10 / MP-11: a lane cannot outlive its executor deadline.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};
use crate::local::{
    AgentDeadlineState, GetAgentDeadlineRequest, LocalDaemonResponse, SetAgentDeadlineRequest,
};

fn refusal(code: &str, message: &str) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.into(),
        message: message.into(),
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn ensure_agent_before_deadline(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        if self
            .durable_state_store
            .agent_deadline(room, agent)?
            .is_some_and(|d| {
                d.state == AgentDeadlineState::Reached
                    || d.deadline_ms <= crate::session::unix_epoch_ms()
            })
        {
            return Err(refusal("agent_deadline_reached", "lane deadline reached; this agent cannot start another turn. Start a new lane agent for new work"));
        }
        Ok(())
    }
}

impl KernelRuntimeState {
    pub(crate) fn set_agent_deadline(
        &self,
        request: SetAgentDeadlineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let agent = self.owned.agent_store.get_agent(&request.agent_id)?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id,
                agent_id: request.agent_id,
            });
        }
        if agent.remote_execution().is_some() {
            return Err(refusal("agent_deadline_requires_executor", "arm a lane deadline on its executing kernel; leased-agent deadlines require worker enforcement"));
        }
        let Outcome::Deadline(deadline) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SetDeadline {
                    room: request.session_id,
                    agent: request.agent_id,
                    deadline: request.deadline_ms,
                })?
        else {
            unreachable!()
        };
        Ok(LocalDaemonResponse::AgentDeadline {
            deadline: Some(deadline),
        })
    }

    pub(crate) fn get_agent_deadline(
        &self,
        request: GetAgentDeadlineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::AgentDeadline {
            deadline: self
                .owned
                .durable_state_store
                .agent_deadline(&request.session_id, &request.agent_id)?,
        })
    }

    /// Called by A03's scheduler independently of Room-tool configuration and caller life.
    pub(super) async fn tick_agent_deadlines(&self, now: u64) -> Result<(), DaemonError> {
        let mut due = Vec::new();
        for deadline in self.owned.durable_state_store.agent_deadlines()? {
            if deadline.deadline_ms > now {
                continue;
            }
            if deadline.stopped_at_ms.is_some()
                && !self.deadline_work_remains(&deadline.session_id, &deadline.agent_id)?
            {
                continue;
            }
            // Fence every due lane before awaiting any physical cleanup.
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::ReachDeadline {
                    room: deadline.session_id.clone(),
                    agent: deadline.agent_id.clone(),
                    now,
                    stopped: false,
                })?;
            due.push(deadline);
        }
        let outcomes = futures_util::future::join_all(due.iter().map(|deadline| async move {
            let room = &deadline.session_id;
            let agent = &deadline.agent_id;
            match tokio::time::timeout(
                Duration::from_secs(5),
                self.stop_deadline_agent(room, agent),
            )
            .await
            {
                Ok(Ok(())) => self
                    .owned
                    .durable_state_store
                    .agent_lifecycle(Operation::ReachDeadline {
                        room: room.clone(),
                        agent: agent.clone(),
                        now: crate::session::unix_epoch_ms(),
                        stopped: true,
                    })
                    .map(|_| ()),
                Ok(Err(error)) => Err(error),
                Err(_) => Err(refusal(
                    "agent_deadline_cleanup_pending",
                    "deadline reached; physical cleanup is pending and will retry",
                )),
            }
        }))
        .await;
        outcomes
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map(|_| ())
    }

    fn deadline_work_remains(&self, room: &str, agent: &str) -> Result<bool, DaemonError> {
        let session = match self.owned.session_store.get_session(room) {
            Ok(session) => session,
            Err(DaemonError::SessionNotFound { .. }) => return Ok(false),
            Err(error) => return Err(error),
        };
        let (active, queued) = self.owned.prompt_state_owner.state_parts(&session, agent);
        Ok(active.is_some()
            || !queued.is_empty()
            || self
                .owned
                .provider_store
                .get_run_for_agent(room, agent)
                .is_some_and(|run| run.state() != crate::provider::ProviderRunState::Ended)
            || !self.owned.unsettled_agent_wakes(room, agent)?.is_empty())
    }

    async fn stop_deadline_agent(&self, room: &str, agent: &str) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let session = match owned.session_store.get_session(room) {
            Ok(session) => session,
            Err(DaemonError::SessionNotFound { .. }) => return Ok(()),
            Err(error) => return Err(error),
        };
        if matches!(
            owned.agent_store.get_agent(agent),
            Err(DaemonError::AgentNotFound { .. })
        ) {
            return Ok(());
        }
        let active = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent);
        let run = owned.provider_store.get_run_for_agent(room, agent);
        owned
            .prompt_state_owner
            .remove_queued_prompts_for_agent(&session, agent);
        if let Some(active) = &active {
            owned.prompt_state_owner.begin_cancelling_prompt_if_matches(
                &session,
                agent,
                Some(active.id()),
            );
        }
        owned.cancel_owned_agent_wakes(room, Some(agent));
        for task in owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
        {
            if !matches!(
                task.state,
                crate::durable_state::agent_lifecycle::ExecutionState::Done
                    | crate::durable_state::agent_lifecycle::ExecutionState::Cancelled
            ) {
                owned
                    .durable_state_store
                    .agent_lifecycle(Operation::CancelTask {
                        task: task.task_id,
                        owner: session.owner_user_id().into(),
                        revision: task.revision,
                    })?;
            }
        }
        if let Some(run) = run
            .as_ref()
            .filter(|run| run.state() != crate::provider::ProviderRunState::Ended)
        {
            // Normal kernel process handles own physical shutdown; no script PID kill.
            let (_, key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(run.id())
                })
                .await?;
            owned
                .connector_adapter_processes
                .shutdown_run(run.id())
                .await;
            owned.remove_provider_process_tracking_for_run(run.id(), key);
            let outcome = owned
                .provider_store
                .terminate_run_provider_only(room, run.id())?;
            owned.clear_active_provider_run_session_pointer(room, run.id())?;
            owned.provider_run_projection.update(outcome.into_run());
        }
        if let Some(active) = active {
            match owned.finalize_local_prompt_cancellation_with_queued_advance(
                room,
                agent,
                active.id(),
                run.as_ref().map(|run| run.id()),
            ) {
                Ok(_) | Err(DaemonError::NoActivePrompt { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        let (active, queued) = owned.prompt_state_owner.state_parts(&session, agent);
        let mirrored = owned
            .session_store
            .mirror_agent_prompt_state(room, agent, active, queued)?;
        owned.persist_prompt_session_state(&mirrored, agent)?;
        owned.await_owned_wakes_settled(room, agent).await
    }
}

impl KernelRuntimeOwnedState {
    /// Wakes (timers, watched processes, terminals, driver sources) that have
    /// not confirmed they stopped. `cancel_owned_agent_wakes` only requests
    /// the stop; a process that ignores SIGTERM stays until SIGKILL.
    pub(super) fn unsettled_agent_wakes(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<Vec<String>, DaemonError> {
        Ok(self
            .durable_state_store
            .agent_wakes(Some(room), Some(agent))?
            .into_iter()
            .filter(|wake| {
                matches!(
                    wake.state.as_str(),
                    "scheduled" | "starting" | "running" | "cancelling"
                )
            })
            .map(|wake| format!("{} ({}, {})", wake.id, wake.kind, wake.state))
            .collect())
    }

    /// The deadline is stopped only once every owned wake has settled; what is
    /// left after the bound is named so the retry and the owner can act on it.
    async fn await_owned_wakes_settled(&self, room: &str, agent: &str) -> Result<(), DaemonError> {
        let bound = Instant::now() + Duration::from_secs(4);
        loop {
            let left = self.unsettled_agent_wakes(room, agent)?;
            if left.is_empty() {
                return Ok(());
            }
            if Instant::now() >= bound {
                return Err(refusal(
                    "agent_deadline_cleanup_pending",
                    &format!(
                        "deadline reached; owned sources have not stopped yet: {}. Cleanup retries until they exit",
                        left.join(", ")
                    ),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
