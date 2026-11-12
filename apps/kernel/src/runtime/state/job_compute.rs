//! Job compute uses the connected service and the kernel gateway. No provider policy lives here.
use super::machine_acquisition::acquisition_error;
use super::*;
use crate::local::{
    ControlPlaneRequest, ControlPlaneResponse, JobMachineOffer, JobMachineRate, JobNeeds,
    MachineAllocationCapability,
};
use crate::room_exec::gateway_placement::{GrantedSend, OriginGrant};
use crate::runtime::command::{KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource};
use crate::runtime::kernel_gateway::Origin;

pub(super) struct JobMachine {
    pub(super) allocation_ref: String,
    pub(super) machine_label: String,
    pub(super) address: Option<String>,
    pub(super) capability_revision: String,
    pub(super) grant: Arc<dyn GrantedSend>,
    pub(super) class_id: String,
    pub(super) rate_per_minute: JobMachineRate,
}

pub(super) fn select_job_class<'a>(
    capability: &'a MachineAllocationCapability,
    needs: &JobNeeds,
) -> Result<&'a JobMachineOffer, DaemonError> {
    if needs.memory_gib == Some(0) || needs.cpus == Some(0) {
        return Err(acquisition_error(
            "invalid_request",
            "Job memory and CPU needs must be positive",
        ));
    }
    let memory = u64::from(needs.memory_gib.unwrap_or(0)).saturating_mul(1024);
    capability
        .job_offers
        .iter()
        .filter(|class| class.usable_memory_mib >= memory && class.vcpu >= needs.cpus.unwrap_or(0))
        .filter(|class| {
            needs.memory_gib.is_some()
                || needs.cpus.is_some()
                || capability.job_default_class_id.as_deref() == Some(class.class_id.as_str())
        })
        .min_by_key(|class| (class.usable_memory_mib, class.vcpu))
        .ok_or_else(|| {
            acquisition_error(
                "capacity_unavailable",
                "No remote machine class covers this job; it can wait locally",
            )
        })
}

impl KernelRuntimeState {
    pub(crate) fn kernel_owner_user_id(&self) -> String {
        self.owned.config_projection.local_owner_user_id()
    }
    pub(crate) fn with_job_router(&self, router: crate::runtime::router::CommandRouter) -> Self {
        let mut state = self.clone();
        state.job_router = Some(Arc::new(router));
        state
    }
    pub(super) async fn job_control_plane(
        &self,
        origin: &Origin,
        request: &ControlPlaneRequest,
        command_id: &str,
    ) -> Result<ControlPlaneResponse, DaemonError> {
        let subject = match origin.grant_id.strip_prefix("job:") {
            Some(job) => Some(self.job_acquisition_subject(job)?),
            None => {
                self.authorize_gateway_request(
                    &origin.grant_id,
                    &LocalDaemonRequest::ControlPlane(request.clone()),
                )?;
                None
            }
        };
        let owner = origin
            .grant_id
            .strip_prefix(super::reach::OWNER_GRANT_PREFIX);
        let internal_owner = owner.is_some() || subject.is_some();
        let caller = KernelCaller {
            user_id: subject
                .as_ref()
                .map(|(_, _, owner)| owner.clone())
                .or_else(|| owner.map(str::to_owned)),
            metaagent_id: subject.as_ref().map(|(_, agent, _)| agent.clone()),
            caller_id: origin.grant_id.clone(),
            caller_kind: if internal_owner {
                KernelCallerKind::LocalClient
            } else {
                KernelCallerKind::Metaagent
            },
            connection_class: Some(if internal_owner {
                crate::local::KernelConnectionClass::Terminal
            } else {
                crate::local::KernelConnectionClass::KernelAgent
            }),
            ..KernelCaller::default()
        };
        let mut command = KernelCommand::from_local_request_with_caller(
            command_id,
            KernelCommandSource::LocalIpc,
            caller,
            None,
            None,
            &LocalDaemonRequest::ControlPlane(request.clone()),
        );
        if let Some((room, agent, _)) = subject {
            command.session_id = Some(room);
            command.agent_id = Some(agent);
        }
        let router = self.job_router.as_ref().ok_or_else(|| {
            acquisition_error(
                "job_router_unavailable",
                "The job gateway is unavailable; retry from your Room",
            )
        })?;
        let response = crate::runtime::control_plane::execute(
            &self.owned.config_projection.snapshot(),
            self,
            &command,
            router.kernel_gateway(),
            request,
        )
        .await?;
        match response {
            LocalDaemonResponse::ControlPlane { result } => Ok(result),
            _ => Err(acquisition_error(
                "control_plane_binding_invalid",
                "The connected service returned another result",
            )),
        }
    }
    /// A rate read is optional for local work and never allocates a machine.
    pub(super) async fn job_compute_quote(
        &self,
        needs: &JobNeeds,
        origin: &Origin,
    ) -> Option<JobMachineOffer> {
        let response = self
            .job_control_plane(origin, &ControlPlaneRequest::Capabilities {}, "job:quote")
            .await
            .ok()?;
        let ControlPlaneResponse::Capabilities {
            machine_allocation: Some(capability),
        } = response
        else {
            return None;
        };
        select_job_class(&capability, needs).ok().cloned()
    }
    pub(super) async fn job_compute_release_job_ref(
        &self,
        job_id: &str,
    ) -> Result<(), DaemonError> {
        let fence = self
            .job_compute_cleanup_request(&ControlPlaneRequest::CancelJob {
                job_ref: job_id.into(),
            })
            .await?;
        if !matches!(fence, ControlPlaneResponse::JobCancelled { ref job_ref } if job_ref == job_id)
        {
            return Err(acquisition_error(
                "control_plane_binding_invalid",
                "The service did not confirm job cancellation",
            ));
        }
        let current = self
            .job_compute_cleanup_request(&ControlPlaneRequest::GetJob {
                job_ref: job_id.into(),
            })
            .await;
        let current = match current {
            Err(error)
                if crate::runtime::cloud_api_client::cloud_error_code(&error)
                    == Some("not_found") =>
            {
                return Ok(())
            }
            response => response?,
        };
        let ControlPlaneResponse::Machine { allocation_ref, .. } = current else {
            return Err(acquisition_error(
                "control_plane_binding_invalid",
                "Job cleanup returned another response",
            ));
        };
        let caps = self
            .job_compute_cleanup_request(&ControlPlaneRequest::Capabilities {})
            .await?;
        let ControlPlaneResponse::Capabilities {
            machine_allocation: Some(capability),
        } = caps
        else {
            return Err(acquisition_error(
                "control_plane_unavailable",
                "The connected machine service is unavailable for cleanup",
            ));
        };
        self.job_compute_release_ref(&allocation_ref, &capability.revision)
            .await
    }
    pub(super) async fn job_compute_acquire(
        &self,
        job_id: &str,
        needs: &JobNeeds,
        timeout_s: u64,
        origin: &Origin,
    ) -> Result<JobMachine, DaemonError> {
        let caps = self
            .job_control_plane(
                origin,
                &ControlPlaneRequest::Capabilities {},
                &format!("job:{job_id}:capabilities"),
            )
            .await?;
        let ControlPlaneResponse::Capabilities {
            machine_allocation: Some(capability),
        } = caps
        else {
            return Err(acquisition_error(
                "capacity_unavailable",
                "This kernel has no remote job service; the job can wait locally",
            ));
        };
        let class = select_job_class(&capability, needs)?.clone();
        let revision = capability.revision.clone();
        let first = match self
            .job_control_plane(
                origin,
                &ControlPlaneRequest::AcquireJob {
                    capability_revision: revision.clone(),
                    request_id: format!("job:{job_id}"),
                    job_ref: job_id.into(),
                    needs: needs.clone(),
                    timeout_s,
                },
                &format!("job:{job_id}:acquire"),
            )
            .await
        {
            Ok(first) => first,
            Err(error) if matches!(&error, DaemonError::MachineRequest { code, .. } if matches!(code.as_str(), "acquisition_denied" | "approval_required" | "control_plane_unavailable")) => {
                return Err(error)
            }
            Err(error) => return Err(self.reconcile_failed_job_acquisition(job_id, error).await),
        };
        let ControlPlaneResponse::Machine { allocation_ref, .. } = &first else {
            let error = acquisition_error(
                "control_plane_binding_invalid",
                "The connected service did not return a job allocation",
            );
            return Err(self.reconcile_failed_job_acquisition(job_id, error).await);
        };
        let allocation_ref = allocation_ref.clone();
        if let Err(error) = self.job_record_allocation(job_id, &allocation_ref, &revision) {
            return Err(self.reconcile_failed_job_acquisition(job_id, error).await);
        }
        let result = self
            .wait_for_job_machine(job_id, &allocation_ref, &revision, class, origin, first)
            .await;
        match result {
            Ok(machine) => Ok(machine),
            Err(error) => Err(self.reconcile_failed_job_acquisition(job_id, error).await),
        }
    }
    async fn reconcile_failed_job_acquisition(
        &self,
        job_id: &str,
        error: DaemonError,
    ) -> DaemonError {
        // A missing HTTP receipt does not prove the service rejected the request.
        // Fence the job before lookup so even a delayed acceptance cannot leak a worker.
        match self.job_compute_release_job_ref(job_id).await {
            Ok(()) => match self.job_record_deleted(job_id) {
                Ok(()) => error,
                Err(receipt) => acquisition_error("job_receipt_failed", &format!("{error}; remote machine cleanup was confirmed, but its receipt could not be saved: {receipt}")),
            },
            Err(cleanup) => acquisition_error(
                "job_cleanup_pending",
                &format!("{error}; remote machine cleanup could not be confirmed: {cleanup}"),
            ),
        }
    }
    async fn wait_for_job_machine(
        &self,
        job_id: &str,
        allocation_ref: &str,
        capability_revision: &str,
        class: JobMachineOffer,
        origin: &Origin,
        mut response: ControlPlaneResponse,
    ) -> Result<JobMachine, DaemonError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        loop {
            let ControlPlaneResponse::Machine {
                state,
                access_state,
                kernel_target,
                revision,
                machine_label,
                address,
                purpose,
                ..
            } = response
            else {
                return Err(acquisition_error(
                    "control_plane_binding_invalid",
                    "The connected service did not return a job machine",
                ));
            };
            if purpose.as_deref() != Some("offload_job") {
                return Err(acquisition_error(
                    "control_plane_binding_invalid",
                    "The allocation is not a job worker",
                ));
            }
            if state == "ready" && access_state == "available" {
                let target = kernel_target.ok_or_else(|| {
                    acquisition_error(
                        "control_plane_binding_invalid",
                        "The job worker has no verified kernel identity",
                    )
                })?;
                let owner = self.owned.config_projection.local_owner_user_id();
                let retained = Origin {
                    grant_id: format!("{}{}", super::reach::OWNER_GRANT_PREFIX, owner),
                    harness: origin.harness.clone(),
                };
                let router = self.job_router.as_ref().ok_or_else(|| {
                    acquisition_error("job_router_unavailable", "The job gateway is unavailable")
                })?;
                return Ok(JobMachine {
                    allocation_ref: allocation_ref.into(),
                    machine_label: machine_label.unwrap_or_else(|| "remote job".into()),
                    address,
                    capability_revision: capability_revision.into(),
                    grant: Arc::new(OriginGrant::new(
                        (**router).clone(),
                        retained,
                        &target.kernel_id,
                    )),
                    class_id: class.class_id,
                    rate_per_minute: class.rate_per_minute,
                });
            }
            if matches!(state.as_str(), "failed" | "deleted" | "released")
                || tokio::time::Instant::now() >= deadline
            {
                return Err(acquisition_error(
                    "capacity_unavailable",
                    "The remote job machine did not become ready; its allocation is being released",
                ));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            response = self
                .job_control_plane(
                    origin,
                    &ControlPlaneRequest::Get {
                        allocation_ref: allocation_ref.into(),
                    },
                    &format!("job:{job_id}:get:{revision}"),
                )
                .await?;
        }
    }
    /// Cleanup keeps only the accepted allocation receipt, so ending a provider run cannot leak its VM.
    pub(super) async fn job_compute_release_ref(
        &self,
        allocation_ref: &str,
        capability_revision: &str,
    ) -> Result<(), DaemonError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        loop {
            let result = tokio::time::timeout_at(
                deadline,
                self.job_compute_release_attempt(allocation_ref, capability_revision),
            )
            .await
            .unwrap_or_else(|_| {
                Err(acquisition_error(
                    "job_cleanup_pending",
                    "The job ended; the machine service has not yet confirmed deletion",
                ))
            });
            match result {
                Err(error)
                    if matches!(error, DaemonError::LocalTransport { .. })
                        && crate::runtime::cloud_api_client::cloud_error_is_retryable(&error)
                        && tokio::time::Instant::now() < deadline =>
                {
                    // An uncertain release may have committed. Re-read its receipt
                    // before deciding whether another release is needed.
                    tracing::warn!(%error, "job machine cleanup retrying after service interruption");
                    tokio::time::sleep_until(
                        (tokio::time::Instant::now() + Duration::from_secs(2)).min(deadline),
                    )
                    .await;
                }
                result => return result,
            }
        }
    }

    async fn job_compute_release_attempt(
        &self,
        allocation_ref: &str,
        capability_revision: &str,
    ) -> Result<(), DaemonError> {
        let current = self
            .job_compute_cleanup_request(&ControlPlaneRequest::Get {
                allocation_ref: allocation_ref.into(),
            })
            .await?;
        let ControlPlaneResponse::Machine {
            revision,
            state,
            allocation_ref: observed,
            kernel_target,
            ..
        } = current
        else {
            return Err(acquisition_error(
                "control_plane_binding_invalid",
                "Cleanup did not return the job allocation",
            ));
        };
        if observed != allocation_ref {
            return Err(acquisition_error(
                "control_plane_binding_invalid",
                "Cleanup returned another allocation",
            ));
        }
        if state == "deleted" {
            if let (Some(router), Some(target)) = (&self.job_router, &kernel_target) {
                router
                    .kernel_gateway()
                    .forget_job_machine(&target.kernel_id)
                    .await;
            }
            return Ok(());
        }
        if state != "deleting" {
            self.job_compute_cleanup_request(&ControlPlaneRequest::Release {
                allocation_ref: allocation_ref.into(),
                capability_revision: capability_revision.into(),
                request_id: format!("job-release:{allocation_ref}:{revision}"),
                expected_revision: revision,
            })
            .await?;
        }
        loop {
            let result = self
                .job_compute_cleanup_request(&ControlPlaneRequest::Get {
                    allocation_ref: allocation_ref.into(),
                })
                .await?;
            match result {
                ControlPlaneResponse::Machine {
                    allocation_ref: observed,
                    state,
                    ..
                } if observed == allocation_ref => {
                    if state == "deleted" {
                        if let (Some(router), Some(target)) = (&self.job_router, &kernel_target) {
                            router
                                .kernel_gateway()
                                .forget_job_machine(&target.kernel_id)
                                .await;
                        }
                        return Ok(());
                    }
                    if state == "failed" {
                        return Err(acquisition_error("job_cleanup_failed", "The remote job ended but its machine could not be deleted; cleanup will need a retry"));
                    }
                }
                _ => {
                    return Err(acquisition_error(
                        "control_plane_binding_invalid",
                        "Cleanup returned another allocation",
                    ))
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    pub(super) async fn job_compute_started(
        &self,
        machine: &JobMachine,
    ) -> Result<(), DaemonError> {
        self.job_compute_cleanup_request(&ControlPlaneRequest::JobStarted {
            allocation_ref: machine.allocation_ref.clone(),
            capability_revision: machine.capability_revision.clone(),
        })
        .await?;
        Ok(())
    }
    async fn job_compute_cleanup_request(
        &self,
        request: &ControlPlaneRequest,
    ) -> Result<ControlPlaneResponse, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let profile = crate::runtime::cloud_relay_control::active_cloud_profile(&config)
            .ok_or_else(|| {
                acquisition_error(
                    "control_plane_unavailable",
                    "Reconnect the kernel's machine service to release its job worker",
                )
            })?;
        let credential = profile
            .kernel_credential
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                acquisition_error(
                    "control_plane_unavailable",
                    "The machine service enrollment is unavailable",
                )
            })?;
        crate::runtime::cloud_api_client::post_cloud_json(profile.api_url.clone(), "/control-plane", serde_json::json!({"accountId":profile.account_id,"kernelCredential":credential,"request":request})).await
    }
}
