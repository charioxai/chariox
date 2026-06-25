use super::*;

#[derive(Debug, Clone)]
pub(crate) struct SliceAgentRelaunchManifest {
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) owner_user_id: String,
    pub(crate) adapter_key: String,
    pub(crate) provider: String,
    pub(crate) account_profile: String,
    pub(crate) model: String,
    pub(crate) variant: Option<String>,
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
        let config = self.owned.config_projection.snapshot();
        let from_saved_state = match request.from_saved_state.as_deref() {
            Some(state_ref) => Some(self.owned.slice_store.saved_state(state_ref)?),
            None if request.base == Some(crate::local::SliceCreateBase::Clean) => None,
            None => crate::slice::default_local_docker_saved_state(
                &crate::slice::LocalDockerSliceOptions::from_config(&config),
                request.backend.clone(),
                &request.os,
            )?,
        };
        let slice = self.owned.slice_store.create(
            &config.daemon_id,
            &config.host_machine_id,
            crate::slice::CreateSliceInput {
                name: request.name,
                backend: request.backend,
                os: request.os,
                display_mode: request.display_mode,
                workspace_id: request.workspace_id,
                worktree_id: request.worktree_id,
                workspace_mount: request.workspace_mount,
                worker_kernel_ref: request.worker_kernel_ref,
                display_url: request.display_url,
                provider_auth: Vec::new(),
                from_saved_state,
                now_ms: crate::session::unix_epoch_ms(),
            },
        )?;
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

    pub(crate) fn reconcile_slice_agent_attachments(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let mut current = slice.clone();
        for agent_id in slice.agent_ids.clone() {
            let detach = match self.owned.agent_store.get_agent(&agent_id) {
                Ok(agent) => {
                    let remote = agent.remote_execution();
                    let matches_slice = remote.is_some_and(|remote| {
                        slice.worker_kernel_id.as_deref() == Some(remote.worker_kernel_id.as_str())
                            || slice.worker_kernel_ref == remote.worker_kernel_id
                    });
                    !matches_slice
                        || self
                            .owned
                            .session_store
                            .get_session(agent.session_id())
                            .is_err()
                }
                Err(_) => true,
            };
            if detach {
                current = self.owned.slice_store.detach_agent(
                    &current.id,
                    &agent_id,
                    crate::session::unix_epoch_ms(),
                )?;
                self.owned.durable_state_store.append_event(
                    "slice.updated",
                    Some(current.id.clone()),
                    serde_json::json!({ "slice": &current }),
                )?;
            }
        }
        Ok(current)
    }

    pub(crate) fn slice_agent_relaunch_manifests(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<Vec<SliceAgentRelaunchManifest>, DaemonError> {
        let mut busy_agents = Vec::new();
        let mut manifests = Vec::new();
        for agent_id in &slice.agent_ids {
            let agent = self.owned.agent_store.get_agent(agent_id)?;
            let session = self.owned.session_store.get_session(agent.session_id())?;
            if agent.is_processing() || agent.state() == crate::agent::AgentState::Working {
                busy_agents.push(agent.id().to_string());
                continue;
            }
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
                    adapter_key: run.adapter_key().to_string(),
                    provider: run.provider().to_string(),
                    account_profile: run.account_profile().to_string(),
                    model: run.model().to_string(),
                    variant: run.variant().map(str::to_string),
                    // A slice restart must spawn a fresh managed provider process inside the
                    // restarted worker. The previous structured endpoint is worker-local and
                    // points at the provider server that was stopped with the old slice.
                    structured_endpoint: None,
                    provider_session_id: run
                        .provider_session_id()
                        .or_else(|| run.resume_state().opencode_session_id())
                        .or_else(|| run.resume_state().codex_thread_id())
                        .or_else(|| run.resume_state().claude_session_id())
                        .or_else(|| run.resume_state().pi_session_id())
                        .map(str::to_string),
                    existing_provider_run_id: Some(run.id().to_string()),
                }
            } else {
                SliceAgentRelaunchManifest {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    owner_user_id: agent.owner_user_id().to_string(),
                    adapter_key: agent.provider().to_string(),
                    provider: agent.provider().to_string(),
                    account_profile: "default".to_string(),
                    model: agent.model().unwrap_or("default").to_string(),
                    variant: None,
                    structured_endpoint: None,
                    provider_session_id: None,
                    existing_provider_run_id: None,
                }
            };
            manifests.push(manifest);
        }
        if !busy_agents.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "slice.state.save",
                message: format!(
                    "cannot save slice while agents are running; wait for them to finish or stop them: {}",
                    busy_agents.join(",")
                ),
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
        worker: &arroba_relay::protocol::RelayKernelPresence,
    ) -> Result<(), DaemonError> {
        for manifest in manifests {
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
        worker: Option<arroba_relay::protocol::RelayKernelPresence>,
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

    pub(crate) fn set_slice_provider_auth_alias(
        &self,
        slice_ref: &str,
        provider: &str,
        alias: Option<&str>,
    ) -> Result<crate::slice::SliceRecord, DaemonError> {
        let slice = self.owned.slice_store.set_provider_auth_alias(
            slice_ref,
            provider,
            alias,
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

    fn append_slice_durable_event(
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
