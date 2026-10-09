//! MP-08: One export-time orchestration for ordinary kernels, slices and M28.
use super::project_environment_review::project_environment_code_summary;
use super::*;
use crate::project_environment::*;
use crate::provider::ProviderUtilityExecutionPolicy;
use crate::runtime::agent_utility_executor::{
    assert_agent_utility_can_run, run_provider_utility_prompt, AgentUtilityPromptParts,
};
use zeroize::Zeroizing;

impl KernelRuntimeState {
    pub(crate) async fn prepare_project_environment_layer(
        &self,
        project_id: &str,
        repositories: &[crate::managed_context::development::DevelopmentRepositorySelection],
        context_id: &str,
        target_kernel_id: &str,
        target_public_key: &str,
        interactive: bool,
    ) -> Result<crate::managed_context::development::DevelopmentProjectEnvironment, DaemonError>
    {
        let prepared = self
            .refresh_project_environment_state(
                project_id,
                repositories,
                interactive,
                target_kernel_id,
            )
            .await?;
        let config = self.owned.config_projection.snapshot();
        let sealed = seal_project_environment(
            context_id,
            &config.daemon_id,
            target_kernel_id,
            &config.relay_private_key,
            target_public_key,
            &prepared.state.manifest,
            &prepared.resolved,
        )?;
        Ok(
            crate::managed_context::development::DevelopmentProjectEnvironment {
                sealed,
                evidence: prepared.state.evidence.clone(),
                repository_workspaces: BTreeMap::new(),
            },
        )
    }

    pub(crate) async fn refresh_project_environment_state(
        &self,
        project_id: &str,
        repositories: &[crate::managed_context::development::DevelopmentRepositorySelection],
        interactive: bool,
        target_name: &str,
    ) -> Result<PreparedProjectEnvironmentExport, DaemonError> {
        let project = self.owned.session_store.get_project(project_id)?;
        if repositories
            .iter()
            .any(|repository| !project.contains_workspace(&repository.workspace_id))
        {
            return Err(environment_failure(
                "environment export workspace does not belong to Project",
            ));
        }
        let roots: BTreeMap<_, _> = repositories
            .iter()
            .map(|r| (r.workspace_id.clone(), r.worktree_path.clone()))
            .collect();
        let config = self.owned.config_projection.snapshot();
        let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
        // Lock spans discovery, missing-input resolution and sealing on this exporting kernel.
        let lock_store = store.clone();
        let lock_project = project_id.to_string();
        let _lock = tokio::task::spawn_blocking(move || lock_store.lock(&lock_project))
            .await
            .map_err(|_| environment_failure("environment refresh lock task failed"))??;
        let previous = store.load(project_id)?;
        let index_roots = roots.clone();
        let names: BTreeSet<_> = std::env::vars_os()
            .filter_map(|(name, _)| name.into_string().ok())
            .collect();
        let names = roots
            .keys()
            .map(|workspace| (workspace.clone(), names.clone()))
            .collect();
        let mut index =
            tokio::task::spawn_blocking(move || index_project_environment(&index_roots, &names))
                .await
                .map_err(|_| environment_failure("environment reference index task failed"))??;
        retain_imported_private_candidates(&mut index, previous.as_ref());
        let mut references = index.references;
        // Explicitly supplied values survive incremental discovery and are never asked again.
        if let Some(previous) = &previous {
            for reference in &mut references {
                if let Some(known) = previous.manifest.entries.iter().find(|e| {
                    e.workspace_id == reference.workspace_id
                        && e.name == reference.name
                        && matches!(e.locator, ProjectEnvironmentLocator::Vault { .. })
                }) {
                    reference.locator = known.locator.clone();
                }
            }
        }
        let digest = index.evidence.digest();
        let changed = previous
            .as_ref()
            .is_none_or(|state| state.manifest.evidence_digest != digest);
        let mut utility_identity = None;
        let mut discovery_input = ProjectEnvironmentDiscoveryInput {
            project_id: project_id.into(),
            evidence_digest: digest.clone(),
            changed_paths: index.evidence.changed_paths(
                &previous
                    .as_ref()
                    .map(|s| s.evidence.clone())
                    .unwrap_or_default(),
            ),
            previous_manifest: previous.as_ref().map(|s| s.manifest.clone()),
            references,
            private_files: index.private_files,
            revision: None,
        };
        let mut manifest = if changed
            && discovery_input.references.is_empty()
            && discovery_input.private_files.is_empty()
        {
            ProjectEnvironmentManifest {
                schema_version: 1,
                project_id: project_id.into(),
                evidence_digest: digest,
                entries: Vec::new(),
                private_files: Vec::new(),
                toolchain_hints: Vec::new(),
                package_hints: Vec::new(),
                service_hints: Vec::new(),
            }
        } else if changed {
            let (session_id, agent_id, cleanup) = self
                .environment_utility_identity(&project, repositories)
                .await?;
            utility_identity = Some((session_id.clone(), agent_id.clone(), cleanup));
            self.discover_project_environment(&session_id, &agent_id, &discovery_input)
                .await?
        } else {
            previous
                .as_ref()
                .expect("unchanged manifest")
                .manifest
                .clone()
        };
        if let Some(previous) = &previous {
            preserve_project_environment_choices(
                &mut manifest,
                &previous.manifest,
                &discovery_input.changed_paths,
            );
        }
        normalize_project_config_file_decisions(&mut manifest);
        // MP-08 / MP-10 / MP-11: Resolve and paste through the normal Vault
        // interaction before environment review; keep its lease through resolution.
        let _vault_guard = if interactive && !manifest.entries.is_empty() {
            if utility_identity.is_none() {
                utility_identity = Some(
                    self.environment_utility_identity(&project, repositories)
                        .await?,
                );
            }
            let (session_id, agent_id, _cleanup) =
                utility_identity.as_ref().expect("environment identity");
            Some(
                self.ensure_vault_unlocked_for_agent(
                    session_id,
                    agent_id,
                    "export Project environment",
                )
                .await?,
            )
        } else {
            None
        };
        let vault = crate::secret::project_environment_vault(&config)?;
        let workspace_environment: BTreeMap<_, _> = roots
            .keys()
            .map(|workspace| {
                let bindings = manifest
                    .entries
                    .iter()
                    .filter(|entry| &entry.workspace_id == workspace)
                    .filter_map(|entry| match &entry.locator {
                        ProjectEnvironmentLocator::WorkspaceEnvironment { name } => {
                            std::env::var(name)
                                .ok()
                                .map(|value| (name.clone(), Zeroizing::new(value)))
                        }
                        _ => None,
                    })
                    .collect();
                (workspace.clone(), bindings)
            })
            .collect();
        let resolved =
            resolve_project_environment(&manifest, &roots, &workspace_environment, vault.as_ref())?;
        for entry in &mut manifest.entries {
            entry.status = resolved
                .unresolved
                .iter()
                .find(|e| e.workspace_id == entry.workspace_id && e.name == entry.name)
                .map_or(ProjectEnvironmentEntryStatus::Found, |e| e.status);
        }
        let mut state = StoredProjectEnvironment {
            source: previous.as_ref().and_then(|state| state.source.clone()),
            manifest,
            evidence: index.evidence,
            reported_missing: previous
                .as_ref()
                .map(|s| s.reported_missing.clone())
                .unwrap_or_default(),
            reviewed_manifest: previous.as_ref().and_then(|s| s.reviewed_manifest.clone()),
            last_review: previous.as_ref().and_then(|s| s.last_review.clone()),
        };
        let review_identity;
        let mut additions = ProjectPrivateFileAdditions::default();
        if project_environment_needs_review(&state) {
            let code = project_environment_code_summary(repositories);
            if interactive {
                review_identity = match utility_identity.take() {
                    Some(identity) => identity,
                    None => {
                        self.environment_utility_identity(&project, repositories)
                            .await?
                    }
                };
                let (session, agent, _cleanup) = &review_identity;
                additions = self
                    .review_project_environment(
                        session,
                        agent,
                        &mut state,
                        &mut discovery_input,
                        &roots,
                        &workspace_environment,
                        project.name(),
                        target_name,
                        code,
                        vault.as_ref(),
                    )
                    .await?;
            } else {
                let review = ProjectEnvironmentReview::build(
                    &state,
                    project.name(),
                    target_name,
                    code,
                    false,
                    true,
                );
                accept_project_environment_review(&mut state, review);
            }
        } else {
            tracing::info!(
                project_id,
                "MP-08 / MP-10: Using saved setup for {}",
                project.name()
            );
        }
        store.save(&state)?;
        additions.commit();
        let resolved = resolve_project_environment(
            &state.manifest,
            &roots,
            &workspace_environment,
            vault.as_ref(),
        )?;
        register_project_file_rules(&state.manifest, &roots);
        Ok(PreparedProjectEnvironmentExport {
            state,
            resolved,
            _lock,
        })
    }

    // MP-08 / MP-10 / MP-11: Detection uses an owned, hidden utility session and settles cleanup.
    pub(super) async fn detect_environment_utility(
        &self,
        project: &crate::session::RuntimeProject,
        folder: &EnvironmentFolder,
        provider: Option<&EnvironmentProvider>,
        input: &ProjectEnvironmentDiscoveryInput,
    ) -> Result<ProjectEnvironmentManifest, DaemonError> {
        let provider = match provider {
            Some(EnvironmentProvider::Codex) => "codex",
            Some(EnvironmentProvider::Claude) => "claude",
            Some(EnvironmentProvider::OpenCode) => "opencode",
            None => "default",
        };
        let response = self
            .create_session_response(
                crate::session::CreateSessionRequest::new(
                    &folder.local_workspace_binding,
                    &folder.local_workspace_binding,
                )
                .with_owner_user_id(project.owner_user_id())
                .with_hidden(true)
                .with_agent_defaults(detection_utility_agent_defaults(
                    self.owned.configured_session_agent_defaults(),
                    provider,
                )),
            )
            .await?;
        let LocalDaemonResponse::SessionCreated { session, agent } = response else {
            return Err(environment_failure("Detect utility session unavailable"));
        };
        let cleanup_guard = EnvironmentUtilityCleanup {
            runtime: self.clone(),
            session: Some(session.clone()),
        };
        let result = self
            .discover_project_environment(session.id(), agent.id(), input)
            .await;
        // The existing parser forbids invented names/locations. Hint prose cannot author requirements.
        let cleanup = self.delete_environment_utility_session(&session).await;
        let mut cleanup_guard = cleanup_guard;
        if cleanup.is_ok() {
            cleanup_guard.session = None;
        }
        let manifest = result?;
        cleanup?;
        Ok(manifest)
    }

    pub(super) async fn discover_project_environment(
        &self,
        session_id: &str,
        agent_id: &str,
        input: &ProjectEnvironmentDiscoveryInput,
    ) -> Result<ProjectEnvironmentManifest, DaemonError> {
        let (_, mut run) = assert_agent_utility_can_run(
            self,
            session_id,
            agent_id,
            &crate::local::AgentUtilityKind::ProjectEnvironmentSetup,
        )
        .await?;
        let scratch = EnvironmentDiscoveryScratch::new(
            &self
                .owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        )?;
        run.set_metadata_only_discovery(scratch.0.clone());
        let output = run_provider_utility_prompt(self, run, AgentUtilityPromptParts {
            visible_user_prompt: project_environment_discovery_prompt(input)?,
            hidden_system_context: "Classify only the supplied Project environment metadata. No tools or source contents are available.".into(),
        }, "discover Project environment", ProviderUtilityExecutionPolicy::MetadataOnlyDiscovery).await?;
        parse_project_environment_discovery_output(&output, input)
    }

    async fn environment_utility_identity(
        &self,
        project: &crate::session::RuntimeProject,
        repositories: &[crate::managed_context::development::DevelopmentRepositorySelection],
    ) -> Result<(String, String, Option<EnvironmentUtilityCleanup>), DaemonError> {
        for session in self.owned.session_store.sessions_in_project(project.id()) {
            if let Some(agent) = self
                .owned
                .agent_store
                .get_session_agents(session.id())
                .into_iter()
                .find(|agent| {
                    agent.remote_execution().is_none()
                        && !session
                            .active_interactions()
                            .iter()
                            .any(|interaction| interaction.agent_id() == Some(agent.id()))
                        && self
                            .owned
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, agent.id())
                            .is_none()
                })
            {
                return Ok((session.id().into(), agent.id().into(), None));
            }
        }
        let primary = repositories
            .iter()
            .find(|r| {
                r.role == crate::managed_context::development::DevelopmentRepositoryRole::Primary
            })
            .ok_or_else(|| environment_failure("environment export has no primary workspace"))?;
        let response = self
            .create_session_response(
                crate::session::CreateSessionRequest::new(
                    &primary.workspace_id,
                    primary.worktree_path.to_string_lossy().to_string(),
                )
                .with_alias(format!("project-environment-{:x}", rand::random::<u64>()))
                .with_owner_user_id(project.owner_user_id())
                .with_project_selection(
                    crate::session::SessionProjectSelection::Existing {
                        project_id: project.id().into(),
                    },
                ),
            )
            .await?;
        match response {
            LocalDaemonResponse::SessionCreated { session, agent } => Ok((
                session.id().into(),
                agent.id().into(),
                Some(EnvironmentUtilityCleanup {
                    runtime: self.clone(),
                    session: Some(session.clone()),
                }),
            )),
            _ => Err(environment_failure(
                "environment utility session could not be created",
            )),
        }
    }
}

pub(super) fn environment_failure(message: &'static str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "Project environment",
        message: message.into(),
    }
}
struct EnvironmentDiscoveryScratch(PathBuf);
impl EnvironmentDiscoveryScratch {
    fn new(root: &Path) -> Result<Self, DaemonError> {
        std::fs::create_dir_all(root)
            .map_err(|_| environment_failure("discovery state root unavailable"))?;
        let path = root.join(format!("project-discovery-{}", rand::random::<u64>()));
        std::fs::create_dir(&path)
            .map_err(|_| environment_failure("discovery scratch unavailable"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| environment_failure("discovery scratch permissions failed"))?;
        }
        Ok(Self(path))
    }
}
impl Drop for EnvironmentDiscoveryScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

pub(crate) struct PreparedProjectEnvironmentExport {
    pub(super) state: StoredProjectEnvironment,
    pub(super) resolved: ResolvedProjectEnvironment,
    _lock: ProjectEnvironmentLock,
}

// MP-08 / MP-11: Only the temporary session created by this utility is retired.
struct EnvironmentUtilityCleanup {
    runtime: KernelRuntimeState,
    session: Option<crate::session::RuntimeSession>,
}
impl Drop for EnvironmentUtilityCleanup {
    fn drop(&mut self) {
        let runtime = self.runtime.clone();
        let Some(session) = self.session.take() else {
            return;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = runtime.delete_environment_utility_session(&session).await {
                    tracing::warn!(
                        session_id = session.id(),
                        "MP-08 / MP-10 / MP-11: utility session cleanup failed: {}",
                        error
                    );
                }
            });
        }
    }
}

// MP-08 / MP-10 / MP-11: utility model selection follows the normal provider defaults.
fn detection_utility_agent_defaults(
    configured: Option<crate::session::SessionAgentDefaults>,
    provider: &str,
) -> crate::session::SessionAgentDefaults {
    configured
        .filter(|defaults| provider == "default" || defaults.provider == provider)
        .unwrap_or_else(|| crate::session::SessionAgentDefaults::new(provider))
        .with_execution_mode(crate::provider::AgentExecutionMode::Plan)
        .with_permission_level(crate::provider::AgentPermissionLevel::Required)
}

#[cfg(test)]
mod detection_utility_tests {
    use super::*;
    #[test]
    fn detection_utility_inherits_configured_model_without_escalating_permissions() {
        let configured = crate::session::SessionAgentDefaults::new("codex")
            .with_model("codex/gpt-6.1-sol")
            .with_effort("low")
            .with_execution_mode(crate::provider::AgentExecutionMode::Build)
            .with_permission_level(crate::provider::AgentPermissionLevel::Yolo);
        for provider in ["default", "codex"] {
            let selected = detection_utility_agent_defaults(Some(configured.clone()), provider);
            assert_eq!(selected.provider, "codex");
            assert_eq!(selected.model.as_deref(), Some("codex/gpt-6.1-sol"));
            assert_eq!(selected.effort.as_deref(), Some("low"));
            assert_eq!(
                selected.execution_mode,
                Some(crate::provider::AgentExecutionMode::Plan)
            );
            assert_eq!(
                selected.permission_level,
                Some(crate::provider::AgentPermissionLevel::Required)
            );
        }
        let selected = detection_utility_agent_defaults(Some(configured), "opencode");
        assert_eq!(selected.provider, "opencode");
        assert_eq!(selected.model, None);
    }
}
