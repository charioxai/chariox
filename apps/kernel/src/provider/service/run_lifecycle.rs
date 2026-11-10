use crate::error::DaemonError;
use crate::provider::AgentEndpointAdapter;

use super::{
    LaunchProviderRequest, ProviderProcessService, ProviderRunEndedOutcome,
    ProviderRunLivenessReconciliation, ProviderRunParkedOutcome, ProviderRunResumedOutcome,
    ProviderRunStartedOutcome, ProviderRunState, ProviderSessionRunsTerminatedOutcome,
    RuntimeProviderRun,
};

#[cfg(test)]
type LaunchHook = Box<dyn FnOnce() + Send>;
#[cfg(test)]
pub(super) static LAUNCH_TEST_HOOK: std::sync::Mutex<Option<LaunchHook>> =
    std::sync::Mutex::new(None);

impl ProviderProcessService {
    pub(crate) fn enroll_local_run_owner(
        &mut self,
        session_id: &str,
        owner: &str,
    ) -> Result<Vec<RuntimeProviderRun>, DaemonError> {
        let mut updated = Vec::new();
        for run in self.runs.values_mut() {
            if run.session_id() == session_id
                && run.owner_user_id() == crate::session::DEFAULT_LOCAL_USER_ID
            {
                run.enroll_local_owner(owner)?;
                updated.push(run.clone());
            }
        }
        Ok(updated)
    }

    pub(crate) fn prepare_run_provider_only(
        registry: super::ProviderRegistry,
        reserved_run_id: String,
        mut request: LaunchProviderRequest,
    ) -> Result<ProviderRunStartedOutcome, DaemonError> {
        if let Some(checkpoint) = request.native_checkpoint.as_ref() {
            request.resume_state = Some(
                checkpoint
                    .prepare_resume_state(&request)
                    .map_err(crate::provider::NativeCheckpointFailure::into_daemon_error)?,
            );
        }
        if let Some(required) = request
            .resume_state
            .as_ref()
            .and_then(|state| state.required_native_session())
        {
            required
                .validate_request(&request)
                .map_err(crate::provider::NativeCheckpointFailure::into_daemon_error)?;
        }
        let adapter = registry.resolve(&request.adapter_key).ok_or_else(|| {
            DaemonError::ProviderAdapterNotFound {
                adapter_key: request.adapter_key.clone(),
            }
        })?;
        if request.requires_workspace_live_sync()
            && !adapter.supports_workspace_live_sync_write_enforcement()
        {
            return Err(DaemonError::ProviderWorkspaceLiveSyncUnsupported {
                adapter_key: request.adapter_key.clone(),
                message: adapter
                    .workspace_live_sync_write_enforcement_unavailable_reason()
                    .to_string(),
            });
        }

        let run_id = reserved_run_id;
        #[cfg(test)]
        {
            let hook = LAUNCH_TEST_HOOK.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let mut launch_result = adapter.connect(&request)?;
        // Everything the kernel starts for the agent writes its scratch in the agent's own folder.
        if let Some(agent_id) = request.agent_id.as_deref() {
            for (name, value) in crate::agent_temp::environment(&request.session_id, agent_id) {
                launch_result
                    .pty_env
                    .entry(name.to_string())
                    .or_insert(value);
            }
        }

        crate::logging::info_with_fields(
            "daemon.provider",
            "provider launch planned",
            serde_json::json!({
                "provider_run_id": run_id,
                "session_id": request.session_id.as_str(),
                "agent_id": request.agent_id.as_deref(),
                "adapter_key": request.adapter_key.as_str(),
                "provider": request.provider.as_str(),
                "model": request.model.as_str(),
                "variant": request.variant.as_deref(),
                "requires_workspace_live_sync": request.requires_workspace_live_sync(),
                "tracks_workspace_live_sync": request.tracks_workspace_live_sync(),
                "runtime_mcp_binding_present": request.runtime_mcp_binding.is_some(),
                "granted_mcp_servers": request
                    .mcp_servers
                    .iter()
                    .map(|server| server.name.as_str())
                    .collect::<Vec<_>>(),
                "endpoint_mode": launch_result.endpoint_mode.to_string(),
                "process_label": launch_result.process_label.as_str(),
                "structured_endpoint": launch_result.structured_endpoint.as_deref(),
                "pty_env_keys": launch_result.pty_env.keys().cloned().collect::<Vec<_>>(),
            }),
        );
        let mut run = RuntimeProviderRun::new(run_id.clone(), &request, launch_result);
        if let Some((home, path)) = request.preparation_environment.clone() {
            run.set_preparation_environment(home, path)?;
        }

        Ok(ProviderRunStartedOutcome { run })
    }

    pub(super) fn commit_prepared_run(
        &mut self,
        mut outcome: ProviderRunStartedOutcome,
    ) -> Result<ProviderRunStartedOutcome, DaemonError> {
        if self.runs.contains_key(outcome.run().id()) {
            return Err(DaemonError::LocalTransport {
                operation: "commit_provider_launch",
                message: "reserved provider run identity is already in use; retry".into(),
            });
        }
        #[cfg(unix)]
        {
            let run = outcome.run();
            if let Some(id) = crate::hot_reload::handed_over_run_id(
                run.session_id(),
                run.agent_instance_id(),
                run.adapter_key(),
            ) {
                if self.runs.contains_key(&id) {
                    return Err(DaemonError::LocalTransport {
                        operation: "commit_provider_launch",
                        message: "handed-over provider run identity is already in use; retry"
                            .into(),
                    });
                }
            }
            if let Some((id, token)) = crate::hot_reload::claim_provider_run(
                run.session_id(),
                run.agent_instance_id(),
                run.adapter_key(),
            ) {
                let endpoint = crate::hot_reload::adopting_codex_endpoint(&id);
                outcome.run.adopt_handed_over_identity(id, token, endpoint);
            }
        }
        self.runs
            .insert(outcome.run().id().to_string(), outcome.run().clone());
        Ok(outcome)
    }

    #[cfg(test)]
    pub(crate) fn start_run_provider_only(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<ProviderRunStartedOutcome, DaemonError> {
        let id = self.next_run_id();
        let outcome = Self::prepare_run_provider_only(self.registry, id, request)?;
        self.commit_prepared_run(outcome)
    }

    #[cfg(test)]
    pub(crate) fn launch_run_detached(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let outcome = self.start_run_provider_only(request)?;
        self.mark_run_running(outcome.run().id())
    }

    pub(crate) fn park_run_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunParkedOutcome, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() == ProviderRunState::Parked {
            return Ok(ProviderRunParkedOutcome { run: run_snapshot });
        }

        if run_snapshot.state() != ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "park",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.park(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_parked();

        Ok(ProviderRunParkedOutcome { run: run.clone() })
    }

    pub(crate) fn resume_run_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunResumedOutcome, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() != ProviderRunState::Parked {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "resume",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.resume(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_running();

        Ok(ProviderRunResumedOutcome { run: run.clone() })
    }

    pub fn resume_run_detached(&mut self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.state() != ProviderRunState::Parked {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "resume",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.resume(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_running();

        Ok(run.clone())
    }

    pub(crate) fn terminate_run_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunEndedOutcome, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;

        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() == ProviderRunState::Ended {
            self.clear_runtime(run_id);
            super::super::mcp_proxy::shutdown_provider_mcp_proxy_run(run_id);
            return Ok(ProviderRunEndedOutcome {
                run: run_snapshot,
                already_ended: true,
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.terminate(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        self.run_actor_mailbox
            .spawn_terminate(run_id.to_string(), run.clone());
        super::super::mcp_proxy::shutdown_provider_mcp_proxy_run(run_id);

        Ok(ProviderRunEndedOutcome {
            run,
            already_ended: false,
        })
    }

    pub fn get_run(&self, run_id: &str) -> Result<RuntimeProviderRun, DaemonError> {
        self.runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| DaemonError::ProviderRunNotFound {
                provider_run_id: run_id.to_string(),
            })
    }

    pub fn list_runs(&self) -> Vec<RuntimeProviderRun> {
        self.runs.values().cloned().collect()
    }

    pub fn record_run_activity(&mut self, run_id: &str) -> Result<(), DaemonError> {
        let run = self.get_run_mut(run_id)?;
        run.touch_activity();
        Ok(())
    }

    pub fn record_run_output(&mut self, run_id: &str) -> Result<(), DaemonError> {
        self.get_run_mut(run_id)?.record_output();
        Ok(())
    }

    pub(crate) fn mark_run_running(
        &mut self,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.get_run_mut(run_id)?;
        if run.state() != ProviderRunState::Starting {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run.state(),
                operation: "finish launch",
            });
        }
        if run.native_checkpoint_needs_import() {
            return Err(crate::provider::NativeCheckpointFailure::ResumeFailed.into_daemon_error());
        }
        run.mark_running();
        Ok(run.clone())
    }

    pub(crate) fn adapter_supports_turn_scoped_execution_config(&self, adapter_key: &str) -> bool {
        self.registry
            .resolve(adapter_key)
            .map(|adapter| adapter.supports_turn_scoped_execution_config())
            .unwrap_or(false)
    }

    pub(crate) fn update_run_execution_config(
        &mut self,
        run_id: &str,
        execution_mode: crate::provider::AgentExecutionMode,
        permission_level: crate::provider::AgentPermissionLevel,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.get_run_mut(run_id)?;
        run.set_execution_config(execution_mode, permission_level);
        Ok(run.clone())
    }

    pub(crate) fn reconcile_run_liveness_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
        process_running: Option<bool>,
    ) -> Result<ProviderRunLivenessReconciliation, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;
        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }

        if run_snapshot.state() == ProviderRunState::Ended {
            self.clear_runtime(run_id);
            return Ok(ProviderRunLivenessReconciliation::AlreadyEnded(
                run_snapshot,
            ));
        }

        if run_snapshot.state() == ProviderRunState::Starting {
            return Ok(ProviderRunLivenessReconciliation::StillRunning(
                run_snapshot,
            ));
        }

        if run_snapshot.endpoint_mode() == crate::provider::AgentEndpointMode::External {
            return Ok(ProviderRunLivenessReconciliation::ExternalEndpoint(
                run_snapshot,
            ));
        }

        let Some(process_running) = process_running else {
            return Ok(ProviderRunLivenessReconciliation::StillRunning(
                run_snapshot,
            ));
        };

        if process_running {
            return Ok(ProviderRunLivenessReconciliation::StillRunning(
                run_snapshot,
            ));
        }

        let ended = self
            .mark_run_ended_provider_only(session_id, run_id)?
            .into_run();
        Ok(ProviderRunLivenessReconciliation::NewlyEnded(ended))
    }

    pub fn get_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .values()
            .filter(|run| {
                run.session_id() == session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.state() != ProviderRunState::Ended
            })
            .max_by(|left, right| left.active_selection_cmp(right))
            .cloned()
    }

    pub fn get_latest_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .values()
            .filter(|run| {
                run.session_id() == session_id && run.agent_instance_id() == Some(agent_id)
            })
            .max_by_key(|run| (run.last_activity_at_ms(), run.started_at_ms()))
            .cloned()
    }

    pub fn get_session_run_for_provider(
        &self,
        session_id: &str,
        provider: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .values()
            .filter(|run| {
                run.session_id() == session_id
                    && run.provider() == provider
                    && run.state() != ProviderRunState::Ended
            })
            .max_by(|left, right| left.active_selection_cmp(right))
            .cloned()
    }

    pub fn get_run_by_runtime_mcp_auth_token(
        &self,
        auth_token: &str,
    ) -> Option<RuntimeProviderRun> {
        self.runs
            .values()
            .find(|run| run.runtime_mcp_auth_token() == Some(auth_token))
            .cloned()
    }

    pub fn get_runs_by_runtime_mcp_auth_token(&self, auth_token: &str) -> Vec<RuntimeProviderRun> {
        self.runs
            .values()
            .filter(|run| {
                run.runtime_mcp_auth_token() == Some(auth_token)
                    && run.state() != ProviderRunState::Ended
            })
            .cloned()
            .collect()
    }

    pub(crate) fn terminate_session_runs_provider_only(
        &mut self,
        session_id: &str,
    ) -> Result<ProviderSessionRunsTerminatedOutcome, DaemonError> {
        *self
            .session_retirement_epochs
            .entry(session_id.to_string())
            .or_default() += 1;
        let run_ids: Vec<String> = self
            .runs
            .values()
            .filter(|run| run.session_id() == session_id && run.state() != ProviderRunState::Ended)
            .map(|run| run.id().to_string())
            .collect();

        let mut terminated_runs = Vec::with_capacity(run_ids.len());

        for run_id in run_ids {
            let outcome = self.terminate_run_provider_only(session_id, &run_id)?;
            terminated_runs.push(outcome);
        }

        Ok(ProviderSessionRunsTerminatedOutcome {
            runs: terminated_runs,
        })
    }

    pub(crate) fn mark_run_ended_provider_only(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Result<ProviderRunEndedOutcome, DaemonError> {
        let run_snapshot = self.get_run(run_id)?;
        if run_snapshot.session_id() != session_id {
            return Err(DaemonError::ProviderRunNotInSession {
                session_id: session_id.to_string(),
                provider_run_id: run_id.to_string(),
            });
        }
        if run_snapshot.state() == ProviderRunState::Ended {
            self.clear_runtime(run_id);
            super::super::mcp_proxy::shutdown_provider_mcp_proxy_run(run_id);
            return Ok(ProviderRunEndedOutcome {
                run: run_snapshot,
                already_ended: true,
            });
        }

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        self.clear_runtime(run_id);
        super::super::mcp_proxy::shutdown_provider_mcp_proxy_run(run_id);

        Ok(ProviderRunEndedOutcome {
            run,
            already_ended: false,
        })
    }

    pub(super) fn get_run_mut(
        &mut self,
        run_id: &str,
    ) -> Result<&mut RuntimeProviderRun, DaemonError> {
        self.runs
            .get_mut(run_id)
            .ok_or_else(|| DaemonError::ProviderRunNotFound {
                provider_run_id: run_id.to_string(),
            })
    }

    pub(super) fn update_run_remote_extension_manifest(
        &mut self,
        run_id: &str,
        manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.get_run_mut(run_id)?;
        run.set_remote_extension_manifest(manifest);
        crate::transport::mcp_server::catalog_changed();
        Ok(run.clone())
    }

    pub(super) fn observe_runtime_tool_catalog(
        &mut self,
        run_id: &str,
        expected_hash: &str,
    ) -> Result<Option<RuntimeProviderRun>, DaemonError> {
        let run = self.get_run_mut(run_id)?;
        Ok(run
            .observe_remote_extension_catalog(expected_hash)
            .then(|| run.clone()))
    }

    pub(super) fn compare_update_run_remote_extension_manifest(
        &mut self,
        run_id: &str,
        expected_revision: u64,
        manifest: crate::extension::RemoteExtensionManifest,
    ) -> Result<Option<RuntimeProviderRun>, DaemonError> {
        let run = self.get_run_mut(run_id)?;
        if run.remote_extension_manifest_revision() != expected_revision {
            return Ok(None);
        }
        run.set_remote_extension_manifest(manifest);
        crate::transport::mcp_server::catalog_changed();
        Ok(Some(run.clone()))
    }

    pub(super) fn enable_workflow_tools(
        &mut self,
        run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.get_run_mut(run_id)?;
        run.enable_workflow_tools();
        Ok(run.clone())
    }

    pub(super) fn mark_workflow_fresh_context(
        &mut self,
        run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<RuntimeProviderRun, DaemonError> {
        let run = self.get_run_mut(run_id)?;
        run.mark_workflow_fresh_context(workflow_node_run_id);
        Ok(run.clone())
    }

    fn adapter_for(
        &self,
        adapter_key: &str,
    ) -> Result<&'static dyn AgentEndpointAdapter, DaemonError> {
        self.registry
            .resolve(adapter_key)
            .ok_or_else(|| DaemonError::ProviderAdapterNotFound {
                adapter_key: adapter_key.to_string(),
            })
    }

    pub(super) fn next_run_id(&mut self) -> String {
        loop {
            self.next_run_number += 1;
            let run_id = format!("provider-run-{}", self.next_run_number);
            #[cfg(unix)]
            if crate::hot_reload::is_handed_over_run(&run_id) || self.runs.contains_key(&run_id) {
                continue;
            }
            return run_id;
        }
    }
}
