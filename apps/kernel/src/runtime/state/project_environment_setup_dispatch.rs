use super::*;

use chariox_relay::protocol::ClientTarget;

const REMOTE_SETUP_RESPONSE_TIMEOUT: Duration = Duration::from_secs(240);

pub(super) async fn start_remote_setup(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let (relay_config, target) = remote_relay_context(state, execution).await?;
    send_setup_request(
        state,
        relay_config,
        target,
        RelayPeerRequest::StartLeasedProjectEnvironmentSetup {
            leased_agent_id: execution
                .remote_leased_agent_id
                .clone()
                .ok_or_else(|| setup_error("remote setup is missing its leased agent binding"))?,
            operation_id: execution.operation_id.clone(),
            project_id: execution.project_id.clone(),
            home_session_id: execution.session_id.clone(),
            home_agent_id: execution.agent_id.clone(),
            workspace_id: execution.workspace_id.clone(),
            target_worker_id: execution.target_worker_id.clone(),
            target_platform: execution.target_platform.clone(),
            definition: execution.definition.clone(),
            validation_commands: execution.validation_commands.clone(),
        },
        RelaySetupResponseKind::Started,
    )
    .await
}

pub(super) async fn get_remote_setup_status(
    state: &KernelRuntimeState,
    execution: &SetupExecution,
) -> Result<RelayProjectEnvironmentSetupStatus, DaemonError> {
    let (relay_config, target) = remote_relay_context(state, execution).await?;
    send_setup_request(
        state,
        relay_config,
        target,
        RelayPeerRequest::GetLeasedProjectEnvironmentSetupStatus {
            leased_agent_id: remote_leased_agent_id(execution)?,
            operation_id: execution.operation_id.clone(),
            home_session_id: execution.session_id.clone(),
            home_agent_id: execution.agent_id.clone(),
        },
        RelaySetupResponseKind::Status,
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
    let response = match state.connected_relay_state_for_config(&relay_config).await {
        Some(relay_state) => {
            crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                &relay_config,
                &relay_state,
                target,
                request,
                REMOTE_SETUP_RESPONSE_TIMEOUT,
            )
            .await
        }
        None => {
            crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                &relay_config,
                target,
                request,
                REMOTE_SETUP_RESPONSE_TIMEOUT,
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
