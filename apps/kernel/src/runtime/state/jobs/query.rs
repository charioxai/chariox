//! Read-only job discovery for the authenticated agent, without command or sudo authority.
use super::*;

const WAIT_ACTION: &str = "Wait for memory or disk pressure to clear; queued work starts automatically. To release your own running work, use chariox.jobs.list, then chariox.jobs.cancel with that job_id.";

impl KernelRuntimeState {
    pub(super) fn own_job_list(&self, task: &AgentTaskExecution) -> serde_json::Value {
        use crate::runtime::process_resources::SessionProcessState;
        let jobs: Vec<_> = self
            .session_jobs(&task.room_id)
            .into_iter()
            .filter(|job| job.agent_id == task.agent_id)
            .collect();
        let waiting: Vec<_> = self.queued_job_processes_for(&task.room_id, Some(&task.agent_id))
            .into_iter().filter_map(|process| match process.state {
                SessionProcessState::Held(pressure) => Some(serde_json::json!({
                    "job_id": process.process.process_id, "pressure": pressure, "next_action": WAIT_ACTION
                })),
                _ => None,
            }).collect();
        serde_json::json!({
            "jobs": jobs,
            "waiting": waiting,
            "running_job_ids": jobs.iter().filter(|job| job.state == "running").map(|job| &job.job_id).collect::<Vec<_>>(),
            "next_action": "Use chariox.jobs.status with job_id to open a result; chariox.jobs.cancel with job_id stops your job. Queued jobs start automatically when memory or disk pressure clears."
        })
    }

    pub(super) fn job_response(&self, record: Record) -> Result<serde_json::Value, DaemonError> {
        let wait = record.exec_id.as_ref().and_then(|id| {
            self.owned
                .room_execs
                .list(&record.room_id)
                .into_iter()
                .find(|exec| &exec.exec_id == id)
                .and_then(|exec| match exec.state {
                    crate::room_exec::ExecState::Queued { wait } => Some(wait),
                    _ => None,
                })
        });
        let running: Vec<_> = wait
            .as_ref()
            .map(|_| {
                self.session_jobs(&record.room_id)
                    .into_iter()
                    .filter(|job| job.agent_id == record.agent_id && job.state == "running")
                    .map(|job| job.job_id)
                    .collect()
            })
            .unwrap_or_default();
        let live_output = if record.result.is_none() {
            // Status stays available while the log cannot be read (not yet written, or
            // being recovered after a restart); the tail is simply absent until it can.
            super::execute::Output::read_tail(self, &record).ok()
        } else {
            None
        };
        let limit = self.limit_cancellation(&record);
        let mut value = public_record(record)?;
        if let Some(tail) = live_output {
            value["output_tail"] = tail.into();
        }
        if let Some(limit) = limit {
            value["cancelled_by"] = limit;
        }
        if let Some(wait) = wait {
            value["waiting"] = serde_json::to_value(wait).expect("launch wait");
            value["running_job_ids"] = serde_json::json!(running);
            value["next_action"] = WAIT_ACTION.into();
        }
        Ok(value)
    }

    /// TL6: a job whose result was saved as cancelled after its agent's time limit ended says
    /// that the limit ended it, and when, for the parent or successor who finds it later.
    fn limit_cancellation(&self, record: &Record) -> Option<serde_json::Value> {
        record
            .result
            .as_ref()
            .filter(|result| result.outcome == crate::local::JobOutcome::Cancelled)?;
        let deadline = self
            .owned
            .durable_state_store
            .agent_deadline(&record.room_id, &record.agent_id)
            .ok()??;
        let reached_at_ms = deadline
            .reached_at_ms
            .filter(|_| deadline.state == crate::local::AgentDeadlineState::Reached)?;
        // The result was saved when the limit ended, not earlier by the agent's own cancel.
        let saved_at_ms =
            std::fs::metadata(self.job_wake_driver.as_ref()?.path(&record.job_id).ok()?)
                .and_then(|meta| meta.modified())
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())?;
        if saved_at_ms + 1_000 < reached_at_ms {
            return None;
        }
        let agent = self.owned.agent_store.get_agent(&record.agent_id).ok();
        let name = agent
            .as_ref()
            .and_then(|agent| agent.alias())
            .unwrap_or(&record.agent_id);
        Some(serde_json::json!({
            "agent_id": record.agent_id,
            "reason": "the agent's time limit ended",
            "at_ms": reached_at_ms,
            "next_step": format!("The job did not finish. The owner, or {name}'s parent, can give {name} a new limit with `agent deadline {name} <duration|none>`; then start the command again."),
        }))
    }

    /// A job of a descendant, read by an ancestor of its agent: the same answer its agent gets,
    /// with the agent named. `None` when the caller is not above the job's agent.
    pub(super) fn ancestor_job_status(
        &self,
        driver: &JobDriver,
        id: &str,
        task: &AgentTaskExecution,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        let Ok(record) = driver.read(id) else {
            return Ok(None);
        };
        let Ok(owner) = self.owned.agent_store.get_agent(&record.agent_id) else {
            return Ok(None);
        };
        let agents = self.owned.agent_store.list_agents();
        if !crate::agent::authority_limit::ancestors(&owner, &agents)
            .iter()
            .any(|ancestor| ancestor.id() == task.agent_id)
        {
            return Ok(None);
        }
        let agent = serde_json::json!({"id": owner.id(), "alias": owner.alias()});
        let mut value = self.job_response(record)?;
        value["agent"] = agent;
        Ok(Some(value))
    }
}
