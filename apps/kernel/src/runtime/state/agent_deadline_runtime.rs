//! MP-08 / MP-09 / MP-10 / MP-11: an agent run cannot outlive its executor deadline.
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
            return Err(refusal("agent_deadline_reached", "agent deadline reached; this agent cannot start another turn. Start a new agent for new work"));
        }
        Ok(())
    }
}

impl KernelRuntimeState {
    pub(crate) async fn set_agent_deadline(
        &self,
        caller_user_id: &str,
        request: SetAgentDeadlineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        // A reached deadline ends the agent's work, so only who acts for its owner arms one.
        let agent = self.owned.ensure_agent_owner(
            &request.agent_id,
            caller_user_id,
            "set agent deadline",
        )?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id,
                agent_id: request.agent_id,
            });
        }
        // A deadline is never later than an ancestor's: the parent's deadline ends its team.
        let agents = self
            .owned
            .agent_store
            .get_session_agents(&request.session_id);
        for ancestor in crate::agent::authority_limit::ancestors(&agent, &agents) {
            if self
                .owned
                .durable_state_store
                .agent_deadline(&request.session_id, ancestor.id())?
                .is_some_and(|parent| request.deadline_ms > parent.deadline_ms)
            {
                return Err(refusal(
                    "agent_deadline_after_parent",
                    "an agent deadline cannot be later than its parent's; use a shorter time",
                ));
            }
        }
        // The worker enforces a leased agent's deadline itself, so the home asks it first: a
        // refused request leaves no deadline the worker does not hold.
        self.arm_leased_agent_deadline(&agent, request.deadline_ms)
            .await?;
        let Outcome::Deadline(deadline) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SetDeadline {
                    room: request.session_id.clone(),
                    agent: request.agent_id.clone(),
                    deadline: request.deadline_ms,
                })?
        else {
            unreachable!()
        };
        // Everything below follows a shortened deadline at once.
        for descendant in crate::agent::authority_limit::descendants(&request.agent_id, &agents) {
            let later = self
                .owned
                .durable_state_store
                .agent_deadline(&request.session_id, descendant.id())?
                .is_none_or(|own| own.deadline_ms > request.deadline_ms);
            if later {
                self.arm_leased_agent_deadline(descendant, request.deadline_ms)
                    .await?;
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::SetDeadline {
                        room: request.session_id.clone(),
                        agent: descendant.id().to_string(),
                        deadline: request.deadline_ms,
                    })?;
            }
        }
        Ok(LocalDaemonResponse::AgentDeadline {
            deadline: Some(deadline),
        })
    }

    /// Asks the worker that runs a leased agent to arm the deadline on its backing agent. An
    /// agent that runs on this kernel needs nothing here.
    async fn arm_leased_agent_deadline(
        &self,
        agent: &crate::agent::AgentInstance,
        deadline_ms: u64,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        if self
            .owned
            .durable_state_store
            .agent_deadline(agent.session_id(), agent.id())?
            .is_some_and(|own| deadline_ms > own.deadline_ms)
        {
            return Err(refusal(
                "agent_deadline_extension_refused",
                "an agent deadline cannot be extended; start a new agent",
            ));
        }
        self.authorize_current_external_command()?;
        let mut config = self.config_snapshot().await;
        if let (Some(relay_url), Some(relay_token)) = (
            remote_execution.relay_url.clone(),
            remote_execution.relay_token.clone(),
        ) {
            config.apply_remote_relay_override(relay_url, relay_token);
        }
        let response = tokio::time::timeout(Duration::from_secs(5), async {
            self.enqueue_leased_worker_request_authorized(
                &config,
                chariox_relay::protocol::ClientTarget {
                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                crate::transport::relay_peer::RelayPeerRequest::SetLeasedAgentDeadline {
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                    deadline_ms,
                },
                Duration::from_secs(5),
                || self.authorize_current_external_command(),
            )
            .await?
            .wait()
            .await
        })
        .await
        .map_err(|_| {
            refusal(
                "agent_deadline_worker_unreachable",
                "the worker did not confirm the agent deadline in time; retry when it is reachable",
            )
        })??;
        match response {
            crate::transport::relay_peer::RelayPeerResponse::LeasedAgentDeadlineSet { .. } => {
                Ok(())
            }
            other => Err(refusal(
                "agent_deadline_worker_refused",
                &format!("unexpected worker deadline response: {other:?}"),
            )),
        }
    }

    /// A client's read: the deadline says when the agent's work stops, so whoever may view the
    /// agent's trace reads it. Arming one stays with who acts for its owner.
    pub(crate) fn read_agent_deadline(
        &self,
        caller_user_id: &str,
        request: GetAgentDeadlineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.owned.ensure_agent_trace_access(
            &request.agent_id,
            caller_user_id,
            "read agent deadline",
        )?;
        self.get_agent_deadline(request)
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
            // Fence every due agent before awaiting any physical cleanup.
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

    /// The agent's own work, or work of any sub-agent below it (A4).
    fn deadline_work_remains(&self, room: &str, agent: &str) -> Result<bool, DaemonError> {
        if self.agent_work_remains(room, agent)? {
            return Ok(true);
        }
        for descendant in self.descendants_depth_first(agent) {
            if self.agent_work_remains(room, descendant.id())? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn agent_work_remains(&self, room: &str, agent: &str) -> Result<bool, DaemonError> {
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

    /// A deadline ends the agent and every sub-agent below it, depth-first.
    /// A sub-agent whose cleanup is pending does not keep the others running.
    async fn stop_deadline_agent(&self, room: &str, agent: &str) -> Result<(), DaemonError> {
        let mut first_error = self.stop_one_deadline_agent(room, agent).await.err();
        for descendant in self.descendants_depth_first(agent) {
            if let Err(error) = self.stop_one_deadline_agent(room, descendant.id()).await {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn stop_one_deadline_agent(&self, room: &str, agent: &str) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let session = match owned.session_store.get_session(room) {
            Ok(session) => session,
            Err(DaemonError::SessionNotFound { .. }) => return Ok(()),
            Err(error) => return Err(error),
        };
        let leased = match owned.agent_store.get_agent(agent) {
            Ok(agent) => agent.remote_execution().is_some(),
            Err(DaemonError::AgentNotFound { .. }) => return Ok(()),
            Err(error) => return Err(error),
        };
        let active = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent);
        owned
            .prompt_state_owner
            .remove_queued_prompts_for_agent(&session, agent);
        if let Some(active) = active.as_ref().filter(|_| leased) {
            // The worker runs this turn: cancel it there, as an explicit cancel does, and
            // keep the cleanup pending until the worker settles it.
            Box::pin(self.cancel_remote_agent_prompt_if_matches(
                room,
                agent,
                active.source_attachment_id(),
                Some(active.id()),
                None,
            ))
            .await?;
            owned.persist_deadline_prompt_state(&session, agent)?;
            return Err(refusal(
                "agent_deadline_cleanup_pending",
                &format!("deadline reached; the worker has not settled the turn of agent {agent} yet. Cleanup retries"),
            ));
        }
        // A leased agent's provider process lives on its worker.
        let run = owned
            .provider_store
            .get_run_for_agent(room, agent)
            .filter(|_| !leased);
        if let Some(active) = &active {
            owned.prompt_state_owner.begin_cancelling_prompt_if_matches(
                &session,
                agent,
                Some(active.id()),
            );
        }
        // The wakes' retirement can end these tasks before the loop below sees them.
        let unfinished_tasks = owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .filter(|task| {
                !matches!(
                    task.state,
                    crate::durable_state::agent_lifecycle::ExecutionState::Done
                        | crate::durable_state::agent_lifecycle::ExecutionState::Cancelled
                )
            })
            .map(|task| task.task_id)
            .collect::<Vec<_>>();
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
        // The deadline ended these tasks: their parent hears of it.
        for task_id in unfinished_tasks {
            let Some(ended) = owned.durable_state_store.agent_task(&task_id)? else {
                continue;
            };
            let answer = owned.answer_with_running_background_note(&ended, None)?;
            owned.background_holds.release(&ended.task_id);
            if let Err(error) = self
                .announce_child_finished(&ended, None, answer.as_ref())
                .await
            {
                tracing::warn!(%error, agent_id=%agent,
                    "child_finished retained for the inbox sweep");
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
        owned.persist_deadline_prompt_state(&session, agent)?;
        owned.await_owned_wakes_settled(room, agent).await
    }
}

impl KernelRuntimeOwnedState {
    fn persist_deadline_prompt_state(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &str,
    ) -> Result<(), DaemonError> {
        let (active, queued) = self.prompt_state_owner.state_parts(session, agent);
        let mirrored =
            self.session_store
                .mirror_agent_prompt_state(session.id(), agent, active, queued)?;
        self.persist_prompt_session_state(&mirrored, agent)
    }

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
    pub(super) async fn await_owned_wakes_settled(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
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
                        "owned sources have not stopped yet: {}. Retry cleanup after they exit",
                        left.join(", ")
                    ),
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
