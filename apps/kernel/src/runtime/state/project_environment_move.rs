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
            let result = async {
                let agent = runtime.owned.agent_store.get_agent(&agent_id)?;
                let run = runtime
                    .launch_agent_move_setup_provider(&target, &agent)
                    .await?;
                runtime
                    .run_project_environment_setup_with_provider(execution, attempt, Some(run))
                    .await;
                Ok::<_, DaemonError>(())
            }
            .await;
            if result.is_err() {
                runtime.owned.project_environment_setups.mark_failed(&operation, attempt, "move_setup_provider_unavailable", "Move target provider could not start. Restore its selected account and Project inputs, then retry Project setup");
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
