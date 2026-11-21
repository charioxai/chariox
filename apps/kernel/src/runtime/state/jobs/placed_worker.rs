//! Run and retain placed jobs in the workspace of the authorized worker turn.
use super::{placed::placed_job_id, *};
use crate::durable_state::agent_lifecycle::{
    LeasedJobStatus, LeasedProcessBinding, LeasedProcessMutation, LeasedProcessReceipt, Operation,
};

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn start_worker_placed_job(
        &self,
        wake: AgentWake,
        binding: LeasedProcessBinding,
        request: RunJobRequest,
    ) -> Result<LeasedProcessReceipt, DaemonError> {
        self.admit_worker_placed_job(wake, binding, request)
            .await
            .map_err(super::refusal::for_relay)
    }

    async fn admit_worker_placed_job(
        &self,
        mut wake: AgentWake,
        binding: LeasedProcessBinding,
        request: RunJobRequest,
    ) -> Result<LeasedProcessReceipt, DaemonError> {
        let driver = self.job_driver()?;
        if wake.kind != KIND || validate(&request)? != wake.command || request.machine.is_some() {
            return Err(ledger::error("Invalid placed job request"));
        }
        let id = placed_job_id(&wake)?;
        driver.path(id)?;
        let (room, agent, prompt) = self.placed_job_target(&wake, &binding).await?;
        let run = self.job_build_run(&binding.worker_provider_run_id)?;
        if run.agent_instance_id() != Some(agent.as_str()) || run.session_id() != room {
            return Err(ledger::error(super::refusal::BUILD_TURN_REQUIRED));
        }
        let (workspace, cwd) = requests::job_workspace(&run, &request)?;
        let admission = crate::runtime::process_resources::LaunchAdmission::default();
        if let Some(shortfall) = super::never_fit::Shortfall::of(
            &request.needs,
            admission.capacity(),
            std::thread::available_parallelism().map_or(u32::MAX, |cpus| cpus.get() as u32),
        ) {
            return Err(ledger::error(
                shortfall.refusal("this agent is assigned to this machine"),
            ));
        }
        let mut approval = wake.clone();
        approval.room_id = room.clone();
        approval.agent_id = agent.clone();
        self.approve_leased_agent_process(
            &run,
            &approval,
            &cwd,
            prompt
                .as_deref()
                .ok_or_else(|| ledger::error(super::refusal::BUILD_TURN_REQUIRED))?,
        )
        .await?;
        self.placed_job_target(&wake, &binding).await?;
        // Consent can outlive a turn or a change to its filesystem mode.
        let run = self.job_build_run(run.id())?;
        let (current_workspace, current_cwd) = requests::job_workspace(&run, &request)?;
        if current_workspace != workspace || current_cwd != cwd {
            return Err(ledger::error(
                "The job workspace changed while the request was open",
            ));
        }
        self.authorize_current_external_command()?;
        let _admission = self.owned.begin_managed_activity_admission()?;

        wake.created_at_ms = crate::session::unix_epoch_ms();
        let mut record = Record::new(&wake, request, workspace, cwd);
        record.room_id = room.clone();
        record.agent_id = agent.clone();
        record.machine_label = Some(binding.worker_machine_id.clone());
        // Resolve named entries before committing the launch. The executor resolves
        // them again when it builds the command's environment.
        execute::selected_environment(self, &record)?;
        driver.save(&record)?;
        let receipt = LeasedProcessReceipt {
            job: None,
            command_digest: super::super::agent_leased_process_worker::launch_digest(
                &wake,
                &None,
                Some(&record.request),
            )?,
            worker_boot_id: String::new(),
            process_start_ticks: None,
            matched_line: None,
            output_tail: String::new(),
            revision: 1,
            backing_room: room,
            backing_agent: agent,
            binding,
            wake: wake.clone(),
        };
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::LeasedProcess(LeasedProcessMutation::Intent(
                receipt.clone(),
            )))?;
        driver.launch(
            wake,
            Pending {
                state: self.clone(),
                record,
            },
        )?;
        self.placed_job_receipt(receipt, false)
    }

    async fn placed_job_target(
        &self,
        wake: &AgentWake,
        binding: &LeasedProcessBinding,
    ) -> Result<(String, String, Option<String>), DaemonError> {
        let (wake, binding) = (wake.clone(), binding.clone());
        self.with_app_side_effect(move |app| {
            crate::app::RemoteLeaseRuntime::new(app).leased_process_target(&binding, &wake, true)
        })
        .await
    }

    pub(in crate::runtime::state) fn placed_job_receipt(
        &self,
        mut receipt: LeasedProcessReceipt,
        cancel: bool,
    ) -> Result<LeasedProcessReceipt, DaemonError> {
        let driver = self.job_driver()?;
        let live = driver.live.lock().expect("job live");
        let mut record = driver.read(placed_job_id(&receipt.wake)?)?;
        if record.wake_id != receipt.wake.id
            || record.room_id != receipt.backing_room
            || record.agent_id != receipt.backing_agent
        {
            return Err(ledger::error(
                "Job receipt does not match its execution record",
            ));
        }
        match live.get(&receipt.wake.id) {
            Some(stop) if cancel => {
                stop.send_replace(true);
            }
            None if record.result.is_none() => {
                // A durable intent without its supervisor is never launched again.
                let mut result = execute::failed(
                    &record,
                    "The execution kernel no longer supervises this job".into(),
                );
                result.outcome = JobOutcome::MachineLost;
                record.state = "finished".into();
                record.result = Some(result);
                driver.save(&record)?;
            }
            _ => {}
        }
        receipt.job = Some(LeasedJobStatus {
            state: record.state,
            running_at_ms: record.running_at_ms,
            result: record.result,
        });
        Ok(receipt)
    }

    pub(super) fn job_wake_for_recovery(&self, id: &str) -> Result<Option<AgentWake>, DaemonError> {
        let store = &self.owned.durable_state_store;
        if let Some(wake) = store.agent_wake(id)? {
            return Ok(Some(wake));
        }
        Ok(store
            .leased_process_receipt(id)?
            .map(|receipt| receipt.wake))
    }

    pub(super) fn recover_placed_jobs(&self, driver: &JobDriver) {
        let receipts = match self.owned.durable_state_store.leased_process_receipts() {
            Ok(receipts) => receipts,
            Err(error) => {
                tracing::warn!(%error, "placed job recovery could not read worker receipts");
                return;
            }
        };
        for receipt in receipts
            .into_iter()
            .filter(|receipt| receipt.wake.kind == KIND)
        {
            let wake = &receipt.wake;
            let unfinished = placed_job_id(wake)
                .and_then(|id| driver.read(id))
                .is_ok_and(|record| record.result.is_none());
            if unfinished {
                if let Recovery::Lost(reason) = driver.recover(wake) {
                    tracing::warn!(%reason, wake_id=%wake.id, "placed job recovery failed");
                }
            }
        }
    }
}
