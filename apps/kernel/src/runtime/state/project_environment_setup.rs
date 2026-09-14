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

pub(super) const MAX_REMOTE_SETUP_TRANSPORT_FAILURES: u8 = 3;

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
use project_environment_setup_storage::{SetupEntry, SetupExecution};
use project_environment_setup_validation::*;

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
                let (execution, status) = self
                    .owned
                    .project_environment_setups
                    .get_entry(&request.operation_id, caller_user_id)?;
                if execution.remote_leased_agent_id.is_some() {
                    let setup = get_remote_setup_status(self, &execution).await;
                    let status = match setup {
                        Ok(setup) => {
                            self.reconcile_remote_project_environment_setup(&execution, setup)?
                        }
                        Err(error) if remote_prompt_error_should_retry_transport(&error) => {
                            if let Some(status) = self
                                .owned
                                .project_environment_setups
                                .note_remote_transport_failure(
                                    &execution.operation_id,
                                    status.attempt,
                                )
                            {
                                return Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus {
                                    status,
                                });
                            }
                            // A relay disconnect is transport uncertainty, not
                            // a worker-authoritative terminal result until the
                            // bounded observation limit is reached. Preserve
                            // the current operation so a later Get can
                            // reconcile the worker's actual status.
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn start_leased_project_environment_setup(
        &self,
        target: crate::app::LeasedProjectEnvironmentSetupTarget,
        leased_agent_id: String,
        operation_id: String,
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
            .begin(execution.clone())?;
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
        let (current_status, definition) = self
            .owned
            .project_environment_setups
            .remote_status(operation_id, leased_agent_id)?;
        let config = self.owned.config_projection.snapshot();
        ensure_worker_setup_status_target(&target, &current_status, &config)?;
        let (execution, attempt, status) = self.owned.project_environment_setups.retry(
            operation_id,
            &target.home_session_id,
            &target.owner_user_id,
        )?;
        if execution.remote_leased_agent_id.as_deref() != Some(leased_agent_id) {
            return Err(setup_error(
                "setup operation is not bound to this leased agent",
            ));
        }
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
                start_remote_setup(&runtime_state, &execution).await
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
                        runtime_state
                            .owned
                            .project_environment_setups
                            .note_remote_transport_failure(&execution.operation_id, attempt);
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
        if current_status.phase == ProjectEnvironmentSetupPhase::Failed
            && current_status.failure_code.as_deref() == Some("worker_status_unavailable")
            && !current_status.retryable
        {
            validate_remote_setup_transition_after_transport_recovery(
                current_status.phase,
                setup.status.phase,
            )?;
        } else {
            validate_remote_setup_transition(current_status.phase, setup.status.phase)?;
        }
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
        )
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
    fn bounded_remote_transport_failure_blocks_retry_until_worker_status_recovers() {
        let store = ProjectEnvironmentSetupStore::default();
        let mut setup = execution();
        setup.remote_leased_agent_id = Some("lease-1".to_string());
        store.begin(setup).expect("setup should start");

        assert!(store.note_remote_transport_failure("setup-1", 1).is_none());
        assert!(store.note_remote_transport_failure("setup-1", 1).is_none());
        let terminal = store
            .note_remote_transport_failure("setup-1", 1)
            .expect("bounded transport failure should become observable");
        assert_eq!(terminal.phase, ProjectEnvironmentSetupPhase::Failed);
        assert_eq!(
            terminal.failure_code.as_deref(),
            Some("worker_status_unavailable")
        );
        assert!(!terminal.retryable);
        assert!(store.retry("setup-1", "session-1", "user-1").is_err());

        let mut execution = execution();
        execution.remote_leased_agent_id = Some("lease-1".to_string());
        let definition = execution.definition.clone();
        let recovered = ProjectEnvironmentSetupStatus {
            phase: ProjectEnvironmentSetupPhase::Ready,
            progress_percent: 100,
            validation: Some(ProjectEnvironmentValidation {
                worker_id: "machine-1".to_string(),
                platform: "linux-x86_64".to_string(),
                commands: vec![ProjectEnvironmentCommandResult {
                    command_digest: command_digest("cargo check --workspace --locked"),
                    exit_code: 0,
                    stdout_bytes: 1,
                    stderr_bytes: 0,
                }],
            }),
            message: Some("worker setup is ready".to_string()),
            failure_code: None,
            failure_message: None,
            retryable: false,
            ..terminal
        };
        let reconciled = store
            .reconcile_remote("setup-1", &execution, recovered, definition)
            .expect("authenticated worker status should recover the bounded failure");
        assert_eq!(reconciled.phase, ProjectEnvironmentSetupPhase::Ready);
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
