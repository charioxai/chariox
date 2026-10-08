//! MP-08: Shared local/remote client query of the source kernel's manifest.
use super::*;

impl KernelRuntimeState {
    // MP-08 / MP-10 / MP-11: Idle prompt admission refreshes through normal activation.
    pub(super) async fn refresh_project_prompt_provider(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some()
            || self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
                .is_some()
        {
            return Ok(());
        }
        let Some(run) = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
        else {
            return Ok(());
        };
        if !self
            .owned
            .project_prompt_provider_requires_resolution(&session, &run)
        {
            return Ok(());
        }
        self.with_project_prompt_environment(session_id, agent_id, |app| {
            app.ensure_prompt_provider_run_for_agent(session_id, agent_id)
                .map(|_| ())
        })
        .await
    }

    // MP-08/MP-10/MP-11: keep an Always-policy unlock alive through resolution,
    // replacement and queue activation; failures leave the unactivated queue intact.
    pub(super) async fn with_project_prompt_environment<T>(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: impl FnOnce(&mut DaemonApp) -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let account = self
            .prepare_prompt_provider_credentials(session_id, agent_id)
            .await?;
        let session = self.owned.session_store.get_session(session_id)?;
        let config = self.owned.config_projection.snapshot();
        let has_environment = crate::project_environment::ProjectEnvironmentStore::new(
            &config.private_runtime_state_root(),
        )
        .load(session.project_id())?
        .is_some();
        let _vault = if has_environment {
            Some(
                self.ensure_vault_unlocked_for_agent(
                    session_id,
                    agent_id,
                    "prepare Project environment",
                )
                .await?,
            )
        } else {
            None
        };
        self.with_authorized_app_side_effect(|app| {
            account.validate(app)?;
            operation(app)
        })
        .await
    }

    pub(crate) fn project_environment_for_shell(
        &self,
        session_id: &str,
        directory: &Path,
    ) -> Result<crate::provider::ProviderCredentialEnvironment, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        crate::project_environment::project_launch_environment(
            &self.owned.config_projection.snapshot(),
            session.project_id(),
            session.workspace_id(),
            directory,
        )
    }

    pub(crate) async fn get_project_environment_manifest(
        &self,
        request: crate::local::GetProjectEnvironmentManifestRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned.session_store.get_project(&request.project_id)?;
        if project.owner_user_id() != caller_user_id {
            return Err(DaemonError::LocalTransport {
                operation: "get Project environment manifest",
                message: "caller does not own the selected Project".into(),
            });
        }
        if let Some(agent_id) = &request.agent_id {
            let agent = self.owned.ensure_agent_owner(
                agent_id,
                caller_user_id,
                "read Project environment",
            )?;
            let session = self.owned.session_store.get_session(agent.session_id())?;
            if session.project_id() != project.id() {
                return Err(super::project_environment_export::environment_failure(
                    "agent does not belong to selected Project",
                ));
            }
            if agent.remote_execution().is_some() {
                return self.remote_project_environment(&agent, false).await;
            }
        }
        let config = self.owned.config_projection.snapshot();
        let store = crate::project_environment::ProjectEnvironmentStore::new(
            &config.private_runtime_state_root(),
        );
        let manifest = store.load(&request.project_id)?.map(|state| state.manifest);
        Ok(LocalDaemonResponse::ProjectEnvironmentManifest { manifest })
    }
}

// MP-08 / MP-10 / MP-11: Adjustment uses the same review and durable selection
// as export. The request acknowledges promptly; the existing session interaction
// survives client reconnect and owns every confirm, flip, revision and paste.
impl KernelRuntimeState {
    pub(crate) async fn start_project_environment_adjustment(
        &self,
        request: crate::local::AdjustProjectEnvironmentRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        use super::project_environment_export::environment_failure;
        use crate::project_environment::*;
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let agent = self.owned.ensure_agent_owner(
            &request.agent_id,
            caller_user_id,
            "adjust Project environment",
        )?;
        let project = self.owned.session_store.get_project(session.project_id())?;
        if agent.session_id() != session.id()
            || project.owner_user_id() != caller_user_id
            || project.status() != crate::session::RuntimeProjectStatus::Active
        {
            return Err(environment_failure(
                "caller does not own this Project environment",
            ));
        }
        if agent.remote_execution().is_some() {
            return self.remote_project_environment(&agent, true).await;
        }
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent.id())
            .is_some()
        {
            return Err(environment_failure(
                "wait for the current agent turn before adjusting the environment",
            ));
        }
        if session
            .active_interactions()
            .iter()
            .any(|interaction| interaction.agent_id() == Some(agent.id()))
        {
            return Err(environment_failure(
                "resolve the current interaction before adjusting the environment",
            ));
        }
        let config = self.owned.config_projection.snapshot();
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        let lock = store.try_lock(project.id())?;
        let mut state = store
            .load(project.id())?
            .ok_or_else(|| environment_failure("Project has no saved environment yet"))?;
        let roots: BTreeMap<_, _> =
            super::project_environment_workspaces::project_environment_repository_selections(
                &project, &session,
            )
            .into_iter()
            .map(|repository| (repository.workspace_id, repository.worktree_path))
            .collect();
        let names = roots
            .keys()
            .map(|workspace| {
                (
                    workspace.clone(),
                    std::env::vars_os()
                        .filter_map(|(name, _)| name.into_string().ok())
                        .collect(),
                )
            })
            .collect();
        let mut index = index_project_environment(&roots, &names)?;
        retain_imported_private_candidates(&mut index, Some(&state));
        let mut input = ProjectEnvironmentDiscoveryInput {
            project_id: project.id().into(),
            evidence_digest: state.manifest.evidence_digest.clone(),
            changed_paths: BTreeMap::new(),
            previous_manifest: Some(state.manifest.clone()),
            references: state.manifest.entries.clone(),
            private_files: index.private_files,
            revision: None,
        };
        // The panel shows all saved decisions, including unchanged ones.
        state.reviewed_manifest = None;
        let workspace_environment = roots
            .keys()
            .map(|workspace| {
                let values = state
                    .manifest
                    .entries
                    .iter()
                    .filter(|entry| &entry.workspace_id == workspace)
                    .filter_map(|entry| match &entry.locator {
                        ProjectEnvironmentLocator::WorkspaceEnvironment { name } => {
                            std::env::var(name)
                                .ok()
                                .map(|value| (name.clone(), zeroize::Zeroizing::new(value)))
                        }
                        _ => None,
                    })
                    .collect();
                (workspace.clone(), values)
            })
            .collect();
        let vault = crate::secret::project_environment_vault(&config)?;
        let runtime = self.clone();
        let session_id = request.session_id.clone();
        let agent_id = request.agent_id.clone();
        tokio::spawn(async move {
            let _lock = lock;
            let result = async {
                let _vault_guard = runtime
                    .ensure_vault_unlocked_for_agent(
                        &session_id,
                        &agent_id,
                        "adjust Project environment",
                    )
                    .await?;
                let additions = runtime
                    .review_project_environment(
                        &session_id,
                        &agent_id,
                        &mut state,
                        &mut input,
                        &roots,
                        &workspace_environment,
                        project.name(),
                        "this machine",
                        "Saved Project setup".into(),
                        vault.as_ref(),
                    )
                    .await?;
                {
                    let current = runtime.owned.session_store.get_project(project.id())?;
                    if current.owner_user_id() != project.owner_user_id()
                        || current.status() != crate::session::RuntimeProjectStatus::Active
                    {
                        return Err(environment_failure(
                            "Project is no longer available for adjustment",
                        ));
                    }
                    store.save(&state)?;
                    additions.commit();
                    Ok(())
                }
            }
            .await;
            if let Err(error) = result {
                if matches!(&error, DaemonError::LocalTransport {message, ..} if message == "Project export cancelled")
                {
                    return;
                }
                let notice = crate::session::RuntimeInteraction::new(
                    format!("project-environment-result:{}", rand::random::<u64>()),
                    &agent_id,
                    crate::session::RuntimeInteractionKind::Choice,
                    crate::session::RuntimeInteractionLevel::Warning,
                    Some("Environment adjustment stopped".into()),
                    "Reopen Environment to review the current saved setup.",
                    vec![crate::session::RuntimeInteractionChoice::new(
                        "close", "Close", "close", None,
                    )],
                    None,
                    Some(60),
                    Some("close".into()),
                );
                let _ = runtime
                    .create_runtime_interaction(&session_id, notice)
                    .await;
            }
        });
        Ok(LocalDaemonResponse::ProjectEnvironmentAdjustmentStarted {
            session_id: request.session_id,
            agent_id: request.agent_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_environment::*;

    #[tokio::test]
    async fn mp08_mp10_mp11_post_launch_adjustment_uses_owned_session_interaction() {
        let root = std::env::temp_dir().join(format!(
            "chariox-envlayer4-adjust-{}",
            rand::random::<u64>()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("app.ts"), "process.env.OPTIONAL_INPUT\n").unwrap();
        let mut config =
            crate::DaemonConfig::for_tests().with_session_history_root(root.join("history"));
        config.user_config_path = root.join("runtime/config.toml");
        config.user_config.state.path = Some(root.join("runtime/state.db").display().to_string());
        config.user_config.history.operational.path =
            Some(root.join("runtime/history.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("runtime/artifacts").display().to_string());
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path =
            root.join("runtime/vault.json").display().to_string();
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::KernelInit;
        let vault_path = config.user_config.credential_vault.path.clone();
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        let mut app = crate::DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = app
            .create_session(
                crate::session::CreateSessionRequest::new(
                    workspace.display().to_string(),
                    workspace.display().to_string(),
                )
                .with_owner_user_id("user-1"),
            )
            .unwrap();
        let evidence = ProjectEnvironmentEvidence::default();
        let state = StoredProjectEnvironment {
            source: None,
            manifest: ProjectEnvironmentManifest {
                schema_version: 1,
                project_id: session.project_id().into(),
                evidence_digest: evidence.digest(),
                entries: vec![ProjectEnvironmentEntry {
                    name: "OPTIONAL_INPUT".into(),
                    workspace_id: session.workspace_id().into(),
                    kind: ProjectEnvironmentEntryKind::Variable,
                    classification: ProjectEnvironmentClassification::Secret,
                    excluded: false,
                    uses: vec![ProjectEnvironmentUse {
                        path: "app.ts".into(),
                        line: 1,
                    }],
                    locator: ProjectEnvironmentLocator::Missing,
                    status: ProjectEnvironmentEntryStatus::Missing,
                }],
                private_files: vec![],
                toolchain_hints: vec![],
                package_hints: vec![],
                service_hints: vec![],
            },
            evidence,
            reported_missing: Default::default(),
            reviewed_manifest: None,
            last_review: None,
        };
        store.save(&state).unwrap();
        let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        )
        .runtime_state();
        let request = crate::local::AdjustProjectEnvironmentRequest {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
        };
        assert!(runtime
            .start_project_environment_adjustment(request.clone(), "another-user")
            .await
            .is_err());
        runtime
            .owned
            .agent_store
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "synthetic-worker".into(),
                    worker_machine_id: "synthetic-machine".into(),
                    execution_lease_id: "synthetic-lease".into(),
                    leased_agent_id: "synthetic-leased-agent".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        assert!(runtime
            .start_project_environment_adjustment(request.clone(), "user-1")
            .await
            .is_err());

        let context = crate::transport::relay_peer::RemoteNativeInteractionContext {
            home_session_id: session.id().into(),
            home_agent_id: agent.id().into(),
            leased_agent_id: "synthetic-leased-agent".into(),
            worker_provider_run_id: "environment-adjustment".into(),
            home_prompt_id: None,
        };
        let mut selected = state.clone();
        std::fs::write(
            workspace.join("notes.md"),
            "MP-08 / MP-10 / MP-11 private notes",
        )
        .unwrap();
        selected
            .manifest
            .private_files
            .push(ProjectPrivateFileDecision {
                workspace_id: session.workspace_id().into(),
                path: "notes.md".into(),
                bring: false,
                reason: "Personal notes".into(),
                secret_looking: false,
            });
        store.save(&selected).unwrap();
        assert!(runtime
            .fetch_project_private_file(
                "other-worker",
                context.clone(),
                session.workspace_id().into(),
                "notes.md".into()
            )
            .await
            .is_err());
        assert!(runtime
            .fetch_project_private_file(
                "synthetic-worker",
                context.clone(),
                session.workspace_id().into(),
                "../notes.md".into()
            )
            .await
            .is_err());
        let bytes = runtime
            .fetch_project_private_file(
                "synthetic-worker",
                context.clone(),
                session.workspace_id().into(),
                "notes.md".into(),
            )
            .await
            .unwrap();
        assert!(!format!("{bytes:?}").contains("private notes"));
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(bytes.into_inner())
                .unwrap(),
            b"MP-08 / MP-10 / MP-11 private notes"
        );
        assert_eq!(
            store.load(session.project_id()).unwrap().unwrap(),
            selected,
            "an explicit fetch never changes source decisions"
        );
        selected.manifest.private_files[0].secret_looking = true;
        store.save(&selected).unwrap();
        assert!(runtime
            .fetch_project_private_file(
                "synthetic-worker",
                context.clone(),
                session.workspace_id().into(),
                "notes.md".into()
            )
            .await
            .is_err());
        selected.manifest.private_files[0].secret_looking = false;
        #[cfg(unix)]
        {
            std::fs::remove_file(workspace.join("notes.md")).unwrap();
            std::os::unix::fs::symlink("/etc/passwd", workspace.join("notes.md")).unwrap();
            store.save(&selected).unwrap();
            assert!(runtime
                .fetch_project_private_file(
                    "synthetic-worker",
                    context,
                    session.workspace_id().into(),
                    "notes.md".into()
                )
                .await
                .is_err());
        }
        store.save(&state).unwrap();
        assert_eq!(store.load(session.project_id()).unwrap().unwrap(), state);
        assert!(runtime
            .session_snapshot(session.id())
            .await
            .unwrap()
            .active_interactions()
            .is_empty());
        runtime
            .owned
            .agent_store
            .clear_remote_execution(agent.id())
            .unwrap();
        assert!(matches!(
            runtime
                .start_project_environment_adjustment(request, "user-1")
                .await
                .unwrap(),
            LocalDaemonResponse::ProjectEnvironmentAdjustmentStarted { .. }
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        let unlock = loop {
            let snapshot = runtime.session_snapshot(session.id()).await.unwrap();
            if let Some(interaction) = snapshot.active_interactions().first() {
                break interaction.clone();
            }
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(unlock.title(), Some("Unlock Chariox Vault"));
        runtime
            .answer_terminal_runtime_interaction(
                session.id(),
                unlock.id(),
                "passphrase",
                Some("synthetic-envlayer4-passphrase"),
                Some(session.owner_user_id()),
                None,
                None,
                Some(crate::local::KernelConnectionClass::Terminal),
            )
            .await
            .unwrap();
        let interaction = loop {
            let snapshot = runtime.session_snapshot(session.id()).await.unwrap();
            if let Some(interaction) = snapshot.active_interactions().first() {
                break interaction.clone();
            }
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(serde_json::to_value(&interaction)
            .unwrap()
            .get("project_environment_review")
            .is_some());
        assert!(store.try_lock(session.project_id()).is_err());
        runtime
            .resolve_runtime_interaction(session.id(), interaction.id(), "skip", None)
            .await
            .unwrap();
        loop {
            if store
                .load(session.project_id())
                .unwrap()
                .unwrap()
                .reviewed_manifest
                .is_some()
                && store.try_lock(session.project_id()).is_ok()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(store
            .load(session.project_id())
            .unwrap()
            .unwrap()
            .reported_missing
            .contains(&(session.workspace_id().into(), "OPTIONAL_INPUT".into())));
        runtime
            .delete_session_ref(session.id(), None)
            .await
            .unwrap();
        drop(runtime);
        crate::secret::lock_chariox_encrypted_vault(&vault_path).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
