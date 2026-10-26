//! MP-08 / MP-10 / MP-11: keep setup-status observation off cancellation poll frames.
use super::*;

impl KernelRuntimeState {
    pub(super) async fn get_project_environment_setup_status_request(
        &self,
        request: GetProjectEnvironmentSetupStatusRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (execution, status, cancel_requested) = self
            .owned
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
            let setup =
                get_remote_setup_status_with_deadline(self, &execution, observation_deadline).await;
            let status = match setup {
                Ok(setup)
                    if !cancel_requested
                        && matches!(
                            status.phase,
                            ProjectEnvironmentSetupPhase::Requested
                                | ProjectEnvironmentSetupPhase::Preparing
                                | ProjectEnvironmentSetupPhase::Validating
                        )
                        && is_replayable_stale_remote_setup_status(&status, &setup.status) =>
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
                            .get_entry_with_cancellation(&execution.operation_id, caller_user_id)?;
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
                                    )
                                    .await
                                {
                                    Ok(status) => {
                                        recovery_reservation.disarm();
                                        status
                                    }
                                    Err(error) => return Err(error),
                                }
                            }
                            Err(error)
                                if project_environment_setup_ack::remote_setup_error_should_retry_transport(&error) =>
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
                Ok(setup) => {
                    self.reconcile_remote_project_environment_setup_with_observation(
                        &execution,
                        setup,
                        Some(observation_generation),
                    )
                    .await?
                }
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
                            .get_entry_with_cancellation(&execution.operation_id, caller_user_id)?;
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
                        return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus { status });
                    }
                    let redispatch_gate = self
                        .owned
                        .project_environment_setups
                        .ordering_gate(&execution.operation_id);
                    let _redispatch_guard = redispatch_gate.lock().await;
                    let (_, _, cancel_requested) = self
                        .owned
                        .project_environment_setups
                        .get_entry_with_cancellation(&execution.operation_id, caller_user_id)?;
                    if cancel_requested {
                        return Err(error);
                    }
                    match self.owned.project_environment_setups.begin_remote_recovery(
                        &execution.operation_id,
                        status.attempt,
                        binding_id,
                        observation_generation,
                    ) {
                        RemoteSetupRecoveryDecision::Cancelled => {
                            return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                status,
                            });
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
                            return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                status,
                            });
                        }
                        RemoteSetupRecoveryDecision::Stale => {
                            return Err(remote_setup_recovery_transport_error(
                                "the setup attempt changed while status was being observed",
                            ));
                        }
                        RemoteSetupRecoveryDecision::Dispatch => {}
                    }
                    // This lock-protected fence linearizes recovery
                    // dispatch against Cancel/Retry. Keep the gate
                    // through reconciliation and its persistence ack.
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
                            .get_entry_with_cancellation(&execution.operation_id, caller_user_id)?;
                        if current_cancel_requested
                            || matches!(
                                current_status.phase,
                                ProjectEnvironmentSetupPhase::Ready
                                    | ProjectEnvironmentSetupPhase::Failed
                                    | ProjectEnvironmentSetupPhase::Cancelled
                            )
                        {
                            return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                status: current_status,
                            });
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
                                match project_environment_setup_ack::reconcile_remote_under_gate(
                                    self,
                                    &execution,
                                    setup,
                                    Some(observation_generation),
                                )
                                .await
                                {
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
                                if project_environment_setup_ack::remote_setup_error_should_retry_transport(&error) =>
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
                Err(error)
                    if project_environment_setup_ack::remote_setup_error_should_retry_transport(
                        &error,
                    ) =>
                {
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
}
