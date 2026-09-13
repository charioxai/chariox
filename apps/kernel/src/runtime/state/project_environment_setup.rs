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

const OPERATION: &str = "project environment setup";
const MAX_OPERATION_ID_CHARS: usize = 128;
const MAX_VALIDATION_COMMANDS: usize = 32;
const MAX_COMMAND_CHARS: usize = 8_192;
const VALIDATION_COMMAND_TIMEOUT_MS: u64 = 120_000;
const VALIDATION_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);

const WORKER_KERNEL_ENV_NAMES: &[&str] = &[
    "CHARIOX_HOME",
    "CHARIOX_MANAGED_VAULT_PATH",
    "CHARIOX_CAPABILITY_ISOLATION_ROOT",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
    "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
    "CHARIOX_MANAGED_BOOTSTRAP_PATH",
    "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
    "CHARIOX_KERNEL_HOST",
    "CHARIOX_KERNEL_PORT",
    "CHARIOX_DAEMON_ID",
    "CHARIOX_MACHINE_ID",
    "CHARIOX_MANAGED_KERNEL_BINARY",
    "CHARIOX_MANAGED_RELEASE_MANIFEST",
    "CHARIOX_MANAGED_RELEASE_SIGNATURE",
    "CHARIOX_MANAGED_RELEASE_PUBLIC_KEY",
    "CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH",
    "CHARIOX_ACCEPT_REMOTE_LEASES",
    "CHARIOX_KERNEL_RUNTIME_ROLE",
    "CHARIOX_REMOTE_LEASE_CAPACITY",
    "CHARIOX_LEASE_WORKER_HOME_CALLER",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SetupExecution {
    owner_user_id: String,
    operation_id: String,
    project_id: String,
    session_id: String,
    agent_id: String,
    workspace_id: String,
    target_worker_id: String,
    target_platform: String,
    definition: Option<ProjectEnvironmentDefinition>,
    validation_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SetupEntry {
    execution: SetupExecution,
    status: ProjectEnvironmentSetupStatus,
    fingerprint: String,
    cancel_requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSetupEntry {
    entry: SetupEntry,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectEnvironmentSetupStore {
    entries: Arc<Mutex<BTreeMap<String, SetupEntry>>>,
    durable_state_store: Option<DurableKernelStateStore>,
}

impl Default for ProjectEnvironmentSetupStore {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            durable_state_store: None,
        }
    }
}

impl ProjectEnvironmentSetupStore {
    pub(crate) fn restore_from_durable_state(
        durable_state_store: &DurableKernelStateStore,
    ) -> Self {
        let store = Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            durable_state_store: Some(durable_state_store.clone()),
        };
        let events =
            match durable_state_store.load_events_by_kind("project.environment_setup.updated") {
                Ok(events) => events,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "project.environment_setup",
                        "failed to restore project environment setup state",
                        serde_json::json!({"error": error.to_string()}),
                    );
                    return store;
                }
            };
        let mut entries = store
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        for event in events {
            let Ok(persisted) = serde_json::from_value::<PersistedSetupEntry>(event.payload) else {
                continue;
            };
            entries.insert(
                persisted.entry.execution.operation_id.clone(),
                persisted.entry,
            );
        }
        for entry in entries.values_mut() {
            if matches!(
                entry.status.phase,
                ProjectEnvironmentSetupPhase::Requested
                    | ProjectEnvironmentSetupPhase::Preparing
                    | ProjectEnvironmentSetupPhase::Validating
            ) {
                entry.status.phase = ProjectEnvironmentSetupPhase::Failed;
                entry.status.progress_percent = 0;
                entry.status.message =
                    Some("kernel restarted before target environment setup completed".to_string());
                entry.status.failure_code = Some("kernel_restarted".to_string());
                entry.status.failure_message = Some(
                    "setup was interrupted by kernel restart; retry the operation".to_string(),
                );
                entry.status.retryable = true;
                entry.status.updated_at_ms = crate::session::unix_epoch_ms();
                entry.cancel_requested = false;
            }
        }
        drop(entries);
        store
    }

    fn begin(
        &self,
        execution: SetupExecution,
    ) -> Result<(ProjectEnvironmentSetupStatus, bool), DaemonError> {
        let fingerprint = setup_fingerprint(&execution)?;
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        if let Some(existing) = entries.get(&execution.operation_id) {
            if existing.fingerprint != fingerprint {
                return Err(setup_error(
                    "operation id was already used for a different setup request",
                ));
            }
            return Ok((existing.status.clone(), false));
        }
        let now = crate::session::unix_epoch_ms();
        let status = ProjectEnvironmentSetupStatus {
            operation_id: execution.operation_id.clone(),
            project_id: execution.project_id.clone(),
            session_id: execution.session_id.clone(),
            agent_id: execution.agent_id.clone(),
            worker_id: execution.target_worker_id.clone(),
            platform: execution.target_platform.clone(),
            phase: ProjectEnvironmentSetupPhase::Requested,
            attempt: 1,
            progress_percent: 0,
            definition_digest: execution
                .definition
                .as_ref()
                .map(ProjectEnvironmentDefinition::digest),
            validation: None,
            message: Some("setup request accepted".to_string()),
            failure_code: None,
            failure_message: None,
            retryable: true,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let entry = SetupEntry {
            execution,
            status: status.clone(),
            fingerprint,
            cancel_requested: false,
        };
        entries.insert(status.operation_id.clone(), entry.clone());
        drop(entries);
        self.persist(&entry);
        Ok((status, true))
    }

    fn retry(
        &self,
        operation_id: &str,
        session_id: &str,
        caller_user_id: &str,
    ) -> Result<(SetupExecution, u32, ProjectEnvironmentSetupStatus), DaemonError> {
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let entry = entries
            .get_mut(operation_id)
            .ok_or_else(|| setup_error("setup operation was not found"))?;
        ensure_entry_owner(entry, session_id, caller_user_id)?;
        if !matches!(
            entry.status.phase,
            ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Cancelled
        ) {
            return Err(setup_error(
                "only failed or cancelled setup operations can be retried",
            ));
        }
        let attempt = entry.status.attempt.saturating_add(1);
        entry.status.attempt = attempt;
        entry.status.phase = ProjectEnvironmentSetupPhase::Requested;
        entry.status.progress_percent = 0;
        entry.status.definition_digest = entry
            .execution
            .definition
            .as_ref()
            .map(ProjectEnvironmentDefinition::digest);
        entry.status.validation = None;
        entry.status.message = Some("setup retry accepted".to_string());
        entry.status.failure_code = None;
        entry.status.failure_message = None;
        entry.status.retryable = true;
        entry.status.updated_at_ms = crate::session::unix_epoch_ms();
        entry.cancel_requested = false;
        let execution = entry.execution.clone();
        let status = entry.status.clone();
        let persisted = entry.clone();
        drop(entries);
        self.persist(&persisted);
        Ok((execution, attempt, status))
    }

    fn get(
        &self,
        operation_id: &str,
        caller_user_id: &str,
    ) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
        let entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let entry = entries
            .get(operation_id)
            .ok_or_else(|| setup_error("setup operation was not found"))?;
        if entry.execution.owner_user_id != caller_user_id {
            return Err(setup_error(
                "caller is not allowed to inspect this setup operation",
            ));
        }
        Ok(entry.status.clone())
    }

    fn cancel(
        &self,
        operation_id: &str,
        session_id: &str,
        caller_user_id: &str,
    ) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let entry = entries
            .get_mut(operation_id)
            .ok_or_else(|| setup_error("setup operation was not found"))?;
        ensure_entry_owner(entry, session_id, caller_user_id)?;
        if !matches!(
            entry.status.phase,
            ProjectEnvironmentSetupPhase::Ready
                | ProjectEnvironmentSetupPhase::Failed
                | ProjectEnvironmentSetupPhase::Cancelled
        ) {
            entry.cancel_requested = true;
            entry.status.phase = ProjectEnvironmentSetupPhase::Cancelled;
            entry.status.message = Some("setup cancellation requested".to_string());
            entry.status.failure_code = None;
            entry.status.failure_message = None;
            entry.status.retryable = true;
            entry.status.updated_at_ms = crate::session::unix_epoch_ms();
        }
        let status = entry.status.clone();
        let persisted = entry.clone();
        drop(entries);
        self.persist(&persisted);
        Ok(status)
    }

    fn is_cancelled(&self, operation_id: &str, attempt: u32) -> bool {
        let entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        entries.get(operation_id).is_some_and(|entry| {
            entry.status.attempt == attempt
                && (entry.cancel_requested
                    || entry.status.phase == ProjectEnvironmentSetupPhase::Cancelled)
        })
    }

    fn update<F>(&self, operation_id: &str, attempt: u32, update: F) -> bool
    where
        F: FnOnce(&mut SetupEntry),
    {
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let Some(entry) = entries.get_mut(operation_id) else {
            return false;
        };
        if entry.status.attempt != attempt
            || matches!(
                entry.status.phase,
                ProjectEnvironmentSetupPhase::Ready
                    | ProjectEnvironmentSetupPhase::Failed
                    | ProjectEnvironmentSetupPhase::Cancelled
            )
        {
            return false;
        }
        update(entry);
        entry.status.updated_at_ms = crate::session::unix_epoch_ms();
        let persisted = entry.clone();
        drop(entries);
        self.persist(&persisted);
        true
    }

    fn persist(&self, entry: &SetupEntry) {
        let Some(durable_state_store) = &self.durable_state_store else {
            return;
        };
        if let Err(error) = durable_state_store.append_event(
            "project.environment_setup.updated",
            Some(entry.execution.operation_id.clone()),
            serde_json::json!(PersistedSetupEntry {
                entry: entry.clone(),
            }),
        ) {
            crate::logging::warn_with_fields(
                "project.environment_setup",
                "failed to persist project environment setup state",
                serde_json::json!({"error": error.to_string()}),
            );
        }
    }
}

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
                    self.spawn_project_environment_setup(execution, status.attempt);
                }
                Ok(LocalDaemonResponse::ProjectEnvironmentSetupStarted { status })
            }
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(request) => {
                let status = self
                    .owned
                    .project_environment_setups
                    .get(&request.operation_id, caller_user_id)?;
                Ok(LocalDaemonResponse::ProjectEnvironmentSetupStatus { status })
            }
            LocalDaemonRequest::CancelProjectEnvironmentSetup(request) => {
                let status = self.owned.project_environment_setups.cancel(
                    &request.operation_id,
                    &request.session_id,
                    caller_user_id,
                )?;
                Ok(LocalDaemonResponse::ProjectEnvironmentSetupCancelled { status })
            }
            LocalDaemonRequest::RetryProjectEnvironmentSetup(request) => {
                let (execution, attempt, status) = self.owned.project_environment_setups.retry(
                    &request.operation_id,
                    &request.session_id,
                    caller_user_id,
                )?;
                self.spawn_project_environment_setup(execution, attempt);
                Ok(LocalDaemonResponse::ProjectEnvironmentSetupRetried { status })
            }
            _ => Err(setup_error("unsupported project environment setup request")),
        }
    }

    fn prepare_setup_execution(
        &self,
        request: StartProjectEnvironmentSetupRequest,
        caller_user_id: &str,
    ) -> Result<SetupExecution, DaemonError> {
        validate_operation_id(&request.operation_id)?;
        let config = self.owned.config_projection.snapshot();
        if config.kernel_runtime_role != KernelRuntimeRole::RemoteLeaseWorker {
            return Err(setup_error(
                "project environment setup is only available on a dedicated worker kernel",
            ));
        }
        let actual_platform = actual_worker_platform();
        if request.target_worker_id != config.host_machine_id {
            return Err(setup_error(
                "requested worker does not match this kernel's worker identity",
            ));
        }
        if request.target_platform != actual_platform {
            return Err(setup_error("requested platform does not match this worker"));
        }
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
        }
        let workspace_id = agent.worktree_id().unwrap_or_else(|| session.worktree_id());
        Ok(SetupExecution {
            owner_user_id: caller_user_id.to_string(),
            operation_id: request.operation_id,
            project_id: request.project_id,
            session_id: request.session_id,
            agent_id: request.agent_id,
            workspace_id: workspace_id.to_string(),
            target_worker_id: request.target_worker_id,
            target_platform: request.target_platform,
            definition,
            validation_commands: request.validation_commands,
        })
    }

    fn spawn_project_environment_setup(&self, execution: SetupExecution, attempt: u32) {
        let runtime_state = self.clone();
        tokio::spawn(async move {
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
            mark_setup_failed(
                store,
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
            session_id: execution.session_id.clone(),
            agent_id: execution.agent_id.clone(),
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
            &execution.session_id,
            &execution.agent_id,
            &AgentUtilityKind::ProjectEnvironmentSetup,
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                mark_setup_failed(
                    store,
                    &execution.operation_id,
                    attempt,
                    "worker_provider_context_unavailable",
                    "the prepared worker provider context is unavailable",
                );
                return;
            }
        };
        if provider_run.owner_user_id() != execution.owner_user_id.as_str() {
            mark_setup_failed(
                store,
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
                    mark_setup_failed(
                        store,
                        &execution.operation_id,
                        attempt,
                        "utility_output_invalid",
                        "utility agent returned an unexpected setup result",
                    );
                    return;
                }
            },
            Err(_) => {
                mark_setup_failed(
                    store,
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
            mark_setup_failed(
                store,
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
                mark_setup_failed(
                    store,
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
            mark_setup_failed(
                store,
                &execution.operation_id,
                attempt,
                "validation_failed",
                "one or more target validation commands failed",
            );
            return;
        }
        if self
            .update_project_environment_definition(
                &execution.project_id,
                definition,
                &execution.owner_user_id,
            )
            .is_err()
        {
            mark_setup_failed(
                store,
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
        if provider_run.session_id() != execution.session_id.as_str()
            || provider_run.agent_instance_id() != Some(execution.agent_id.as_str())
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
        let validation = tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let mut results = Vec::with_capacity(commands.len());
            for command in commands {
                if cancellation.is_cancelled(&operation_id, attempt)
                    || started.elapsed() >= VALIDATION_TOTAL_TIMEOUT
                {
                    break;
                }
                let result = run_worker_validation_command(&command, &workspace_root, &environment);
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

fn mark_setup_failed(
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    code: &str,
    message: &str,
) {
    let _ = store.update(operation_id, attempt, |entry| {
        entry.status.phase = ProjectEnvironmentSetupPhase::Failed;
        entry.status.progress_percent = 0;
        entry.status.message = Some(message.to_string());
        entry.status.failure_code = Some(code.to_string());
        entry.status.failure_message = Some(message.to_string());
        entry.status.retryable = true;
    });
}

fn ensure_worker_validation_boundary(config: &DaemonConfig) -> Result<(), DaemonError> {
    if config.kernel_runtime_role != KernelRuntimeRole::RemoteLeaseWorker {
        return Err(setup_error(
            "project environment setup is only executable by a dedicated worker kernel",
        ));
    }
    let receipt_path = std::env::var_os(crate::managed_bootstrap::worker::ACTIVITY_RECEIPT_ENV)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| setup_error("disposable worker receipt is missing"))?;
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| setup_error("disposable worker Cloud binding is missing"))?;
    crate::managed_bootstrap::worker::confirmed_activity_allocation(
        Path::new(&receipt_path),
        config,
        profile,
    )
    .map(|_| ())
    .map_err(|error| setup_error(&format!("worker boundary is not confirmed: {error}")))
}

fn canonical_worker_workspace(
    path: impl AsRef<Path>,
    kernel_home: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, DaemonError> {
    let path = path.as_ref();
    let canonical = path
        .canonicalize()
        .map_err(|error| setup_error(&format!("worker worktree is unavailable: {error}")))?;
    if !canonical.is_dir() || canonical == Path::new("/") {
        return Err(setup_error("worker worktree must be a non-root directory"));
    }
    let kernel_home = kernel_home
        .map(PathBuf::from)
        .ok_or_else(|| setup_error("worker kernel home is not configured"))?
        .canonicalize()
        .map_err(|error| setup_error(&format!("worker kernel home is unavailable: {error}")))?;
    if canonical == kernel_home || canonical.starts_with(&kernel_home) {
        return Err(setup_error(
            "worker worktree overlaps kernel-owned home state",
        ));
    }
    Ok(canonical)
}

fn worker_validation_environment(provider_run: &RuntimeProviderRun) -> BTreeMap<String, String> {
    let mut removed = BTreeSet::new();
    removed.extend(
        crate::provider::managed_provider_control_env_remove()
            .into_iter()
            .collect::<BTreeSet<_>>(),
    );
    removed.extend(provider_run.pty_env_remove().iter().cloned());

    let mut environment = BTreeMap::new();
    for (name, value) in std::env::vars() {
        if worker_validation_environment_allowed(&name, &removed) {
            environment.insert(name, value);
        }
    }
    for (name, value) in provider_run.pty_env() {
        if worker_validation_environment_allowed(name, &removed) {
            environment.insert(name.clone(), value.clone());
        }
    }
    environment
}

fn worker_validation_environment_allowed(name: &str, removed: &BTreeSet<String>) -> bool {
    !removed.contains(name)
        && !crate::secret::secret_like_env_name(name)
        && !WORKER_KERNEL_ENV_NAMES.contains(&name)
        && !name.starts_with("CHARIOX_")
}

fn run_worker_validation_command(
    command_text: &str,
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<(i32, usize, usize), String> {
    // This child is spawned only by a confirmed disposable worker kernel after
    // the provider run context and workspace have been fenced above. Keep the
    // shell local to that worker boundary and do not route through the home
    // kernel's general ShellCommandService.
    let (shell, shell_flag) = if cfg!(windows) {
        ("C:\\Windows\\System32\\cmd.exe", "/C")
    } else {
        // Preserve the provider PTY's prepared PATH and other environment
        // exactly; a login shell could source kernel-home startup files.
        ("/bin/sh", "-c")
    };
    let mut command = Command::new(shell);
    command
        .arg(shell_flag)
        .arg(command_text)
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(environment);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // Keep descendants in a worker-local process group so cancellation or
        // timeout cannot leave a compiler holding the validation pipes open.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("worker validation command did not provide stdout capture".to_string());
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("worker validation command did not provide stderr capture".to_string());
    };
    let stdout_reader = std::thread::spawn(move || count_validation_output(stdout));
    let stderr_reader = std::thread::spawn(move || count_validation_output(stderr));
    let timeout = Duration::from_millis(VALIDATION_COMMAND_TIMEOUT_MS);
    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            terminate_validation_process_group(&mut child);
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err("worker validation command timed out".to_string());
        }
        Err(error) => {
            terminate_validation_process_group(&mut child);
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(error.to_string());
        }
    };
    terminate_validation_process_group(&mut child);
    let stdout_bytes = stdout_reader
        .join()
        .map_err(|_| "worker validation stdout reader panicked".to_string())?;
    let stdout_bytes = stdout_bytes?;
    let stderr_bytes = stderr_reader
        .join()
        .map_err(|_| "worker validation stderr reader panicked".to_string())?;
    let stderr_bytes = stderr_bytes?;
    Ok((status.code().unwrap_or(-1), stdout_bytes, stderr_bytes))
}

#[cfg(unix)]
fn terminate_validation_process_group(child: &mut std::process::Child) {
    let process_group = child.id() as libc::pid_t;
    let _ = unsafe { libc::kill(-process_group, libc::SIGKILL) };
}

#[cfg(not(unix))]
fn terminate_validation_process_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

fn count_validation_output(mut output: impl Read) -> Result<usize, String> {
    let mut buffer = [0_u8; 8192];
    let mut bytes = 0_usize;
    loop {
        let read = output
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes = bytes.saturating_add(read);
    }
}

fn ensure_entry_owner(
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

fn validate_operation_id(operation_id: &str) -> Result<(), DaemonError> {
    if operation_id.trim().is_empty() || operation_id.chars().count() > MAX_OPERATION_ID_CHARS {
        return Err(setup_error("operation id must be non-empty and bounded"));
    }
    Ok(())
}

fn validate_commands(commands: &[String]) -> Result<(), DaemonError> {
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

fn setup_fingerprint(execution: &SetupExecution) -> Result<String, DaemonError> {
    let encoded = serde_json::to_vec(execution)
        .map_err(|error| setup_error(&format!("could not fingerprint setup request: {error}")))?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn command_digest(command: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(command.as_bytes()))
}

fn actual_worker_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn setup_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: OPERATION,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        ProjectEnvironmentDefinitionOrigin, ProjectEnvironmentDefinitionSource,
        ProjectEnvironmentSetupStep, ProjectEnvironmentSetupStepKind,
    };
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

    fn execution() -> SetupExecution {
        SetupExecution {
            owner_user_id: "user-1".to_string(),
            operation_id: "setup-1".to_string(),
            project_id: "project-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
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
        let (exit_code, _, _) = run_worker_validation_command(command, &workspace, &environment)
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
        let (exit_code, _, _) =
            run_worker_validation_command("command -v worker-local-tool", &workspace, &environment)
                .expect("worker validation shell should execute");
        assert_eq!(exit_code, 0, "installed worker-local tool must resolve");
        let _ = std::fs::remove_dir_all(root);
    }
}
