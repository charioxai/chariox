//! A job placed on a machine the agent already holds runs through that machine's own kernel. The job
//! never allocates or releases a machine: the agent keeps holding it afterwards, so what the job
//! uploaded there must be removed by the job itself.
//!
//! Naming a held machine is not an offload policy choice: the policy places jobs on worker machines
//! the kernel rents, and the agent chose this machine itself.
//!
//! The plan (`return.json`) names the uploaded workspace and is saved, with stage `Planned`, before
//! the first byte goes up. It is forgotten only once the workspace is removed, so "plan saved" means
//! "the machine may still hold the job's workspace". The result and the settled stage are saved
//! before the removal.
//!
//! | stage / moment              | durable record            | workspace on the machine | cancel or failure                       | restart                                         |
//! |-----------------------------|---------------------------|--------------------------|-----------------------------------------|-------------------------------------------------|
//! | Idle, before the plan       | Idle, no plan             | none                     | nothing to clean                        | nothing to clean; job ends "not relaunched"     |
//! | plan saved, record not yet  | Idle + plan               | none                     | save fails: plan forgotten              | recovery removes (nothing there) and forgets    |
//! | Planned, uploading          | Planned + plan            | partial                  | Settled + result saved, then removed    | recovery gives up Planned, removes, forgets     |
//! | Planned, command opened     | Planned + plan            | complete                 | as above; the command is not stopped    | removed; the opened command cannot be named     |
//! | Running                     | Running + plan            | complete                 | stop by name, then as Settled           | reattach and follow to its end                  |
//! | Finished, files not all home| Finished + plan + result  | complete                 | status collects; cancel gives up        | kept for collecting                             |
//! | Settled, result saved       | Settled + plan + result   | present until removed    | removal fails: plan kept, warned        | recovery and status retry; result is kept       |
//! | Settled, plan forgotten     | Settled + result          | none                     | nothing                                 | nothing                                         |
//!
//! A machine that is released or no longer held while a removal is owed has lost its disk: the plan
//! is forgotten. A machine that is only unreachable keeps the plan.
//!
//! Who retries a removal that is owed (plan on disk), and with which command ids:
//! - a job whose wake is still active (Planned, Running): `JobDriver::recover` at startup;
//! - a job that already finished (Settled, plan kept): `JobDriver::reconcile_ended` at startup, which
//!   `recover` cannot do because the finished wake is no longer active, and `chariox.jobs.status`;
//! - a job whose files are still to be collected (Finished): nobody until `chariox.jobs.status`
//!   collects or `chariox.jobs.cancel` gives them up, because the files are at stake.
//!
//! The machine's gateway answers a repeated state-changing command id from its ledger, failures
//! included, so every removal attempt draws fresh ids (`Remote::remove_workspace`); the ids of one
//! attempt stay fixed so the transport's retry of a lost reply is not run twice.
use super::super::job_compute::JobMachine;
use super::super::machine_acquisition::acquisition_error;
use super::execute::{cancelled_result, Output};
use super::pending_return::PendingReturn;
use super::remote::Remote;
use super::*;
use crate::local::{ControlPlaneRequest, ControlPlaneResponse};
use crate::runtime::kernel_gateway::Origin;
use zeroize::Zeroizing;

#[derive(Clone, Copy)]
pub(super) enum HeldRelease {
    Deleting,
    Deleted,
}

impl HeldRelease {
    pub(super) fn notice(self) -> &'static str {
        match self {
            Self::Deleting => "The machine is being released.\n",
            Self::Deleted => "The machine was released.\n",
        }
    }
}

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
    pub(super) async fn run_job_on_named_machine(
        &self,
        record: &mut Record,
        target: &JobMachineTarget,
        environment: &BTreeMap<String, Zeroizing<String>>,
        output: &mut Output,
        cancelled: &mut watch::Receiver<bool>,
    ) -> Result<JobResult, DaemonError> {
        if *cancelled.borrow() {
            return Ok(cancelled_result(record));
        }
        // An enrolled placement needs no acquisition authority from the initiating turn.
        let require_active_turn = matches!(target, JobMachineTarget::Allocation(_));
        self.live_job_subject(&record.job_id, require_active_turn)?;
        let origin = Origin {
            grant_id: format!("job:{}", record.job_id),
            harness: self
                .agent_harness_name(&record.room_id, &record.agent_id)
                .unwrap_or_else(|| "Chariox".into()),
        };
        let resolve = async {
            match target {
                JobMachineTarget::Allocation(allocation) => {
                    self.job_held_machine(allocation, &origin, &record.job_id)
                        .await
                }
                JobMachineTarget::Enrolled { kernel, .. } => {
                    self.check_enrolled_job_agent(&record.agent_id, kernel)?;
                    let machine = self.enrolled_job_machine(kernel, &origin.harness).await?;
                    if machine.kernel_id != *kernel {
                        return Err(ledger::error("The enrolled kernel changed. Run the job again with its current kernel ID."));
                    }
                    Ok(machine)
                }
            }
        };
        let machine = tokio::select! {
            machine = resolve => machine?,
            _ = cancelled.changed() => return Ok(cancelled_result(record)),
        };
        record.placement = JobPlacement::Remote;
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

    /// A named machine is never deleted, so a command whose end was never read keeps running there
    /// until it is stopped by name. Returns what to tell the agent when it could not be stopped.
    pub(super) async fn stop_named_command(&self, record: &Record) -> Option<&'static str> {
        let (RemoteStage::Running { room, exec_id }, Some(_)) =
            (&record.remote, &record.request.machine)
        else {
            return None;
        };
        let stopped = async {
            let plan = PendingReturn::load_saved(self, &record.job_id)?;
            let grant = self.return_grant(record, plan.as_ref()).await?;
            Remote::new(grant, room.clone(), record.job_id.clone())
                .cancel(exec_id)
                .await
        };
        match tokio::time::timeout(Duration::from_secs(30), stopped).await {
            Ok(Ok(())) => None,
            _ => Some(super::execute::NAMED_COMMAND_MAY_RUN),
        }
    }
}

impl KernelRuntimeState {
    fn held_job_origin(&self, record: &Record) -> Origin {
        Origin {
            grant_id: format!(
                "{}{}",
                super::super::reach::OWNER_GRANT_PREFIX,
                self.owned.config_projection.local_owner_user_id()
            ),
            harness: self
                .agent_harness_name(&record.room_id, &record.agent_id)
                .unwrap_or_else(|| "Chariox".into()),
        }
    }

    pub(super) async fn held_job_release(&self, record: &Record) -> Option<HeldRelease> {
        let Some(JobMachineTarget::Allocation(allocation_ref)) = &record.request.machine else {
            return None;
        };
        let request = ControlPlaneRequest::Get {
            allocation_ref: allocation_ref.clone(),
        };
        let origin = self.held_job_origin(record);
        let command_id = format!("{}:release-state", record.job_id);
        let Ok(Ok(ControlPlaneResponse::Machine { state, .. })) = tokio::time::timeout(
            Duration::from_secs(3),
            self.job_control_plane(&origin, &request, &command_id),
        )
        .await
        else {
            // The job output is still available if the lifecycle service cannot confirm the
            // state. File return remains pending until the allocation can be checked again.
            return None;
        };
        match state.as_str() {
            "deleting" => Some(HeldRelease::Deleting),
            "deleted" | "released" => Some(HeldRelease::Deleted),
            _ => None,
        }
    }

    pub(super) fn mark_held_job_deleted(&self, record: &mut Record) {
        record.deleted = true;
        self.give_up_pending_return(record);
        self.forget_pending_return(&record.job_id);
    }

    /// After a restart: the live grant to the held machine is gone. The service names the machine
    /// again, and only the kernel the job's plan pinned is granted.
    pub(super) async fn held_machine_reattach(
        &self,
        record: &Record,
        kernel_id: &str,
    ) -> Result<Arc<dyn crate::room_exec::gateway_placement::GrantedSend>, DaemonError> {
        let Some(JobMachineTarget::Allocation(allocation_ref)) = &record.request.machine else {
            return Err(ledger::error("The job has no held machine"));
        };
        let origin = self.held_job_origin(record);
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
