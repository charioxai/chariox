use crate::error::DaemonError;
use crate::provider::AgentEndpointAdapter;

use super::{
    LaunchProviderRequest, ProviderProcessService, ProviderRunEndedOutcome,
    ProviderRunLivenessReconciliation, ProviderRunParkedOutcome, ProviderRunResumedOutcome,
    ProviderRunStartedOutcome, ProviderRunState, ProviderSessionRunsTerminatedOutcome,
    RuntimeProviderRun,
};

impl ProviderProcessService {
    pub(crate) fn start_run_provider_only(
        &mut self,
        request: LaunchProviderRequest,
    ) -> Result<ProviderRunStartedOutcome, DaemonError> {
        let adapter = self.registry.resolve(&request.adapter_key).ok_or_else(|| {
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

        let run_id = self.next_run_id();
        let launch_result = adapter.connect(&request)?;
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
        let run = RuntimeProviderRun::new(run_id.clone(), &request, launch_result);

        self.runs.insert(run_id, run.clone());

        Ok(ProviderRunStartedOutcome { run })
    }

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
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: run_id.to_string(),
                state: run_snapshot.state(),
                operation: "terminate",
            });
        }

        let adapter = self.adapter_for(run_snapshot.adapter_key())?;
        adapter.terminate(&run_snapshot);

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        self.run_actor_mailbox
            .spawn_terminate(run_id.to_string(), run.clone());

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
            return Ok(ProviderRunEndedOutcome {
                run: run_snapshot,
                already_ended: true,
            });
        }

        let run = self.get_run_mut(run_id)?;
        run.mark_ended();
        let run = run.clone();

        self.clear_runtime(run_id);

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

    fn next_run_id(&mut self) -> String {
        self.next_run_number += 1;
        format!("provider-run-{}", self.next_run_number)
    }
}
