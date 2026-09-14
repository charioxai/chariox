use super::*;

pub(super) const OPERATION: &str = "project environment setup";
pub(super) const MAX_OPERATION_ID_CHARS: usize = 128;
pub(super) const MAX_VALIDATION_COMMANDS: usize = 32;
pub(super) const MAX_COMMAND_CHARS: usize = 8_192;

pub(super) fn ensure_entry_owner(
    entry: &SetupEntry,
    session_id: &str,
    caller_user_id: &str,
) -> Result<(), DaemonError> {
    if entry.execution.owner_user_id != caller_user_id || entry.execution.session_id != session_id {
        return Err(setup_error(
            "caller is not allowed to mutate this setup operation",
        ));
    }
    Ok(())
}

pub(super) fn validate_operation_id(operation_id: &str) -> Result<(), DaemonError> {
    if operation_id.trim().is_empty() || operation_id.chars().count() > MAX_OPERATION_ID_CHARS {
        return Err(setup_error("operation id must be non-empty and bounded"));
    }
    Ok(())
}

pub(super) fn validate_commands(commands: &[String]) -> Result<(), DaemonError> {
    if commands.len() > MAX_VALIDATION_COMMANDS {
        return Err(setup_error(
            "validation command count exceeds the bounded setup limit",
        ));
    }
    if commands
        .iter()
        .any(|command| command.trim().is_empty() || command.chars().count() > MAX_COMMAND_CHARS)
    {
        return Err(setup_error(
            "validation commands must be non-empty and bounded",
        ));
    }
    Ok(())
}

pub(super) fn validate_setup_definition(
    definition: Option<ProjectEnvironmentDefinition>,
    target_platform: &str,
    validation_commands: &[String],
) -> Result<Option<ProjectEnvironmentDefinition>, DaemonError> {
    let Some(definition) = definition else {
        return Ok(None);
    };
    definition
        .validate()
        .map_err(|message| setup_error(&message))?;
    if definition.target_platform != target_platform {
        return Err(setup_error(
            "environment definition targets a different platform",
        ));
    }
    if !validation_commands.is_empty() {
        return Err(setup_error(
            "additional validation commands are only allowed when no definition exists",
        ));
    }
    if definition.validation_commands.is_empty() {
        return Err(setup_error(
            "environment definition must include at least one validation command",
        ));
    }
    Ok(Some(definition))
}

pub(super) fn setup_fingerprint(execution: &SetupExecution) -> Result<String, DaemonError> {
    let encoded = serde_json::to_vec(execution)
        .map_err(|error| setup_error(&format!("could not fingerprint setup request: {error}")))?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

pub(super) fn command_digest(command: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(command.as_bytes()))
}

pub(super) fn setup_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: OPERATION,
        message: message.to_string(),
    }
}

pub(super) fn is_missing_remote_setup_operation(error: &DaemonError) -> bool {
    let DaemonError::RelayTransport {
        operation,
        code,
        retryable: false,
        ..
    } = error
    else {
        return false;
    };
    matches!(
        *operation,
        "read relay peer response" | "read temporary relay peer response"
    ) && code == crate::transport::relay_peer::PROJECT_ENVIRONMENT_SETUP_NOT_FOUND_CODE
}

pub(super) fn is_stale_remote_setup_binding_error(error: &DaemonError) -> bool {
    if super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding(
        error,
    ) {
        return true;
    }
    matches!(
        error,
        DaemonError::RelayTransport {
            operation,
            code,
            message,
            retryable: false,
        } if matches!(
            *operation,
            "read relay peer response" | "read temporary relay peer response"
        ) && code == "unauthorized"
            && message == "authenticated home kernel does not own the leased resource"
    )
}

pub(super) fn is_replayable_stale_remote_setup_status(
    current: &ProjectEnvironmentSetupStatus,
    remote: &ProjectEnvironmentSetupStatus,
) -> bool {
    current.operation_id == remote.operation_id
        && current.project_id == remote.project_id
        && current.session_id == remote.session_id
        && current.agent_id == remote.agent_id
        && current.worker_id == remote.worker_id
        && current.platform == remote.platform
        && remote.attempt.saturating_add(1) == current.attempt
        && remote.retryable
        && matches!(
            remote.phase,
            ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Cancelled
        )
}

pub(super) fn actual_worker_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub(super) fn ensure_worker_setup_status_target(
    target: &crate::app::LeasedProjectEnvironmentSetupTarget,
    status: &ProjectEnvironmentSetupStatus,
    config: &DaemonConfig,
) -> Result<(), DaemonError> {
    if config.kernel_runtime_role != KernelRuntimeRole::RemoteLeaseWorker {
        return Err(setup_error(
            "project environment setup status is only served by a lease worker",
        ));
    }
    if status.session_id != target.home_session_id || status.agent_id != target.home_agent_id {
        return Err(setup_error(
            "worker setup status does not match the leased home agent",
        ));
    }
    if status.worker_id != config.host_machine_id || status.platform != actual_worker_platform() {
        return Err(setup_error(
            "worker setup status does not match this worker identity or platform",
        ));
    }
    Ok(())
}

pub(super) fn validate_remote_setup_transition(
    current: ProjectEnvironmentSetupPhase,
    next: ProjectEnvironmentSetupPhase,
) -> Result<(), DaemonError> {
    if current == ProjectEnvironmentSetupPhase::Cancelled
        && next != ProjectEnvironmentSetupPhase::Cancelled
    {
        return Err(setup_error(
            "cancelled home setup cannot be reopened by a worker status",
        ));
    }
    if matches!(
        current,
        ProjectEnvironmentSetupPhase::Ready | ProjectEnvironmentSetupPhase::Failed
    ) && current != next
    {
        return Err(setup_error(
            "terminal setup status cannot be replaced without an explicit retry",
        ));
    }
    Ok(())
}

pub(super) fn validate_remote_setup_status(
    expected: &SetupExecution,
    current_attempt: u32,
    status: &ProjectEnvironmentSetupStatus,
    definition: Option<&ProjectEnvironmentDefinition>,
) -> Result<(), DaemonError> {
    if status.operation_id != expected.operation_id
        || status.project_id != expected.project_id
        || status.session_id != expected.session_id
        || status.agent_id != expected.agent_id
        || status.worker_id != expected.target_worker_id
        || status.platform != expected.target_platform
        || status.attempt != current_attempt
    {
        return Err(setup_error(
            "remote setup status identity or attempt does not match the home operation",
        ));
    }
    if let Some(definition) = definition {
        definition
            .validate()
            .map_err(|message| setup_error(&message))?;
        if definition.validation_commands.is_empty()
            || definition.target_platform != expected.target_platform
            || status.definition_digest.as_deref() != Some(definition.digest().as_str())
        {
            return Err(setup_error(
                "remote setup definition does not match the target status",
            ));
        }
    }
    if status.phase == ProjectEnvironmentSetupPhase::Ready {
        let Some(definition) = definition else {
            return Err(setup_error(
                "worker cannot report setup ready without a definition",
            ));
        };
        let Some(validation) = status.validation.as_ref() else {
            return Err(setup_error(
                "worker cannot report setup ready without measured target validation",
            ));
        };
        if validation.worker_id != expected.target_worker_id
            || validation.platform != expected.target_platform
            || validation.commands.len() != definition.validation_commands.len()
            || !validation.passed()
            || validation
                .commands
                .iter()
                .zip(definition.validation_commands.iter())
                .any(|(result, command)| result.command_digest != command_digest(command))
        {
            return Err(setup_error(
                "worker cannot report setup ready without measured target validation",
            ));
        }
    }
    Ok(())
}
