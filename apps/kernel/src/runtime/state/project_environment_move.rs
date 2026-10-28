//! Move runs the ordinary Project setup engine against its staged target.
use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn prepare_agent_move_project_setup(
        &self,
        target: &crate::agent_move::MoveTarget,
        requirements: &crate::agent_move::AgentMoveRequirements,
    ) -> Result<Option<String>, DaemonError> {
        let Some(definition) = &requirements.environment_definition else {
            return Ok(None);
        };
        let receipt = target
            .receipt
            .as_ref()
            .ok_or_else(|| setup_error("Move Project receipt is missing"))?;
        let session = target
            .backing_session_id
            .as_deref()
            .ok_or_else(|| setup_error("Move target Room is missing"))?;
        let agent_id = target
            .backing_agent_id
            .as_deref()
            .ok_or_else(|| setup_error("Move target agent is missing"))?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let config = self.owned.config_projection.snapshot();
        let platform = actual_worker_platform();
        let definition = validate_setup_definition(Some(definition.clone()), &platform, &[])?;
        let operation_id = format!("{}-setup", target.capture.identity.operation_id);
        let execution = SetupExecution {
            owner_user_id: agent.owner_user_id().into(),
            operation_id: operation_id.clone(),
            project_id: receipt.target_project_id.clone(),
            session_id: session.into(),
            agent_id: agent_id.into(),
            execution_session_id: session.into(),
            execution_agent_id: agent_id.into(),
            workspace_id: receipt.primary_path()?.display().to_string(),
            target_worker_id: config.host_machine_id.clone(),
            target_platform: platform,
            definition,
            validation_commands: Vec::new(),
            persist_project_definition: false,
            remote_leased_agent_id: None,
        };
        let (status, should_spawn) = self
            .owned
            .project_environment_setups
            .begin(execution.clone())?;
        let terminal = matches!(
            status.phase,
            ProjectEnvironmentSetupPhase::Ready
                | ProjectEnvironmentSetupPhase::Failed
                | ProjectEnvironmentSetupPhase::Cancelled
        );
        // Restart resumes the admitted attempt; a failed attempt still needs the ordinary retry action.
        if should_spawn
            || (!terminal
                && self
                    .owned
                    .project_environment_setups
                    .execution_is_settled(&operation_id, status.attempt))
        {
            self.spawn_agent_move_project_setup(target.clone(), execution, status.attempt);
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(240);
        loop {
            self.authorize_current_external_command()?;
            let (_, status) = self
                .owned
                .project_environment_setups
                .get_entry(&operation_id, agent.owner_user_id())?;
            if !self
                .owned
                .project_environment_setups
                .execution_is_settled(&operation_id, status.attempt)
            {
                if tokio::time::Instant::now() >= deadline {
                    self.owned.project_environment_setups.request_cancel(
                        &operation_id,
                        session,
                        agent.owner_user_id(),
                        false,
                    )?;
                    return Err(setup_error("Move Project setup is cancelling after its deadline; query its existing setup operation before retrying"));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            match status.phase {
                ProjectEnvironmentSetupPhase::Ready => return Ok(Some(operation_id)),
                ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Cancelled => return Err(setup_error(&format!("Move target Project setup failed. Retry Project setup operation `{operation_id}` on the target kernel, then retry this Move"))),
                _ if tokio::time::Instant::now() >= deadline => return Err(setup_error("Move target Project setup is pending. Query its existing setup operation before retrying; execution remains fenced")),
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }

    pub(super) fn spawn_agent_move_project_setup(
        &self,
        target: crate::agent_move::MoveTarget,
        execution: SetupExecution,
        attempt: u32,
    ) {
        let Some(guard) = self
            .owned
            .project_environment_setups
            .begin_execution(&execution.operation_id, attempt)
        else {
            return;
        };
        let runtime = self.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let operation = execution.operation_id.clone();
            let session = execution.execution_session_id.clone();
            let agent_id = execution.execution_agent_id.clone();
            let owner = execution.owner_user_id.clone();
            let result = async {
                let agent = runtime.owned.agent_store.get_agent(&agent_id)?;
                let run = runtime
                    .launch_agent_move_setup_provider(&target, &agent)
                    .await?;
                runtime
                    .run_project_environment_setup_with_provider(execution, attempt, Some(run))
                    .await;
                let (_, status) = runtime
                    .owned
                    .project_environment_setups
                    .get_entry(&operation, &owner)?;
                Ok::<_, DaemonError>(status.phase)
            }
            .await;
            // Ordinary setup restored the prepared HOME/PATH and grants on this
            // run. Keep it for the first turn; no teardown remains after Ready.
            if matches!(result, Ok(ProjectEnvironmentSetupPhase::Ready)) {
                return;
            }
            if let Err(error) = result {
                tracing::warn!(
                    operation_id = %operation,
                    attempt,
                    error = %crate::provider::sanitize_provider_diagnostic(&error.to_string()),
                    "Move Project setup provider launch failed"
                );
                runtime.owned.project_environment_setups.mark_failed(&operation, attempt, "move_setup_provider_unavailable", &format!("Move target provider could not start: {}. Restore its selected account and Project inputs, then retry Project setup", crate::provider::sanitize_provider_diagnostic(&error.to_string())));
            }
            if runtime
                .with_app_side_effect(|app| app.end_agent_provider_run(&session, &agent_id))
                .await
                .is_err()
            {
                runtime.owned.project_environment_setups.mark_failed(&operation, attempt, "move_setup_teardown_failed", "Project setup process teardown is uncertain; restore the target before retrying Move");
            }
        });
    }
}

pub(crate) fn restore_agent_move_preparation_environment(
    config: &DaemonConfig,
    durable: &DurableKernelStateStore,
    moves: &crate::agent_move::AgentMoveStore,
    mut request: crate::provider::LaunchProviderRequest,
) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
    if request.preparation_environment.is_some() {
        return Ok(request);
    }
    let Some(target) = request
        .agent_id
        .as_deref()
        .and_then(|agent| moves.target_for_agent(&request.session_id, agent))
    else {
        return Ok(request);
    };
    if target
        .composition
        .requirements
        .as_ref()
        .and_then(|requirements| requirements.environment_definition.as_ref())
        .is_none()
    {
        return Ok(request);
    }
    let operation = format!("{}-setup", target.capture.identity.operation_id);
    let Some(execution) =
        project_environment_setup_storage::ready_move_setup_execution(durable, &operation)?
    else {
        return if target.active || target.composition.readiness.is_some() {
            Err(setup_error("Committed Move Project setup is unavailable; restore its Ready setup journal before launching"))
        } else {
            Ok(request)
        };
    };
    let receipt = target
        .receipt
        .as_ref()
        .ok_or_else(|| setup_error("Move Project receipt is missing"))?;
    if execution.operation_id != operation
        || execution.owner_user_id != request.owner_user_id
        || execution.execution_session_id != request.session_id
        || Some(execution.execution_agent_id.as_str()) != request.agent_id.as_deref()
        || execution.project_id != receipt.target_project_id
        || execution.target_worker_id != config.host_machine_id
        || execution.target_platform != actual_worker_platform()
        || Path::new(&execution.workspace_id) != receipt.primary_path()?
        || request.working_directory.as_deref() != Some(Path::new(&execution.workspace_id))
    {
        return Err(setup_error("Move Project setup no longer matches its execution target; restore the prepared workspace before launching"));
    }
    let kernel_home = resolved_worker_kernel_home(config)?;
    let workspace =
        canonical_worker_workspace(&execution.workspace_id, Some(kernel_home.as_os_str()))?;
    let home = WorkerPreparationHome::for_project_worker(
        &workspace,
        &kernel_home,
        &execution.project_id,
        &execution.target_worker_id,
    )?;
    let path = worker_preparation_path(
        home.path(),
        Some(&workspace),
        &std::env::var("PATH").unwrap_or_default(),
        execution.definition.as_ref(),
    );
    request = request.with_preparation_environment(home.path().display().to_string(), path);
    Ok(request)
}
