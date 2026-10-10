use std::path::PathBuf;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, ProviderRunState};

impl DaemonApp {
    pub(crate) fn ensure_prompt_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<String, DaemonError> {
        self.retire_finished_turn_substitute_run(session_id, agent_id)?;
        let (agent, turn_substitute) = self.agent_launch_profile(self.agents.get_agent(agent_id)?);
        let account_owner =
            self.provider_account_owner_for_execution(session_id, Some(agent_id))?;
        self.provider_account_profiles.require_agent_authenticated(
            &account_owner,
            &agent,
            "ensure prompt provider run for agent",
        )?;

        if agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "ensure prompt provider run for agent",
                message: format!(
                    "agent `{agent_id}` is remote-backed and must launch its provider on the worker kernel"
                ),
            });
        }
        let provider = crate::provider::provider_id_for_launch(agent.provider());
        let adapter_key = crate::provider::adapter_key_for_provider(provider);
        let session = self.sessions.get_session(session_id)?;
        let effective_config =
            crate::session::effective_agent_execution_config(&session, Some(&agent));
        let mut request = LaunchProviderRequest::new(
            session_id,
            adapter_key,
            provider,
            agent.provider_account_profile(),
            agent.model().unwrap_or("default"),
        )
        .with_agent_id(agent.id().to_string())
        .with_variant(agent.effort().map(str::to_string))
        .with_execution_mode(effective_config.mode)
        .with_permission_level(effective_config.permission_level);
        request = request.with_workspace_live_sync_mode(
            crate::provider::provider_workspace_live_sync_mode_for_session(
                provider,
                &self.config,
                Some(&session),
            ),
        );
        if let Some(worktree_id) = agent.worktree_id() {
            request = request.with_working_directory(PathBuf::from(worktree_id));
        }
        request = request.with_turn_substitute(turn_substitute);
        // MP-08 / MP-10 / MP-11: Re-resolve before reusing a process, including ordinary turns.
        let directory = agent.worktree_id().unwrap_or_else(|| session.worktree_id());
        request = request.with_working_directory(PathBuf::from(directory));
        request = crate::project_environment::attach_project_provider_environment(
            &self.config,
            &session,
            Some(&agent),
            request,
        )?;
        let mut replace_environment = false;
        if let Some(agent_run) = self.providers.get_run_for_agent(session_id, agent_id) {
            if agent_run.project_environment_revision()
                == request.project_environment_revision.as_deref()
            {
                match agent_run.state() {
                    ProviderRunState::Running | ProviderRunState::Starting => {
                        return Ok(agent_run.id().to_string());
                    }
                    ProviderRunState::Parked => {
                        let resumed = self.providers.resume_run_detached(agent_run.id())?;
                        self.update_provider_run_projection(resumed.clone());
                        return Ok(resumed.id().to_string());
                    }
                    ProviderRunState::Ended => {}
                }
            }
            if agent_run.client_interface() == crate::provider::ProviderClientInterface::NativeTui {
                return Err(DaemonError::LocalTransport {
                    operation: "refresh native TUI Project environment",
                    message: "restart the native provider TUI to load changed Project inputs"
                        .into(),
                });
            }
            if self.provider_run_has_active_prompt(session_id, &agent_run)? {
                return Err(DaemonError::InvalidProviderRunState {
                    provider_run_id: agent_run.id().into(),
                    state: agent_run.state(),
                    operation: "refresh active Project provider environment",
                });
            }
            // Codex holds a native thread writer for the life of its app-server.
            // Settle the idle process before resuming that same native conversation.
            self.end_agent_provider_run(session_id, agent_id)?;
            replace_environment = true;
        }

        // Replacement uses normal activation after settling the previous process.
        let provider_run = if replace_environment {
            self.launch_provider(request)?
        } else {
            self.launch_provider_detached(request)?
        };
        Ok(provider_run.id().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::CreateAgentRequest;
    use crate::app::KernelSessionService;
    use crate::config::{DaemonConfig, WorkspaceLiveSyncMode};
    use crate::provider::ProviderWriteAccessMode;
    use crate::session::CreateSessionRequest;

    #[test]
    fn mp08_mp10_mp11_ordinary_turn_reuses_unchanged_environment_and_relaunches_changed_values() {
        use crate::project_environment::*;
        let workspace = crate::test_support::TestWorktree::new("project-prompt-refresh");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let (session, _) = KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let agent = KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub"))
            .unwrap();
        let manifest = ProjectEnvironmentManifest {
            schema_version: 1,
            project_id: session.project_id().into(),
            evidence_digest: ProjectEnvironmentEvidence::default().digest(),
            entries: vec![ProjectEnvironmentEntry {
                name: "APP_LABEL".into(),
                workspace_id: session.workspace_id().into(),
                kind: ProjectEnvironmentEntryKind::Variable,
                classification: ProjectEnvironmentClassification::NonSecret,
                excluded: false,
                uses: vec![ProjectEnvironmentUse {
                    path: "app.ts".into(),
                    line: 1,
                }],
                locator: ProjectEnvironmentLocator::EnvFile {
                    path: ".env".into(),
                    key: "APP_LABEL".into(),
                },
                status: ProjectEnvironmentEntryStatus::Found,
            }],
            private_files: vec![],
            toolchain_hints: vec![],
            package_hints: vec![],
            service_hints: vec![],
        };
        ProjectEnvironmentStore::new(&app.config.private_runtime_state_root())
            .save(&StoredProjectEnvironment {
                source: None,
                manifest,
                evidence: ProjectEnvironmentEvidence::default(),
                reported_missing: Default::default(),
                reviewed_manifest: None,
                last_review: None,
            })
            .unwrap();
        std::fs::write(workspace.path().join(".env"), "APP_LABEL=first\n").unwrap();
        let first = app
            .ensure_prompt_provider_run_for_agent(session.id(), agent.id())
            .unwrap();
        let unchanged = app
            .ensure_prompt_provider_run_for_agent(session.id(), agent.id())
            .unwrap();
        assert_eq!(first, unchanged);
        std::fs::write(workspace.path().join(".env"), "APP_LABEL=second\n").unwrap();
        let changed = app
            .ensure_prompt_provider_run_for_agent(session.id(), agent.id())
            .unwrap();
        assert_ne!(first, changed);
        assert_eq!(
            app.providers.get_run(&first).unwrap().state(),
            ProviderRunState::Ended
        );
        assert_ne!(
            app.providers
                .get_run(&first)
                .unwrap()
                .project_environment_revision(),
            app.providers
                .get_run(&changed)
                .unwrap()
                .project_environment_revision()
        );
    }

    #[test]
    fn prompt_launched_agents_inherit_session_workspace_live_sync_mode() {
        let workspace = std::env::temp_dir().join(format!(
            "chariox-prompt-provider-live-sync-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace fixture should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon boot");
        let (session, _default_agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                workspace.to_string_lossy(),
                workspace.to_string_lossy(),
            ))
            .expect("session should be created");
        app.sessions_mut()
            .set_workspace_live_sync_mode(session.id(), WorkspaceLiveSyncMode::Managed)
            .expect("session sync mode should update");
        let worker = KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub"))
            .expect("worker should spawn");

        let error = app
            .ensure_prompt_provider_run_for_agent(session.id(), worker.id())
            .expect_err("managed mode should fail closed for an unsupported adapter");
        assert!(matches!(
            error,
            DaemonError::ProviderWorkspaceLiveSyncUnsupported { .. }
        ));

        app.sessions_mut()
            .set_workspace_live_sync_mode(session.id(), WorkspaceLiveSyncMode::Tracked)
            .expect("session sync mode should update");
        let run_id = app
            .ensure_prompt_provider_run_for_agent(session.id(), worker.id())
            .expect("tracked worker provider run should launch");
        let run = app.providers.get_run(&run_id).expect("run should exist");

        assert_eq!(
            run.write_access_mode(),
            ProviderWriteAccessMode::WorkspaceLiveSyncTracked
        );
        assert!(!run.requires_workspace_live_sync());
        assert!(run.tracks_workspace_live_sync());
        assert_eq!(
            app.sessions
                .get_session(session.id())
                .expect("session should remain available")
                .active_provider_run_id(),
            Some(run_id.as_str()),
            "prompt-launched provider must replace any stale session runtime pointer",
        );
        drop(app);
        let _ = std::fs::remove_dir_all(workspace);
    }
}
