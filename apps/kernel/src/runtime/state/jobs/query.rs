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
            match super::execute::Output::read_tail(self, &record) {
                Ok(tail) => Some(tail),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(ledger::error(format!("Cannot read job output: {error}")))
                }
            }
        } else {
            None
        };
        let mut value = public_record(record)?;
        if let Some(tail) = live_output {
            value["output_tail"] = tail.into();
        }
        if let Some(wait) = wait {
            value["waiting"] = serde_json::to_value(wait).expect("launch wait");
            value["running_job_ids"] = serde_json::json!(running);
            value["next_action"] = WAIT_ACTION.into();
        }
        Ok(value)
    }
}
