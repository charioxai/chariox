use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SetupExecution {
    pub(super) owner_user_id: String,
    pub(super) operation_id: String,
    pub(super) project_id: String,
    /// Home identifiers are retained for caller/status correlation.
    pub(super) session_id: String,
    pub(super) agent_id: String,
    /// Provider utility and validation execution must use the target worker's
    /// backing session/agent, never a home-kernel identity.
    pub(super) execution_session_id: String,
    pub(super) execution_agent_id: String,
    pub(super) workspace_id: String,
    pub(super) target_worker_id: String,
    pub(super) target_platform: String,
    pub(super) definition: Option<ProjectEnvironmentDefinition>,
    pub(super) validation_commands: Vec<String>,
    /// Home setup persists the generated recipe; a leased worker only returns
    /// it over the authenticated relay and must not mutate home project state.
    pub(super) persist_project_definition: bool,
    pub(super) remote_leased_agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SetupEntry {
    pub(super) execution: SetupExecution,
    pub(super) status: ProjectEnvironmentSetupStatus,
    pub(super) fingerprint: String,
    pub(super) cancel_requested: bool,
    #[serde(skip)]
    active_executions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSetupEntry {
    entry: SetupEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RemoteSetupRecoveryDecision {
    Dispatch,
    InFlight,
    Unknown,
    Acknowledged,
    Cancelled,
    Stale,
}

#[derive(Debug, Clone, Copy)]
enum RemoteSetupRecoveryState {
    InFlight { attempt: u32 },
    Unknown { attempt: u32 },
    Acknowledged { attempt: u32 },
}

#[derive(Debug, Clone)]
pub(in crate::runtime::state) struct ProjectEnvironmentSetupStore {
    entries: Arc<Mutex<BTreeMap<String, SetupEntry>>>,
    // Recovery reservations are deliberately process-local: an uncertain
    // replay must fence same-attempt duplicate Starts until a worker status,
    // explicit cancellation, or a new retry attempt resolves it.
    remote_recoveries: Arc<Mutex<BTreeMap<String, RemoteSetupRecoveryState>>>,
    durable_state_store: Option<DurableKernelStateStore>,
    execution_settled: Arc<tokio::sync::Notify>,
}

pub(super) struct SetupExecutionGuard {
    store: ProjectEnvironmentSetupStore,
    operation_id: String,
    attempt: u32,
}

impl Drop for SetupExecutionGuard {
    fn drop(&mut self) {
        self.store
            .finish_execution(&self.operation_id, self.attempt);
    }
}

impl Default for ProjectEnvironmentSetupStore {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            remote_recoveries: Arc::new(Mutex::new(BTreeMap::new())),
            durable_state_store: None,
            execution_settled: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

impl ProjectEnvironmentSetupStore {
    pub(in crate::runtime::state) fn restore_from_durable_state(
        durable_state_store: &DurableKernelStateStore,
    ) -> Self {
        let store = Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            remote_recoveries: Arc::new(Mutex::new(BTreeMap::new())),
            durable_state_store: Some(durable_state_store.clone()),
            execution_settled: Arc::new(tokio::sync::Notify::new()),
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

    pub(super) fn begin(
        &self,
        execution: SetupExecution,
    ) -> Result<(ProjectEnvironmentSetupStatus, bool), DaemonError> {
        self.begin_with_attempt(execution, None)
    }

    pub(super) fn begin_at_attempt(
        &self,
        execution: SetupExecution,
        attempt: u32,
    ) -> Result<(ProjectEnvironmentSetupStatus, bool), DaemonError> {
        if attempt == 0 {
            return Err(setup_error("setup attempt must be positive"));
        }
        self.begin_with_attempt(execution, Some(attempt))
    }

    fn begin_with_attempt(
        &self,
        execution: SetupExecution,
        requested_attempt: Option<u32>,
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
            if requested_attempt.is_some_and(|attempt| existing.status.attempt != attempt) {
                return Err(setup_error(
                    "setup attempt does not match the existing worker operation",
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
            attempt: requested_attempt.unwrap_or(1),
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
            active_executions: 0,
        };
        entries.insert(status.operation_id.clone(), entry.clone());
        drop(entries);
        self.persist(&entry);
        Ok((status, true))
    }

    pub(super) fn retry(
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
        if entry.active_executions != 0
            || !matches!(
                entry.status.phase,
                ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Cancelled
            )
        {
            return Err(setup_error(
                "only failed or cancelled setup operations can be retried",
            ));
        }
        if !entry.status.retryable {
            return Err(setup_error(
                "setup operation is not retryable; reconnect the worker and query status first",
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
        self.clear_remote_recovery(operation_id);
        Ok((execution, attempt, status))
    }

    pub(super) fn get(
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

    pub(super) fn get_entry(
        &self,
        operation_id: &str,
        caller_user_id: &str,
    ) -> Result<(SetupExecution, ProjectEnvironmentSetupStatus), DaemonError> {
        self.get_entry_with_cancellation(operation_id, caller_user_id)
            .map(|(execution, status, _)| (execution, status))
    }

    pub(super) fn get_entry_with_cancellation(
        &self,
        operation_id: &str,
        caller_user_id: &str,
    ) -> Result<(SetupExecution, ProjectEnvironmentSetupStatus, bool), DaemonError> {
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
        Ok((
            entry.execution.clone(),
            entry.status.clone(),
            entry.cancel_requested,
        ))
    }

    pub(super) fn rebind_remote_leased_agent(
        &self,
        operation_id: &str,
        attempt: u32,
        expected_leased_agent_id: &str,
        replacement_leased_agent_id: String,
    ) -> Result<SetupExecution, DaemonError> {
        if replacement_leased_agent_id.trim().is_empty() {
            return Err(setup_error(
                "replacement remote setup binding must have a leased agent",
            ));
        }
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let entry = entries
            .get_mut(operation_id)
            .ok_or_else(|| setup_error("setup operation was not found"))?;
        if entry.status.attempt != attempt {
            return Err(setup_error(
                "setup attempt changed before its remote binding could be refreshed",
            ));
        }
        if entry.cancel_requested
            || matches!(
                entry.status.phase,
                ProjectEnvironmentSetupPhase::Ready
                    | ProjectEnvironmentSetupPhase::Failed
                    | ProjectEnvironmentSetupPhase::Cancelled
            )
        {
            return Err(setup_error(
                "terminal setup cannot refresh its remote binding",
            ));
        }
        match entry.execution.remote_leased_agent_id.as_deref() {
            Some(current) if current != expected_leased_agent_id => {
                return Err(setup_error(
                    "remote setup binding changed before it could be refreshed",
                ));
            }
            Some(current) if current == replacement_leased_agent_id => {
                return Ok(entry.execution.clone())
            }
            Some(_) => {}
            None => {
                return Err(setup_error("local setup has no remote binding to refresh"));
            }
        }
        entry.execution.remote_leased_agent_id = Some(replacement_leased_agent_id);
        entry.fingerprint = setup_fingerprint(&entry.execution)?;
        let execution = entry.execution.clone();
        let persisted = entry.clone();
        drop(entries);
        self.persist(&persisted);
        Ok(execution)
    }

    pub(super) fn rebind_remote_worker_target(
        &self,
        operation_id: &str,
        leased_agent_id: &str,
        target: &crate::app::LeasedProjectEnvironmentSetupTarget,
        target_worker_id: &str,
        target_platform: &str,
    ) -> Result<
        (
            ProjectEnvironmentSetupStatus,
            Option<ProjectEnvironmentDefinition>,
        ),
        DaemonError,
    > {
        if leased_agent_id.trim().is_empty() {
            return Err(setup_error("remote setup is missing its leased agent"));
        }
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let entry = entries
            .get_mut(operation_id)
            .ok_or_else(|| setup_error("setup operation was not found"))?;
        if entry.execution.owner_user_id != target.owner_user_id
            || entry.execution.session_id != target.home_session_id
            || entry.execution.agent_id != target.home_agent_id
            || entry.execution.target_worker_id != target_worker_id
            || entry.execution.target_platform != target_platform
        {
            return Err(setup_error(
                "remote setup status does not match the worker target",
            ));
        }
        if entry.execution.remote_leased_agent_id.as_deref() == Some(leased_agent_id) {
            return Ok((entry.status.clone(), entry.execution.definition.clone()));
        }
        if entry.execution.remote_leased_agent_id.is_none() {
            return Err(setup_error(
                "worker setup is not bound to a remote leased agent",
            ));
        }
        if !matches!(
            entry.status.phase,
            ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Cancelled
        ) || !entry.status.retryable
        {
            return Err(setup_error(
                "worker setup binding cannot be replaced outside a retryable terminal attempt",
            ));
        }
        entry.execution.remote_leased_agent_id = Some(leased_agent_id.to_string());
        entry.execution.execution_session_id = target.backing_session_id.clone();
        entry.execution.execution_agent_id = target.backing_agent_id.clone();
        entry.execution.workspace_id = target.workspace_id.clone();
        entry.fingerprint = setup_fingerprint(&entry.execution)?;
        let status = entry.status.clone();
        let definition = entry.execution.definition.clone();
        let persisted = entry.clone();
        drop(entries);
        self.persist(&persisted);
        Ok((status, definition))
    }

    pub(super) fn begin_remote_recovery(
        &self,
        operation_id: &str,
        attempt: u32,
    ) -> RemoteSetupRecoveryDecision {
        let entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let Some(entry) = entries.get(operation_id) else {
            return RemoteSetupRecoveryDecision::Stale;
        };
        if entry.status.attempt != attempt {
            return RemoteSetupRecoveryDecision::Stale;
        }
        if entry.cancel_requested
            || matches!(
                entry.status.phase,
                ProjectEnvironmentSetupPhase::Ready
                    | ProjectEnvironmentSetupPhase::Failed
                    | ProjectEnvironmentSetupPhase::Cancelled
            )
        {
            return RemoteSetupRecoveryDecision::Cancelled;
        }
        let mut remote_recoveries = self
            .remote_recoveries
            .lock()
            .expect("remote setup recovery lock should not be poisoned");
        match remote_recoveries.get(operation_id).copied() {
            Some(RemoteSetupRecoveryState::InFlight {
                attempt: current_attempt,
            }) if current_attempt == attempt => RemoteSetupRecoveryDecision::InFlight,
            Some(RemoteSetupRecoveryState::Unknown {
                attempt: current_attempt,
            }) if current_attempt == attempt => RemoteSetupRecoveryDecision::Unknown,
            Some(RemoteSetupRecoveryState::Acknowledged {
                attempt: current_attempt,
            }) if current_attempt == attempt => RemoteSetupRecoveryDecision::Acknowledged,
            Some(_) => RemoteSetupRecoveryDecision::Stale,
            None => {
                remote_recoveries.insert(
                    operation_id.to_owned(),
                    RemoteSetupRecoveryState::InFlight { attempt },
                );
                RemoteSetupRecoveryDecision::Dispatch
            }
        }
    }

    pub(super) fn remote_recovery_dispatch_allowed(
        &self,
        operation_id: &str,
        attempt: u32,
    ) -> bool {
        let entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let Some(entry) = entries.get(operation_id) else {
            return false;
        };
        if entry.status.attempt != attempt
            || entry.cancel_requested
            || !matches!(
                entry.status.phase,
                ProjectEnvironmentSetupPhase::Requested
                    | ProjectEnvironmentSetupPhase::Preparing
                    | ProjectEnvironmentSetupPhase::Validating
            )
        {
            return false;
        }
        let remote_recoveries = self
            .remote_recoveries
            .lock()
            .expect("remote setup recovery lock should not be poisoned");
        matches!(
            remote_recoveries.get(operation_id),
            Some(RemoteSetupRecoveryState::InFlight {
                attempt: current_attempt
            }) if *current_attempt == attempt
        )
    }

    pub(super) fn mark_remote_recovery_unknown(&self, operation_id: &str, attempt: u32) {
        let mut remote_recoveries = self
            .remote_recoveries
            .lock()
            .expect("remote setup recovery lock should not be poisoned");
        if matches!(
            remote_recoveries.get(operation_id),
            Some(RemoteSetupRecoveryState::InFlight {
                attempt: current_attempt
            }) if *current_attempt == attempt
        ) {
            remote_recoveries.insert(
                operation_id.to_owned(),
                RemoteSetupRecoveryState::Unknown { attempt },
            );
        }
    }

    pub(super) fn mark_remote_recovery_observed(&self, operation_id: &str, attempt: u32) {
        let mut remote_recoveries = self
            .remote_recoveries
            .lock()
            .expect("remote setup recovery lock should not be poisoned");
        let should_acknowledge = matches!(
            remote_recoveries.get(operation_id),
            Some(RemoteSetupRecoveryState::InFlight {
                attempt: current_attempt
            })
                | Some(RemoteSetupRecoveryState::Unknown {
                    attempt: current_attempt
                }) if *current_attempt == attempt
        );
        if should_acknowledge {
            remote_recoveries.insert(
                operation_id.to_owned(),
                RemoteSetupRecoveryState::Acknowledged { attempt },
            );
        }
    }

    pub(super) fn clear_remote_recovery(&self, operation_id: &str) {
        self.remote_recoveries
            .lock()
            .expect("remote setup recovery lock should not be poisoned")
            .remove(operation_id);
    }

    pub(super) fn remote_status(
        &self,
        operation_id: &str,
        leased_agent_id: &str,
    ) -> Result<
        (
            ProjectEnvironmentSetupStatus,
            Option<ProjectEnvironmentDefinition>,
        ),
        DaemonError,
    > {
        let entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let entry = entries
            .get(operation_id)
            .ok_or_else(|| setup_error("setup operation was not found"))?;
        if entry.execution.remote_leased_agent_id.as_deref() != Some(leased_agent_id) {
            return Err(setup_error(
                "setup operation is not bound to this leased agent",
            ));
        }
        Ok((entry.status.clone(), entry.execution.definition.clone()))
    }

    pub(super) fn reconcile_remote(
        &self,
        operation_id: &str,
        expected: &SetupExecution,
        status: ProjectEnvironmentSetupStatus,
        definition: Option<ProjectEnvironmentDefinition>,
    ) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let entry = entries
            .get_mut(operation_id)
            .ok_or_else(|| setup_error("setup operation was not found"))?;
        if entry.execution.owner_user_id != expected.owner_user_id
            || entry.execution.session_id != expected.session_id
            || entry.execution.agent_id != expected.agent_id
            || entry.execution.project_id != expected.project_id
            || entry.execution.target_worker_id != expected.target_worker_id
            || entry.execution.target_platform != expected.target_platform
            || entry.execution.remote_leased_agent_id != expected.remote_leased_agent_id
        {
            return Err(setup_error(
                "remote setup status does not match the home operation",
            ));
        }
        validate_remote_setup_status(expected, entry.status.attempt, &status, definition.as_ref())?;
        validate_remote_setup_transition(entry.status.phase, status.phase)?;
        if let Some(definition) = definition {
            entry.execution.definition = Some(definition);
        }
        let cancellation_requested = entry.cancel_requested;
        entry.status = status.clone();
        entry.cancel_requested = match status.phase {
            ProjectEnvironmentSetupPhase::Cancelled => true,
            ProjectEnvironmentSetupPhase::Ready | ProjectEnvironmentSetupPhase::Failed => false,
            ProjectEnvironmentSetupPhase::Requested
            | ProjectEnvironmentSetupPhase::Preparing
            | ProjectEnvironmentSetupPhase::Validating => cancellation_requested,
        };
        let persisted = entry.clone();
        drop(entries);
        self.persist(&persisted);
        self.mark_remote_recovery_observed(operation_id, status.attempt);
        Ok(status)
    }

    pub(super) fn cancel(
        &self,
        operation_id: &str,
        session_id: &str,
        caller_user_id: &str,
    ) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
        self.request_cancel(operation_id, session_id, caller_user_id, false)
    }

    pub(super) fn request_cancel(
        &self,
        operation_id: &str,
        session_id: &str,
        caller_user_id: &str,
        requires_worker_acknowledgement: bool,
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
            if entry.active_executions == 0 && !requires_worker_acknowledgement {
                entry.status.phase = ProjectEnvironmentSetupPhase::Cancelled;
            }
            entry.status.message = Some("setup cancellation requested".to_string());
            entry.status.failure_code = None;
            entry.status.failure_message = None;
            entry.status.retryable = entry.status.phase == ProjectEnvironmentSetupPhase::Cancelled;
            entry.status.updated_at_ms = crate::session::unix_epoch_ms();
        }
        let status = entry.status.clone();
        let persisted = entry.clone();
        drop(entries);
        self.persist(&persisted);
        self.clear_remote_recovery(operation_id);
        Ok(status)
    }

    pub(super) fn begin_execution(
        &self,
        operation_id: &str,
        attempt: u32,
    ) -> Option<SetupExecutionGuard> {
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let entry = entries.get_mut(operation_id)?;
        if entry.status.attempt != attempt
            || entry.cancel_requested
            || matches!(
                entry.status.phase,
                ProjectEnvironmentSetupPhase::Ready
                    | ProjectEnvironmentSetupPhase::Failed
                    | ProjectEnvironmentSetupPhase::Cancelled
            )
        {
            return None;
        }
        entry.active_executions += 1;
        Some(SetupExecutionGuard {
            store: self.clone(),
            operation_id: operation_id.to_owned(),
            attempt,
        })
    }

    fn finish_execution(&self, operation_id: &str, attempt: u32) {
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let Some(entry) = entries.get_mut(operation_id) else {
            return;
        };
        if entry.status.attempt != attempt {
            return;
        }
        entry.active_executions = entry.active_executions.saturating_sub(1);
        if entry.active_executions == 0 && entry.cancel_requested {
            entry.status.phase = ProjectEnvironmentSetupPhase::Cancelled;
            entry.status.message = Some("setup cancellation completed on the worker".to_owned());
            entry.status.retryable = true;
            entry.status.updated_at_ms = crate::session::unix_epoch_ms();
            let persisted = entry.clone();
            drop(entries);
            self.persist(&persisted);
        } else {
            drop(entries);
        }
        self.execution_settled.notify_waiters();
    }

    pub(super) async fn wait_for_cancellation(
        &self,
        operation_id: &str,
        caller_user_id: &str,
    ) -> Result<ProjectEnvironmentSetupStatus, DaemonError> {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let settled = self.execution_settled.notified();
                tokio::pin!(settled);
                settled.as_mut().enable();
                let status = self.get(operation_id, caller_user_id)?;
                if matches!(
                    status.phase,
                    ProjectEnvironmentSetupPhase::Ready
                        | ProjectEnvironmentSetupPhase::Failed
                        | ProjectEnvironmentSetupPhase::Cancelled
                ) {
                    return Ok(status);
                }
                settled.await;
            }
        })
        .await
        .map_err(|_| {
            setup_error("setup cancellation is still waiting for worker execution to settle")
        })?
    }

    pub(super) fn is_cancelled(&self, operation_id: &str, attempt: u32) -> bool {
        let entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        entries.get(operation_id).is_some_and(|entry| {
            // A retry must never revive execution belonging to the old attempt.
            entry.status.attempt != attempt
                || entry.cancel_requested
                || entry.status.phase == ProjectEnvironmentSetupPhase::Cancelled
        })
    }

    pub(super) fn update<F>(&self, operation_id: &str, attempt: u32, update: F) -> bool
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
            || entry.cancel_requested
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

    pub(super) fn mark_failed(&self, operation_id: &str, attempt: u32, code: &str, message: &str) {
        self.mark_failed_with_retryability(operation_id, attempt, code, message, true);
    }

    pub(super) fn mark_failed_non_retryable(
        &self,
        operation_id: &str,
        attempt: u32,
        code: &str,
        message: &str,
    ) {
        self.mark_failed_with_retryability(operation_id, attempt, code, message, false);
    }

    fn mark_failed_with_retryability(
        &self,
        operation_id: &str,
        attempt: u32,
        code: &str,
        message: &str,
        retryable: bool,
    ) {
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let Some(entry) = entries.get_mut(operation_id) else {
            return;
        };
        if entry.status.attempt != attempt
            || entry.cancel_requested
            || matches!(
                entry.status.phase,
                ProjectEnvironmentSetupPhase::Ready | ProjectEnvironmentSetupPhase::Cancelled
            )
        {
            return;
        }
        entry.status.phase = ProjectEnvironmentSetupPhase::Failed;
        entry.status.progress_percent = 0;
        entry.status.message = Some(message.to_string());
        entry.status.failure_code = Some(code.to_string());
        entry.status.failure_message = Some(message.to_string());
        entry.status.retryable = retryable;
        entry.status.updated_at_ms = crate::session::unix_epoch_ms();
        entry.cancel_requested = false;
        let persisted = entry.clone();
        drop(entries);
        self.persist(&persisted);
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
