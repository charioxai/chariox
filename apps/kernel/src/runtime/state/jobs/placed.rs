//! A placed job has a home wake and a worker-owned execution receipt.
use super::*;
use crate::durable_state::agent_lifecycle::{LeasedProcessReceipt, Operation};

impl KernelRuntimeState {
    pub(super) async fn start_placed_job(
        &self,
        task: &AgentTaskExecution,
        wake: AgentWake,
        request: RunJobRequest,
    ) -> Result<serde_json::Value, DaemonError> {
        let agent = self.owned.agent_store.get_agent(&task.agent_id)?;
        let remote = agent
            .remote_execution()
            .ok_or_else(|| ledger::error(super::refusal::BUILD_TURN_REQUIRED))?;
        if request.machine.is_some() {
            return Err(ledger::error(
                "A placed agent runs jobs on its current machine; omit the machine argument",
            ));
        }
        let mut record = Record::new(&wake, request.clone(), PathBuf::new(), PathBuf::new());
        record.placement = JobPlacement::Remote;
        record.machine_label = Some(remote.worker_machine_id.clone());
        let driver = self.job_driver()?;
        driver.save(&record)?;
        if let Err(error) = self
            .start_home_leased_process(
                wake.clone(),
                None,
                Some(Box::new(request.clone())),
                &task.task_id,
                &task.prompt_id,
            )
            .await
        {
            // An admission failure before the source was committed has nothing to reconcile.
            if self
                .owned
                .durable_state_store
                .agent_wake(&wake.id)?
                .is_none()
            {
                driver.remove(&record)?;
            }
            return Err(error);
        }
        self.wait_for_job(&record.job_id, request.wait_s).await
    }

    pub(in crate::runtime::state) fn project_placed_job(
        &self,
        wake: &AgentWake,
        receipt: &LeasedProcessReceipt,
    ) -> Result<(), DaemonError> {
        let driver = self.job_driver()?;
        let mut record = driver.read(placed_job_id(wake)?)?;
        let job = receipt
            .job
            .as_ref()
            .ok_or_else(|| ledger::error("Execution kernel returned no job status"))?;
        let sink = self.wake_sink(KIND);
        if wake.state == "starting" {
            sink.verified(&wake.id)?;
        }
        if let Some(mut result) = job.result.clone() {
            let protect = super::super::agent_process_output::room_protector(
                self.owned.room_secret_observations.clone(),
                wake.room_id.clone(),
            );
            result.output_tail = protect(result.output_tail)
                .unwrap_or_else(|| "[sensitive job output withheld]".into());
            self.store_placed_job_result(wake, record, result)
        } else {
            if record.state != job.state || record.running_at_ms != job.running_at_ms {
                record.state.clone_from(&job.state);
                record.running_at_ms = job.running_at_ms;
                driver.save(&record)?;
            }
            Ok(())
        }
    }

    pub(in crate::runtime::state) fn lose_placed_job(
        &self,
        wake: &AgentWake,
        reason: &str,
    ) -> Result<(), DaemonError> {
        let record = self.job_driver()?.read(placed_job_id(wake)?)?;
        if let Some(result) = &record.result {
            return self.report_job_completion(
                wake,
                &record,
                result,
                super::driver::completion_end(result),
            );
        }
        let mut result = execute::failed(
            &record,
            format!("The Room lost contact with this job on its execution kernel. Check that machine for the original job before starting it again. {reason}"),
        );
        result.outcome = JobOutcome::MachineLost;
        self.store_placed_job_result(wake, record, result)
    }

    fn store_placed_job_result(
        &self,
        wake: &AgentWake,
        mut record: Record,
        result: JobResult,
    ) -> Result<(), DaemonError> {
        record.state = "finished".into();
        record.result = Some(result.clone());
        self.job_driver()?.save(&record)?;
        self.report_job_completion(
            wake,
            &record,
            &result,
            super::driver::completion_end(&result),
        )
    }

    pub(in crate::runtime::state) fn refuse_placed_job(
        &self,
        wake: &AgentWake,
        task: &str,
        prompt: &str,
    ) -> Result<(), DaemonError> {
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::CancelWake {
                id: wake.id.clone(),
                task: task.into(),
                prompt: Some(prompt.into()),
                pending: false,
            })?;
        self.refresh_wake_projection(&wake.room_id);
        let driver = self.job_driver()?;
        driver.remove(&driver.read(placed_job_id(wake)?)?)
    }
}

pub(super) fn placed_job_id(wake: &AgentWake) -> Result<&str, DaemonError> {
    wake.detail["job_id"]
        .as_str()
        .ok_or_else(|| ledger::error("Placed job identity missing"))
}
