//! MP-08: Shared local/remote client query of the source kernel's manifest.
use super::*;

impl KernelRuntimeState {
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

    pub(crate) fn get_project_environment_manifest(
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
            .any(|interaction| interaction.agent_id() == agent.id())
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
        let roots: BTreeMap<_, _> = project
            .workspace_ids()
            .iter()
            .map(|workspace| {
                let path = if workspace == session.workspace_id() {
                    PathBuf::from(session.worktree_id())
                } else {
                    PathBuf::from(workspace)
                };
                (workspace.clone(), path)
            })
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
        let index = index_project_environment(&roots, &names)?;
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
                runtime
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
                    store.save(&state)
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
            .resolve_runtime_interaction(
                session.id(),
                unlock.id(),
                "passphrase",
                Some("synthetic-envlayer4-passphrase".into()),
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
