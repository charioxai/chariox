use super::*;

use chariox_relay::protocol::ClientTarget;

pub(super) const DEFAULT_REMOTE_SETUP_OBSERVATION_BUDGET: Duration = Duration::from_secs(5);
const REMOTE_SETUP_RESPONSE_TIMEOUT: Duration = Duration::from_secs(240);

pub(super) fn remote_setup_observation_budget(
    relay_config: &crate::config::DaemonConfig,
) -> Duration {
    Duration::from_millis(relay_config.relay_request_timeout_ms)
        .min(DEFAULT_REMOTE_SETUP_OBSERVATION_BUDGET)
}

fn setup_response_timeout(
    relay_config: &crate::config::DaemonConfig,
    response_kind: RelaySetupResponseKind,
) -> Duration {
    // Status is one bounded observation. Control operations keep their longer
    // setup budget; a status timeout must not trigger or settle setup.
    match response_kind {
        RelaySetupResponseKind::Status => remote_setup_observation_budget(relay_config),
        RelaySetupResponseKind::Started
        | RelaySetupResponseKind::Cancelled
        | RelaySetupResponseKind::Retried => REMOTE_SETUP_RESPONSE_TIMEOUT,
    }
}

pub(super) fn observation_timeout(deadline: Instant) -> Result<Duration, DaemonError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(observation_timeout_error())
    } else {
        Ok(remaining)
    }
}

fn observation_timeout_error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "read relay peer response",
        message: format!(
            "timed out waiting for relay peer response before the remote setup observation deadline of {}ms",
            DEFAULT_REMOTE_SETUP_OBSERVATION_BUDGET.as_millis()
        ),
    }
}

pub(super) fn remote_setup_recovery_transport_error(reason: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "read relay peer response",
        message: format!("remote setup recovery is temporarily unavailable: {reason}"),
    }
}

pub(super) async fn start_remote_setup(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    attempt: u32,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    start_remote_setup_with_timeout(state, execution, attempt, REMOTE_SETUP_RESPONSE_TIMEOUT).await
}

pub(super) async fn start_remote_setup_with_timeout(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    attempt: u32,
    response_timeout: Duration,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let (relay_config, target) = remote_relay_context(state, execution).await?;
    send_setup_request_with_timeout(
        state,
        relay_config,
        target,
        start_remote_setup_request(execution, attempt)?,
        RelaySetupResponseKind::Started,
        response_timeout,
    )
    .await
}

pub(super) async fn start_remote_setup_with_deadline(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    attempt: u32,
    deadline: Instant,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let (relay_config, target) =
        remote_relay_context_by_deadline(state, execution, deadline).await?;
    send_setup_request_with_timeout(
        state,
        relay_config,
        target,
        start_remote_setup_request(execution, attempt)?,
        RelaySetupResponseKind::Started,
        observation_timeout(deadline)?,
    )
    .await
}

pub(super) async fn get_remote_setup_status(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let (relay_config, target) = remote_relay_context(state, execution).await?;
    send_setup_request_with_timeout(
        state,
        relay_config.clone(),
        target,
        get_remote_setup_status_request(execution)?,
        RelaySetupResponseKind::Status,
        setup_response_timeout(&relay_config, RelaySetupResponseKind::Status),
    )
    .await
}

pub(super) async fn get_remote_setup_status_with_deadline(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    deadline: Instant,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let (relay_config, target) =
        remote_relay_context_by_deadline(state, execution, deadline).await?;
    send_setup_request_with_timeout(
        state,
        relay_config,
        target,
        get_remote_setup_status_request(execution)?,
        RelaySetupResponseKind::Status,
        observation_timeout(deadline)?,
    )
    .await
}

pub(super) async fn cancel_remote_setup(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let (relay_config, target) = remote_relay_context(state, execution).await?;
    send_setup_request(
        state,
        relay_config,
        target,
        RelayPeerRequest::CancelLeasedProjectEnvironmentSetup {
            leased_agent_id: remote_leased_agent_id(execution)?,
            operation_id: execution.operation_id.clone(),
            home_session_id: execution.session_id.clone(),
            home_agent_id: execution.agent_id.clone(),
        },
        RelaySetupResponseKind::Cancelled,
    )
    .await
}

pub(super) async fn retry_remote_setup(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let (relay_config, target) = remote_relay_context(state, execution).await?;
    send_setup_request(
        state,
        relay_config,
        target,
        RelayPeerRequest::RetryLeasedProjectEnvironmentSetup {
            leased_agent_id: remote_leased_agent_id(execution)?,
            operation_id: execution.operation_id.clone(),
            home_session_id: execution.session_id.clone(),
            home_agent_id: execution.agent_id.clone(),
        },
        RelaySetupResponseKind::Retried,
    )
    .await
}

pub(super) async fn refresh_remote_setup_binding_and_retry(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    attempt: u32,
) -> Result<(SetupExecution, RelayProjectEnvironmentSetupStatus), DaemonError> {
    let stale_leased_agent_id = remote_leased_agent_id(execution)?;
    let current_agent = state.owned.agent_store.get_agent(&execution.agent_id)?;
    let current_binding = current_agent
        .remote_execution()
        .ok_or_else(|| setup_error("selected agent lost its remote worker binding"))?;
    if current_binding.leased_agent_id != stale_leased_agent_id {
        return Err(setup_error(
            "remote setup binding changed before its stale lease could be refreshed",
        ));
    }
    let agent_id = execution.agent_id.clone();
    let rebound_agent = state
        .with_app_side_effect_blocking(move |app| app.refresh_remote_agent_binding(&agent_id))
        .await?;
    let rebound_execution = rebound_agent
        .remote_execution()
        .ok_or_else(|| setup_error("agent lost its remote execution after binding refresh"))?;
    if rebound_execution.worker_machine_id != execution.target_worker_id
        || rebound_execution.worker_kernel_id.trim().is_empty()
        || rebound_execution.execution_lease_id.trim().is_empty()
        || rebound_execution.leased_agent_id.trim().is_empty()
        || !rebound_execution.relay_peer_protocol_compatible()
    {
        return Err(setup_error(
            "refreshed remote setup binding does not match the selected worker",
        ));
    }
    let rebound_execution = state
        .owned
        .project_environment_setups
        .rebind_remote_leased_agent(
            &execution.operation_id,
            attempt,
            &stale_leased_agent_id,
            rebound_execution.leased_agent_id.clone(),
        )?;
    let setup = retry_remote_setup(state, &rebound_execution).await?;
    Ok((rebound_execution, setup))
}

fn remote_leased_agent_id(execution: &SetupExecution) -> Result<String, DaemonError> {
    execution
        .remote_leased_agent_id
        .clone()
        .ok_or_else(|| setup_error("remote setup is missing its leased agent binding"))
}

async fn remote_relay_context(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
) -> Result<(crate::config::DaemonConfig, ClientTarget), DaemonError> {
    let leased_agent_id = remote_leased_agent_id(execution)?;
    let agent = state.owned.agent_store.get_agent(&execution.agent_id)?;
    let remote_execution = agent
        .remote_execution()
        .ok_or_else(|| setup_error("selected agent lost its remote worker binding"))?;
    if remote_execution.leased_agent_id != leased_agent_id
        || remote_execution.worker_machine_id != execution.target_worker_id
        || remote_execution.worker_kernel_id.trim().is_empty()
        || remote_execution.worker_machine_id.trim().is_empty()
        || remote_execution.execution_lease_id.trim().is_empty()
        || !remote_execution.relay_peer_protocol_compatible()
    {
        return Err(setup_error(
            "remote setup target no longer matches the selected worker binding",
        ));
    }
    let relay_config = state
        .with_app_side_effect(|app| app.relay_config_for_remote_execution(remote_execution))
        .await;
    Ok((
        relay_config,
        ClientTarget {
            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
            daemon_alias: None,
        },
    ))
}

async fn remote_relay_context_by_deadline(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
    deadline: Instant,
) -> Result<(crate::config::DaemonConfig, ClientTarget), DaemonError> {
    let remaining = observation_timeout(deadline)?;
    match tokio::time::timeout(remaining, remote_relay_context(state, execution)).await {
        Ok(result) => result,
        Err(_) => Err(observation_timeout_error()),
    }
}

#[derive(Debug, Copy, Clone)]
enum RelaySetupResponseKind {
    Started,
    Status,
    Cancelled,
    Retried,
}

async fn send_setup_request(
    state: &KernelRuntimeState,
    relay_config: crate::config::DaemonConfig,
    target: ClientTarget,
    request: RelayPeerRequest,
    response_kind: RelaySetupResponseKind,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let response_timeout = setup_response_timeout(&relay_config, response_kind);
    send_setup_request_with_timeout(
        state,
        relay_config,
        target,
        request,
        response_kind,
        response_timeout,
    )
    .await
}

async fn send_setup_request_with_timeout(
    state: &KernelRuntimeState,
    relay_config: crate::config::DaemonConfig,
    target: ClientTarget,
    request: RelayPeerRequest,
    response_kind: RelaySetupResponseKind,
    response_timeout: Duration,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let response = match state.connected_relay_state_for_config(&relay_config).await {
        Some(relay_state) => {
            crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                &relay_config,
                &relay_state,
                target,
                request,
                response_timeout,
            )
            .await
        }
        None => {
            crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                &relay_config,
                target,
                request,
                response_timeout,
            )
            .await
        }
    }?;
    let expected = match response_kind {
        RelaySetupResponseKind::Started => "LeasedProjectEnvironmentSetupStarted",
        RelaySetupResponseKind::Status => "LeasedProjectEnvironmentSetupStatus",
        RelaySetupResponseKind::Cancelled => "LeasedProjectEnvironmentSetupCancelled",
        RelaySetupResponseKind::Retried => "LeasedProjectEnvironmentSetupRetried",
    };
    match response {
        RelayPeerResponse::LeasedProjectEnvironmentSetupStarted { setup }
            if matches!(response_kind, RelaySetupResponseKind::Started) =>
        {
            Ok(setup)
        }
        RelayPeerResponse::LeasedProjectEnvironmentSetupStatus { setup }
            if matches!(response_kind, RelaySetupResponseKind::Status) =>
        {
            Ok(setup)
        }
        RelayPeerResponse::LeasedProjectEnvironmentSetupCancelled { setup }
            if matches!(response_kind, RelaySetupResponseKind::Cancelled) =>
        {
            Ok(setup)
        }
        RelayPeerResponse::LeasedProjectEnvironmentSetupRetried { setup }
            if matches!(response_kind, RelaySetupResponseKind::Retried) =>
        {
            Ok(setup)
        }
        other => Err(setup_error(&format!(
            "remote setup returned an unexpected {expected} response: {other:?}"
        ))),
    }
}

fn start_remote_setup_request(
    execution: &SetupExecution,
    attempt: u32,
) -> Result<RelayPeerRequest, DaemonError> {
    Ok(RelayPeerRequest::StartLeasedProjectEnvironmentSetup {
        leased_agent_id: execution
            .remote_leased_agent_id
            .clone()
            .ok_or_else(|| setup_error("remote setup is missing its leased agent binding"))?,
        operation_id: execution.operation_id.clone(),
        attempt,
        project_id: execution.project_id.clone(),
        home_session_id: execution.session_id.clone(),
        home_agent_id: execution.agent_id.clone(),
        workspace_id: execution.workspace_id.clone(),
        target_worker_id: execution.target_worker_id.clone(),
        target_platform: execution.target_platform.clone(),
        definition: execution.definition.clone(),
        validation_commands: execution.validation_commands.clone(),
    })
}

fn get_remote_setup_status_request(
    execution: &SetupExecution,
) -> Result<RelayPeerRequest, DaemonError> {
    Ok(RelayPeerRequest::GetLeasedProjectEnvironmentSetupStatus {
        leased_agent_id: remote_leased_agent_id(execution)?,
        operation_id: execution.operation_id.clone(),
        home_session_id: execution.session_id.clone(),
        home_agent_id: execution.agent_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonConfig;
    use std::time::Duration;

    #[test]
    fn remote_setup_observation_budget_defaults_to_five_seconds() {
        let config = DaemonConfig::for_tests();
        assert_eq!(
            remote_setup_observation_budget(&config),
            Duration::from_secs(5)
        );

        let mut test_config = config;
        test_config.relay_request_timeout_ms = 100;
        assert_eq!(
            remote_setup_observation_budget(&test_config),
            Duration::from_millis(100)
        );
        assert_eq!(
            setup_response_timeout(&test_config, RelaySetupResponseKind::Started),
            Duration::from_secs(240)
        );
    }
}
