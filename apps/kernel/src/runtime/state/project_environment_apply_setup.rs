//! MP-08 / MP-10 / MP-11: first Apply adapts the existing confirmed-worker runner and process leases.
use super::*;
use crate::project_environment::*;

fn command_identity(
    operation: &EnvironmentOperation,
    requirement: &Requirement,
    index: usize,
) -> String {
    format!(
        "environment-apply-{}",
        metadata_digest(&(
            operation.local_project_id.as_str(),
            operation.revision_digest.as_str(),
            operation.operation_id.as_str(),
            &requirement.requirement_id,
            index
        ))
    )
}
// MP-11: an explicit retry can continue a known failed, settled command. A
// lost success or kernel-restart outcome must first be proven by target checks.
pub(in crate::runtime::state) fn setup_retry_is_settled(
    state: &KernelRuntimeState,
    env: &ProjectEnvironment,
    requirement: &Requirement,
    operation: &EnvironmentOperation,
) -> bool {
    let RequirementSpec::SetupChecks { commands, .. } = &requirement.spec else {
        return false;
    };
    let Some(owner) = &env.reviewed_by else {
        return false;
    };
    let config = state.owned.config_projection.snapshot();
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let admitted = store
        .load_applies(&env.local_project_id)
        .ok()
        .and_then(|mut journal| {
            latest_apply_attempt(&mut journal, operation).and_then(|a| {
                a.setup_admitted_commands
                    .get(&requirement.requirement_id)
                    .copied()
            })
        });
    let statuses = commands
        .iter()
        .enumerate()
        .map(|(index, _)| {
            state
                .owned
                .project_environment_setups
                .get(&command_identity(operation, requirement, index), owner)
                .ok()
        })
        .collect::<Vec<_>>();
    setup_retry_progress_is_settled(admitted, &statuses)
}

// MP-11: missing state is safe only beyond a known durable admission frontier.
fn setup_retry_progress_is_settled(
    admitted: Option<usize>,
    statuses: &[Option<ProjectEnvironmentSetupStatus>],
) -> bool {
    let Some(admitted) = admitted.filter(|n| *n <= statuses.len()) else {
        return false;
    };
    !statuses.is_empty()
        && statuses.iter().enumerate().all(|(index, status)| {
            if index >= admitted {
                return status.is_none();
            }
            status.as_ref().is_some_and(|status| {
                status.phase == ProjectEnvironmentSetupPhase::Ready
                    || (status.phase == ProjectEnvironmentSetupPhase::Failed
                        && status.failure_code.as_deref() == Some("apply_command_unconfirmed")
                        && status.retryable)
            })
        })
}

// MP-08 / MP-10 / MP-11: the Project lock protects this revision check and admission.
fn validate_setup_project(
    state: &KernelRuntimeState,
    env: &ProjectEnvironment,
    store: &ProjectEnvironmentStore,
) -> Result<(), DaemonError> {
    let owner = env
        .reviewed_by
        .as_deref()
        .ok_or_else(|| environment_error("Setup revision is not reviewed"))?;
    let project = state
        .owned
        .session_store
        .get_project(&env.local_project_id)?;
    crate::runtime::state::project_environment_apply::validate_project(
        state, &project, owner, env, store,
    )
}

fn record_setup_admission(
    state: &KernelRuntimeState,
    config: &DaemonConfig,
    env: &ProjectEnvironment,
    requirement: &Requirement,
    operation: &EnvironmentOperation,
    count: usize,
) -> Result<bool, DaemonError> {
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let _lock = store.lock(&env.local_project_id)?;
    validate_setup_project(state, env, &store)?;
    let mut journal = store.load_applies(&env.local_project_id)?;
    let a = latest_apply_attempt(&mut journal, operation)
        .filter(|a| apply_active(&a.operation) && a.operation.cancellation.is_none())
        .ok_or_else(|| environment_error("Setup admission attempt changed or cancelled"))?;
    let admitted = a
        .setup_admitted_commands
        .entry(requirement.requirement_id.clone())
        .or_default();
    let previously_admitted = count > 0 && *admitted >= count;
    *admitted = (*admitted).max(count);
    store.save_applies(&env.local_project_id, &journal)?;
    Ok(previously_admitted)
}

pub(in crate::runtime::state) fn setup_commands_completed(
    state: &KernelRuntimeState,
    env: &ProjectEnvironment,
    requirement: &Requirement,
    operation: &EnvironmentOperation,
) -> bool {
    let RequirementSpec::SetupChecks { commands, .. } = &requirement.spec else {
        return false;
    };
    let Some(owner) = &env.reviewed_by else {
        return false;
    };
    !commands.is_empty()
        && commands.iter().enumerate().all(|(index, _)| {
            state
                .owned
                .project_environment_setups
                .get(&command_identity(operation, requirement, index), owner)
                .is_ok_and(|status| status.phase == ProjectEnvironmentSetupPhase::Ready)
        })
}

// Cancellation cannot claim settlement while the existing runner reports
// an owned descendant or durable process lease with incomplete cleanup.
pub(in crate::runtime::state) fn apply_setup_cleanup_settled(
    state: &KernelRuntimeState,
    env: &ProjectEnvironment,
    operation: &EnvironmentOperation,
) -> bool {
    let Some(owner) = &env.reviewed_by else {
        return false;
    };
    environment_requirements(env).all(|r| match &r.spec {
        RequirementSpec::SetupChecks { commands, .. } => {
            commands.iter().enumerate().all(|(i, _)| {
                match state
                    .owned
                    .project_environment_setups
                    .get(&command_identity(operation, r, i), owner)
                {
                    Ok(status) => {
                        matches!(
                            status.phase,
                            ProjectEnvironmentSetupPhase::Ready
                                | ProjectEnvironmentSetupPhase::Failed
                                | ProjectEnvironmentSetupPhase::Cancelled
                        ) && status.failure_code.as_deref() != Some("validation_cleanup_incomplete")
                    }
                    Err(_) => true,
                }
            })
        }
        _ => true,
    })
}

pub(in crate::runtime::state) fn apply_setup_requirement(
    state: &KernelRuntimeState,
    config: &DaemonConfig,
    env: &ProjectEnvironment,
    requirement: &Requirement,
    operation: &EnvironmentOperation,
    should_cancel: impl Fn() -> bool,
) -> Result<(), DaemonError> {
    // P05a never inherits General-role host execution for an opaque command.
    if config.kernel_runtime_role != KernelRuntimeRole::RemoteLeaseWorker
        || !cfg!(target_os = "linux")
    {
        return Err(environment_error(
            "Opaque Setup needs a confirmed disposable Linux worker; no host command ran",
        ));
    }
    ensure_worker_validation_boundary(config)?;
    let RequirementSpec::SetupChecks {
        commands,
        boundary: EnvironmentExecutionBoundary::DisposableWorker,
        source_path,
        source_digest,
        legacy: None,
        ..
    } = &requirement.spec
    else {
        return Err(environment_error(
            "Review bounded Setup on the disposable worker",
        ));
    };
    // MP-08 / MP-10 / MP-11: preparation errors precede every command.
    // Persist the zero frontier before workspace/public-input preparation.
    record_setup_admission(state, config, env, requirement, operation, 0)?;
    let root = match &requirement.scope {
        RequirementScope::Project => env.folders.first(),
        RequirementScope::Folder { folder_id } => {
            env.folders.iter().find(|f| &f.folder_id == folder_id)
        }
    }
    .ok_or_else(|| environment_error("Setup folder unavailable"))?;
    let home = resolved_worker_kernel_home(config)?;
    let workspace =
        canonical_worker_workspace(&root.local_workspace_binding, Some(home.as_os_str()))?;
    let preparation = WorkerPreparationHome::for_project_worker(
        &workspace,
        &home,
        &env.local_project_id,
        &config.host_machine_id,
    )?;
    let mut environment = worker_validation_environment_without_provider(preparation.path());
    // Only explicitly dependent public bindings can enter Setup; named secrets
    // continue through provider/Agent-tool services, not generic injection.
    let dependencies: Vec<_> = environment_requirements(env)
        .filter(|r| requirement.depends_on.contains(&r.requirement_id))
        .collect();
    let bindings = project_public_setup_environment(
        config,
        &env.local_project_id,
        root,
        &workspace,
        &dependencies,
    )?;
    for (name, value) in bindings.iter() {
        environment.insert(name.into(), value.into());
    }
    // A different client may Save while the runner owns a live subprocess.
    // Revoke that process through the existing cancellation/cleanup path when
    // its saved Project authority changes or can no longer be established.
    let should_stop = || {
        if should_cancel() {
            return true;
        }
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        // MP-08 / MP-10 / MP-11: the saved revision is atomically replaced.
        // Observe it during execution without waiting on a Project lock that
        // Plan/Detect/export may hold. Locked admission still fences every
        // command; a changed owner, revision or folder still revokes this one.
        validate_setup_project(state, env, &store).is_err()
    };
    let deadline = Instant::now() + VALIDATION_TOTAL_TIMEOUT;
    for (index, command) in commands.iter().enumerate() {
        state.authorize_current_external_command()?;
        if should_stop() {
            return Err(environment_error("Setup cancelled before command start"));
        }
        if let Some(path) = source_path {
            let mut file = open_workspace_file(&workspace, path)?;
            let mut digest = Sha256::new();
            let mut bytes = 0_u64;
            let mut buffer = [0; 8192];
            loop {
                let n = file
                    .read(&mut buffer)
                    .map_err(|_| environment_error("Setup source unavailable"))?;
                if n == 0 {
                    break;
                }
                bytes += n as u64;
                if bytes > 16 * 1024 * 1024 {
                    return Err(environment_error("Setup source exceeds bounds"));
                }
                digest.update(&buffer[..n]);
            }
            let actual = format!("{:x}", digest.finalize());
            if source_digest
                .as_ref()
                .is_none_or(|expected| expected.trim_start_matches("sha256:") != actual)
            {
                return Err(environment_error(
                    "Setup source changed; review before executing",
                ));
            }
        }
        // Literal shell quoting is only an adapter for the existing opaque
        // runner; command executable/arguments remain the reviewed typed input.
        let quoted = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        let text = std::iter::once(command.executable.as_str())
            .chain(command.arguments.iter().map(String::as_str))
            .map(quoted)
            .collect::<Vec<_>>()
            .join(" ");
        let command_id = command_identity(operation, requirement, index);
        let execution = SetupExecution {
            owner_user_id: env
                .reviewed_by
                .clone()
                .ok_or_else(|| environment_error("Setup revision is not reviewed"))?,
            operation_id: command_id.clone(),
            project_id: env.local_project_id.clone(),
            session_id: String::new(),
            agent_id: String::new(),
            execution_session_id: String::new(),
            execution_agent_id: String::new(),
            workspace_id: root.local_workspace_binding.clone(),
            target_worker_id: config.host_machine_id.clone(),
            target_platform: actual_worker_platform(),
            definition: None,
            validation_commands: vec![text.clone()],
            persist_project_definition: false,
            remote_leased_agent_id: None,
        };
        // Publish intent before runner admission/spawn, including a new suffix
        // command. A crash after this write remains uncertain, never replayable.
        let previously_admitted =
            record_setup_admission(state, config, env, requirement, operation, index + 1)?;
        let store = &state.owned.project_environment_setups;
        let command_attempt = match store.get(&command_id, &execution.owner_user_id) {
            Ok(status) if status.phase == ProjectEnvironmentSetupPhase::Ready => continue,
            Ok(status)
                if operation.attempt > 1
                    && status.phase == ProjectEnvironmentSetupPhase::Failed
                    && status.failure_code.as_deref() == Some("apply_command_unconfirmed")
                    && status.retryable =>
            {
                store.retry(&command_id, "", &execution.owner_user_id)?.1
            }
            Ok(_) => {
                return Err(environment_error(
                    "Prior Setup command requires target reconciliation, not replay",
                ))
            }
            Err(_) if previously_admitted => {
                return Err(environment_error(
                    "Prior Setup command state is missing; no replay",
                ));
            }
            Err(_) => {
                let (_, begin) = store.begin_at_attempt(execution, operation.attempt)?;
                if !begin {
                    return Err(environment_error("Setup command identity changed"));
                }
                operation.attempt
            }
        };
        let _execution = store
            .begin_execution(&command_id, command_attempt)
            .ok_or_else(|| environment_error("Setup execution not admitted"))?;
        store.update(&command_id, command_attempt, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Validating;
        });
        let scratch = WorkerValidationScratch::create_for_worker(
            &workspace,
            preparation.path(),
            &command_id,
            command_attempt,
            index,
        )
        .map_err(|_| environment_error("Setup scratch unavailable"))?;
        let command_deadline =
            deadline.min(Instant::now() + Duration::from_millis(u64::from(command.timeout_ms)));
        let result = run_worker_validation_command_with_recovery(
            &text,
            &workspace,
            &environment,
            &scratch,
            store,
            &command_id,
            command_attempt,
            index,
            should_stop,
            Some(command_deadline),
        );
        match result {
            Ok((0, stdout, stderr))
                if stdout.saturating_add(stderr) <= command.output_limit_bytes as usize =>
            {
                store.update(&command_id, command_attempt, |entry| {
                    entry.status.phase = ProjectEnvironmentSetupPhase::Ready;
                    entry.status.progress_percent = 100;
                });
            }
            _ => {
                store.mark_failed(
                    &command_id,
                    command_attempt,
                    "apply_command_unconfirmed",
                    "Bounded command failed, was cancelled or could not settle; no output retained",
                );
                return Err(environment_error(
                    "Setup command did not pass its bounded execution",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "project_environment_apply_setup_tests.rs"]
mod tests;
