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

    // MP-08/MP-11: use the native review while omitting all environment values.
    pub(crate) async fn review_credential_free_project_context(
        &self,
        project_id: &str,
        repositories: &[crate::managed_context::development::DevelopmentRepositorySelection],
        target: &str,
    ) -> Result<(), DaemonError> {
        self.refresh_project_environment_state_mode(project_id, repositories, true, target, true)
            .await
            .map(|_| ())
            .map_err(|error| match &error {
                DaemonError::LocalTransport {
                    operation: "Project environment",
                    message,
                } if matches!(
                    message.as_str(),
                    "Project export cancelled" | "Project review timed out"
                ) =>
                {
                    DaemonError::ManagedContext {
                        code: "managed_context_review_cancelled",
                        operation: "owner-managed context review",
                        message: "Owner-managed context review was cancelled or expired".into(),
                        retryable: false,
                    }
                }
                _ => error,
            })
    }

    pub(crate) async fn refresh_project_environment_state(
        &self,
        project_id: &str,
        repositories: &[crate::managed_context::development::DevelopmentRepositorySelection],
        interactive: bool,
        target_name: &str,
    ) -> Result<PreparedProjectEnvironmentExport, DaemonError> {
        self.refresh_project_environment_state_mode(
            project_id,
            repositories,
            interactive,
            target_name,
            false,
        )
        .await
    }

    async fn refresh_project_environment_state_mode(
        &self,
        project_id: &str,
        repositories: &[crate::managed_context::development::DevelopmentRepositorySelection],
        interactive: bool,
        target_name: &str,
        without_credentials: bool,
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
        let mut previous = store.load(project_id)?;
        if without_credentials {
            if let Some(previous) = previous.as_mut() {
                previous.manifest.entries.clear();
            }
        }
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
        if without_credentials {
            references.clear();
        }
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
        if without_credentials {
            manifest.entries.clear();
        }
        // MP-08 / MP-10 / MP-11: Resolve and paste through the normal Vault
        // interaction before environment review; keep its lease through resolution.
        let _vault_guard = if !without_credentials && interactive && !manifest.entries.is_empty() {
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
        let vault: Arc<dyn crate::secret::CredentialVaultStore> = if without_credentials {
            Arc::new(CredentialFreeVault)
        } else {
            crate::secret::project_environment_vault(&config)?
        };
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
        if without_credentials || project_environment_needs_review(&state) {
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
        if without_credentials {
            if !state.manifest.entries.is_empty() {
                return Err(environment_failure(
                    "owner-managed review cannot include environment credentials",
                ));
            }
            // The credential-free review does not replace the source's saved environment choices.
        } else {
            store.save(&state)?;
        }
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
                    session_id: session.id().into(),
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
    session_id: String,
}
impl Drop for EnvironmentUtilityCleanup {
    fn drop(&mut self) {
        let runtime = self.runtime.clone();
        let session_id = self.session_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = runtime.delete_session_ref(&session_id, None).await {
                    tracing::warn!(
                        session_id,
                        "MP-08 / MP-11: utility session cleanup failed: {}",
                        error
                    );
                }
            });
        }
    }
}

// MP-11: even revised reviews cannot access or mutate the source Vault.
#[derive(Debug)]
struct CredentialFreeVault;
impl crate::secret::CredentialVaultStore for CredentialFreeVault {
    fn get_secret(&self, _: &str, _: &str) -> Result<String, DaemonError> {
        Err(environment_failure(
            "credential access is excluded from this transfer",
        ))
    }
    fn set_secret(&self, _: &str, _: &str, _: &str) -> Result<(), DaemonError> {
        Err(environment_failure(
            "credential access is excluded from this transfer",
        ))
    }
    fn delete_secret(&self, _: &str, _: &str) -> Result<(), DaemonError> {
        Err(environment_failure(
            "credential access is excluded from this transfer",
        ))
    }
}

#[cfg(test)]
#[path = "project_environment_owner_tests.rs"]
mod owner_tests;
