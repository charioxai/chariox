use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wait_timeout::ChildExt;

use crate::config::{DaemonConfig, KernelRuntimeRole};
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::local::{
    AgentUtilityInput, AgentUtilityKind, AgentUtilityOutput, CancelProjectEnvironmentSetupRequest,
    GetProjectEnvironmentSetupStatusRequest, LocalDaemonRequest, LocalDaemonResponse,
    ProjectEnvironmentCommandResult, ProjectEnvironmentDefinition, ProjectEnvironmentSetupPhase,
    ProjectEnvironmentSetupStatus, ProjectEnvironmentSetupUtilityInput,
    ProjectEnvironmentValidation, RetryProjectEnvironmentSetupRequest, RunAgentUtilityRequest,
    StartProjectEnvironmentSetupRequest,
};
use crate::provider::RuntimeProviderRun;
use crate::runtime::agent_utility_executor::{
    assert_agent_utility_can_run, run_agent_utility_on_provider_run,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RelayProjectEnvironmentSetupStatus,
};

use super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_retry_transport;

#[path = "project_environment_setup_dispatch.rs"]
mod project_environment_setup_dispatch;
#[path = "project_environment_setup_policy.rs"]
mod project_environment_setup_policy;
#[path = "project_environment_setup_storage.rs"]
mod project_environment_setup_storage;
#[path = "project_environment_setup_validation.rs"]
mod project_environment_setup_validation;
use project_environment_setup_dispatch::*;
use project_environment_setup_policy::*;
pub(super) use project_environment_setup_storage::ProjectEnvironmentSetupStore;
use project_environment_setup_storage::{
    RemoteSetupRecoveryDecision, RemoteSetupRecoveryReservationGuard, SetupEntry, SetupExecution,
};
use project_environment_setup_validation::*;

fn remote_setup_recovery_should_dispatch(
    decision: RemoteSetupRecoveryDecision,
) -> Result<bool, DaemonError> {
    match decision {
        RemoteSetupRecoveryDecision::Dispatch => Ok(true),
        RemoteSetupRecoveryDecision::Cancelled | RemoteSetupRecoveryDecision::Acknowledged => {
            Ok(false)
        }
        RemoteSetupRecoveryDecision::InFlight => Err(remote_setup_recovery_transport_error(
            "the same-attempt replay is already in flight",
        )),
        RemoteSetupRecoveryDecision::Unknown => Err(remote_setup_recovery_transport_error(
            "the same-attempt replay result is unresolved",
        )),
        RemoteSetupRecoveryDecision::Stale => Err(remote_setup_recovery_transport_error(
            "the setup attempt or worker binding changed while status was being observed",
        )),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn execute_project_environment_setup_request(
        &self,
        request: LocalDaemonRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::StartProjectEnvironmentSetup(request) => {
                let execution = self.prepare_setup_execution(request, caller_user_id)?;
                let (status, should_spawn) = self
                    .owned
                    .project_environment_setups
                    .begin(execution.clone())?;
                if should_spawn {
                    if execution.remote_leased_agent_id.is_some() {
                        self.spawn_remote_project_environment_setup(execution, status.attempt);
                    } else {
                        self.spawn_project_environment_setup(execution, status.attempt);
                    }
                }
                Ok(LocalDaemonResponse::ProjectEnvironmentSetupStarted { status })
            }
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(request) => {
                let (execution, status, cancel_requested) =
                    self.owned
                        .project_environment_setups
                        .get_entry_with_cancellation(&request.operation_id, caller_user_id)?;
                if execution.remote_leased_agent_id.is_some() {
                    let observation_generation = self
                        .owned
                        .project_environment_setups
                        .begin_remote_observation();
                    let binding_id = execution
                        .remote_leased_agent_id
                        .as_deref()
                        .expect("remote setup observations require a leased-agent binding");
                    let observation_deadline = Instant::now()
                        + remote_setup_observation_budget(&self.owned.config_projection.snapshot());
                    let setup = get_remote_setup_status_with_deadline(
                        self,
                        &execution,
                        observation_deadline,
                    )
                    .await;
                    let status = match setup {
                        Ok(setup)
                            if !cancel_requested
                                && matches!(
                                    status.phase,
                                    ProjectEnvironmentSetupPhase::Requested
                                        | ProjectEnvironmentSetupPhase::Preparing
                                        | ProjectEnvironmentSetupPhase::Validating
                                )
                                && is_replayable_stale_remote_setup_status(
                                    &status,
                                    &setup.status,
                                ) =>
                        {
                            // A retained worker may still report a retryable
                            // terminal record from the previous attempt after
                            // home accepted Retry. Reuse the authenticated
                            // public retry request; do not accept that stale
                            // record as the current attempt.
                            let recovery_decision =
                                self.owned.project_environment_setups.begin_remote_recovery(
                                    &execution.operation_id,
                                    status.attempt,
                                    binding_id,
                                    observation_generation,
                                );
                            if !remote_setup_recovery_should_dispatch(recovery_decision)? {
                                let (_, current_status, _) = self
                                    .owned
                                    .project_environment_setups
                                    .get_entry_with_cancellation(
                                        &execution.operation_id,
                                        caller_user_id,
                                    )?;
                                return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                    status: current_status,
                                });
                            }
                            if let Some(current_status) = self.recovery_dispatch_status_if_lost(
                                &execution.operation_id,
                                status.attempt,
                                binding_id,
                                observation_generation,
                                caller_user_id,
                            )? {
                                return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                    status: current_status,
                                });
                            }
                            let mut recovery_reservation = RemoteSetupRecoveryReservationGuard::new(
                                &self.owned.project_environment_setups,
                                &execution.operation_id,
                                status.attempt,
                                binding_id,
                                observation_generation,
                            );
                            match retry_remote_setup_with_deadline(
                                self,
                                &execution,
                                observation_deadline,
                            )
                            .await
                            {
                                Ok(setup) => {
                                    match self
                                        .reconcile_remote_project_environment_setup_with_observation(
                                            &execution,
                                            setup,
                                            Some(observation_generation),
                                        ) {
                                        Ok(status) => {
                                            recovery_reservation.disarm();
                                            status
                                        }
                                        Err(error) => return Err(error),
                                    }
                                }
                                Err(error)
                                    if remote_prompt_error_should_retry_transport(&error) =>
                                {
                                    return Err(error);
                                }
                                Err(error) => {
                                    recovery_reservation.clear_if_matches();
                                    self.settle_remote_setup_recovery_rejection(
                                        &execution,
                                        status.attempt,
                                        &error,
                                    );
                                    return Err(error);
                                }
                            }
                        }
                        Ok(setup) => self
                            .reconcile_remote_project_environment_setup_with_observation(
                                &execution,
                                setup,
                                Some(observation_generation),
                            )?,
                        Err(error)
                            if is_stale_remote_setup_binding_error(&error)
                                && !cancel_requested
                                && matches!(
                                    status.phase,
                                    ProjectEnvironmentSetupPhase::Requested
                                        | ProjectEnvironmentSetupPhase::Preparing
                                        | ProjectEnvironmentSetupPhase::Validating
                                ) =>
                        {
                            // A worker restart clears its ephemeral lease
                            // authorization. Refresh the home binding through
                            // the normal lease provisioning path, rebind the
                            // retained worker operation through the
                            // authenticated Retry request, and reconcile only
                            // the current home attempt.
                            let recovery_decision =
                                self.owned.project_environment_setups.begin_remote_recovery(
                                    &execution.operation_id,
                                    status.attempt,
                                    binding_id,
                                    observation_generation,
                                );
                            if !remote_setup_recovery_should_dispatch(recovery_decision)? {
                                let (_, current_status, _) = self
                                    .owned
                                    .project_environment_setups
                                    .get_entry_with_cancellation(
                                        &execution.operation_id,
                                        caller_user_id,
                                    )?;
                                return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                    status: current_status,
                                });
                            }
                            if let Some(current_status) = self.recovery_dispatch_status_if_lost(
                                &execution.operation_id,
                                status.attempt,
                                binding_id,
                                observation_generation,
                                caller_user_id,
                            )? {
                                return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                    status: current_status,
                                });
                            }
                            self.await_remote_setup_binding_recovery(
                                execution.clone(),
                                status.attempt,
                                observation_generation,
                                observation_deadline,
                            )
                            .await?
                        }
                        Err(error)
                            if is_missing_remote_setup_operation(&error)
                                && matches!(
                                    status.phase,
                                    ProjectEnvironmentSetupPhase::Requested
                                        | ProjectEnvironmentSetupPhase::Preparing
                                        | ProjectEnvironmentSetupPhase::Validating
                                ) =>
                        {
                            if cancel_requested {
                                return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                    status,
                                });
                            }
                            match self.owned.project_environment_setups.begin_remote_recovery(
                                &execution.operation_id,
                                status.attempt,
                                binding_id,
                                observation_generation,
                            ) {
                                RemoteSetupRecoveryDecision::Cancelled => {
                                    return Ok(
                                        LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                            status,
                                        },
                                    );
                                }
                                RemoteSetupRecoveryDecision::InFlight => {
                                    return Err(remote_setup_recovery_transport_error(
                                        "the same-attempt replay is already in flight",
                                    ));
                                }
                                RemoteSetupRecoveryDecision::Unknown => {
                                    return Err(remote_setup_recovery_transport_error(
                                        "the same-attempt replay result is unresolved",
                                    ));
                                }
                                RemoteSetupRecoveryDecision::Acknowledged => {
                                    return Ok(
                                        LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                            status,
                                        },
                                    );
                                }
                                RemoteSetupRecoveryDecision::Stale => {
                                    return Err(remote_setup_recovery_transport_error(
                                        "the setup attempt changed while status was being observed",
                                    ));
                                }
                                RemoteSetupRecoveryDecision::Dispatch => {}
                            }
                            // This lock-protected fence linearizes recovery
                            // dispatch against Cancel/Retry. It is not a
                            // second unsynchronized snapshot of cancellation.
                            if !self
                                .owned
                                .project_environment_setups
                                .remote_recovery_dispatch_allowed(
                                    &execution.operation_id,
                                    status.attempt,
                                    binding_id,
                                    observation_generation,
                                )
                            {
                                let (_, current_status, current_cancel_requested) = self
                                    .owned
                                    .project_environment_setups
                                    .get_entry_with_cancellation(
                                        &execution.operation_id,
                                        caller_user_id,
                                    )?;
                                if current_cancel_requested
                                    || matches!(
                                        current_status.phase,
                                        ProjectEnvironmentSetupPhase::Ready
                                            | ProjectEnvironmentSetupPhase::Failed
                                            | ProjectEnvironmentSetupPhase::Cancelled
                                    )
                                {
                                    return Ok(
                                        LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                            status: current_status,
                                        },
                                    );
                                }
                                return Err(remote_setup_recovery_transport_error(
                                    "the setup attempt changed before replay dispatch",
                                ));
                            }
                            let mut recovery_reservation = RemoteSetupRecoveryReservationGuard::new(
                                &self.owned.project_environment_setups,
                                &execution.operation_id,
                                status.attempt,
                                binding_id,
                                observation_generation,
                            );
                            match start_remote_setup_with_deadline(
                                self,
                                &execution,
                                status.attempt,
                                observation_deadline,
                            )
                            .await
                            {
                                Ok(setup) => {
                                    match self
                                        .reconcile_remote_project_environment_setup_with_observation(
                                            &execution,
                                            setup,
                                            Some(observation_generation),
                                        ) {
                                        Ok(status) => {
                                            recovery_reservation.disarm();
                                            status
                                        }
                                        Err(error) => {
                                            return Err(error);
                                        }
                                    }
                                }
                                Err(error)
                                    if remote_prompt_error_should_retry_transport(&error) =>
                                {
                                    return Err(error);
                                }
                                Err(error) => {
                                    recovery_reservation.clear_if_matches();
                                    if let DaemonError::RelayTransport {
                                        code,
                                        retryable: false,
                                        ..
                                    } = &error
                                    {
                                        self.owned
                                            .project_environment_setups
                                            .mark_failed_non_retryable(
                                            &execution.operation_id,
                                            status.attempt,
                                            code,
                                            "the remote worker rejected project environment setup",
                                        );
                                    } else {
                                        self.owned.project_environment_setups.mark_failed(
                                            &execution.operation_id,
                                            status.attempt,
                                            "worker_dispatch_failed",
                                            "the remote worker could not be reached for environment setup",
                                        );
                                    }
                                    return Err(error);
                                }
                            }
                        }
                        Err(error) if remote_prompt_error_should_retry_transport(&error) => {
                            // A relay disconnect is transport uncertainty, not
                            // a worker-authoritative terminal result. Return
                            // the structured diagnostic while preserving the
                            // current operation and attempt for a later Get.
                            return Err(error);
                        }
                        Err(error) => {
                            if let DaemonError::RelayTransport {
                                code,
                                retryable: false,
                                ..
                            } = &error
                            {
                                self.owned
                                    .project_environment_setups
                                    .mark_failed_non_retryable(
                                        &execution.operation_id,
                                        status.attempt,
                                        code,
                                        "the remote worker rejected setup status",
                                    );
                            } else {
                                self.owned.project_environment_setups.mark_failed(
                                    &execution.operation_id,
                                    status.attempt,
                                    "worker_status_unavailable",
                                    "the remote worker status could not be confirmed",
                                );
                            }
                            return Err(error);
                        }
                    };
                    return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus { status });
                }
                Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus { status })
            }
            LocalDaemonRequest::CancelProjectEnvironmentSetup(request) => {
                let (execution, _) = self
                    .owned
                    .project_environment_setups
                    .get_entry(&request.operation_id, caller_user_id)?;
                self.owned.project_environment_setups.request_cancel(
                    &request.operation_id,
                    &request.session_id,
                    caller_user_id,
                    execution.remote_leased_agent_id.is_some(),
                )?;
                if execution.remote_leased_agent_id.is_some() {
                    let setup = cancel_remote_setup(self, &execution).await?;
                    let status =
                        self.reconcile_remote_project_environment_setup(&execution, setup)?;
                    return Ok(LocalDaemonResponse::ProjectEnvironmentSetupCancelled { status });
                }
                let status = self
                    .owned
                    .project_environment_setups
                    .wait_for_cancellation(&request.operation_id, caller_user_id)
                    .await?;
                Ok(LocalDaemonResponse::ProjectEnvironmentSetupCancelled { status })
            }
            LocalDaemonRequest::RetryProjectEnvironmentSetup(request) => {
                let (execution, attempt, status) = self.owned.project_environment_setups.retry(
                    &request.operation_id,
                    &request.session_id,
                    caller_user_id,
                )?;
                if execution.remote_leased_agent_id.is_some() {
                    self.spawn_remote_project_environment_setup_retry(execution, attempt);
                } else {
                    self.spawn_project_environment_setup(execution, attempt);
                }
                Ok(LocalDaemonResponse::ProjectEnvironmentSetupRetried { status })
            }
            _ => Err(setup_error("unsupported project environment setup request")),
        }
    }

    fn recovery_dispatch_status_if_lost(
        &self,
        operation_id: &str,
        attempt: u32,
        binding_id: &str,
        observation_generation: u64,
        caller_user_id: &str,
    ) -> Result<Option<ProjectEnvironmentSetupStatus>, DaemonError> {
        if self
            .owned
            .project_environment_setups
            .remote_recovery_dispatch_allowed(
                operation_id,
                attempt,
                binding_id,
                observation_generation,
            )
        {
            return Ok(None);
        }
        let (_, current_status, current_cancel_requested) =
            self.owned
                .project_environment_setups
                .get_entry_with_cancellation(operation_id, caller_user_id)?;
        if current_cancel_requested
            || matches!(
                current_status.phase,
                ProjectEnvironmentSetupPhase::Ready
                    | ProjectEnvironmentSetupPhase::Failed
                    | ProjectEnvironmentSetupPhase::Cancelled
            )
        {
            return Ok(Some(current_status));
        }
        Err(remote_setup_recovery_transport_error(
            "the setup attempt or worker binding changed before recovery dispatch",
        ))
    }

    async fn await_remote_setup_binding_recovery(
        &self,
        execution: SetupExecution,
        attempt: u32,
        observation_generation: u64,
        observation_deadline: Instant,
    ) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let state = self.clone();
        tokio::spawn(async move {
            let store = state.owned.project_environment_setups.clone();
            let Some(stale_binding_id) = execution.remote_leased_agent_id.clone() else {
                let _ = result_tx.send(Err(setup_error(
                    "binding recovery requires a stale leased-agent binding",
                )));
                return;
            };
            let mut recovery_reservation = RemoteSetupRecoveryReservationGuard::new(
                &store,
                &execution.operation_id,
                attempt,
                &stale_binding_id,
                observation_generation,
            );
            let result = match refresh_remote_setup_binding(
                &state,
                &execution,
                attempt,
                observation_generation,
            )
            .await
            {
                Ok(rebound_execution) => match rebound_execution.remote_leased_agent_id.clone() {
                    Some(rebound_binding_id) => {
                        recovery_reservation.rebind(&rebound_binding_id);
                        match state.recovery_dispatch_status_if_lost(
                            &execution.operation_id,
                            attempt,
                            &rebound_binding_id,
                            observation_generation,
                            &execution.owner_user_id,
                        ) {
                            Ok(Some(status)) => {
                                recovery_reservation.clear_if_matches();
                                Ok(status)
                            }
                            Ok(None) => {
                                match retry_remote_setup_with_deadline(
                                        &state,
                                        &rebound_execution,
                                        observation_deadline,
                                    )
                                    .await
                                    {
                                        Ok(setup) => match state
                                            .reconcile_remote_project_environment_setup_with_observation(
                                                &rebound_execution,
                                                setup,
                                                Some(observation_generation),
                                            ) {
                                            Ok(status) => {
                                                recovery_reservation.disarm();
                                                Ok(status)
                                            }
                                            Err(error) => Err(error),
                                        },
                                        Err(error) => Err(error),
                                    }
                            }
                            Err(error) => Err(error),
                        }
                    }
                    None => Err(setup_error(
                        "refreshed remote setup lost its leased-agent binding",
                    )),
                },
                Err(error) => Err(error),
            };
            let result = match result {
                Err(error) if remote_setup_recovery_permanent_rejection_code(&error).is_some() => {
                    recovery_reservation.clear_if_matches();
                    state.settle_remote_setup_recovery_rejection(&execution, attempt, &error);
                    Err(error)
                }
                result => result,
            };
            let _ = result_tx.send(result);
        });

        let wait = observation_timeout(observation_deadline)?;
        match tokio::time::timeout(wait, result_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(remote_setup_recovery_transport_error(
                "the binding refresh recovery task ended before it reported a result",
            )),
            Err(_) => Err(remote_setup_recovery_transport_error(
                "the binding refresh recovery continues after the observation deadline",
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn start_leased_project_environment_setup(
        &self,
        target: crate::app::LeasedProjectEnvironmentSetupTarget,
        leased_agent_id: String,
        operation_id: String,
        attempt: u32,
        project_id: String,
        workspace_id: String,
        target_worker_id: String,
        target_platform: String,
        definition: Option<ProjectEnvironmentDefinition>,
        validation_commands: Vec<String>,
    ) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
        validate_operation_id(&operation_id)?;
        validate_commands(&validation_commands)?;
        let config = self.owned.config_projection.snapshot();
        if config.kernel_runtime_role != KernelRuntimeRole::RemoteLeaseWorker {
            return Err(setup_error(
                "project environment setup is only executable by a lease worker",
            ));
        }
        if target_worker_id != config.host_machine_id {
            return Err(setup_error(
                "requested worker does not match this kernel's worker identity",
            ));
        }
        if target_platform.trim().is_empty() || target_platform != actual_worker_platform() {
            return Err(setup_error("requested platform does not match this worker"));
        }
        if !workspace_id.is_empty() && workspace_id != target.workspace_id {
            return Err(setup_error(
                "requested workspace does not match the leased worker worktree",
            ));
        }
        let definition =
            validate_setup_definition(definition, &target_platform, &validation_commands)?;
        ensure_worker_validation_boundary(&config)?;
        let execution = SetupExecution {
            owner_user_id: target.owner_user_id.clone(),
            operation_id,
            project_id,
            session_id: target.home_session_id.clone(),
            agent_id: target.home_agent_id.clone(),
            execution_session_id: target.backing_session_id,
            execution_agent_id: target.backing_agent_id,
            workspace_id: target.workspace_id,
            target_worker_id,
            target_platform,
            definition,
            validation_commands,
            persist_project_definition: false,
            remote_leased_agent_id: Some(leased_agent_id),
        };
        let (status, should_spawn) = self
            .owned
            .project_environment_setups
            .begin_at_attempt(execution.clone(), attempt)?;
        if should_spawn {
            self.spawn_project_environment_setup(execution.clone(), status.attempt);
        }
        let (_, definition) = self.owned.project_environment_setups.remote_status(
            &execution.operation_id,
            execution
                .remote_leased_agent_id
                .as_deref()
                .unwrap_or_default(),
        )?;
        Ok(RelayProjectEnvironmentSetupStatus { status, definition })
    }

    pub(crate) async fn get_leased_project_environment_setup_status(
        &self,
        target: crate::app::LeasedProjectEnvironmentSetupTarget,
        leased_agent_id: &str,
        operation_id: &str,
    ) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
        let (status, definition) = self
            .owned
            .project_environment_setups
            .remote_status(operation_id, leased_agent_id)?;
        let config = self.owned.config_projection.snapshot();
        ensure_worker_setup_status_target(&target, &status, &config)?;
        Ok(RelayProjectEnvironmentSetupStatus { status, definition })
    }

    pub(crate) async fn cancel_leased_project_environment_setup(
        &self,
        target: crate::app::LeasedProjectEnvironmentSetupTarget,
        leased_agent_id: &str,
        operation_id: &str,
    ) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
        let (current_status, definition) = self
            .owned
            .project_environment_setups
            .remote_status(operation_id, leased_agent_id)?;
        let config = self.owned.config_projection.snapshot();
        ensure_worker_setup_status_target(&target, &current_status, &config)?;
        self.owned.project_environment_setups.cancel(
            operation_id,
            &target.home_session_id,
            &target.owner_user_id,
        )?;
        let status = self
            .owned
            .project_environment_setups
            .wait_for_cancellation(operation_id, &target.owner_user_id)
            .await?;
        ensure_worker_setup_status_target(&target, &status, &config)?;
        Ok(RelayProjectEnvironmentSetupStatus { status, definition })
    }

    pub(crate) async fn retry_leased_project_environment_setup(
        &self,
        target: crate::app::LeasedProjectEnvironmentSetupTarget,
        leased_agent_id: &str,
        operation_id: &str,
    ) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let target_platform = actual_worker_platform();
        let (current_status, definition) = self
            .owned
            .project_environment_setups
            .rebind_remote_worker_target(
                operation_id,
                leased_agent_id,
                &target,
                &config.host_machine_id,
                &target_platform,
            )?;
        ensure_worker_setup_status_target(&target, &current_status, &config)?;
        let (execution, attempt, status) = self.owned.project_environment_setups.retry(
            operation_id,
            &target.home_session_id,
            &target.owner_user_id,
        )?;
        ensure_worker_setup_status_target(&target, &status, &config)?;
        self.spawn_project_environment_setup(execution, attempt);
        Ok(RelayProjectEnvironmentSetupStatus { status, definition })
    }

    fn spawn_remote_project_environment_setup(&self, execution: SetupExecution, attempt: u32) {
        self.spawn_remote_project_environment_setup_request(execution, attempt, false);
    }

    fn spawn_remote_project_environment_setup_retry(
        &self,
        execution: SetupExecution,
        attempt: u32,
    ) {
        self.spawn_remote_project_environment_setup_request(execution, attempt, true);
    }

    fn spawn_remote_project_environment_setup_request(
        &self,
        execution: SetupExecution,
        attempt: u32,
        retry: bool,
    ) {
        let runtime_state = self.clone();
        tokio::spawn(async move {
            let result = if retry {
                retry_remote_setup(&runtime_state, &execution).await
            } else {
                start_remote_setup(&runtime_state, &execution, attempt).await
            };
            match result {
                Ok(setup) => {
                    if let Err(error) =
                        runtime_state.reconcile_remote_project_environment_setup(&execution, setup)
                    {
                        runtime_state.owned.project_environment_setups.mark_failed(
                            &execution.operation_id,
                            attempt,
                            "worker_status_invalid",
                            "the remote worker returned an invalid setup status",
                        );
                        crate::logging::warn_with_fields(
                            "project.environment_setup",
                            "remote setup status was rejected",
                            serde_json::json!({
                                "operation_id": execution.operation_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
                Err(error) => {
                    if remote_prompt_error_should_retry_transport(&error) {
                        // The worker may still be executing after this
                        // transport observation. Leave the active operation
                        // untouched so a later status query can reconcile
                        // the worker-authoritative result.
                    } else if let DaemonError::RelayTransport {
                        code,
                        retryable: false,
                        ..
                    } = &error
                    {
                        runtime_state
                            .owned
                            .project_environment_setups
                            .mark_failed_non_retryable(
                                &execution.operation_id,
                                attempt,
                                code,
                                "the remote worker rejected project environment setup",
                            );
                    } else {
                        runtime_state.owned.project_environment_setups.mark_failed(
                            &execution.operation_id,
                            attempt,
                            "worker_dispatch_failed",
                            "the remote worker could not be reached for environment setup",
                        );
                    }
                    crate::logging::warn_with_fields(
                        "project.environment_setup",
                        "remote setup dispatch failed",
                        serde_json::json!({
                            "operation_id": execution.operation_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        });
    }

    fn reconcile_remote_project_environment_setup(
        &self,
        execution: &SetupExecution,
        setup: RelayProjectEnvironmentSetupStatus,
    ) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
        self.reconcile_remote_project_environment_setup_with_observation(execution, setup, None)
    }

    fn reconcile_remote_project_environment_setup_with_observation(
        &self,
        execution: &SetupExecution,
        setup: RelayProjectEnvironmentSetupStatus,
        observation_generation: Option<u64>,
    ) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
        let (_, current_status) = self
            .owned
            .project_environment_setups
            .get_entry(&execution.operation_id, &execution.owner_user_id)?;
        validate_remote_setup_status(
            execution,
            current_status.attempt,
            &setup.status,
            setup.definition.as_ref(),
        )?;
        validate_remote_setup_transition(current_status.phase, setup.status.phase)?;
        if setup.status.phase == ProjectEnvironmentSetupPhase::Ready {
            let definition = setup.definition.clone().ok_or_else(|| {
                setup_error("worker cannot report setup ready without a definition")
            })?;
            self.update_project_environment_definition(
                &execution.project_id,
                definition,
                &execution.owner_user_id,
            )?;
        }
        self.owned.project_environment_setups.reconcile_remote(
            &execution.operation_id,
            execution,
            setup.status,
            setup.definition,
            observation_generation,
        )
    }

    fn settle_remote_setup_recovery_rejection(
        &self,
        execution: &SetupExecution,
        attempt: u32,
        error: &DaemonError,
    ) {
        Self::settle_remote_setup_recovery_rejection_in_store(
            &self.owned.project_environment_setups,
            execution,
            attempt,
            error,
        );
    }

    fn settle_remote_setup_recovery_rejection_in_store(
        store: &ProjectEnvironmentSetupStore,
        execution: &SetupExecution,
        attempt: u32,
        error: &DaemonError,
    ) {
        let Some(code) = remote_setup_recovery_permanent_rejection_code(error) else {
            return;
        };
        store.mark_failed_non_retryable(
            &execution.operation_id,
            attempt,
            code,
            "the remote worker rejected project environment setup after binding recovery",
        );
    }

    fn prepare_setup_execution(
        &self,
        request: StartProjectEnvironmentSetupRequest,
        caller_user_id: &str,
    ) -> Result<SetupExecution, DaemonError> {
        validate_operation_id(&request.operation_id)?;
        let config = self.owned.config_projection.snapshot();
        if request.target_platform.trim().is_empty() {
            return Err(setup_error("target platform must not be empty"));
        }
        validate_commands(&request.validation_commands)?;
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let project = self.owned.session_store.get_project(&request.project_id)?;
        if project.owner_user_id() != caller_user_id {
            return Err(setup_error("caller does not own the selected project"));
        }
        if session.project_id() != request.project_id {
            return Err(setup_error(
                "session is not attached to the selected project",
            ));
        }
        let agent = self
            .owned
            .agent_store
            .get_session_agents(&request.session_id)
            .into_iter()
            .find(|agent| agent.id() == request.agent_id)
            .ok_or_else(|| setup_error("agent does not belong to the selected session"))?;
        let remote_execution = agent.remote_execution().cloned();
        let is_remote_worker_dispatch = match config.kernel_runtime_role {
            KernelRuntimeRole::RemoteLeaseWorker => {
                if request.target_worker_id != config.host_machine_id {
                    return Err(setup_error(
                        "requested worker does not match this kernel's worker identity",
                    ));
                }
                if request.target_platform != actual_worker_platform() {
                    return Err(setup_error("requested platform does not match this worker"));
                }
                if remote_execution.is_some() {
                    return Err(setup_error(
                        "a lease worker cannot execute setup for another remote worker",
                    ));
                }
                false
            }
            KernelRuntimeRole::General => {
                let Some(remote_execution) = remote_execution.as_ref() else {
                    return Err(setup_error(
                        "project environment setup requires a dedicated worker or remote-backed agent",
                    ));
                };
                if request.target_worker_id != remote_execution.worker_machine_id {
                    return Err(setup_error(
                        "requested worker does not match the remote agent binding",
                    ));
                }
                if remote_execution.worker_kernel_id.trim().is_empty()
                    || remote_execution.worker_machine_id.trim().is_empty()
                    || remote_execution.leased_agent_id.trim().is_empty()
                    || !remote_execution.relay_peer_protocol_compatible()
                {
                    return Err(setup_error(
                        "remote agent binding is incomplete or uses an obsolete relay protocol",
                    ));
                }
                true
            }
        };
        let definition = request
            .definition
            .or_else(|| project.environment_definition().cloned());
        if let Some(definition) = &definition {
            definition
                .validate()
                .map_err(|message| setup_error(&message))?;
            if definition.target_platform != request.target_platform {
                return Err(setup_error(
                    "environment definition targets a different platform",
                ));
            }
            if !request.validation_commands.is_empty() {
                return Err(setup_error(
                    "additional validation commands are only allowed when no definition exists",
                ));
            }
            if definition.validation_commands.is_empty() {
                return Err(setup_error(
                    "environment definition must include at least one validation command",
                ));
            }
        }
        // A home worktree path is not a worker worktree path. The worker
        // derives its canonical backing worktree from the authenticated lease
        // and rejects any non-empty path that does not match it.
        let workspace_id = if is_remote_worker_dispatch {
            String::new()
        } else {
            agent
                .worktree_id()
                .unwrap_or_else(|| session.worktree_id())
                .to_string()
        };
        Ok(SetupExecution {
            owner_user_id: caller_user_id.to_string(),
            operation_id: request.operation_id,
            project_id: request.project_id,
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
            execution_session_id: request.session_id,
            execution_agent_id: request.agent_id,
            workspace_id,
            target_worker_id: request.target_worker_id,
            target_platform: request.target_platform,
            definition,
            validation_commands: request.validation_commands,
            persist_project_definition: true,
            remote_leased_agent_id: is_remote_worker_dispatch.then(|| {
                remote_execution
                    .expect("remote dispatch binding was checked")
                    .leased_agent_id
                    .clone()
            }),
        })
    }

    fn spawn_project_environment_setup(&self, execution: SetupExecution, attempt: u32) {
        let Some(guard) = self
            .owned
            .project_environment_setups
            .begin_execution(&execution.operation_id, attempt)
        else {
            return;
        };
        let runtime_state = self.clone();
        tokio::spawn(async move {
            let _guard = guard;
            runtime_state
                .run_project_environment_setup(execution, attempt)
                .await;
        });
    }

    async fn run_project_environment_setup(&self, execution: SetupExecution, attempt: u32) {
        let store = &self.owned.project_environment_setups;
        if store.is_cancelled(&execution.operation_id, attempt) {
            return;
        }
        if ensure_worker_validation_boundary(&self.owned.config_projection.snapshot()).is_err() {
            store.mark_failed(
                &execution.operation_id,
                attempt,
                "worker_boundary_unavailable",
                "project environment setup requires a confirmed disposable worker boundary",
            );
            return;
        }
        if !store.update(&execution.operation_id, attempt, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Preparing;
            entry.status.progress_percent = 10;
            entry.status.message =
                Some("utility agent is preparing the target environment".to_string());
        }) {
            return;
        }
        if store.is_cancelled(&execution.operation_id, attempt) {
            return;
        }
        let _ = store.update(&execution.operation_id, attempt, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Validating;
            entry.status.progress_percent = 45;
            entry.status.message =
                Some("kernel is validating commands in the target worker".to_string());
        });

        let utility_request = RunAgentUtilityRequest {
            session_id: execution.execution_session_id.clone(),
            agent_id: execution.execution_agent_id.clone(),
            kind: AgentUtilityKind::ProjectEnvironmentSetup,
            input: AgentUtilityInput::ProjectEnvironmentSetup(
                ProjectEnvironmentSetupUtilityInput {
                    project_id: execution.project_id.clone(),
                    workspace_id: execution.workspace_id.clone(),
                    target_worker_id: execution.target_worker_id.clone(),
                    target_platform: execution.target_platform.clone(),
                    definition: execution.definition.clone(),
                    validation_commands: execution.validation_commands.clone(),
                },
            ),
        };
        let (_agent, provider_run) = match assert_agent_utility_can_run(
            self,
            &execution.execution_session_id,
            &execution.execution_agent_id,
            &AgentUtilityKind::ProjectEnvironmentSetup,
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                store.mark_failed(
                    &execution.operation_id,
                    attempt,
                    "worker_provider_context_unavailable",
                    "the prepared worker provider context is unavailable",
                );
                return;
            }
        };
        if provider_run.owner_user_id() != execution.owner_user_id.as_str() {
            store.mark_failed(
                &execution.operation_id,
                attempt,
                "worker_provider_context_unauthorized",
                "the prepared worker provider context belongs to a different user",
            );
            return;
        }
        let archive_config = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .history
            .archive;
        let utility_result = run_agent_utility_on_provider_run(
            self,
            archive_config,
            utility_request,
            provider_run.clone(),
        )
        .await;
        if store.is_cancelled(&execution.operation_id, attempt) {
            return;
        }
        let definition = match utility_result {
            Ok(result) => match result.output {
                AgentUtilityOutput::ProjectEnvironmentSetup { definition } => definition,
                _ => {
                    store.mark_failed(
                        &execution.operation_id,
                        attempt,
                        "utility_output_invalid",
                        "utility agent returned an unexpected setup result",
                    );
                    return;
                }
            },
            Err(_) => {
                store.mark_failed(
                    &execution.operation_id,
                    attempt,
                    "utility_failed",
                    "utility agent could not prepare the target environment",
                );
                return;
            }
        };
        if definition.target_platform != execution.target_platform
            || definition.validation_commands.is_empty()
            || execution
                .validation_commands
                .iter()
                .any(|command| !definition.validation_commands.contains(command))
        {
            store.mark_failed(
                &execution.operation_id,
                attempt,
                "definition_invalid",
                "utility agent returned an incomplete or mismatched environment definition",
            );
            return;
        }
        if !store.update(&execution.operation_id, attempt, |entry| {
            entry.execution.definition = Some(definition.clone());
            entry.status.definition_digest = Some(definition.digest());
            entry.status.progress_percent = 55;
            entry.status.message =
                Some("target definition recorded; running kernel validation".to_string());
        }) {
            return;
        }
        let validation = match self
            .validate_definition_on_worker(&execution, attempt, &definition, &provider_run)
            .await
        {
            Ok(validation) => validation,
            Err(_) => {
                store.mark_failed(
                    &execution.operation_id,
                    attempt,
                    "worker_validation_unavailable",
                    "kernel could not execute target validation commands",
                );
                return;
            }
        };
        if store.is_cancelled(&execution.operation_id, attempt) {
            return;
        }
        let validation_passed = validation.worker_id == execution.target_worker_id
            && validation.platform == execution.target_platform
            && validation.commands.len() == definition.validation_commands.len()
            && validation.passed()
            && validation
                .commands
                .iter()
                .zip(definition.validation_commands.iter())
                .all(|(result, command)| result.command_digest == command_digest(command));
        let _ = store.update(&execution.operation_id, attempt, |entry| {
            entry.status.validation = Some(validation.clone());
            entry.status.progress_percent = 85;
            entry.status.message = Some(if validation_passed {
                "target commands passed kernel validation".to_string()
            } else {
                "target validation reported one or more failures".to_string()
            });
        });
        if !validation_passed {
            store.mark_failed(
                &execution.operation_id,
                attempt,
                "validation_failed",
                "one or more target validation commands failed",
            );
            return;
        }
        if execution.persist_project_definition
            && self
                .update_project_environment_definition(
                    &execution.project_id,
                    definition,
                    &execution.owner_user_id,
                )
                .is_err()
        {
            store.mark_failed(
                &execution.operation_id,
                attempt,
                "definition_persist_failed",
                "kernel could not persist the target environment definition",
            );
            return;
        }
        let _ = store.update(&execution.operation_id, attempt, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Ready;
            entry.status.progress_percent = 100;
            entry.status.retryable = false;
            entry.status.message =
                Some("project environment is ready on the validated worker".to_string());
        });
    }

    async fn validate_definition_on_worker(
        &self,
        execution: &SetupExecution,
        attempt: u32,
        definition: &ProjectEnvironmentDefinition,
        provider_run: &RuntimeProviderRun,
    ) -> Result<ProjectEnvironmentValidation, DaemonError> {
        // Validation is a worker-kernel operation, not a home-kernel shell
        // capability. The confirmed receipt is the worker isolation boundary;
        // the pinned provider run supplies the same prepared cwd/PATH and the
        // child receives no kernel, vault, provider credential, or control env.
        let config = self.owned.config_projection.snapshot();
        ensure_worker_validation_boundary(&config)?;
        let worker_id = config.host_machine_id;
        let platform = actual_worker_platform();
        if worker_id != execution.target_worker_id || platform != execution.target_platform {
            return Err(setup_error(
                "worker identity or platform changed during setup",
            ));
        }
        if provider_run.session_id() != execution.execution_session_id.as_str()
            || provider_run.agent_instance_id() != Some(execution.execution_agent_id.as_str())
            || provider_run.owner_user_id() != execution.owner_user_id.as_str()
            || provider_run.state() != crate::provider::ProviderRunState::Running
        {
            return Err(setup_error(
                "prepared provider context does not match the setup target",
            ));
        }
        let current_provider_run = self.owned.provider_store.get_run(provider_run.id())?;
        if current_provider_run.session_id() != provider_run.session_id()
            || current_provider_run.agent_instance_id() != provider_run.agent_instance_id()
            || current_provider_run.owner_user_id() != provider_run.owner_user_id()
            || current_provider_run.state() != crate::provider::ProviderRunState::Running
            || current_provider_run.pty_env() != provider_run.pty_env()
            || current_provider_run.pty_env_remove() != provider_run.pty_env_remove()
            || current_provider_run.working_directory() != provider_run.working_directory()
        {
            return Err(setup_error(
                "prepared provider context changed before worker validation",
            ));
        }
        let commands = definition.validation_commands.clone();
        let operation_id = execution.operation_id.clone();
        let workspace_root = canonical_worker_workspace(
            &execution.workspace_id,
            std::env::var_os("CHARIOX_HOME").as_deref(),
        )?;
        let provider_working_directory = provider_run
            .working_directory()
            .cloned()
            .ok_or_else(|| setup_error("prepared provider context has no working directory"))?;
        let provider_working_directory = canonical_worker_workspace(
            &provider_working_directory,
            std::env::var_os("CHARIOX_HOME").as_deref(),
        )?;
        if provider_working_directory != workspace_root {
            return Err(setup_error(
                "prepared provider context uses a different worker worktree",
            ));
        }
        let environment = worker_validation_environment(provider_run);
        let cancellation = self.owned.project_environment_setups.clone();
        let guard = cancellation
            .begin_execution(&operation_id, attempt)
            .ok_or_else(|| setup_error("setup attempt is no longer executing"))?;
        let validation = tokio::task::spawn_blocking(move || {
            // The blocking command owns this guard even if its async waiter exits.
            let _guard = guard;
            let started = Instant::now();
            let mut results = Vec::with_capacity(commands.len());
            for command in commands {
                if cancellation.is_cancelled(&operation_id, attempt)
                    || started.elapsed() >= VALIDATION_TOTAL_TIMEOUT
                {
                    break;
                }
                let result =
                    run_worker_validation_command(&command, &workspace_root, &environment, || {
                        cancellation.is_cancelled(&operation_id, attempt)
                    });
                match result {
                    Ok((exit_code, stdout_bytes, stderr_bytes)) => {
                        results.push(ProjectEnvironmentCommandResult {
                            command_digest: command_digest(&command),
                            exit_code,
                            stdout_bytes,
                            stderr_bytes,
                        });
                    }
                    Err(_) => {
                        results.push(ProjectEnvironmentCommandResult {
                            command_digest: command_digest(&command),
                            exit_code: -1,
                            stdout_bytes: 0,
                            stderr_bytes: 0,
                        });
                        break;
                    }
                }
            }
            ProjectEnvironmentValidation {
                worker_id,
                platform,
                commands: results,
            }
        })
        .await
        .map_err(|error| setup_error(&format!("worker validation task failed: {error}")))?;
        Ok(validation)
    }
}

#[cfg(test)]
#[path = "project_environment_setup_cancellation_tests.rs"]
mod cancellation_tests;

#[cfg(test)]
#[path = "project_environment_setup_lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        ProjectEnvironmentDefinitionOrigin, ProjectEnvironmentDefinitionSource,
        ProjectEnvironmentSetupStep, ProjectEnvironmentSetupStepKind,
    };
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

    pub(super) fn execution() -> SetupExecution {
        SetupExecution {
            owner_user_id: "user-1".to_string(),
            operation_id: "setup-1".to_string(),
            project_id: "project-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            execution_session_id: "session-1".to_string(),
            execution_agent_id: "agent-1".to_string(),
            workspace_id: "/tmp/project".to_string(),
            target_worker_id: "machine-1".to_string(),
            target_platform: "linux-x86_64".to_string(),
            definition: Some(ProjectEnvironmentDefinition {
                schema_version: 1,
                origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
                source: ProjectEnvironmentDefinitionSource::Commands,
                target_platform: "linux-x86_64".to_string(),
                source_path: None,
                setup_steps: vec![ProjectEnvironmentSetupStep {
                    kind: ProjectEnvironmentSetupStepKind::Compiler,
                    command: "rustup toolchain install stable".to_string(),
                }],
                validation_commands: vec!["cargo check --workspace --locked".to_string()],
            }),
            validation_commands: Vec::new(),
            persist_project_definition: true,
            remote_leased_agent_id: None,
        }
    }

    #[test]
    fn setup_store_is_idempotent_and_rejects_fingerprint_reuse() {
        let store = ProjectEnvironmentSetupStore::default();
        let (first, should_spawn) = store.begin(execution()).expect("setup should start");
        assert!(should_spawn);
        assert_eq!(first.phase, ProjectEnvironmentSetupPhase::Requested);

        let (replayed, should_spawn) = store.begin(execution()).expect("replay should be safe");
        assert!(!should_spawn);
        assert_eq!(replayed, first);

        let mut changed = execution();
        changed.agent_id = "agent-2".to_string();
        let error = store
            .begin(changed)
            .expect_err("same id with changed input must fail");
        assert!(error.to_string().contains("different setup request"));
    }

    #[test]
    fn worker_setup_recovery_preserves_attempt_and_rejects_stale_replay() {
        let store = ProjectEnvironmentSetupStore::default();
        let (started, should_spawn) = store
            .begin_at_attempt(execution(), 2)
            .expect("worker recovery should start at the home attempt");
        assert!(should_spawn);
        assert_eq!(started.attempt, 2);

        let (replayed, should_spawn) = store
            .begin_at_attempt(execution(), 2)
            .expect("same worker recovery request should be idempotent");
        assert!(!should_spawn);
        assert_eq!(replayed, started);

        let stale = store
            .begin_at_attempt(execution(), 1)
            .expect_err("stale worker recovery must not reopen attempt 1");
        assert!(stale
            .to_string()
            .contains("attempt does not match the existing worker operation"));

        let zero = store
            .begin_at_attempt(execution(), 0)
            .expect_err("worker recovery must reject an invalid attempt");
        assert!(zero.to_string().contains("attempt must be positive"));
    }

    #[test]
    fn stale_remote_setup_recovery_is_narrow_and_rejects_stale_ready() {
        let stale_binding = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "unauthorized".to_string(),
            message: "authenticated home kernel does not own the leased resource".to_string(),
            retryable: false,
        };
        assert!(is_stale_remote_setup_binding_error(&stale_binding));

        let business_unauthorized = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "unauthorized".to_string(),
            message: "provider rejected the business request".to_string(),
            retryable: false,
        };
        assert!(!is_stale_remote_setup_binding_error(&business_unauthorized));
        let retryable_ownership_error = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "unauthorized".to_string(),
            message: "authenticated home kernel does not own the leased resource".to_string(),
            retryable: true,
        };
        assert!(!is_stale_remote_setup_binding_error(
            &retryable_ownership_error
        ));

        let mut current = ProjectEnvironmentSetupStatus {
            operation_id: "setup-1".to_string(),
            project_id: "project-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            worker_id: "machine-1".to_string(),
            platform: "linux-x86_64".to_string(),
            phase: ProjectEnvironmentSetupPhase::Requested,
            attempt: 2,
            progress_percent: 0,
            definition_digest: None,
            validation: None,
            message: None,
            failure_code: None,
            failure_message: None,
            retryable: true,
            created_at_ms: 1,
            updated_at_ms: 2,
        };
        let mut stale = current.clone();
        stale.attempt = 1;
        stale.phase = ProjectEnvironmentSetupPhase::Cancelled;
        stale.retryable = true;
        assert!(is_replayable_stale_remote_setup_status(&current, &stale));

        stale.phase = ProjectEnvironmentSetupPhase::Ready;
        stale.retryable = false;
        assert!(!is_replayable_stale_remote_setup_status(&current, &stale));
        let expected = execution();
        let stale_ready = validate_remote_setup_status(&expected, 2, &stale, None)
            .expect_err("stale Ready must fail the unchanged identity/attempt validator");
        assert!(stale_ready.to_string().contains("identity or attempt"));

        current.agent_id = "different-agent".to_string();
        assert!(!is_replayable_stale_remote_setup_status(&current, &stale));
    }

    #[test]
    fn permanent_worker_rejection_settles_recovery_but_transport_uncertainty_does_not() {
        let store = ProjectEnvironmentSetupStore::default();
        let mut execution = execution();
        execution.remote_leased_agent_id = Some("leased-agent-old".to_string());
        store
            .begin(execution.clone())
            .expect("remote setup should start");
        let rejection = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: crate::transport::relay_peer::PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE.to_string(),
            message: "worker rejected setup".to_string(),
            retryable: false,
        };
        let code = remote_setup_recovery_permanent_rejection_code(&rejection)
            .expect("worker rejection should be classified as terminal");
        KernelRuntimeState::settle_remote_setup_recovery_rejection_in_store(
            &store, &execution, 1, &rejection,
        );
        let (_, failed) = store
            .get_entry(&execution.operation_id, &execution.owner_user_id)
            .expect("settled setup should remain inspectable");
        assert_eq!(failed.phase, ProjectEnvironmentSetupPhase::Failed);
        assert_eq!(failed.failure_code.as_deref(), Some(code));
        assert!(!failed.retryable);

        let metadata_failure = DaemonError::RelayTransport {
            operation: "read relay metadata response",
            code: crate::transport::relay_peer::PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE.to_string(),
            message: "metadata request failed".to_string(),
            retryable: false,
        };
        let uncertain_store = ProjectEnvironmentSetupStore::default();
        uncertain_store
            .begin(execution.clone())
            .expect("uncertain setup should start");
        KernelRuntimeState::settle_remote_setup_recovery_rejection_in_store(
            &uncertain_store,
            &execution,
            1,
            &metadata_failure,
        );
        let (_, still_active) = uncertain_store
            .get_entry(&execution.operation_id, &execution.owner_user_id)
            .expect("uncertain setup should remain inspectable");
        assert_eq!(still_active.phase, ProjectEnvironmentSetupPhase::Requested);
        assert!(still_active.retryable);
        assert_eq!(
            remote_setup_recovery_permanent_rejection_code(&metadata_failure),
            None,
            "metadata failure is transport uncertainty, not worker authority"
        );
        let ownership_failure = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "unauthorized".to_string(),
            message: "authenticated home kernel does not own the leased resource".to_string(),
            retryable: false,
        };
        assert_eq!(
            remote_setup_recovery_permanent_rejection_code(&ownership_failure),
            None,
            "stale binding must remain eligible for binding recovery"
        );
        let business_unauthorized = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "unauthorized".to_string(),
            message: "provider rejected the business request".to_string(),
            retryable: false,
        };
        assert_eq!(
            remote_setup_recovery_permanent_rejection_code(&business_unauthorized),
            None,
            "business authorization failure is not worker setup authority"
        );
    }

    #[test]
    fn remote_setup_binding_rebind_is_compare_and_swap_and_updates_worker_target() {
        let store = ProjectEnvironmentSetupStore::default();
        let mut execution = execution();
        execution.remote_leased_agent_id = Some("leased-agent-old".to_string());
        store.begin(execution).expect("remote setup should start");

        let changed = store
            .rebind_remote_leased_agent(
                "setup-1",
                1,
                "leased-agent-other",
                "leased-agent-new".to_string(),
                0,
            )
            .expect_err("home binding refresh must not overwrite a changed binding");
        assert!(changed.to_string().contains("binding changed"));

        store
            .cancel("setup-1", "session-1", "user-1")
            .expect("worker setup cancellation should be recorded");
        let target = crate::app::LeasedProjectEnvironmentSetupTarget {
            owner_user_id: "user-1".to_string(),
            home_session_id: "session-1".to_string(),
            home_agent_id: "agent-1".to_string(),
            backing_session_id: "worker-session-2".to_string(),
            backing_agent_id: "worker-agent-2".to_string(),
            workspace_id: "/worker/project".to_string(),
        };
        let (worker_status, _) = store
            .rebind_remote_worker_target(
                "setup-1",
                "leased-agent-new",
                &target,
                "machine-1",
                "linux-x86_64",
            )
            .expect("retryable worker state should accept the authenticated fresh binding");
        assert_eq!(worker_status.phase, ProjectEnvironmentSetupPhase::Cancelled);
        let (restarted_execution, attempt, restarted_status) = store
            .retry("setup-1", "session-1", "user-1")
            .expect("rebound worker setup should retry");
        assert_eq!(attempt, 2);
        assert_eq!(
            restarted_status.phase,
            ProjectEnvironmentSetupPhase::Requested
        );
        assert_eq!(
            restarted_execution.remote_leased_agent_id.as_deref(),
            Some("leased-agent-new")
        );
        assert_eq!(restarted_execution.execution_session_id, "worker-session-2");
        assert_eq!(restarted_execution.execution_agent_id, "worker-agent-2");
        assert_eq!(restarted_execution.workspace_id, "/worker/project");
    }

    #[test]
    fn remote_recovery_reopens_only_after_a_fresh_bound_observation() {
        let store = ProjectEnvironmentSetupStore::default();
        let mut execution = execution();
        execution.remote_leased_agent_id = Some("leased-agent-1".to_string());
        store.begin(execution).expect("remote setup should start");

        let first_observation = store.begin_remote_observation();
        let overlapping_observation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery("setup-1", 1, "leased-agent-1", first_observation,),
            RemoteSetupRecoveryDecision::Dispatch
        );
        store.mark_remote_recovery_observed(
            "setup-1",
            1,
            "leased-agent-1",
            Some(first_observation),
        );
        assert_eq!(
            store.begin_remote_recovery("setup-1", 1, "leased-agent-1", overlapping_observation,),
            RemoteSetupRecoveryDecision::Acknowledged,
            "an overlapping not-found must not clear an acknowledged replay"
        );

        let fresh_observation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery("setup-1", 1, "leased-agent-1", fresh_observation,),
            RemoteSetupRecoveryDecision::Dispatch,
            "a fresh same-binding not-found may reopen recovery"
        );
        store.mark_remote_recovery_unknown("setup-1", 1, "leased-agent-1", first_observation);
        assert_eq!(
            store.begin_remote_recovery("setup-1", 1, "leased-agent-1", fresh_observation,),
            RemoteSetupRecoveryDecision::InFlight,
            "a stale response must not alter the fresh recovery reservation"
        );
        assert_eq!(
            store.begin_remote_recovery("setup-1", 1, "leased-agent-2", fresh_observation,),
            RemoteSetupRecoveryDecision::Stale,
            "an observation from an old binding must be fenced"
        );
    }

    #[test]
    fn unscoped_start_cannot_acknowledge_a_recovery_reservation() {
        let store = ProjectEnvironmentSetupStore::default();
        let mut execution = execution();
        execution.remote_leased_agent_id = Some("leased-agent-1".to_string());
        store.begin(execution).expect("remote setup should start");

        let observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery("setup-1", 1, "leased-agent-1", observation_generation,),
            RemoteSetupRecoveryDecision::Dispatch
        );
        store.mark_remote_recovery_observed("setup-1", 1, "leased-agent-1", None);

        let next_observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery(
                "setup-1",
                1,
                "leased-agent-1",
                next_observation_generation,
            ),
            RemoteSetupRecoveryDecision::InFlight,
            "an unscoped Start response must not acknowledge a newer Get reservation"
        );
    }

    #[test]
    fn dropped_recovery_observation_becomes_unknown_for_the_same_fence() {
        let store = ProjectEnvironmentSetupStore::default();
        let mut execution = execution();
        execution.remote_leased_agent_id = Some("leased-agent-1".to_string());
        store.begin(execution).expect("remote setup should start");

        let observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery("setup-1", 1, "leased-agent-1", observation_generation,),
            RemoteSetupRecoveryDecision::Dispatch
        );
        {
            let _reservation = RemoteSetupRecoveryReservationGuard::new(
                &store,
                "setup-1",
                1,
                "leased-agent-1",
                observation_generation,
            );
        }

        let next_observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery(
                "setup-1",
                1,
                "leased-agent-1",
                next_observation_generation,
            ),
            RemoteSetupRecoveryDecision::Unknown,
            "cancelling replay observation must fence duplicate dispatch until explicit recovery"
        );
    }

    #[test]
    fn dropped_recovery_observation_after_binding_refresh_fences_the_new_binding() {
        let store = ProjectEnvironmentSetupStore::default();
        let mut execution = execution();
        execution.remote_leased_agent_id = Some("leased-agent-old".to_string());
        store.begin(execution).expect("remote setup should start");

        let observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery("setup-1", 1, "leased-agent-old", observation_generation,),
            RemoteSetupRecoveryDecision::Dispatch
        );
        let mut reservation = RemoteSetupRecoveryReservationGuard::new(
            &store,
            "setup-1",
            1,
            "leased-agent-old",
            observation_generation,
        );
        store
            .rebind_remote_leased_agent(
                "setup-1",
                1,
                "leased-agent-old",
                "leased-agent-new".to_string(),
                observation_generation,
            )
            .expect("the active recovery should rebind to the fresh lease");
        reservation.rebind("leased-agent-new");
        drop(reservation);

        let next_observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery(
                "setup-1",
                1,
                "leased-agent-old",
                next_observation_generation,
            ),
            RemoteSetupRecoveryDecision::Stale,
            "the old lease must never regain the dropped recovery reservation"
        );
        assert_eq!(
            store.begin_remote_recovery(
                "setup-1",
                1,
                "leased-agent-new",
                next_observation_generation,
            ),
            RemoteSetupRecoveryDecision::Unknown,
            "a dropped refresh observer must fence duplicate dispatch on the fresh lease"
        );
    }

    #[test]
    fn late_recovery_rejection_cannot_clear_a_newer_reservation() {
        let store = ProjectEnvironmentSetupStore::default();
        let mut execution = execution();
        execution.remote_leased_agent_id = Some("leased-agent-old".to_string());
        store.begin(execution).expect("remote setup should start");

        let old_observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery(
                "setup-1",
                1,
                "leased-agent-old",
                old_observation_generation,
            ),
            RemoteSetupRecoveryDecision::Dispatch
        );

        // Model the old recovery continuation returning a permanent rejection
        // after a newer public Retry has already taken ownership of the entry.
        store.mark_failed(
            "setup-1",
            1,
            "worker_failed",
            "the retained worker rejected attempt one",
        );
        let (_, new_attempt, _) = store
            .retry("setup-1", "session-1", "user-1")
            .expect("the newer public Retry should advance the operation");
        assert_eq!(new_attempt, 2);
        store
            .rebind_remote_leased_agent(
                "setup-1",
                new_attempt,
                "leased-agent-old",
                "leased-agent-new".to_string(),
                old_observation_generation,
            )
            .expect("the newer attempt should use the fresh lease");
        let new_observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery(
                "setup-1",
                new_attempt,
                "leased-agent-new",
                new_observation_generation,
            ),
            RemoteSetupRecoveryDecision::Dispatch
        );

        // This is the old continuation's identity-scoped cleanup. It must
        // not erase the newer attempt-two reservation.
        assert!(!store.clear_remote_recovery_if_matches(
            "setup-1",
            1,
            "leased-agent-old",
            old_observation_generation,
        ));
        let later_observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery(
                "setup-1",
                new_attempt,
                "leased-agent-new",
                later_observation_generation,
            ),
            RemoteSetupRecoveryDecision::InFlight,
            "a late old rejection must not release the newer recovery reservation"
        );
    }

    #[test]
    fn late_recovery_rebind_cannot_mutate_a_newer_observation_reservation() {
        let store = ProjectEnvironmentSetupStore::default();
        let mut execution = execution();
        execution.remote_leased_agent_id = Some("leased-agent-old".to_string());
        store.begin(execution).expect("remote setup should start");

        let old_observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery(
                "setup-1",
                1,
                "leased-agent-old",
                old_observation_generation,
            ),
            RemoteSetupRecoveryDecision::Dispatch
        );
        assert!(store.clear_remote_recovery_if_matches(
            "setup-1",
            1,
            "leased-agent-old",
            old_observation_generation,
        ));

        let new_observation_generation = store.begin_remote_observation();
        assert_eq!(
            store.begin_remote_recovery(
                "setup-1",
                1,
                "leased-agent-old",
                new_observation_generation,
            ),
            RemoteSetupRecoveryDecision::Dispatch
        );

        // The old refresh continuation arrives after the newer observation
        // has reserved the same attempt. Its binding update must be fenced by
        // the old generation as well as attempt and binding.
        store
            .rebind_remote_leased_agent(
                "setup-1",
                1,
                "leased-agent-old",
                "leased-agent-new".to_string(),
                old_observation_generation,
            )
            .expect("the old refresh still updates the home binding CAS");
        assert!(
            !store.remote_recovery_dispatch_allowed(
                "setup-1",
                1,
                "leased-agent-new",
                new_observation_generation,
            ),
            "an old refresh must not make a newer reservation dispatchable under its replacement binding"
        );
    }

    #[test]
    fn non_retryable_worker_rejection_does_not_suggest_reconnect() {
        let store = ProjectEnvironmentSetupStore::default();
        store.begin(execution()).expect("setup should start");
        store.mark_failed_non_retryable(
            "setup-1",
            1,
            "worker_rejected",
            "the worker rejected the setup request",
        );

        let error = store
            .retry("setup-1", "session-1", "user-1")
            .expect_err("a non-retryable worker rejection must remain terminal");
        let message = error.to_string();
        assert!(message.contains("rejected by the worker"), "{message}");
        assert!(message.contains("not retryable"), "{message}");
        assert!(!message.contains("reconnect"), "{message}");
    }

    #[test]
    fn cancellation_is_terminal_until_explicit_retry() {
        let store = ProjectEnvironmentSetupStore::default();
        store.begin(execution()).expect("setup should start");
        let cancelled = store
            .cancel("setup-1", "session-1", "user-1")
            .expect("cancel should be accepted");
        assert_eq!(cancelled.phase, ProjectEnvironmentSetupPhase::Cancelled);
        assert!(store.is_cancelled("setup-1", 1));

        let (_execution, attempt, retried) = store
            .retry("setup-1", "session-1", "user-1")
            .expect("cancelled setup should retry");
        assert_eq!(attempt, 2);
        assert_eq!(retried.phase, ProjectEnvironmentSetupPhase::Requested);
        assert!(!store.is_cancelled("setup-1", 2));
    }

    #[test]
    fn late_failure_cannot_reopen_cancelled_setup() {
        let store = ProjectEnvironmentSetupStore::default();
        store.begin(execution()).expect("setup should start");
        store
            .cancel("setup-1", "session-1", "user-1")
            .expect("cancel should be accepted");

        store.mark_failed(
            "setup-1",
            1,
            "worker_validation_unavailable",
            "late worker failure",
        );

        let status = store
            .get("setup-1", "user-1")
            .expect("cancelled setup should remain observable");
        assert_eq!(status.phase, ProjectEnvironmentSetupPhase::Cancelled);
        assert_eq!(status.failure_code, None);
    }

    #[test]
    fn validation_requires_nonempty_measured_results() {
        let validation = ProjectEnvironmentValidation {
            worker_id: "machine-1".to_string(),
            platform: "linux-x86_64".to_string(),
            commands: Vec::new(),
        };
        assert!(!validation.passed());
    }

    #[test]
    fn home_kernel_cannot_enter_the_worker_validation_boundary() {
        let error = ensure_worker_validation_boundary(&DaemonConfig::for_tests())
            .expect_err("general home kernels must not execute project setup commands");
        assert!(error.to_string().contains("dedicated worker kernel"));
    }

    #[test]
    fn worker_validation_environment_removes_kernel_and_credential_bindings() {
        let mut provider_env = BTreeMap::new();
        provider_env.insert("PATH".to_string(), "/worker/toolchain/bin".to_string());
        provider_env.insert(
            "CHARIOX_HOME".to_string(),
            "/worker/kernel-home".to_string(),
        );
        provider_env.insert(
            "CHARIOX_MANAGED_VAULT_PATH".to_string(),
            "/worker/kernel-home/vault.json".to_string(),
        );
        provider_env.insert(
            "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE".to_string(),
            "/worker/kernel-home/auth-token".to_string(),
        );
        provider_env.insert("OPENAI_API_KEY".to_string(), "must-not-cross".to_string());
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default");
        let run = RuntimeProviderRun::new(
            "provider-run-1",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "worker-provider".to_string(),
                pty_target: None,
                pty_program: Some("/bin/sh".to_string()),
                pty_args: Vec::new(),
                pty_env: provider_env,
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );

        let environment = worker_validation_environment(&run);
        assert_eq!(
            environment.get("PATH").map(String::as_str),
            Some("/worker/toolchain/bin")
        );
        for name in [
            "CHARIOX_HOME",
            "CHARIOX_MANAGED_VAULT_PATH",
            "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
            "OPENAI_API_KEY",
        ] {
            assert!(
                !environment.contains_key(name),
                "{name} must not reach validation"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn validation_commands_cannot_use_protected_home_bindings_or_kernel_controls() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-protected-home-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let protected_home = root.join("kernel-home");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(protected_home.join("vault")).expect("protected home exists");
        std::fs::create_dir_all(&workspace).expect("worker workspace exists");
        std::fs::write(protected_home.join("vault/secret.json"), "must-not-be-read")
            .expect("protected vault exists");

        let mut provider_env = BTreeMap::new();
        provider_env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        provider_env.insert(
            "CHARIOX_HOME".to_string(),
            protected_home.display().to_string(),
        );
        provider_env.insert(
            "CHARIOX_MANAGED_VAULT_PATH".to_string(),
            protected_home
                .join("vault/secret.json")
                .display()
                .to_string(),
        );
        provider_env.insert(
            "CHARIOX_MANAGED_KERNEL_BINARY".to_string(),
            "/usr/local/bin/chariox-kernel".to_string(),
        );
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default");
        let run = RuntimeProviderRun::new(
            "provider-run-protected-home",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "worker-provider".to_string(),
                pty_target: None,
                pty_program: Some("/bin/sh".to_string()),
                pty_args: Vec::new(),
                pty_env: provider_env,
                pty_env_remove: Vec::new(),
                working_directory: Some(workspace.clone()),
                structured_endpoint: None,
            },
        );
        let environment = worker_validation_environment(&run);
        // The confirmed disposable-worker VM supplies filesystem separation
        // from the home kernel. This fixture covers the complementary local
        // contract: validation receives no path/control binding that points at
        // the protected home or a kernel replacement target.
        let command = r#"
            test -z "${CHARIOX_HOME-}" &&
            test -z "${CHARIOX_MANAGED_VAULT_PATH-}" &&
            test -z "${CHARIOX_MANAGED_KERNEL_BINARY-}" &&
            test ! -r "${CHARIOX_HOME:-/no-worker-home}/vault/secret.json" &&
            test ! -w "${CHARIOX_MANAGED_KERNEL_BINARY:-/no-worker-kernel}"
        "#;
        let (exit_code, _, _) =
            run_worker_validation_command(command, &workspace, &environment, || false)
                .expect("worker validation shell should execute");
        assert_eq!(
            exit_code, 0,
            "validation must not inherit protected home/vault or kernel replacement controls"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn worker_worktree_cannot_overlap_kernel_home() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-boundary-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let kernel_home = root.join("kernel-home");
        let worktree = kernel_home.join("workspace");
        std::fs::create_dir_all(&worktree).expect("worker worktree should exist");
        let error = canonical_worker_workspace(&worktree, Some(kernel_home.as_os_str()))
            .expect_err("kernel-owned home must not be a project worktree");
        assert!(error.to_string().contains("overlaps kernel-owned home"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn worker_local_tool_resolves_through_the_prepared_provider_path() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-tool-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("worker tool directory should exist");
        let tool = bin.join("worker-local-tool");
        std::fs::write(&tool, "#!/bin/sh\nexit 0\n").expect("worker tool should exist");
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755))
            .expect("worker tool should be executable");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).expect("worker workspace should exist");
        let environment = BTreeMap::from([(String::from("PATH"), bin.display().to_string())]);
        let (exit_code, _, _) = run_worker_validation_command(
            "command -v worker-local-tool",
            &workspace,
            &environment,
            || false,
        )
        .expect("worker validation shell should execute");
        assert_eq!(exit_code, 0, "installed worker-local tool must resolve");
        let _ = std::fs::remove_dir_all(root);
    }
}
