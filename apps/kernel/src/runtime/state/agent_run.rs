//! What `chariox.agents.run` needs of the runtime: the wait for a worker's end, its answer, and
//! the hand-over of a worker that outlives the call to the event path.
use super::agent_run_waiters::WorkerEnd;
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, ExecutionState, Operation};

/// The answer a call returns for one worker; the whole answer is read with `chariox.history.turn`.
const WORKER_ANSWER_CHARS: usize = 16 * 1024;

impl KernelRuntimeState {
    pub(crate) fn wait_for_worker(
        &self,
        worker: &str,
    ) -> tokio::sync::oneshot::Receiver<WorkerEnd> {
        self.owned.agent_run_waiters.register(worker)
    }

    pub(crate) fn stop_waiting_for_worker(&self, worker: &str) {
        self.owned.agent_run_waiters.release(worker);
    }

    /// The call handed this worker's end to its caller in place of an inbox event, so the caller's
    /// delegate obligation is met: handle the end's source event as `chariox.events.ack` would.
    pub(crate) fn settle_delivered_worker(
        &self,
        room: &str,
        caller: &str,
        end: &WorkerEnd,
    ) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        let mut ends = Vec::new();
        store.scan_agent_inbox(room, caller, 0, None, |event| {
            if event.is_source_outcome()
                && event.reports_source(&end.task.task_id)
                && event.state != "handled"
            {
                ends.push(event.sequence);
            }
            std::ops::ControlFlow::Continue(())
        })?;
        for sequence in ends {
            store.agent_lifecycle(Operation::Ack {
                room: room.into(),
                agent: caller.into(),
                sequence,
                handled: true,
                now: crate::session::unix_epoch_ms(),
            })?;
        }
        Ok(())
    }

    /// One worker of a finished call: its status and, for a worker that finished, its answer.
    pub(crate) fn worker_result(
        &self,
        name: &str,
        end: &WorkerEnd,
    ) -> Result<serde_json::Value, DaemonError> {
        let task = &end.task;
        let mut result = serde_json::json!({
            "name": name,
            "agent": task.agent_id,
            "task_id": task.task_id,
        });
        if task.state == ExecutionState::Done {
            let answer = self
                .owned
                .public_agent_task_answer_within(task, None, WORKER_ANSWER_CHARS)?
                .and_then(|answer| answer["excerpt"].as_str().map(str::to_string))
                .unwrap_or_default();
            result["status"] = "done".into();
            if answer.starts_with('…') {
                result["answer_truncated"] = true.into();
            }
            result["answer"] = answer.into();
        } else {
            result["status"] = if end.outcome.kind == crate::provider::TurnOutcomeKind::Cancelled {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            result["reason"] = end.outcome.summary().into();
            result["recovery"] = end.outcome.notice().into();
        }
        if end.deadline_reached {
            result["deadline_reached"] = true.into();
        }
        crate::secret_redaction::redact_json_secrets(&mut result);
        Ok(result)
    }

    /// The call stops waiting for a worker that is still running: the caller follows it as it
    /// would after `agent spawn`, and its end arrives as `child_finished`.
    pub(crate) async fn follow_running_worker(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        origin_prompt_id: &str,
        worker: &str,
    ) -> Result<String, DaemonError> {
        let followed = self
            .dispatch_agent_event_tool(
                run,
                "chariox.events.subscribe",
                self.caller_task_args(run, origin_prompt_id, "source_id", worker)?,
            )
            .await?;
        followed.payload["registration_id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| ledger::error("following the worker returned no registration"))
    }

    /// The worker ended while the call was handing it over: the call answers it instead.
    pub(crate) async fn unfollow_worker(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        origin_prompt_id: &str,
        registration_id: &str,
    ) -> Result<(), DaemonError> {
        self.dispatch_agent_event_tool(
            run,
            "chariox.events.unsubscribe",
            self.caller_task_args(run, origin_prompt_id, "registration_id", registration_id)?,
        )
        .await
        .map(|_| ())
    }

    fn caller_task_args(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        origin_prompt_id: &str,
        key: &str,
        value: &str,
    ) -> Result<serde_json::Value, DaemonError> {
        let task = self
            .owned
            .durable_state_store
            .agent_turn_task(
                run.session_id(),
                run.agent_instance_id().unwrap_or_default(),
                origin_prompt_id,
            )?
            .ok_or_else(|| ledger::error("this turn's task is unavailable"))?;
        Ok(serde_json::json!({
            "task_id": task.task_id,
            "origin_prompt_id": origin_prompt_id,
            key: value,
        }))
    }
}
