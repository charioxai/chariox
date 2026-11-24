//! Admission for a job on a kernel in the owner's machine directory.
use super::super::job_compute::JobMachine;
use super::super::machine_acquisition::acquisition_error;
use super::*;
use crate::runtime::kernel_gateway::GatewayError;

impl KernelRuntimeState {
    pub(super) fn check_enrolled_job_agent(
        &self,
        agent_id: &str,
        kernel: &str,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        self.peer_owner_of_agent(&agent, kernel)?;
        Ok(())
    }

    pub(super) async fn enrolled_job_machine(
        &self,
        reference: &str,
        harness: &str,
    ) -> Result<JobMachine, DaemonError> {
        let router = self.job_router.as_ref().ok_or_else(|| {
            ledger::error("The job gateway is unavailable. Retry when the kernel is ready.")
        })?;
        #[cfg(test)]
        if let Some(id) = self
            .resolve_peer_kernel(reference)
            .await
            .ok()
            .flatten()
            .filter(|id| {
                super::super::job_compute::stand_in_kernels()
                    .lock()
                    .expect("stand-in kernels")
                    .contains_key(id)
            })
        {
            return Ok(JobMachine {
                machine_label: reference.into(),
                address: None,
                grant: self.job_machine_grant(&id, harness)?,
                kernel_id: id,
                class: None,
            });
        }
        let unavailable = || {
            acquisition_error(
                "machine_unreachable",
                &format!("Machine {reference} is unavailable. Check that it is online and enrolled under your account, then retry."),
            )
        };
        let target = router
            .kernel_gateway()
            .resolve_enrolled(reference.trim())
            .await
            .map_err(|error| match error {
                GatewayError::Ambiguous { .. } => acquisition_error(
                    "machine_ambiguous",
                    &format!("Several kernels match {reference}. Choose one by its kernel ID."),
                ),
                _ => unavailable(),
            })?;
        if target.kernel_id == self.owned.config_projection.snapshot().daemon_id {
            return Err(ledger::error("Omit machine to run the job on this kernel."));
        }
        let grant = self.job_machine_grant(&target.kernel_id, harness)?;
        crate::runtime::kernel_gateway::forward_with_retry(
            format!("job-enrolled:{:032x}:health", rand::random::<u128>()),
            |id| {
                grant.send(
                    id,
                    &LocalDaemonRequest::GetDaemonHealth(crate::local::GetDaemonHealthRequest),
                )
            },
        )
        .await
        .map_err(|_| unavailable())?;
        Ok(JobMachine {
            machine_label: target
                .machine_alias
                .or(target.kernel_alias)
                .or(target.relay_alias)
                .unwrap_or_else(|| reference.into()),
            address: None,
            kernel_id: target.kernel_id,
            grant,
            class: None,
        })
    }
}
