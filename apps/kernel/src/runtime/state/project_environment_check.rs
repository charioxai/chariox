//! MP-08 / MP-10 / MP-11: Project-owned Check wiring; target identity and measured policy are shared below clients.
use super::*;
use crate::project_environment::*;

impl KernelRuntimeState {
    pub(crate) async fn check_project_environment(
        &self,
        request: crate::local::CheckProjectEnvironmentRequest,
        user: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let target = current_check_target(&config);
        // The initial UI selects the durable home-kernel identity using RelayStatus.
        // Bind that selection to this process before acknowledging the operation.
        // Requests carrying an observed generation must match it exactly.
        let initial = EnvironmentTargetBinding {
            machine_id: config.host_machine_id.clone(),
            target_instance_generation: config.daemon_id.clone(),
            slice_ref: None,
        };
        if request.target != target && request.target != initial {
            return Err(environment_error(
                "Check target is unavailable or its generation changed; select the owning kernel",
            ));
        }
        if request.operation_id.is_empty()
            || request.operation_id.len() > 128
            || request.operation_id.chars().any(char::is_control)
        {
            return Err(environment_error("invalid Check operation identity"));
        }
        let project = self.owned.session_store.get_project(&request.project_id)?;
        if project.owner_user_id() != user {
            return Err(environment_error(
                "caller does not own the selected Project",
            ));
        }
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        let admission_store = store.clone();
        let admission_request = request.clone();
        let admission_target = target.clone();
        let owner = user.to_owned();
        let project_id = request.project_id.clone();
        let runtime = self.clone();
        let started = tokio::task::spawn_blocking(move || {
            admission_store.start_check(admission_request, &owner, admission_target, || {
                runtime.owned.session_store.get_project(&project_id)
            })
        })
        .await
        .map_err(|_| environment_error("Check admission task failed"))??;
        let (env, operation, workspaces) = match started {
            CheckStart::Replay(operation) => {
                return Ok(LocalDaemonResponse::EnvironmentOperation { operation })
            }
            CheckStart::New {
                environment,
                operation,
                workspace_ids,
            } => (environment, operation, workspace_ids),
        };
        let ctx = EnvironmentCheckContext {
            config,
            owner: user.into(),
            profiles: self.owned.provider_account_profiles.clone(),
            apps: self.owned.durable_state_store.clone(),
        };
        let state = self.clone();
        let task_state = state.clone();
        let acknowledged = operation.clone();
        let failure_identity = operation.clone();
        tokio::spawn(async move {
            // Let the admitted response reach transport before probes compete for I/O.
            tokio::task::yield_now().await;
            let result = tokio::task::spawn_blocking(move || {
                let owner = ctx.owner.clone();
                let project_id = request.project_id.clone();
                store.run_check(ctx, env, operation, move || {
                    let project = task_state.owned.session_store.get_project(&project_id)?;
                    if project.owner_user_id() != owner
                        || project.workspace_ids() != workspaces.as_slice()
                    {
                        return Err(environment_error("Project bindings changed during Check"));
                    }
                    Ok(project)
                })
            })
            .await;
            if !matches!(result, Ok(Ok(()))) {
                // No arbitrary error/output projection. Unsettled operations remain visibly failed.
                let config = self_config(&state);
                let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
                if let Ok(_lock) = store.lock(&failure_identity.local_project_id) {
                    if state
                        .owned
                        .session_store
                        .get_project(&failure_identity.local_project_id)
                        .is_ok()
                    {
                        if let Ok(mut checks) =
                            store.load_checks(&failure_identity.local_project_id)
                        {
                            if let Some(op) = checks
                                .operations
                                .iter_mut()
                                .find(|o| o.operation_id == failure_identity.operation_id)
                            {
                                op.phase = EnvironmentOperationPhase::Failed;
                                op.updated_at_ms = crate::session::unix_epoch_ms();
                                op.recovery_state = EnvironmentRecoveryState::Settled;
                            }
                            let _ = store.save_checks(&failure_identity.local_project_id, &checks);
                        }
                    }
                }
            }
        });
        Ok(LocalDaemonResponse::EnvironmentOperation {
            operation: acknowledged,
        })
    }
    pub(crate) async fn get_environment_operation(
        &self,
        request: crate::local::EnvironmentOperationRequest,
        user: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let response = self
            .get_project_environment(
                crate::local::GetProjectEnvironmentRequest {
                    project_id: request.project_id,
                },
                user,
            )
            .await?;
        let LocalDaemonResponse::ProjectEnvironment { environment } = response else {
            return Err(environment_error("Environment unavailable"));
        };
        let current = environment
            .operations
            .into_iter()
            .find(|o| o.operation_id == request.operation_id);
        let operation = match current {
            Some(operation) => operation,
            None => {
                let config = self.owned.config_projection.snapshot();
                ProjectEnvironmentStore::new(&config.private_runtime_state_root())
                    .check_operation(
                        &environment.local_project_id,
                        &request.operation_id,
                        &current_check_target(&config),
                    )?
                    .ok_or_else(|| environment_error("Environment operation not found"))?
            }
        };
        Ok(LocalDaemonResponse::EnvironmentOperation { operation })
    }
}
fn self_config(state: &KernelRuntimeState) -> crate::config::DaemonConfig {
    state.owned.config_projection.snapshot()
}
