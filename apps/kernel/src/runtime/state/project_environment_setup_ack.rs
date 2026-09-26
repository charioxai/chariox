use super::super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_retry_transport;
use super::project_environment_setup_storage::RemoteSetupReconcileSource;
use super::*;

const REMOTE_SETUP_RECONCILE_INTERVAL: Duration = Duration::from_millis(250);

impl KernelRuntimeState {
    pub(crate) async fn acknowledge_leased_project_environment_setup_definition(
        &self,
        target: crate::app::LeasedProjectEnvironmentSetupTarget,
        leased_agent_id: &str,
        operation_id: &str,
        attempt: u32,
        project_id: &str,
        home_session_id: &str,
        home_agent_id: &str,
        definition_digest: &str,
    ) -> Result<crate::transport::relay_peer::RelayProjectEnvironmentSetupDefinitionAck, DaemonError>
    {
        let (execution, status) = self
            .owned
            .project_environment_setups
            .get_entry(operation_id, &target.owner_user_id)?;
        let config = self.owned.config_projection.snapshot();
        ensure_worker_setup_status_target(&target, &status, &config)?;
        if execution.project_id != project_id
            || execution.session_id != home_session_id
            || execution.agent_id != home_agent_id
        {
            return Err(setup_error(
                "home definition acknowledgment does not match the worker setup binding",
            ));
        }
        let acknowledgment = self
            .owned
            .project_environment_setups
            .acknowledge_home_definition_persistence(
                operation_id,
                attempt,
                leased_agent_id,
                home_session_id,
                home_agent_id,
                project_id,
                definition_digest,
            )?;
        Ok(
            crate::transport::relay_peer::RelayProjectEnvironmentSetupDefinitionAck {
                operation_id: operation_id.to_string(),
                attempt: acknowledgment.attempt,
                project_id: acknowledgment.project_id,
                definition_digest: acknowledgment.definition_digest,
            },
        )
    }
}

pub(super) fn requires_home_persistence_ack(
    execution: &SetupExecution,
    definition: &ProjectEnvironmentDefinition,
) -> bool {
    !execution.persist_project_definition
        && execution.remote_leased_agent_id.is_some()
        && definition.origin == ProjectEnvironmentDefinitionOrigin::UtilityGenerated
}

pub(super) fn remote_setup_error_should_retry_transport(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::RelayTransport {
            code,
            retryable: true,
            ..
        } if code == "target_not_connected"
    ) || remote_prompt_error_should_retry_transport(error)
}

pub(super) async fn retry_remote_setup_ordered(
    state: &KernelRuntimeState,
    operation_id: &str,
    session_id: &str,
    caller_user_id: &str,
) -> Result<(SetupExecution, u32, ProjectEnvironmentSetupStatus), DaemonError> {
    let store = &state.owned.project_environment_setups;
    let gate = store.ordering_gate(operation_id);
    let _guard = gate.lock().await;
    store.retry(operation_id, session_id, caller_user_id)
}

pub(super) async fn resolve_setup_execution(
    state: &KernelRuntimeState,
    prepared: PreparedProjectEnvironmentSetup,
) -> Result<SetupExecution, DaemonError> {
    let PreparedProjectEnvironmentSetup {
        mut execution,
        requested_target_platform,
    } = prepared;
    let platform = if execution.remote_leased_agent_id.is_some() {
        match resolve_remote_setup_target_platform(state, &execution).await {
            Ok(platform) => platform,
            Err(error)
                if requested_target_platform.is_some()
                    && can_defer_remote_setup_target_resolution(&error) =>
            {
                // Retain the public operation even when the worker is
                // temporarily absent. The authenticated Start request checks
                // this provisional platform against the actual worker before
                // it admits the setup attempt.
                requested_target_platform
                    .clone()
                    .expect("a supplied target platform was checked above")
            }
            Err(error) => return Err(error),
        }
    } else {
        actual_worker_platform()
    };
    if platform.trim().is_empty() {
        return Err(setup_error(
            "selected setup worker returned an empty platform",
        ));
    }
    if requested_target_platform
        .as_deref()
        .is_some_and(|requested| requested != platform)
    {
        return Err(setup_error(
            "requested platform does not match the selected setup worker",
        ));
    }
    execution.target_platform = platform;
    execution.definition = validate_setup_definition(
        execution.definition,
        &execution.target_platform,
        &execution.validation_commands,
    )?;
    Ok(execution)
}

fn can_defer_remote_setup_target_resolution(error: &DaemonError) -> bool {
    remote_setup_error_should_retry_transport(error)
}

pub(super) fn spawn_remote_setup(
    state: &KernelRuntimeState,
    execution: SetupExecution,
    attempt: u32,
    retry: bool,
) {
    let state = state.clone();
    tokio::spawn(async move {
        if !retry {
            match start_remote_setup(&state, &execution, attempt).await {
                Ok(setup) => {
                    if let Err(error) =
                        reconcile_initial_start_response(&state, &execution, setup).await
                    {
                        crate::logging::warn_with_fields(
                            "project.environment_setup",
                            "remote setup status was rejected",
                            serde_json::json!({
                                "operation_id": execution.operation_id,
                                "error": error.to_string(),
                            }),
                        );
                        if !remote_setup_error_should_retry_transport(&error) {
                            fail_remote_setup(&state, &execution, attempt, &error).await;
                            return;
                        }
                    }
                }
                Err(error) if remote_setup_error_should_retry_transport(&error) => {}
                Err(error) => {
                    settle_initial_dispatch_error(&state, &execution, attempt, &error);
                    return;
                }
            }
        }

        loop {
            let Ok((current_execution, status, cancel_requested)) = state
                .owned
                .project_environment_setups
                .get_entry_with_cancellation(&execution.operation_id, &execution.owner_user_id)
            else {
                return;
            };
            if status.attempt != attempt {
                return;
            }
            if matches!(
                status.phase,
                ProjectEnvironmentSetupPhase::Ready
                    | ProjectEnvironmentSetupPhase::Failed
                    | ProjectEnvironmentSetupPhase::Cancelled
            ) {
                return;
            }
            if cancel_requested {
                match cancel_remote_and_reconcile(&state, &current_execution, attempt).await {
                    Ok(status)
                        if matches!(
                            status.phase,
                            ProjectEnvironmentSetupPhase::Ready
                                | ProjectEnvironmentSetupPhase::Failed
                                | ProjectEnvironmentSetupPhase::Cancelled
                        ) =>
                    {
                        return;
                    }
                    Ok(_) => {}
                    Err(error) if remote_setup_error_should_retry_transport(&error) => {}
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "project.environment_setup",
                            "remote setup cancellation recovery is pending",
                            serde_json::json!({
                                "operation_id": execution.operation_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
            } else {
                let result = state
                    .execute_project_environment_setup_request(
                        LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                            GetProjectEnvironmentSetupStatusRequest {
                                operation_id: execution.operation_id.clone(),
                            },
                        ),
                        &execution.owner_user_id,
                    )
                    .await;
                match result {
                    Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus { status }) => {
                        if matches!(
                            status.phase,
                            ProjectEnvironmentSetupPhase::Ready
                                | ProjectEnvironmentSetupPhase::Failed
                                | ProjectEnvironmentSetupPhase::Cancelled
                        ) {
                            return;
                        }
                    }
                    Err(error) if remote_setup_error_should_retry_transport(&error) => {}
                    Err(error) => {
                        let current_status = state
                            .owned
                            .project_environment_setups
                            .get_entry_with_cancellation(
                                &execution.operation_id,
                                &execution.owner_user_id,
                            )
                            .ok()
                            .map(|(_, status, _)| status);
                        let Some(current_status) = current_status else {
                            return;
                        };
                        if !matches!(
                            current_status.phase,
                            ProjectEnvironmentSetupPhase::Ready
                                | ProjectEnvironmentSetupPhase::Failed
                                | ProjectEnvironmentSetupPhase::Cancelled
                        ) {
                            fail_remote_setup(&state, &execution, attempt, &error).await;
                            return;
                        }
                    }
                    Ok(_) => return,
                }
            }
            tokio::time::sleep(REMOTE_SETUP_RECONCILE_INTERVAL).await;
        }
    });
}

async fn fail_remote_setup(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    attempt: u32,
    error: &DaemonError,
) {
    let current_execution = mark_remote_setup_ack_rejection(
        &state.owned.project_environment_setups,
        execution,
        attempt,
    )
    .await;
    crate::logging::warn_with_fields(
        "project.environment_setup",
        "leased setup acknowledgment failed closed",
        serde_json::json!({
            "operation_id": execution.operation_id,
            "error": error.to_string(),
        }),
    );
    if let Some(current_execution) = current_execution {
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            cancel_remote_setup(state, &current_execution),
        )
        .await;
    }
}

async fn mark_remote_setup_ack_rejection(
    store: &ProjectEnvironmentSetupStore,
    execution: &SetupExecution,
    attempt: u32,
) -> Option<SetupExecution> {
    let gate = store.ordering_gate(&execution.operation_id);
    let _guard = gate.lock().await;
    let (current_execution, status, cancel_requested) = store
        .get_entry_with_cancellation(&execution.operation_id, &execution.owner_user_id)
        .ok()?;
    if current_execution.operation_id != execution.operation_id
        || current_execution.owner_user_id != execution.owner_user_id
        || status.attempt != attempt
        || cancel_requested
        || matches!(
            status.phase,
            ProjectEnvironmentSetupPhase::Ready
                | ProjectEnvironmentSetupPhase::Failed
                | ProjectEnvironmentSetupPhase::Cancelled
        )
    {
        return None;
    }
    let failed = store.mark_failed_non_retryable_if_active(
        &execution.operation_id,
        attempt,
        "worker_setup_ack_rejected",
        "the leased worker rejected the home definition persistence acknowledgment",
    );
    drop(_guard);
    failed.then_some(current_execution)
}

pub(super) async fn reconcile_remote(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    setup: RelayProjectEnvironmentSetupStatus,
    observation_generation: Option<u64>,
) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
    let gate = state
        .owned
        .project_environment_setups
        .ordering_gate(&execution.operation_id);
    let _guard = gate.lock().await;
    reconcile_remote_under_gate_from_source(
        state,
        execution,
        setup,
        observation_generation,
        RemoteSetupReconcileSource::StatusObservation,
    )
    .await
}

async fn reconcile_initial_start_response(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    setup: RelayProjectEnvironmentSetupStatus,
) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
    let gate = state
        .owned
        .project_environment_setups
        .ordering_gate(&execution.operation_id);
    let _guard = gate.lock().await;
    reconcile_remote_under_gate_from_source(
        state,
        execution,
        setup,
        None,
        RemoteSetupReconcileSource::InitialStartReply,
    )
    .await
}

/// Reconcile while the caller already owns the operation's ordering gate.
pub(super) async fn reconcile_remote_under_gate(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    setup: RelayProjectEnvironmentSetupStatus,
    observation_generation: Option<u64>,
) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
    reconcile_remote_under_gate_from_source(
        state,
        execution,
        setup,
        observation_generation,
        RemoteSetupReconcileSource::StatusObservation,
    )
    .await
}

async fn reconcile_remote_under_gate_from_source(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    setup: RelayProjectEnvironmentSetupStatus,
    observation_generation: Option<u64>,
    source: RemoteSetupReconcileSource,
) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
    let definition = setup.definition.clone();
    let status = setup.status.clone();
    let response_phase = status.phase;
    let committed = state
        .owned
        .project_environment_setups
        .reconcile_remote_with(
            &execution.operation_id,
            execution,
            status,
            definition.clone(),
            observation_generation,
            source,
            |_, status, definition| {
                let Some(definition) = definition else {
                    if status.phase == ProjectEnvironmentSetupPhase::Ready {
                        return Err(setup_error(
                            "worker cannot report setup ready without a definition",
                        ));
                    }
                    return Ok(());
                };
                if status.phase != ProjectEnvironmentSetupPhase::Cancelled
                    && (status.phase == ProjectEnvironmentSetupPhase::Ready
                        || (execution.persist_project_definition
                            && definition.origin
                                == ProjectEnvironmentDefinitionOrigin::UtilityGenerated))
                {
                    state.persist_project_environment_definition_if_changed(
                        &execution.project_id,
                        definition,
                        &execution.owner_user_id,
                    )?;
                }
                Ok(())
            },
        )?;

    let delayed_initial_start_snapshot = source == RemoteSetupReconcileSource::InitialStartReply
        && response_phase == ProjectEnvironmentSetupPhase::Requested
        && matches!(
            committed.phase,
            ProjectEnvironmentSetupPhase::Preparing | ProjectEnvironmentSetupPhase::Validating
        );
    if !delayed_initial_start_snapshot
        && !matches!(
            committed.phase,
            ProjectEnvironmentSetupPhase::Ready
                | ProjectEnvironmentSetupPhase::Failed
                | ProjectEnvironmentSetupPhase::Cancelled
        )
    {
        if let Some(definition) = definition.as_ref().filter(|definition| {
            execution.persist_project_definition
                && definition.origin == ProjectEnvironmentDefinitionOrigin::UtilityGenerated
        }) {
            acknowledge_home_persistence(state, execution, &committed, definition).await?;
        }
    }
    Ok(committed)
}

async fn acknowledge_home_persistence(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    status: &ProjectEnvironmentSetupStatus,
    definition: &ProjectEnvironmentDefinition,
) -> Result<(), DaemonError> {
    let digest = definition.digest();
    let (relay_config, target) = remote_relay_context(state, execution).await?;
    let request = RelayPeerRequest::AcknowledgeLeasedProjectEnvironmentSetupDefinition {
        leased_agent_id: execution
            .remote_leased_agent_id
            .clone()
            .ok_or_else(|| setup_error("remote setup is missing its leased-agent binding"))?,
        operation_id: execution.operation_id.clone(),
        attempt: status.attempt,
        project_id: execution.project_id.clone(),
        home_session_id: execution.session_id.clone(),
        home_agent_id: execution.agent_id.clone(),
        definition_digest: digest.clone(),
    };
    let response = match state.connected_relay_state_for_config(&relay_config).await {
        Some(relay_state) => {
            crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                &relay_config,
                &relay_state,
                target,
                request,
                remote_setup_observation_budget(&relay_config),
            )
            .await
        }
        None => {
            crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                &relay_config,
                target,
                request,
                remote_setup_observation_budget(&relay_config),
            )
            .await
        }
    }?;
    match response {
        RelayPeerResponse::LeasedProjectEnvironmentSetupDefinitionAcknowledged {
            operation_id,
            attempt,
            project_id,
            definition_digest,
        } if operation_id == execution.operation_id
            && attempt == status.attempt
            && project_id == execution.project_id
            && definition_digest == digest =>
        {
            Ok(())
        }
        other => Err(setup_error(&format!(
            "remote setup returned an unexpected definition acknowledgment: {other:?}"
        ))),
    }
}

pub(super) async fn cancel_remote_and_reconcile(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    attempt: u32,
) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
    let gate = state
        .owned
        .project_environment_setups
        .ordering_gate(&execution.operation_id);
    let guard = gate.lock().await;
    // Recovery may have replaced the leased-agent binding while Cancel waited.
    let (current_execution, status, cancel_requested) = state
        .owned
        .project_environment_setups
        .get_entry_with_cancellation(&execution.operation_id, &execution.owner_user_id)?;
    if current_execution.operation_id != execution.operation_id
        || current_execution.owner_user_id != execution.owner_user_id
        || current_execution.session_id != execution.session_id
    {
        return Err(setup_error(
            "setup operation identity changed while cancellation was pending",
        ));
    }
    if status.attempt != attempt
        || !cancel_requested
        || matches!(
            status.phase,
            ProjectEnvironmentSetupPhase::Ready
                | ProjectEnvironmentSetupPhase::Failed
                | ProjectEnvironmentSetupPhase::Cancelled
        )
    {
        return Ok(status);
    }
    match cancel_remote_setup(state, &current_execution).await {
        Ok(setup) => reconcile_remote_under_gate(state, &current_execution, setup, None).await,
        Err(error) if is_missing_remote_setup_operation(&error) => {
            drop(guard);
            state
                .owned
                .project_environment_setups
                .settle_cancel_without_worker(
                    &current_execution.operation_id,
                    &current_execution.session_id,
                    &current_execution.owner_user_id,
                )
                .await
        }
        Err(error) => Err(error),
    }
}

fn settle_initial_dispatch_error(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    attempt: u32,
    error: &DaemonError,
) {
    if let DaemonError::RelayTransport {
        code,
        retryable: false,
        ..
    } = error
    {
        state
            .owned
            .project_environment_setups
            .mark_failed_non_retryable(
                &execution.operation_id,
                attempt,
                code,
                "the remote worker rejected project environment setup",
            );
    } else {
        state.owned.project_environment_setups.mark_failed(
            &execution.operation_id,
            attempt,
            "worker_dispatch_failed",
            "the remote worker could not be reached for environment setup",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_target_resolution_can_retain_a_supplied_platform() {
        assert!(can_defer_remote_setup_target_resolution(
            &DaemonError::RelayTransport {
                operation: "send relay peer request",
                code: "target_not_connected".to_string(),
                message: "target daemon is not connected to relay".to_string(),
                retryable: true,
            }
        ));
        assert!(!can_defer_remote_setup_target_resolution(
            &DaemonError::RelayTransport {
                operation: "send relay peer request",
                code: "target_not_connected".to_string(),
                message: "target daemon is not connected to relay".to_string(),
                retryable: false,
            }
        ));
        assert!(!can_defer_remote_setup_target_resolution(
            &DaemonError::RelayTransport {
                operation: "send relay peer request",
                code: "target_mismatch".to_string(),
                message: "target daemon identity changed".to_string(),
                retryable: true,
            }
        ));
    }

    fn worker_execution() -> SetupExecution {
        let mut execution = super::super::tests::execution();
        execution.persist_project_definition = false;
        execution.remote_leased_agent_id = Some("lease-1".to_string());
        let definition = execution
            .definition
            .as_mut()
            .expect("test setup has a definition");
        definition.origin = ProjectEnvironmentDefinitionOrigin::UtilityGenerated;
        execution
    }

    #[tokio::test]
    async fn home_ack_is_bound_to_attempt_lease_and_definition_digest() {
        let execution = worker_execution();
        let definition = execution.definition.as_ref().unwrap().clone();
        let digest = definition.digest();
        let store = ProjectEnvironmentSetupStore::default();
        store.begin(execution.clone()).unwrap();
        assert!(store.update(&execution.operation_id, 1, |entry| {
            entry.execution.definition = Some(definition.clone());
            entry.status.definition_digest = Some(digest.clone());
            entry.status.phase = ProjectEnvironmentSetupPhase::Preparing;
        }));

        assert!(store
            .acknowledge_home_definition_persistence(
                &execution.operation_id,
                2,
                "lease-1",
                &execution.session_id,
                &execution.agent_id,
                &execution.project_id,
                &digest,
            )
            .is_err());
        assert!(store
            .acknowledge_home_definition_persistence(
                &execution.operation_id,
                1,
                "stale-lease",
                &execution.session_id,
                &execution.agent_id,
                &execution.project_id,
                &digest,
            )
            .is_err());
        assert!(store
            .acknowledge_home_definition_persistence(
                &execution.operation_id,
                1,
                "lease-1",
                &execution.session_id,
                &execution.agent_id,
                &execution.project_id,
                "sha256:wrong",
            )
            .is_err());

        store
            .acknowledge_home_definition_persistence(
                &execution.operation_id,
                1,
                "lease-1",
                &execution.session_id,
                &execution.agent_id,
                &execution.project_id,
                &digest,
            )
            .unwrap();
        store
            .wait_for_home_definition_persistence_ack(
                &execution.operation_id,
                1,
                "lease-1",
                &execution.project_id,
                &digest,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancellation_and_retry_invalidate_previous_attempt_acknowledgment() {
        let execution = worker_execution();
        let definition = execution.definition.as_ref().unwrap().clone();
        let digest = definition.digest();
        let store = ProjectEnvironmentSetupStore::default();
        store.begin(execution.clone()).unwrap();
        assert!(store.update(&execution.operation_id, 1, |entry| {
            entry.execution.definition = Some(definition.clone());
            entry.status.definition_digest = Some(digest.clone());
            entry.status.phase = ProjectEnvironmentSetupPhase::Preparing;
        }));
        store
            .request_cancel(
                &execution.operation_id,
                &execution.session_id,
                &execution.owner_user_id,
                true,
            )
            .unwrap();
        assert!(store
            .acknowledge_home_definition_persistence(
                &execution.operation_id,
                1,
                "lease-1",
                &execution.session_id,
                &execution.agent_id,
                &execution.project_id,
                &digest,
            )
            .is_err());
        store
            .settle_cancel_without_worker(
                &execution.operation_id,
                &execution.session_id,
                &execution.owner_user_id,
            )
            .await
            .unwrap();
        let (_, attempt, status) = store
            .retry(
                &execution.operation_id,
                &execution.session_id,
                &execution.owner_user_id,
            )
            .unwrap();
        assert_eq!(attempt, 2);
        assert_eq!(status.phase, ProjectEnvironmentSetupPhase::Requested);
        assert!(store
            .acknowledge_home_definition_persistence(
                &execution.operation_id,
                1,
                "lease-1",
                &execution.session_id,
                &execution.agent_id,
                &execution.project_id,
                &digest,
            )
            .is_err());
    }

    #[tokio::test]
    async fn late_setup_ack_rejection_preserves_cancellation_new_attempt_and_ready() {
        let execution = worker_execution();
        let store = ProjectEnvironmentSetupStore::default();
        store.begin(execution.clone()).unwrap();
        assert!(store.update(&execution.operation_id, 1, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Preparing;
        }));

        store
            .request_cancel_ordered(
                &execution.operation_id,
                &execution.session_id,
                &execution.owner_user_id,
                true,
            )
            .await
            .unwrap();
        assert!(!store.mark_failed_non_retryable_if_active(
            &execution.operation_id,
            1,
            "worker_setup_ack_rejected",
            "the leased worker rejected the home definition persistence acknowledgment",
        ));
        assert!(
            mark_remote_setup_ack_rejection(&store, &execution, 1)
                .await
                .is_none(),
            "a worker rejection after cancellation must not replace the pending cancellation"
        );
        let (_, status, cancel_requested) = store
            .get_entry_with_cancellation(&execution.operation_id, &execution.owner_user_id)
            .unwrap();
        assert!(cancel_requested);
        assert_eq!(status.phase, ProjectEnvironmentSetupPhase::Preparing);
        assert_eq!(status.failure_code, None);

        store
            .settle_cancel_without_worker(
                &execution.operation_id,
                &execution.session_id,
                &execution.owner_user_id,
            )
            .await
            .unwrap();
        let (_, attempt, status) = store
            .retry(
                &execution.operation_id,
                &execution.session_id,
                &execution.owner_user_id,
            )
            .unwrap();
        assert_eq!(attempt, 2);
        assert_eq!(status.phase, ProjectEnvironmentSetupPhase::Requested);
        assert!(store.update(&execution.operation_id, attempt, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Preparing;
        }));
        assert!(!store.mark_failed_non_retryable_if_active(
            &execution.operation_id,
            1,
            "worker_setup_ack_rejected",
            "the leased worker rejected the home definition persistence acknowledgment",
        ));
        assert!(
            mark_remote_setup_ack_rejection(&store, &execution, 1)
                .await
                .is_none(),
            "a late attempt-one rejection must not fail attempt two"
        );
        assert!(store.update(&execution.operation_id, attempt, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Ready;
        }));
        assert!(!store.mark_failed_non_retryable_if_active(
            &execution.operation_id,
            attempt,
            "worker_setup_ack_rejected",
            "the leased worker rejected the home definition persistence acknowledgment",
        ));
        assert!(
            mark_remote_setup_ack_rejection(&store, &execution, attempt)
                .await
                .is_none(),
            "a late rejection must not replace Ready"
        );
        let (_, status, cancel_requested) = store
            .get_entry_with_cancellation(&execution.operation_id, &execution.owner_user_id)
            .unwrap();
        assert!(!cancel_requested);
        assert_eq!(status.attempt, 2);
        assert_eq!(status.phase, ProjectEnvironmentSetupPhase::Ready);
        assert_eq!(status.failure_code, None);
    }

    #[tokio::test]
    async fn active_setup_ack_rejection_still_fails_closed() {
        let execution = worker_execution();
        let store = ProjectEnvironmentSetupStore::default();
        store.begin(execution.clone()).unwrap();
        assert!(store.update(&execution.operation_id, 1, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Preparing;
        }));

        assert!(
            mark_remote_setup_ack_rejection(&store, &execution, 1)
                .await
                .is_some(),
            "an invalid worker acknowledgment must fail the active attempt closed"
        );
        let (_, status, cancel_requested) = store
            .get_entry_with_cancellation(&execution.operation_id, &execution.owner_user_id)
            .unwrap();
        assert!(!cancel_requested);
        assert_eq!(status.phase, ProjectEnvironmentSetupPhase::Failed);
        assert_eq!(status.failure_code.as_deref(), Some("worker_setup_ack_rejected"));
        assert_eq!(
            status.failure_message.as_deref(),
            Some("the leased worker rejected the home definition persistence acknowledgment")
        );
        assert!(!status.retryable);
        assert!(!store.mark_failed_non_retryable_if_active(
            &execution.operation_id,
            1,
            "worker_setup_ack_rejected_late",
            "a later worker acknowledgment failure must not overwrite the first failure",
        ));
        assert!(
            mark_remote_setup_ack_rejection(&store, &execution, 1)
                .await
                .is_none(),
            "a later rejection must not overwrite the committed failure"
        );
    }

    #[test]
    fn delayed_initial_start_reply_cannot_regress_an_authenticated_status() {
        let mut execution = super::super::tests::execution();
        execution.remote_leased_agent_id = Some("lease-1".to_string());
        let definition = execution.definition.as_ref().unwrap().clone();
        let store = ProjectEnvironmentSetupStore::default();
        let (start_reply, _) = store.begin(execution.clone()).unwrap();
        assert_eq!(start_reply.phase, ProjectEnvironmentSetupPhase::Requested);
        assert!(store.update(&execution.operation_id, 1, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Preparing;
            entry.status.progress_percent = 40;
            entry.status.message = Some("authenticated status advanced the setup".to_string());
        }));
        let (_, observed, _) = store
            .get_entry_with_cancellation(&execution.operation_id, &execution.owner_user_id)
            .unwrap();

        let mut committed = false;
        let reconciled = store
            .reconcile_remote_with(
                &execution.operation_id,
                &execution,
                start_reply.clone(),
                Some(definition.clone()),
                None,
                RemoteSetupReconcileSource::InitialStartReply,
                |_, _, _| {
                    committed = true;
                    Ok(())
                },
            )
            .expect("a delayed same-attempt Start reply is a stale observation");
        assert_eq!(reconciled, observed);
        assert_eq!(reconciled.phase, ProjectEnvironmentSetupPhase::Preparing);
        assert!(!committed, "a stale Start reply cannot run the commit callback");

        let mut wrong_attempt = start_reply.clone();
        wrong_attempt.attempt = 2;
        assert!(store
            .reconcile_remote_with(
                &execution.operation_id,
                &execution,
                wrong_attempt,
                Some(definition.clone()),
                None,
                RemoteSetupReconcileSource::InitialStartReply,
                |_, _, _| Ok(()),
            )
            .is_err());

        let mut wrong_binding = start_reply.clone();
        wrong_binding.worker_id = "other-worker".to_string();
        assert!(store
            .reconcile_remote_with(
                &execution.operation_id,
                &execution,
                wrong_binding,
                Some(definition.clone()),
                None,
                RemoteSetupReconcileSource::InitialStartReply,
                |_, _, _| Ok(()),
            )
            .is_err());

        let mut wrong_digest = start_reply.clone();
        wrong_digest.definition_digest = Some("sha256:wrong".to_string());
        assert!(store
            .reconcile_remote_with(
                &execution.operation_id,
                &execution,
                wrong_digest,
                Some(definition.clone()),
                None,
                RemoteSetupReconcileSource::InitialStartReply,
                |_, _, _| Ok(()),
            )
            .is_err());

        let mut changed_definition = definition.clone();
        changed_definition.setup_steps[0].command = "mutated setup command".to_string();
        let mut changed_definition_reply = start_reply.clone();
        changed_definition_reply.definition_digest = Some(changed_definition.digest());
        assert!(store
            .reconcile_remote_with(
                &execution.operation_id,
                &execution,
                changed_definition_reply,
                Some(changed_definition),
                None,
                RemoteSetupReconcileSource::InitialStartReply,
                |_, _, _| Ok(()),
            )
            .is_err());

        assert!(store.update(&execution.operation_id, 1, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Validating;
        }));
        let mut other_backward_phase = start_reply;
        other_backward_phase.phase = ProjectEnvironmentSetupPhase::Preparing;
        assert!(store
            .reconcile_remote_with(
                &execution.operation_id,
                &execution,
                other_backward_phase,
                Some(definition),
                None,
                RemoteSetupReconcileSource::InitialStartReply,
                |_, _, _| Ok(()),
            )
            .is_err());
    }

    #[tokio::test]
    async fn remote_project_commit_callback_is_fenced_by_cancel_and_new_attempt() {
        let mut execution = super::super::tests::execution();
        execution.remote_leased_agent_id = Some("lease-1".to_string());
        let mut definition = execution.definition.as_ref().unwrap().clone();
        definition.origin = ProjectEnvironmentDefinitionOrigin::UtilityGenerated;
        let digest = definition.digest();
        let store = ProjectEnvironmentSetupStore::default();
        let (status, _) = store.begin(execution.clone()).unwrap();
        let mut candidate = status.clone();
        candidate.phase = ProjectEnvironmentSetupPhase::Preparing;
        candidate.definition_digest = Some(digest.clone());

        store
            .request_cancel(
                &execution.operation_id,
                &execution.session_id,
                &execution.owner_user_id,
                true,
            )
            .unwrap();
        let mut committed = false;
        let observed = store
            .reconcile_remote_with(
                &execution.operation_id,
                &execution,
                candidate.clone(),
                Some(definition.clone()),
                None,
                RemoteSetupReconcileSource::StatusObservation,
                |_, _, _| {
                    committed = true;
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(observed.phase, ProjectEnvironmentSetupPhase::Requested);
        assert!(!committed, "cancelled attempt cannot write its recipe");

        store
            .settle_cancel_without_worker(
                &execution.operation_id,
                &execution.session_id,
                &execution.owner_user_id,
            )
            .await
            .unwrap();
        store
            .retry(
                &execution.operation_id,
                &execution.session_id,
                &execution.owner_user_id,
            )
            .unwrap();
        candidate.phase = ProjectEnvironmentSetupPhase::Ready;
        candidate.attempt = 1;
        let mut stale_commit = false;
        assert!(store
            .reconcile_remote_with(
                &execution.operation_id,
                &execution,
                candidate,
                Some(definition),
                None,
                RemoteSetupReconcileSource::StatusObservation,
                |_, _, _| {
                    stale_commit = true;
                    Ok(())
                },
            )
            .is_err());
        assert!(!stale_commit, "old attempt cannot write after retry");
    }
}
