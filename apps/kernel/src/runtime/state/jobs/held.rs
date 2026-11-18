//! A job placed on a machine the agent already holds runs through that machine's own kernel. The job
//! never allocates or releases a machine: the agent keeps holding it afterwards.
use super::super::job_compute::JobMachine;
use super::super::machine_acquisition::acquisition_error;
use super::execute::{cancelled_result, Output};
use super::pending_return::PendingReturn;
use super::remote::Remote;
use super::*;
use crate::local::{ControlPlaneRequest, ControlPlaneResponse};
use crate::runtime::kernel_gateway::Origin;
use zeroize::Zeroizing;

impl KernelRuntimeState {
    /// The held machine `allocation_ref` names, proven ready and reachable now, or the refusal that
    /// says what to do next.
    pub(super) async fn job_held_machine(
        &self,
        allocation_ref: &str,
        origin: &Origin,
        command_id: &str,
    ) -> Result<JobMachine, DaemonError> {
        let response = self
            .job_control_plane(
                origin,
                &ControlPlaneRequest::Get {
                    allocation_ref: allocation_ref.into(),
                },
                &format!("{command_id}:held"),
            )
            .await
            .map_err(|error| match crate::runtime::cloud_api_client::cloud_error_code(&error) {
                Some("not_found") => acquisition_error(
                    "machine_not_held",
                    &format!("You hold no machine {allocation_ref}. Use the allocation_ref chariox.machines.acquire returned, or acquire one first."),
                ),
                _ => error,
            })?;
        let ControlPlaneResponse::Machine {
            state,
            kernel_target,
            machine_label,
            address,
            purpose,
            ..
        } = response
        else {
            return Err(acquisition_error(
                "control_plane_binding_invalid",
                "The connected service did not return the machine",
            ));
        };
        if purpose.as_deref() == Some("offload_job") {
            return Err(acquisition_error(
                "machine_not_held",
                &format!("{allocation_ref} is a job's own worker, not a machine you hold. Acquire one with chariox.machines.acquire."),
            ));
        }
        match state.as_str() {
            "ready" => {}
            "deleting" | "deleted" | "released" | "failed" => {
                return Err(acquisition_error(
                    "machine_released",
                    &format!("The machine {allocation_ref} is {state}. Acquire another with chariox.machines.acquire."),
                ))
            }
            _ => {
                return Err(acquisition_error(
                    "machine_not_ready",
                    &format!("The machine {allocation_ref} is {state}, not ready. Check chariox.machines.status; the job needs a ready machine."),
                ))
            }
        }
        let target = kernel_target.ok_or_else(|| {
            acquisition_error(
                "machine_unreachable",
                &format!("The machine {allocation_ref} has no verified kernel yet. Poll chariox.machines.status until it has one, then run the job."),
            )
        })?;
        let grant = self.job_machine_grant(&target.kernel_id, &origin.harness)?;
        crate::runtime::kernel_gateway::forward_with_retry(format!("{command_id}:health"), |id| {
            grant.send(
                id,
                &LocalDaemonRequest::GetDaemonHealth(crate::local::GetDaemonHealthRequest),
            )
        })
        .await
        .map_err(|error| {
            acquisition_error(
                "machine_unreachable",
                &format!("The machine {allocation_ref} did not answer: {}. Retry in a moment, or check chariox.machines.status.", error.transport_error().message),
            )
        })?;
        Ok(JobMachine {
            allocation_ref: allocation_ref.into(),
            machine_label: machine_label.unwrap_or_else(|| "held machine".into()),
            address,
            kernel_id: target.kernel_id,
            grant,
            class: None,
        })
    }
}

impl KernelRuntimeState {
    /// Runs the job on the machine its request names. Nothing is allocated, so there is no capacity
    /// fallback: a machine that is gone fails the job with the reason.
    pub(super) async fn run_job_on_held_machine(
        &self,
        record: &mut Record,
        allocation_ref: &str,
        environment: &BTreeMap<String, Zeroizing<String>>,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<JobResult, DaemonError> {
        if *cancelled.borrow() {
            return Ok(cancelled_result(record));
        }
        self.job_acquisition_subject(&record.job_id)?;
        let origin = Origin {
            grant_id: format!("job:{}", record.job_id),
            harness: self
                .agent_harness_name(&record.room_id, &record.agent_id)
                .unwrap_or_else(|| "Chariox".into()),
        };
        let machine = tokio::select! {
            machine = self.job_held_machine(allocation_ref, &origin, &record.job_id) => machine?,
            _ = cancelled.changed() => return Ok(cancelled_result(record)),
        };
        record.machine_label = Some(machine.machine_label.clone());
        record.address = machine.address.clone();
        save_record(self, record)?;
        let result = self
            .run_remote_job(record, &machine, environment, output, cancelled)
            .await;
        let was_cancelled = *cancelled.borrow();
        self.complete_remote_job(record, output, result, was_cancelled)
            .await
    }

    /// A held machine is never deleted, so a command whose end was never read keeps running there
    /// until it is stopped by name. Returns what to tell the agent when it could not be stopped.
    pub(super) async fn stop_held_command(&self, record: &Record) -> Option<&'static str> {
        let (RemoteStage::Running { room, exec_id }, Some(_)) =
            (&record.remote, &record.request.machine)
        else {
            return None;
        };
        let stopped = async {
            let plan = PendingReturn::load(self, &record.job_id)?;
            let grant = self.return_grant(record, &plan.kernel_id).await?;
            Remote::new(grant, room.clone(), record.job_id.clone())
                .cancel(exec_id)
                .await
        };
        match tokio::time::timeout(Duration::from_secs(30), stopped).await {
            Ok(Ok(())) => None,
            _ => Some(super::execute::HELD_COMMAND_MAY_RUN),
        }
    }

    /// The held machine outlives the job, so the copy of the workspace uploaded to it goes once its
    /// files are home. A failure leaves only disk use behind and does not fail the job.
    pub(super) async fn remove_held_workspace(&self, record: &Record, remote: &Remote, root: &str) {
        if record.request.machine.is_none() {
            return;
        }
        if let Err(error) = remote.remove_workspace(root).await {
            tracing::warn!(%error, job = %record.job_id, "the held machine keeps the job's uploaded workspace");
        }
    }
}

impl KernelRuntimeState {
    /// After a restart: the live grant to the held machine is gone. The service names the machine
    /// again, and only the kernel the job's plan pinned is granted.
    pub(super) async fn held_machine_reattach(
        &self,
        record: &Record,
        kernel_id: &str,
    ) -> Result<Arc<dyn crate::room_exec::gateway_placement::GrantedSend>, DaemonError> {
        let allocation_ref = record
            .request
            .machine
            .as_deref()
            .ok_or_else(|| ledger::error("The job has no held machine"))?;
        let origin = Origin {
            grant_id: format!(
                "{}{}",
                super::super::reach::OWNER_GRANT_PREFIX,
                self.owned.config_projection.local_owner_user_id()
            ),
            harness: self
                .agent_harness_name(&record.room_id, &record.agent_id)
                .unwrap_or_else(|| "Chariox".into()),
        };
        let machine = self
            .job_held_machine(
                allocation_ref,
                &origin,
                &format!("{}:return", record.job_id),
            )
            .await?;
        if machine.kernel_id != kernel_id {
            return Err(acquisition_error(
                "machine_unreachable",
                &format!("The machine {allocation_ref} now runs another kernel than the one this job used. chariox.jobs.cancel gives up its files"),
            ));
        }
        Ok(machine.grant)
    }
}

impl KernelRuntimeState {
    /// The held machine a `chariox.jobs.run` names, checked as its own agent before any job record
    /// exists.
    pub(super) async fn held_machine_for_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        allocation_ref: &str,
    ) -> Result<JobMachine, DaemonError> {
        if !self.remote_jobs_enabled() {
            return Err(acquisition_error(
                "remote_jobs_off",
                "Remote jobs are switched off on this kernel (CHARIOX_REMOTE_JOBS), so a job cannot run on a machine. Run it without machine here, or start the kernel with remote jobs on.",
            ));
        }
        let reach = self
            .reach_for_id(&format!(
                "{}{}",
                super::super::reach::REACH_PREFIX,
                run.id()
            ))
            .ok_or_else(|| {
                acquisition_error(
                    "not_allowed",
                    "Only a running agent of the kernel owner may run a job on a machine",
                )
            })?;
        self.job_held_machine(
            allocation_ref,
            &reach.origin(),
            &format!("job-held:{:032x}", rand::random::<u128>()),
        )
        .await
    }
}
