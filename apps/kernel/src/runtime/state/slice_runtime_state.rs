use super::*;

#[derive(Debug, Clone)]
pub(crate) struct SliceAgentRelaunchManifest {
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) owner_user_id: String,
    pub(crate) source_remote_execution: crate::agent::RemoteAgentBinding,
    pub(crate) adapter_key: String,
    pub(crate) provider: String,
    pub(crate) account_profile: String,
    pub(crate) model: String,
    pub(crate) variant: Option<String>,
    pub(crate) execution_mode: crate::provider::AgentExecutionMode,
    pub(crate) permission_level: crate::provider::AgentPermissionLevel,
    pub(crate) structured_endpoint: Option<String>,
    pub(crate) provider_session_id: Option<String>,
    pub(crate) existing_provider_run_id: Option<String>,
}

impl KernelRuntimeState {
    pub(crate) fn list_slices(&self) -> Vec<crate::slice::SliceRecord> {
        self.owned.slice_store.list()
    }

    pub(crate) async fn create_slice(
        &self,
        request: crate::local::CreateSliceRequest,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.validate_slice_development_selection(
            request.development.as_ref(),
            &request.backend,
            request.workspace_id.as_deref(),
            request.worktree_id.as_deref(),
        )?;
        let config = self.owned.config_projection.snapshot();
        let development_storage_parent = matches!(
            request.development.as_ref(),
            Some(
                crate::managed_context::package::ManagedContextDevelopmentSelection::SourceProject {
                    ..
                }
            )
        )
        .then(|| self.prepare_slice_development_storage_parent(&config))
        .transpose()?;
        let from_saved_state = match request.from_saved_state.as_deref() {
            Some(state_ref) => Some(self.owned.slice_store.saved_state(state_ref)?),
            None if request.base == Some(crate::local::SliceCreateBase::Clean) => None,
            None => crate::slice::default_local_docker_saved_state(
                &crate::slice::LocalDockerSliceOptions::from_config(&config),
                request.backend.clone(),
                &request.os,
            )?,
        };
        let mut slice = self.owned.slice_store.create(
            &config.daemon_id,
            &config.host_machine_id,
            crate::slice::CreateSliceInput {
                name: request.name,
                backend: request.backend,
                os: request.os,
                display_mode: request.display_mode,
                display_backend: request.display_backend,
                workspace_id: request.workspace_id,
                worktree_id: request.worktree_id,
                workspace_mount: request.workspace_mount,
                development: request.development,
                worker_kernel_ref: request.worker_kernel_ref,
                display_url: request.display_url,
                provider_auth: Vec::new(),
                from_saved_state,
                now_ms: crate::session::unix_epoch_ms(),
            },
        )?;
        if let Some(storage_parent) = development_storage_parent {
            let storage_root = storage_parent.join(&slice.id);
            slice = self.owned.slice_store.set_development_storage_root(
                &slice.id,
                storage_root
                    .to_str()
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "slice.development.storage",
                        message: "slice development storage path is not portable UTF-8".to_string(),
                    })?
                    .to_string(),
                crate::session::unix_epoch_ms(),
            )?;
        }
        self.append_slice_durable_event("slice.created", &slice)?;
        self.record_slice_audit_event(&slice, "create", "completed", None, None)?;
        Ok(slice)
    }

    pub(crate) fn resolve_slice(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.resolve(slice_ref)
    }

    pub(crate) async fn reconcile_slice_agent_attachments(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let mut current = slice.clone();
        let missing_attachments = self
            .owned
            .agent_store
            .list_agents()
            .into_iter()
            .filter_map(|agent| {
                let remote = agent.remote_execution()?;
                let targets_slice = remote.worker_machine_id == format!("slice:{}", slice.id)
                    || slice.worker_kernel_id.as_deref() == Some(remote.worker_kernel_id.as_str())
                    || slice.worker_kernel_ref == remote.worker_kernel_id;
                (targets_slice
                    && !slice
                        .agent_ids
                        .iter()
                        .any(|agent_id| agent_id == agent.id()))
                .then(|| crate::slice::SliceAgentAttachment {
                    slice_ref: slice.id.clone(),
                    session_id: agent.session_id().to_string(),
                    agent_id: agent.id().to_string(),
                })
            })
            .collect::<Vec<_>>();
        for attached in self
            .owned
            .slice_store
            .attach_agents(missing_attachments, crate::session::unix_epoch_ms())?
        {
            current = attached;
            self.append_slice_durable_event("slice.updated", &current)?;
        }

        for agent_id in current.agent_ids.clone() {
            let matches_canonical_agent = self
                .owned
                .agent_store
                .get_agent(&agent_id)
                .ok()
                .is_some_and(|agent| {
                    let session_exists = self
                        .owned
                        .session_store
                        .get_session(agent.session_id())
                        .is_ok();
                    let remote = agent.remote_execution();
                    session_exists
                        && remote.is_some_and(|remote| {
                            let live_worker_identity_available = current.worker_kernel_id.is_some()
                                || current.worker_machine_id.is_some();
                            (!live_worker_identity_available
                                && current.status != crate::slice::SliceStatus::Running)
                                || remote.worker_machine_id == format!("slice:{}", current.id)
                                || current.worker_kernel_id.as_deref()
                                    == Some(remote.worker_kernel_id.as_str())
                                || current.worker_kernel_ref == remote.worker_kernel_id
                        })
                });
            if !matches_canonical_agent {
                current = self.owned.slice_store.detach_agent(
                    &current.id,
                    &agent_id,
                    crate::session::unix_epoch_ms(),
                )?;
                self.append_slice_durable_event("slice.updated", &current)?;
            }
        }
        Ok(current)
    }

    pub(crate) fn slice_agent_relaunch_manifests(
        &self,
        slice: &crate::slice::SliceRecord,
        operation: &'static str,
    ) -> Result<Vec<SliceAgentRelaunchManifest>, DaemonError> {
        let mut busy_agents = Vec::new();
        let mut manifests = Vec::new();
        for agent_id in &slice.agent_ids {
            let agent = self.owned.agent_store.get_agent(agent_id)?;
            let session = self.owned.session_store.get_session(agent.session_id())?;
            let source_remote_execution = agent.remote_execution().cloned().ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation,
                    message: format!(
                        "cannot relaunch slice agent `{}` because its remote execution binding is missing; detach the stale agent or start it again",
                        agent.id()
                    ),
                }
            })?;
            let effective_config =
                crate::session::effective_agent_execution_config(&session, Some(&agent));
            if self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent.id())
                .is_some()
            {
                busy_agents.push(agent.id().to_string());
                continue;
            }
            let projected_run = self
                .owned
                .provider_store
                .get_run_for_agent(session.id(), agent.id())
                .or_else(|| {
                    self.owned
                        .provider_run_projection
                        .get_for_agent(session.id(), agent.id())
                });
            let manifest = if let Some(run) = projected_run {
                if run.state() == crate::provider::ProviderRunState::Starting {
                    busy_agents.push(agent.id().to_string());
                    continue;
                }
                SliceAgentRelaunchManifest {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    owner_user_id: agent.owner_user_id().to_string(),
                    source_remote_execution: source_remote_execution.clone(),
                    adapter_key: run.adapter_key().to_string(),
                    provider: run.provider().to_string(),
                    account_profile: run.account_profile().to_string(),
                    model: run.model().to_string(),
                    variant: run.variant().or_else(|| agent.effort()).map(str::to_string),
                    execution_mode: effective_config.mode,
                    permission_level: effective_config.permission_level,
                    // A slice restart must spawn a fresh managed provider process inside the
                    // restarted worker. The previous structured endpoint is worker-local and
                    // points at the provider server that was stopped with the old slice.
                    structured_endpoint: None,
                    provider_session_id: run
                        .provider_session_id()
                        .or_else(|| run.resume_state().provider_session_id(run.adapter_key()))
                        .map(str::to_string),
                    existing_provider_run_id: Some(run.id().to_string()),
                }
            } else {
                let adapter_key =
                    crate::provider::adapter_key_for_provider(agent.provider()).to_string();
                SliceAgentRelaunchManifest {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    owner_user_id: agent.owner_user_id().to_string(),
                    source_remote_execution,
                    adapter_key: adapter_key.clone(),
                    provider: agent.provider().to_string(),
                    account_profile: agent.provider_account_profile().to_string(),
                    model: agent.model().unwrap_or("default").to_string(),
                    variant: agent.effort().map(str::to_string),
                    execution_mode: effective_config.mode,
                    permission_level: effective_config.permission_level,
                    structured_endpoint: None,
                    provider_session_id: agent
                        .provider_resume_state()
                        .provider_session_id(&adapter_key)
                        .map(str::to_string),
                    existing_provider_run_id: None,
                }
            };
            manifests.push(manifest);
        }
        if !busy_agents.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation,
                message: busy_slice_agents_message(operation, &busy_agents),
            });
        }
        Ok(manifests)
    }

    pub(crate) async fn park_slice_agent_provider_runs(
        &self,
        manifests: &[SliceAgentRelaunchManifest],
    ) -> Result<(), DaemonError> {
        for manifest in manifests {
            if let Some(run_id) = manifest.existing_provider_run_id.as_deref() {
                if let Ok(run) = self.owned.provider_store.get_run(run_id) {
                    if run.state() != crate::provider::ProviderRunState::Ended {
                        if let Ok(outcome) = self
                            .owned
                            .provider_store
                            .terminate_run_provider_only(&manifest.session_id, run_id)
                        {
                            self.owned.clear_active_provider_run_session_pointer(
                                &manifest.session_id,
                                outcome.run().id(),
                            )?;
                            self.owned
                                .provider_run_projection
                                .update(outcome.into_run());
                        }
                    }
                    let remove_run_id = run_id.to_string();
                    let (_, process_key) = self
                        .with_app_side_effect(|app| {
                            crate::app::ProviderLaunchProcessRuntime::new(app)
                                .remove_run(&remove_run_id)
                        })
                        .await
                        .unwrap_or((false, None));
                    self.owned
                        .remove_provider_process_tracking_for_run(run_id, process_key);
                }
            }
            let _ = self
                .owned
                .agent_store
                .set_remote_execution_active_worker_provider_run_id(&manifest.agent_id, None)?;
        }
        Ok(())
    }

    pub(crate) async fn rebind_and_relaunch_slice_agents(
        &self,
        manifests: Vec<SliceAgentRelaunchManifest>,
        worker: &chariox_relay::protocol::RelayKernelPresence,
    ) -> Result<(), DaemonError> {
        for manifest in manifests {
            let source_agent = self.owned.agent_store.get_agent(&manifest.agent_id)?;
            let source_session = self.owned.session_store.get_session(&manifest.session_id)?;
            let current_binding = source_agent.remote_execution().ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "slice.agent.relaunch",
                    message: format!(
                        "cannot relaunch slice agent `{}` because its remote execution binding disappeared; retry /slice start",
                        manifest.agent_id
                    ),
                }
            })?;
            if !same_slice_relaunch_source_binding(
                current_binding,
                &manifest.source_remote_execution,
            ) {
                return Err(DaemonError::LocalTransport {
                    operation: "slice.agent.relaunch",
                    message: format!(
                        "cannot relaunch slice agent `{}` because its remote execution binding changed; retry /slice start",
                        manifest.agent_id
                    ),
                });
            }
            let current_config = crate::session::effective_agent_execution_config(
                &source_session,
                Some(&source_agent),
            );
            if current_config.mode != manifest.execution_mode
                || current_config.permission_level != manifest.permission_level
            {
                return Err(DaemonError::LocalTransport {
                    operation: "slice.agent.relaunch",
                    message: format!(
                        "cannot relaunch slice agent `{}` because its execution permissions changed; retry /slice start",
                        manifest.agent_id
                    ),
                });
            }
            let agent_id = manifest.agent_id.clone();
            let worker = worker.clone();
            let rebound = self
                .with_app_side_effect(move |app| {
                    app.refresh_remote_agent_binding_to_worker_kernel(&agent_id, &worker)
                })
                .await?;
            self.append_agent_durable_event("agent.remote_binding_refreshed", &rebound, None)
                .await?;

            let request = crate::local::LaunchProviderRunRequest {
                session_id: manifest.session_id.clone(),
                agent_id: Some(manifest.agent_id.clone()),
                adapter_key: manifest.adapter_key.clone(),
                provider: manifest.provider.clone(),
                account_profile: manifest.account_profile.clone(),
                model: manifest.model.clone(),
                variant: manifest.variant.clone(),
                structured_endpoint: manifest.structured_endpoint.clone(),
                provider_session_id: manifest.provider_session_id.clone(),
                native_tui: true,
            };
            let _ = self
                .launch_remote_native_provider_run(&request, &manifest.owner_user_id)
                .await?;
        }
        Ok(())
    }

    pub(crate) fn begin_slice_operation(
        &self,
        slice_ref: &str,
        operation: &'static str,
    ) -> Result<crate::slice::SliceOperationGuard, DaemonError> {
        self.owned
            .slice_store
            .try_begin_operation(slice_ref, operation)
    }

    pub(crate) async fn ensure_slice_private_relay_home_connection(
        &self,
        slice_id: &str,
        relay_url: String,
        relay_token: String,
    ) -> Result<(), DaemonError> {
        let mut relay_config = self.owned.config_projection.snapshot();
        relay_config.apply_remote_relay_override(relay_url.clone(), relay_token.clone());
        let home_kernel_id = relay_config.daemon_id.clone();
        let state = {
            let mut connectors = self.owned.slice_private_relay_connectors.lock().await;
            if let Some(existing) = connectors.get(slice_id) {
                if existing.relay_url == relay_url
                    && self
                        .slice_private_relay_home_is_visible(
                            &relay_config,
                            &home_kernel_id,
                            &relay_url,
                            &existing.state,
                        )
                        .await
                {
                    return Ok(());
                }
                let existing = connectors.remove(slice_id).expect("connector existed");
                let _ = existing.shutdown_tx.send(true);
            }

            let state = Arc::new(tokio::sync::RwLock::new(
                crate::transport::relay_client::RelayClientState::default(),
            ));
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let app = Arc::clone(&self.app);
            let task_state = Arc::clone(&state);
            let task_slice_id = slice_id.to_string();
            let task_relay_url = relay_url.clone();
            let task_relay_token = relay_token;
            let task = std::thread::spawn(move || {
                crate::logging::info_with_fields(
                    "daemon.slice_private_relay",
                    "home connector thread starting",
                    serde_json::json!({
                        "slice_id": task_slice_id.clone(),
                        "relay_url": task_relay_url.clone(),
                    }),
                );
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "slice.private_relay",
                            "failed to start private relay runtime",
                            serde_json::json!({
                                "error": error.to_string(),
                            }),
                        );
                        return;
                    }
                };
                runtime.block_on(
                    crate::transport::relay_client::run_daemon_relay_connector_with_static_relay(
                        app,
                        task_state,
                        shutdown_rx,
                        task_relay_url.clone(),
                        task_relay_token,
                    ),
                );
                crate::logging::info_with_fields(
                    "daemon.slice_private_relay",
                    "home connector thread exited",
                    serde_json::json!({
                        "slice_id": task_slice_id,
                        "relay_url": task_relay_url.clone(),
                    }),
                );
            });
            connectors.insert(
                slice_id.to_string(),
                SlicePrivateRelayConnector {
                    relay_url: relay_url.clone(),
                    state: Arc::clone(&state),
                    shutdown_tx,
                    task,
                },
            );
            state
        };

        for _ in 0..150 {
            if self
                .slice_private_relay_home_is_visible(
                    &relay_config,
                    &home_kernel_id,
                    &relay_url,
                    &state,
                )
                .await
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let (state_connected, connector_task_finished) = {
            let state_connected = state.read().await.connected();
            let connectors = self.owned.slice_private_relay_connectors.lock().await;
            let connector_task_finished = connectors
                .get(slice_id)
                .map(|connector| connector.task.is_finished())
                .unwrap_or(true);
            (state_connected, connector_task_finished)
        };
        Err(DaemonError::LocalTransport {
            operation: "slice.private_relay.home_connect",
            message: format!(
                "home kernel did not attach to private relay `{relay_url}` (state_connected={state_connected}, connector_task_finished={connector_task_finished})"
            ),
        })
    }

    async fn slice_private_relay_home_is_visible(
        &self,
        relay_config: &crate::config::DaemonConfig,
        home_kernel_id: &str,
        relay_url: &str,
        state: &Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>,
    ) -> bool {
        if state.read().await.connected_relay_url().as_deref() != Some(relay_url) {
            return false;
        }
        crate::transport::relay_discovery::get_live_kernel(relay_config, home_kernel_id)
            .await
            .is_ok()
    }

    pub(crate) async fn stop_slice_private_relay_home_connection(&self, slice_id: &str) {
        let connector = {
            let mut connectors = self.owned.slice_private_relay_connectors.lock().await;
            connectors.remove(slice_id)
        };
        let Some(connector) = connector else {
            return;
        };
        let _ = connector.shutdown_tx.send(true);
        drop(connector.task);
    }

    pub(crate) fn record_slice_audit_event(
        &self,
        slice: &crate::slice::SliceRecord,
        action: &'static str,
        outcome: &'static str,
        provider: Option<&str>,
        message: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            "slice.audit",
            Some(slice.id.clone()),
            serde_json::json!({
                "slice_id": slice.id,
                "slice_name": slice.name,
                "action": action,
                "outcome": outcome,
                "result": outcome,
                "actor": "kernel",
                "client_type": "local_daemon",
                "provider": provider,
                "message": message,
                "redacted_error": if outcome == "failed" { message } else { None },
                "status": slice.status,
                "backend": slice.backend,
                "display_mode": slice.display_mode,
                "workspace_id": slice.workspace_id,
                "worktree_id": slice.worktree_id,
                "workspace_mount": slice.workspace_mount,
                "session_ids": slice.session_ids,
                "agent_ids": slice.agent_ids,
                "owner_kernel_id": slice.owner_kernel_id,
                "owner_machine_id": slice.owner_machine_id,
                "worker_kernel_ref": slice.worker_kernel_ref,
                "worker_kernel_id": slice.worker_kernel_id,
                "worker_machine_id": slice.worker_machine_id,
                "at_ms": crate::session::unix_epoch_ms(),
            }),
        )?;
        Ok(())
    }

    pub(crate) fn mark_slice_starting(
        &self,
        slice_ref: &str,
        relay_endpoint: crate::slice::SliceRelayEndpoint,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.update_slice_operation(
            slice_ref,
            "start",
            crate::slice::SliceOperationStatus::InProgress,
            None,
        )?;
        self.owned.slice_store.set_relay_endpoint(
            slice_ref,
            Some(relay_endpoint),
            crate::session::unix_epoch_ms(),
        )?;
        let slice = self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Starting,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_stopping(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.update_slice_operation(
            slice_ref,
            "stop",
            crate::slice::SliceOperationStatus::InProgress,
            None,
        )?;
        let slice = self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Stopping,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_operation_failed(
        &self,
        slice_ref: &str,
        operation: &'static str,
        error: &DaemonError,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Unhealthy,
            crate::session::unix_epoch_ms(),
        )?;
        let slice = self.owned.slice_store.set_operation_diagnostics(
            slice_ref,
            operation,
            crate::slice::SliceOperationStatus::Failed,
            Some(&error.to_string()),
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_operation_rejected(
        &self,
        slice_ref: &str,
        operation: &'static str,
        error: &DaemonError,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.update_slice_operation(
            slice_ref,
            operation,
            crate::slice::SliceOperationStatus::Failed,
            Some(&error.to_string()),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_running(
        &self,
        slice_ref: &str,
        worker: Option<chariox_relay::protocol::RelayKernelPresence>,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )?;
        if let Some(worker) = worker {
            self.owned.slice_store.set_worker_presence(
                slice_ref,
                Some(worker.kernel_id),
                Some(worker.machine_id),
                worker.available_providers,
                crate::session::unix_epoch_ms(),
            )?;
        }
        let slice = self.owned.slice_store.set_operation_diagnostics(
            slice_ref,
            "start",
            crate::slice::SliceOperationStatus::Completed,
            None,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn claim_slice_starting_worker_identity(
        &self,
        slice_ref: &str,
        worker_kernel_id: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.claim_starting_worker_identity(
            slice_ref,
            worker_kernel_id,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_stopped(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.set_status(
            slice_ref,
            crate::slice::SliceStatus::Stopped,
            crate::session::unix_epoch_ms(),
        )?;
        let slice = self.owned.slice_store.set_operation_diagnostics(
            slice_ref,
            "stop",
            crate::slice::SliceOperationStatus::Completed,
            None,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_delete_in_progress(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.update_slice_operation(
            slice_ref,
            "delete",
            crate::slice::SliceOperationStatus::InProgress,
            None,
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_delete_failed(
        &self,
        slice_ref: &str,
        error: &DaemonError,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.update_slice_operation(
            slice_ref,
            "delete",
            crate::slice::SliceOperationStatus::Failed,
            Some(&error.to_string()),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn set_slice_provider_auth(
        &self,
        slice_ref: &str,
        provider_auth: Vec<crate::slice_provider_auth::SliceProviderAuthSummary>,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.set_provider_auth(
            slice_ref,
            provider_auth,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn delete_slice(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.delete(slice_ref)?;
        self.append_slice_durable_event("slice.deleted", &slice)?;
        Ok(slice)
    }

    fn update_slice_operation(
        &self,
        slice_ref: &str,
        operation: &'static str,
        status: crate::slice::SliceOperationStatus,
        error: Option<&str>,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        self.owned.slice_store.set_operation_diagnostics(
            slice_ref,
            operation,
            status,
            error,
            crate::session::unix_epoch_ms(),
        )
    }

    pub(crate) fn slice_display_endpoint(
        &self,
        slice_ref: &str,
    ) -> Result<crate::slice::SliceDisplayEndpoint, DaemonError> {
        self.owned.slice_store.display_endpoint(slice_ref)
    }

    pub(crate) fn list_slice_audit_events(
        &self,
        slice_ref: &str,
        limit: Option<usize>,
    ) -> Result<Vec<crate::durable_state::DurableStateEvent>, DaemonError> {
        let slice = self.resolve_slice(slice_ref)?;
        let limit = limit.unwrap_or(50);
        self.owned
            .durable_state_store
            .load_subject_events_by_kind(&slice.id, "slice.audit", limit)
    }

    pub(crate) fn active_saved_state_for_slice(
        &self,
        slice_ref: &str,
    ) -> Result<Option<crate::slice::SliceSavedStateRecord>, DaemonError> {
        self.owned
            .slice_store
            .active_saved_state_for_slice(slice_ref)
    }

    pub(crate) fn save_slice_state_record(
        &self,
        slice_ref: &str,
        state: crate::slice::SliceSavedStateRecord,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.upsert_saved_state(
            slice_ref,
            state.clone(),
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        self.owned.durable_state_store.append_event(
            "slice.state.saved",
            Some(state.id.clone()),
            serde_json::json!({ "state": state }),
        )?;
        Ok(slice)
    }

    pub(crate) fn mark_slice_state_save_failed(
        &self,
        slice_ref: &str,
        error: &DaemonError,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.mark_saved_state_failed(
            slice_ref,
            error,
            crate::session::unix_epoch_ms(),
        )?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        Ok(slice)
    }

    pub(crate) fn reset_slice_state_record(
        &self,
        slice_ref: &str,
    ) -> Result<
        (
            crate::slice::SliceRecord,
            Option<crate::slice::SliceSavedStateRecord>,
        ),
        DaemonError,
    > {
        let (slice, removed_state) = self
            .owned
            .slice_store
            .reset_saved_state(slice_ref, crate::session::unix_epoch_ms())?;
        self.append_slice_durable_event("slice.updated", &slice)?;
        if let Some(state) = &removed_state {
            self.owned.durable_state_store.append_event(
                "slice.state.deleted",
                Some(state.id.clone()),
                serde_json::json!({ "state": state }),
            )?;
        }
        Ok((slice, removed_state))
    }

    pub(crate) fn save_slice_backup_record(
        &self,
        backup: crate::slice::SliceBackupRecord,
    ) -> Result<crate::slice::SliceBackupRecord, DaemonError> {
        let backup = self.owned.slice_store.upsert_backup(backup);
        self.owned.durable_state_store.append_event(
            "slice.backup.created",
            Some(backup.id.clone()),
            serde_json::json!({ "backup": backup }),
        )?;
        Ok(backup)
    }

    pub(super) fn append_slice_durable_event(
        &self,
        kind: &'static str,
        slice: &crate::slice::SliceRecord,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            kind,
            Some(slice.id.clone()),
            serde_json::json!({ "slice": slice }),
        )?;
        self.owned.runtime_projection_changes.record_change();
        Ok(())
    }
}

fn same_slice_relaunch_source_binding(
    current: &crate::agent::RemoteAgentBinding,
    captured: &crate::agent::RemoteAgentBinding,
) -> bool {
    current.worker_kernel_id == captured.worker_kernel_id
        && current.worker_machine_id == captured.worker_machine_id
        && current.execution_lease_id == captured.execution_lease_id
        && current.leased_agent_id == captured.leased_agent_id
        && current.relay_url == captured.relay_url
        && current.relay_token == captured.relay_token
        && current.relay_peer_protocol_version == captured.relay_peer_protocol_version
}

fn busy_slice_agents_message(operation: &'static str, agent_ids: &[String]) -> String {
    let action = if operation == "slice.state.save" {
        "save"
    } else {
        "start"
    };
    format!(
        "cannot {action} slice while agents are running; wait for them to finish or stop them: {}",
        agent_ids.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_busy_agent_error_matches_the_user_recovery_contract() {
        assert_eq!(
            busy_slice_agents_message(
                "slice.state.save",
                &["agent-1".to_string(), "agent-2".to_string()]
            ),
            "cannot save slice while agents are running; wait for them to finish or stop them: agent-1,agent-2"
        );
    }

    #[test]
    fn relaunch_binding_comparison_ignores_only_the_parked_provider_run() {
        let captured = crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker-1".to_string(),
            worker_machine_id: "slice:slice-1".to_string(),
            execution_lease_id: "lease-1".to_string(),
            leased_agent_id: "leased-agent-1".to_string(),
            active_worker_provider_run_id: Some("run-before-save".to_string()),
            relay_url: Some("ws://127.0.0.1:4100".to_string()),
            relay_token: Some("test-token".to_string()),
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        let mut parked = captured.clone();
        parked.active_worker_provider_run_id = None;
        assert!(same_slice_relaunch_source_binding(&parked, &captured));

        let mut changed = parked;
        changed.execution_lease_id = "different-lease".to_string();
        assert!(!same_slice_relaunch_source_binding(&changed, &captured));
    }

    #[tokio::test]
    async fn relaunch_manifests_ignore_stale_legacy_agent_busy_state() {
        let (_app, runtime, slice, _session_id, agent_id) = slice_runtime().await;

        runtime
            .owned
            .agent_store
            .set_agent_runtime_profile_with_account_profile(
                &agent_id,
                "codex",
                Some("gpt-5.6-sol".to_string()),
                Some("low".to_string()),
                Some("work".to_string()),
                crate::provider::ProviderResumeState::from_codex_thread_id("thread-before-save"),
            )
            .expect("agent runtime profile should update");
        runtime
            .owned
            .agent_store
            .update_agent_config(
                &agent_id,
                Some(Some(crate::provider::AgentExecutionMode::Plan)),
                Some(Some(crate::provider::AgentPermissionLevel::Required)),
                None,
                None,
            )
            .expect("agent execution config should update");
        runtime
            .owned
            .agent_store
            .set_agent_processing(&agent_id, true)
            .expect("stale processing flag should be set");
        runtime
            .owned
            .agent_store
            .set_agent_state(&agent_id, crate::agent::AgentState::Working)
            .expect("stale working state should be set");

        let manifests = runtime
            .slice_agent_relaunch_manifests(&slice, "slice.start")
            .expect("stale legacy state should not block relaunch manifests");

        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].agent_id, agent_id);
        assert_eq!(manifests[0].adapter_key, "codex");
        assert_eq!(manifests[0].provider, "codex");
        assert_eq!(manifests[0].account_profile, "work");
        assert_eq!(manifests[0].model, "gpt-5.6-sol");
        assert_eq!(manifests[0].variant.as_deref(), Some("low"));
        assert_eq!(
            manifests[0].provider_session_id.as_deref(),
            Some("thread-before-save")
        );
        assert_eq!(
            manifests[0].source_remote_execution.worker_machine_id,
            format!("slice:{}", slice.id)
        );
        assert_eq!(
            manifests[0].execution_mode,
            crate::provider::AgentExecutionMode::Plan
        );
        assert_eq!(
            manifests[0].permission_level,
            crate::provider::AgentPermissionLevel::Required
        );
    }

    #[tokio::test]
    async fn relaunch_manifests_block_when_prompt_owner_has_active_prompt() {
        let (app, runtime, slice, session_id, agent_id) = slice_runtime().await;
        sync_active_prompt(&app, &session_id, &agent_id).await;

        let error = runtime
            .slice_agent_relaunch_manifests(&slice, "slice.start")
            .expect_err("active prompt ownership should block relaunch manifests");

        match error {
            DaemonError::LocalTransport { operation, message } => {
                assert_eq!(operation, "slice.start");
                assert!(message.contains("cannot start slice while agents are running"));
                assert!(message.contains(&agent_id));
            }
            other => panic!("expected active prompt ownership error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn relaunch_manifests_preserve_projected_provider_selection_after_parking() {
        let (_app, runtime, slice, session_id, agent_id) = slice_runtime().await;
        runtime
            .owned
            .agent_store
            .update_agent_config(
                &agent_id,
                Some(Some(crate::provider::AgentExecutionMode::Plan)),
                Some(Some(crate::provider::AgentPermissionLevel::Required)),
                None,
                None,
            )
            .expect("agent execution config should update");
        let request = crate::provider::LaunchProviderRequest::new(
            &session_id,
            "codex",
            "codex",
            "work",
            "gpt-5.6-sol",
        )
        .with_agent_id(&agent_id)
        .with_variant(Some("high".to_string()))
        .with_resume_state(crate::provider::ProviderResumeState::from_codex_thread_id(
            "thread-projected",
        ))
        .with_execution_mode(crate::provider::AgentExecutionMode::Plan)
        .with_permission_level(crate::provider::AgentPermissionLevel::Required)
        .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
        let mut projected = crate::provider::RuntimeProviderRun::new(
            "projected-run",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "projected-codex".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: Default::default(),
                pty_env_remove: Vec::new(),
                working_directory: Some(std::path::PathBuf::from("/workspace")),
                structured_endpoint: Some("http://old-worker.invalid".to_string()),
            },
        );
        projected.mark_running();
        runtime.owned.provider_run_projection.update(projected);

        let manifests = runtime
            .slice_agent_relaunch_manifests(&slice, "slice.state.save")
            .expect("active projected provider should be captured");
        runtime
            .park_slice_agent_provider_runs(&manifests)
            .await
            .expect("projected provider should park for slice shutdown");
        let recaptured = runtime
            .slice_agent_relaunch_manifests(&slice, "slice.start")
            .expect("parked projected provider selection should remain recoverable");

        assert_eq!(recaptured.len(), 1);
        let manifest = &recaptured[0];
        assert_eq!(manifest.adapter_key, "codex");
        assert_eq!(manifest.provider, "codex");
        assert_eq!(manifest.account_profile, "work");
        assert_eq!(manifest.model, "gpt-5.6-sol");
        assert_eq!(manifest.variant.as_deref(), Some("high"));
        assert_eq!(
            manifest.provider_session_id.as_deref(),
            Some("thread-projected")
        );
        assert_eq!(
            manifest.existing_provider_run_id.as_deref(),
            Some("projected-run")
        );
        assert_eq!(
            manifest.execution_mode,
            crate::provider::AgentExecutionMode::Plan
        );
        assert_eq!(
            manifest.permission_level,
            crate::provider::AgentPermissionLevel::Required
        );
    }

    #[tokio::test]
    async fn stopped_slice_reconciliation_uses_canonical_session_without_projection() {
        let (_app, runtime, slice, session_id, agent_id) = slice_runtime().await;
        runtime
            .owned
            .agent_store
            .bind_remote_execution(
                &agent_id,
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "previous-worker-kernel".to_string(),
                    worker_machine_id: "slice:previous-worker".to_string(),
                    execution_lease_id: "previous-lease".to_string(),
                    leased_agent_id: "previous-leased-agent".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("remote execution should bind");
        runtime.owned.session_projection.remove(&session_id);

        let reconciled = runtime
            .reconcile_slice_agent_attachments(&slice)
            .await
            .expect("stopped slice attachments should reconcile");

        assert_eq!(reconciled.agent_ids, vec![agent_id]);
    }

    #[tokio::test]
    async fn slice_reconciliation_restores_missing_canonical_remote_attachment() {
        let (_app, runtime, slice, session_id, agent_id) = slice_runtime().await;
        runtime
            .owned
            .agent_store
            .bind_remote_execution(
                &agent_id,
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: format!("slice:{}", slice.id),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("remote execution should bind");
        let detached = runtime
            .owned
            .slice_store
            .detach_agent(&slice.id, &agent_id, 3)
            .expect("attachment should detach for the regression setup");
        runtime.owned.session_projection.remove(&session_id);

        let reconciled = runtime
            .reconcile_slice_agent_attachments(&detached)
            .await
            .expect("missing canonical attachment should reconcile");

        assert_eq!(reconciled.session_ids, vec![session_id]);
        assert_eq!(reconciled.agent_ids, vec![agent_id]);
    }

    async fn slice_runtime() -> (
        Arc<Mutex<DaemonApp>>,
        KernelRuntimeState,
        crate::slice::SliceRecord,
        String,
        String,
    ) {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let slice = app
            .slices()
            .create(
                "owner-kernel-1",
                "owner-machine-1",
                crate::slice::CreateSliceInput {
                    name: "slice-1".to_string(),
                    backend: crate::slice::SliceBackendKind::LocalDocker,
                    os: "linux".to_string(),
                    display_mode: crate::slice::SliceDisplayMode::Headless,
                    display_backend: Default::default(),
                    workspace_id: Some("workspace-1".to_string()),
                    worktree_id: Some("worktree-1".to_string()),
                    workspace_mount: None,
                    development: None,
                    worker_kernel_ref: Some("worker-kernel-1".to_string()),
                    display_url: None,
                    provider_auth: Vec::new(),
                    from_saved_state: None,
                    now_ms: 1,
                },
            )
            .expect("slice should be created");
        let slice = app
            .slices()
            .attach_agent(&slice.id, &session_id, &agent_id, 2)
            .expect("agent should attach to slice");
        app.agents_mut()
            .bind_remote_execution(
                &agent_id,
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: format!("slice:{}", slice.id),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("slice agent should bind to its worker");
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        (app, runtime, slice, session_id, agent_id)
    }

    async fn sync_active_prompt(app: &Arc<Mutex<DaemonApp>>, session_id: &str, agent_id: &str) {
        let prompt = crate::session::PromptQueueItem::new(
            "active-prompt",
            "attachment-1",
            agent_id,
            "active prompt",
            crate::session::PromptStatus::Running,
        );
        app.lock()
            .await
            .prompt_owner_sync_external_active_prompt(session_id, agent_id, Some(prompt))
            .expect("active prompt should sync");
    }

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }
}
