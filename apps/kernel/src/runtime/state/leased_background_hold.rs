//! A child on another kernel whose turn ended while its provider call still runs in the
//! background is not finished (see `background_hold`). Its home prompt is over, so nothing asks
//! the worker for output any more: the home keeps draining the held run until the provider's
//! own follow-up turn ends on the worker, or the worker's run is gone, and settles the task then.
use super::*;
use crate::durable_state::agent_lifecycle::ExecutionState;

impl KernelRuntimeState {
    /// The worker run to keep draining for `agent`'s held task, while no prompt of it is active.
    pub(super) fn held_projection_drain_target(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<(crate::agent::RemoteAgentBinding, String)> {
        let projected = self
            .owned
            .background_holds
            .leased_run_of_agent(session_id, agent_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id).ok()?;
        let binding = agent.remote_execution().cloned()?;
        let worker_run = crate::provider::worker_provider_run_id_from_projected_leased_id(
            &binding.leased_agent_id,
            &projected,
        )?;
        Some((binding, worker_run))
    }

    /// What a drained projection of a held run means for the task: the provider's follow-up turn
    /// ended on the worker (the parent hears of the finish now), or the worker's run ended
    /// without one.
    pub(super) async fn settle_held_tasks_after_projection(
        &self,
        session_id: &str,
        agent_id: &str,
        projected_run: &str,
        outcome: HeldProjection,
    ) -> Result<(), DaemonError> {
        let held = self
            .owned
            .background_holds
            .leased_run_of_agent(session_id, agent_id)
            .is_some();
        if held || outcome.notification_ended || outcome.run_ended {
            crate::logging::debug_with_fields(
                "daemon.agent_lifecycle",
                "a worker projection reached a leased agent's held task",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "provider_run_id": projected_run,
                    "held": held,
                    "worked": outcome.worked,
                    "notification_ended": outcome.notification_ended,
                    "run_ended": outcome.run_ended,
                }),
            );
        }
        // A turn of the agent's own is not the provider's follow-up turn.
        if outcome.worked && !self.has_active_prompt(session_id, agent_id) {
            self.owned
                .background_holds
                .note_output_of_run(projected_run, None);
        }
        if outcome.notification_ended {
            return self
                .release_tasks_held_for_notification(session_id, agent_id, projected_run)
                .await;
        }
        if outcome.run_ended {
            return self
                .end_held_leased_tasks(
                    session_id,
                    agent_id,
                    "The worker's provider run ended while its background command was still running",
                )
                .await;
        }
        Ok(())
    }

    fn has_active_prompt(&self, session_id: &str, agent_id: &str) -> bool {
        self.owned
            .session_store
            .get_session(session_id)
            .ok()
            .is_some_and(|session| {
                self.owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)
                    .is_some()
            })
    }

    /// The agent has no prompt, only a task held on its worker's run: drains keep asking the
    /// worker for that run, however a drain fails.
    pub(super) fn awaits_held_run(&self, session_id: &str, agent_id: &str) -> bool {
        self.held_projection_drain_target(session_id, agent_id)
            .is_some()
            && !self.has_active_prompt(session_id, agent_id)
    }

    /// The worker no longer knows the agent or its lease: the run the task waits on is gone.
    /// Any other failure may pass, so the drain tries again.
    pub(super) async fn end_held_tasks_after_drain_error(
        &self,
        session_id: &str,
        agent_id: &str,
        error: &DaemonError,
    ) {
        if self.has_active_prompt(session_id, agent_id)
            || !super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding(error)
        {
            return;
        }
        let reason = format!("The worker no longer has this agent's run: {error}");
        if let Err(error) = self
            .end_held_leased_tasks(session_id, agent_id, &reason)
            .await
        {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "a held leased task could not be ended after its worker lost the run",
                serde_json::json!({"session_id": session_id, "agent_id": agent_id, "error": error.to_string()}),
            );
        }
    }

    /// The worker cannot finish what the task waits for: it ends failed with `reason`, and its
    /// parent hears of that instead of waiting for a turn that will not come.
    pub(super) async fn end_held_leased_tasks(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) -> Result<(), DaemonError> {
        let Some(run) = self
            .owned
            .background_holds
            .leased_run_of_agent(session_id, agent_id)
        else {
            return Ok(());
        };
        let outcome = crate::provider::ProviderRunTermination::host_lost(
            &format!("{reason}. The turn may have partly run; Chariox did not run it again"),
            crate::session::unix_epoch_ms(),
        )
        .failure
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "end held leased task",
            message: "a lost host carries no failure".into(),
        })?;
        self.owned.end_open_provider_tools(
            session_id,
            &run,
            agent_id,
            crate::app::ProviderToolEnding::Failed,
        )?;
        self.end_lost_leased_provider_run(session_id, &run)?;
        for (task_id, held) in self
            .owned
            .background_holds
            .of_run(session_id, agent_id, &run)
        {
            self.owned.background_holds.release(&task_id);
            let prompt = crate::session::PromptQueueItem::new(
                held.prompt,
                "kernel-supervisor",
                agent_id,
                "",
                crate::session::PromptStatus::Completed,
            );
            let still_working = self
                .owned
                .durable_state_store
                .agent_task(&task_id)?
                .is_some_and(|task| task.state == ExecutionState::Working);
            if !still_working {
                continue;
            }
            let settlement = self
                .owned
                .end_failed_agent_task(session_id, agent_id, &prompt, &run, &outcome)?;
            Box::pin(self.finish_failed_agent_task_settlement(settlement, &prompt, Some(&outcome)))
                .await?;
        }
        Ok(())
    }
}

/// What one drained projection said about the run a task is held on.
#[derive(Clone, Copy, Default)]
pub(super) struct HeldProjection {
    /// The provider did work of a turn: its follow-up turn is executing.
    pub(super) worked: bool,
    /// The follow-up turn ended on the worker.
    pub(super) notification_ended: bool,
    /// The worker's run ended.
    pub(super) run_ended: bool,
}
