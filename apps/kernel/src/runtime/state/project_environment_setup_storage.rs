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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSetupEntry {
    entry: SetupEntry,
}

#[derive(Debug, Clone)]
pub(in crate::runtime::state) struct ProjectEnvironmentSetupStore {
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
    pub(in crate::runtime::state) fn restore_from_durable_state(
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

    pub(super) fn begin(
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
        Ok((entry.execution.clone(), entry.status.clone()))
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
        entry.status = status.clone();
        entry.cancel_requested = status.phase == ProjectEnvironmentSetupPhase::Cancelled;
        let persisted = entry.clone();
        drop(entries);
        self.persist(&persisted);
        Ok(status)
    }

    pub(super) fn cancel(
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
        let mut entries = self
            .entries
            .lock()
            .expect("setup state lock should not be poisoned");
        let Some(entry) = entries.get_mut(operation_id) else {
            return;
        };
        if entry.status.attempt != attempt
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
        entry.status.retryable = true;
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
