//! The Project Environment as the kernel answers it: read, save, a Secret's value, and Setup runs.
use super::*;
use crate::local::{
    EnvironmentEntryKind, EnvironmentOperationRequest, EnvironmentOperationState,
    EnvironmentOperationView, ExtensionPresence, GetEnvironmentOperationRequest,
    GetProjectEnvironmentRequest, ProjectEnvironmentExtension, ProjectEnvironmentExtensionView,
    ProjectEnvironmentFileView, ProjectEnvironmentSecretSource, ProjectEnvironmentSecretView,
    ProjectEnvironmentSetupField, ProjectEnvironmentSetupState, ProjectEnvironmentView,
    RunProjectEnvironmentSetupRequest, SaveProjectEnvironmentRequest,
    SetProjectEnvironmentSecretRequest,
};
use crate::project_environment::{
    declared_in, environment_error, file_presence, project_environment_vault_service,
    secret_presence, ExtensionEntry, ProjectEnvironmentRecord, ProjectEnvironmentStore,
    RegistrationHost, SaveOutcome, SetupSpec, UserRegistrations, PROJECT_ENVIRONMENT_SCHEMA,
};
use std::path::{Path, PathBuf};

fn extension_entry(extension: ProjectEnvironmentExtension) -> ExtensionEntry {
    ExtensionEntry {
        name: extension.name,
        source: extension.source,
    }
}

/// A project-file entry is present while its declaring file still declares it in this checkout (Git carries the file,
/// so a clone has it); a registration is present while this kernel holds it.
fn extension_views(
    registrations: &UserRegistrations,
    kind: EnvironmentEntryKind,
    checkout: &Path,
    entries: &[ExtensionEntry],
) -> Vec<ProjectEnvironmentExtensionView> {
    entries
        .iter()
        .map(|entry| {
            let here = match entry.file_path() {
                Some(path) => declared_in(checkout, path).contains(&entry.name),
                None => registrations.has(kind, &entry.name),
            };
            ProjectEnvironmentExtensionView {
                name: entry.name.clone(),
                source: entry.source.clone(),
                presence: if here {
                    ExtensionPresence::Present
                } else {
                    ExtensionPresence::Missing
                },
            }
        })
        .collect()
}

const OPERATION_WAIT: std::time::Duration = std::time::Duration::from_secs(20);
const MAX_SECRET_VALUE_BYTES: usize = 64 * 1024;

impl KernelRuntimeState {
    pub(super) fn environment_store(&self) -> ProjectEnvironmentStore {
        ProjectEnvironmentStore::new(
            &self
                .owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        )
    }

    pub(super) fn machine_label(config: &crate::config::DaemonConfig) -> String {
        config
            .machine_name()
            .unwrap_or_else(|| config.host_machine_id.clone())
    }

    pub(super) fn owned_project(
        &self,
        project_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::session::RuntimeProject, DaemonError> {
        let project = self.owned.session_store.get_project(project_id)?;
        if project.owner_user_id() != caller_user_id {
            return Err(environment_error(
                "the caller does not own the selected Project",
            ));
        }
        Ok(project)
    }

    /// The Project a room's Environment tool works on.
    pub(super) fn tool_project(
        &self,
        session: &crate::session::RuntimeSession,
        owner_user_id: &str,
    ) -> Result<crate::session::RuntimeProject, DaemonError> {
        let project = self
            .owned
            .session_store
            .environment_project(session)
            .ok_or_else(|| {
                environment_error("this folder belongs to no Project of yours on this machine")
            })?;
        self.owned_project(project.id(), owner_user_id)
    }

    /// The Project's own folder, a git worktree of it or a worktree of one of its sessions: nothing else is a checkout.
    pub(super) fn project_checkout(
        &self,
        project: &crate::session::RuntimeProject,
        requested: Option<&str>,
    ) -> Result<PathBuf, DaemonError> {
        let first = project
            .workspace_ids()
            .first()
            .map(String::as_str)
            .unwrap_or_else(|| project.workspace_id());
        let Some(requested) = requested else {
            return Ok(PathBuf::from(first));
        };
        let known = self
            .owned
            .session_store
            .project_has_checkout(project, requested)?;
        if !known {
            return Err(environment_error(
                "the folder is not a checkout of this Project",
            ));
        }
        Ok(PathBuf::from(requested))
    }

    /// `record` is the saved Environment, or `None` for a Project that has none yet.
    pub(super) fn project_environment_view(
        &self,
        project: &crate::session::RuntimeProject,
        checkout: PathBuf,
        record: Option<ProjectEnvironmentRecord>,
    ) -> Result<ProjectEnvironmentView, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let exists = record.is_some();
        let record = record.unwrap_or(ProjectEnvironmentRecord {
            schema: PROJECT_ENVIRONMENT_SCHEMA,
            ..Default::default()
        });
        let presence = secret_presence(&config, project.id(), &record.secrets)?;
        let registrations = self.user_registrations(project.id());
        let setup_state = if exists {
            self.owned.project_environment_setups.state_for(
                project.id(),
                &checkout,
                &record.setup_digest(),
            )?
        } else {
            ProjectEnvironmentSetupState::NotRun
        };
        Ok(ProjectEnvironmentView {
            project_id: project.id().into(),
            exists,
            revision: record.revision,
            setup: ProjectEnvironmentSetupField {
                script: record.setup_script.clone(),
            },
            variables: record.variables.clone(),
            secrets: record
                .secrets
                .iter()
                .zip(presence)
                .map(|(name, presence)| ProjectEnvironmentSecretView {
                    name: name.clone(),
                    presence,
                })
                .collect(),
            files: record
                .files
                .iter()
                .map(|path| {
                    ProjectEnvironmentFileView::new(path.clone(), file_presence(&checkout, path))
                })
                .collect(),
            mcps: extension_views(
                &registrations,
                EnvironmentEntryKind::Mcp,
                &checkout,
                &record.mcps,
            ),
            skills: extension_views(
                &registrations,
                EnvironmentEntryKind::Skill,
                &checkout,
                &record.skills,
            ),
            changed_at_ms: record.changed_at_ms,
            machine_label: Self::machine_label(&config),
            checkout_path: checkout.to_string_lossy().into_owned(),
            setup_state,
            proposals: self.proposal_store().list(project.id())?,
        })
    }

    pub(crate) fn get_project_environment(
        &self,
        request: GetProjectEnvironmentRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned_project(&request.project_id, caller_user_id)?;
        let checkout = self.project_checkout(&project, request.checkout_path.as_deref())?;
        let record = self.environment_store().load(project.id())?;
        Ok(LocalDaemonResponse::ProjectEnvironment {
            environment: self.project_environment_view(&project, checkout, record)?,
        })
    }

    pub(crate) fn save_project_environment(
        &self,
        request: SaveProjectEnvironmentRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned_project(&request.project_id, caller_user_id)?;
        let checkout = self.project_checkout(&project, None)?;
        let fields = ProjectEnvironmentRecord {
            schema: PROJECT_ENVIRONMENT_SCHEMA,
            setup_script: request.setup.script,
            variables: request.variables,
            secrets: request
                .secrets
                .into_iter()
                .map(|secret| secret.name)
                .collect(),
            files: request.files.into_iter().map(|file| file.path).collect(),
            mcps: request.mcps.into_iter().map(extension_entry).collect(),
            skills: request.skills.into_iter().map(extension_entry).collect(),
            ..Default::default()
        };
        fields.validate().map_err(environment_error)?;
        let outcome =
            self.environment_store()
                .save(project.id(), request.expected_revision, fields)?;
        if let SaveOutcome::Saved(record) = &outcome {
            let checkouts: Vec<PathBuf> =
                project.workspace_ids().iter().map(PathBuf::from).collect();
            crate::project_environment::register_environment_files(&checkouts, &record.files);
        }
        Ok(match outcome {
            SaveOutcome::Saved(record) => LocalDaemonResponse::ProjectEnvironmentSaved {
                environment: self.project_environment_view(&project, checkout, Some(record))?,
            },
            SaveOutcome::Conflict(current) => LocalDaemonResponse::ProjectEnvironmentConflict {
                environment: self.project_environment_view(&project, checkout, Some(current))?,
            },
        })
    }

    /// The owner's value for a Secret row, typed or taken from an entry of this kernel's Vault, goes into
    /// this kernel's Vault under the Project scope and nowhere else.
    pub(crate) async fn set_project_environment_secret(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: SetProjectEnvironmentSecretRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let owner = self.owner_client_user_id(command)?;
        let project = self.owned_project(&request.project_id, &owner)?;
        let record = self.environment_secret_record(project.id(), &request.name)?;
        let links = self.secret_links();
        match &request.source {
            ProjectEnvironmentSecretSource::Value(value) => {
                self.require_unlocked_vault()?;
                // A typed value replaces the copy, so nothing stands behind it any more; under the
                // link lock, so a re-fill that is underway cannot overwrite it.
                links.write_linked(project.id(), &request.name, None, || {
                    self.store_environment_secret(project.id(), &request.name, value.expose_secret())
                        .map(|()| true)
                })?;
            }
            ProjectEnvironmentSecretSource::VaultEntry(credential_id) => {
                self.require_unlocked_vault()?;
                self.copy_vault_entry_into_environment(
                    &owner,
                    Default::default(),
                    project.id(),
                    &request.name,
                    credential_id,
                    || Ok(()),
                )
                .await?
            }
        }
        let checkout = self.project_checkout(&project, None)?;
        Ok(LocalDaemonResponse::ProjectEnvironmentSaved {
            environment: self.project_environment_view(&project, checkout, Some(record))?,
        })
    }

    /// The saved record, which must already name the Secret.
    fn environment_secret_record(
        &self,
        project_id: &str,
        name: &str,
    ) -> Result<ProjectEnvironmentRecord, DaemonError> {
        self.environment_store()
            .load(project_id)?
            .filter(|record| record.secrets.iter().any(|secret| secret == name))
            .ok_or_else(|| {
                environment_error("add the Secret to the Environment before giving it a value")
            })
    }

    fn store_environment_secret(
        &self,
        project_id: &str,
        name: &str,
        value: &str,
    ) -> Result<(), DaemonError> {
        if value.is_empty() || value.len() > MAX_SECRET_VALUE_BYTES || value.contains('\0') {
            return Err(environment_error(
                "a Secret value takes 1 byte to 64 KiB without NUL",
            ));
        }
        let config = self.owned.config_projection.snapshot();
        crate::secret::project_environment_vault(&config)?.set_secret(
            &project_environment_vault_service(project_id),
            name,
            value,
        )?;
        Ok(())
    }

    /// Copies one Vault entry into the Project's Secret inside the kernel: the entry must be made for
    /// the environment variable of the same name, and the use is checked and recorded like any other
    /// use of that entry. The value never leaves this function.
    pub(super) async fn copy_vault_entry_into_environment(
        &self,
        actor: &str,
        ask: super::vault_use::VaultUseAsk<'_>,
        project_id: &str,
        name: &str,
        credential_id: &str,
        still_current: impl Fn() -> Result<(), DaemonError>,
    ) -> Result<(), DaemonError> {
        let target = Self::environment_use_target(
            &self.home_runtime_secret_service()?,
            credential_id,
            name,
        )?;
        let value = self
            .read_vault_value_for_use(actor, ask, credential_id, &target, still_current)
            .await?;
        self.secret_links().write_linked(
            project_id,
            name,
            Some(crate::project_environment::SecretLink::Entry(
                credential_id.to_owned(),
            )),
            || {
                self.store_environment_secret(project_id, name, &value)
                    .map(|()| true)
            },
        )?;
        Ok(())
    }

    pub(super) fn secret_links(&self) -> crate::project_environment::SecretLinks {
        crate::project_environment::SecretLinks::new(
            &self
                .owned
                .config_projection
                .snapshot()
                .private_runtime_state_root(),
        )
    }

    /// The Vault entry named `entry_id` changed: every Secret filled from it takes the new value, through the
    /// same use check and audit as the first fill. Every Secret is tried; an error names those that could not
    /// take the value and so still hold the old one, so the owner's replacement never reads as a clean rotation.
    pub(crate) fn refill_linked_environment_secrets(
        &self,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        let mut stale = Vec::new();
        for (project_id, name) in self.secret_links().filled_from(entry_id)? {
            if let Err(error) = self.refill_environment_secret(&project_id, &name, entry_id) {
                tracing::warn!(%error, project_id, name, "an Environment secret could not be re-filled");
                stale.push(format!("{name} (Project {project_id})"));
            }
        }
        if stale.is_empty() {
            return Ok(());
        }
        Err(environment_error(format!(
            "The entry was replaced, but these Environment secrets could not take the new value and still hold the old one: {}. Fix them or replace the entry again.",
            stale.join(", ")
        )))
    }

    fn refill_environment_secret(
        &self,
        project_id: &str,
        name: &str,
        entry_id: &str,
    ) -> Result<(), DaemonError> {
        let service = self.home_runtime_secret_service()?;
        let target = Self::environment_use_target(&service, entry_id, name)?;
        let actor = "Environment secret link";
        let attempt = service.begin_vault_use(
            entry_id,
            &format!(
                "Use attempted at {} by {actor}",
                super::vault_use::vault_use_destination(&target)
            ),
        )?;
        let value =
            zeroize::Zeroizing::new(service.workspace_secret_input_for_target(entry_id, &target)?);
        attempt.finish(&format!(
            "Used at {} by {actor}",
            super::vault_use::vault_use_destination(&target)
        ))?;
        // A value typed meanwhile ended the link and is kept.
        self.secret_links().write_if_linked(
            project_id,
            name,
            &crate::project_environment::SecretLink::Entry(entry_id.to_owned()),
            || self.store_environment_secret(project_id, name, &value),
        )?;
        Ok(())
    }

    /// A raw write of the Vault value at `key`: every entry whose value lives there re-fills its Secrets,
    /// the same as the owner's Replace. Try every entry before reporting any refresh failures.
    pub(crate) fn refill_environment_secrets_at_vault_key(
        &self,
        key: &str,
    ) -> Result<(), DaemonError> {
        let entries = crate::credential::CharioxCredentialRegistry::user()?.list()?;
        let mut failures = Vec::new();
        for entry in entries {
            if matches!(&entry.source, crate::config::UserCredentialSourceConfig::Vault { key: at } if at == key)
            {
                if let Err(error) = self.refill_linked_environment_secrets(&entry.id) {
                    tracing::warn!(%error, "an Environment secret could not be re-filled after a raw Vault write");
                    failures.push(format!("{}: {error}", entry.id));
                }
            }
        }
        if failures.is_empty() {
            return Ok(());
        }
        Err(environment_error(format!(
            "The Vault value was stored, but some linked Environment secrets could not be refreshed: {}",
            failures.join("; ")
        )))
    }

    /// The owner deleted the entry: Secrets filled from it keep their value as normal Secrets.
    pub(crate) fn unlink_deleted_vault_entry(&self, entry_id: &str) {
        if let Err(error) = self.secret_links().unlink_entry(entry_id) {
            tracing::warn!(%error, "the Environment secret links could not be saved");
        }
    }

    /// The use the entry was made for, when that is a command's environment variable of this name.
    fn environment_use_target(
        service: &crate::secret::RuntimeSecretService,
        credential_id: &str,
        name: &str,
    ) -> Result<crate::config::VaultUseTarget, DaemonError> {
        let binding = service
            .credential(credential_id)?
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.use_binding.as_ref());
        match binding.map(|binding| &binding.target) {
            Some(target @ crate::config::VaultUseTarget::CommandEnvironment { name: bound, .. })
                if bound == name =>
            {
                Ok(target.clone())
            }
            _ => Err(environment_error(format!(
                "this Vault entry is not made for the environment variable {name}; capture it for {name}"
            ))),
        }
    }

    pub(crate) fn run_project_environment_setup(
        &self,
        request: RunProjectEnvironmentSetupRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let project = self.owned_project(&request.project_id, caller_user_id)?;
        let checkout = self.project_checkout(&project, request.checkout_path.as_deref())?;
        let record = self
            .environment_store()
            .load(project.id())?
            .filter(|record| !record.setup_script.trim().is_empty())
            .ok_or_else(|| environment_error("write a Setup script before running it"))?;
        let config = self.owned.config_projection.snapshot();
        let launch =
            crate::project_environment::project_launch_environment(&config, project.id(), &record)?;
        let secrets = record
            .secrets
            .iter()
            .filter_map(|name| {
                launch
                    .environment
                    .iter()
                    .find(|(existing, _)| existing == name)
                    .map(|(_, value)| (name.clone(), zeroize::Zeroizing::new(value.to_string())))
            })
            .collect();
        let environment = launch
            .environment
            .iter()
            .map(|(name, value)| (name.to_string(), zeroize::Zeroizing::new(value.to_string())))
            .collect();
        let operation = self.owned.project_environment_setups.start(SetupSpec {
            operation_id: request.operation_id,
            project_id: project.id().into(),
            checkout,
            script_digest: record.setup_digest(),
            script: record.setup_script,
            environment,
            secrets,
        })?;
        Ok(LocalDaemonResponse::EnvironmentOperation { operation })
    }

    /// The operation now, or, when the reader says how many updates it has seen, the first state with
    /// more of them (or an ended one), at most `OPERATION_WAIT` later.
    pub(crate) async fn get_environment_operation(
        &self,
        request: GetEnvironmentOperationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut changes = self.owned.environment_operation_changes.subscribe();
        let deadline = tokio::time::Instant::now() + OPERATION_WAIT;
        loop {
            let operation =
                self.read_environment_operation(&request.operation_id, caller_user_id)?;
            let seen = request.wait_after_updates;
            let settled = seen.is_none_or(|seen| {
                operation.updates > seen || operation.state != EnvironmentOperationState::Running
            });
            if settled
                || tokio::time::timeout_at(deadline, changes.changed())
                    .await
                    .is_err()
            {
                return Ok(LocalDaemonResponse::EnvironmentOperation { operation });
            }
        }
    }

    /// The operation as it is now, for the owner of its Project.
    pub(crate) fn read_environment_operation(
        &self,
        operation_id: &str,
        caller_user_id: &str,
    ) -> Result<EnvironmentOperationView, DaemonError> {
        let operation = self
            .environment_operation(operation_id)?
            .ok_or_else(|| environment_error("this operation is unknown"))?;
        self.owned_project(&operation.project_id, caller_user_id)?;
        Ok(operation)
    }

    pub(crate) fn cancel_environment_operation(
        &self,
        request: EnvironmentOperationRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let operation = self
            .environment_operation(&request.operation_id)?
            .ok_or_else(|| environment_error("this operation is unknown"))?;
        self.owned_project(&operation.project_id, caller_user_id)?;
        Ok(LocalDaemonResponse::EnvironmentOperation {
            operation: self.cancel_environment_operation_by_id(&request.operation_id)?,
        })
    }

    /// A new worktree of a Project gets the Environment's Files outside Git copied in.
    pub(crate) fn copy_environment_files_into_worktree(
        &self,
        base: &Path,
        worktree: &Path,
    ) -> Result<(), DaemonError> {
        self.environment_store()
            .copy_files_into_worktree(&self.owned.session_store, base, worktree)
    }

    /// Moves and slices export the listed files with the checkout, so the ignore rules must know them.
    pub(crate) fn register_environment_files(
        &self,
        project_id: &str,
        checkouts: &[PathBuf],
    ) -> Result<(), DaemonError> {
        let files = self
            .environment_store()
            .load(project_id)?
            .map(|record| record.files)
            .unwrap_or_default();
        crate::project_environment::register_environment_files(checkouts, &files);
        Ok(())
    }

    // Idle prompt admission refreshes through normal activation; `vault_use` is how long it waits
    // on the Vault unlock card (see `with_project_prompt_environment`).
    pub(super) async fn refresh_project_prompt_provider(
        &self,
        session_id: &str,
        agent_id: &str,
        vault_use: &'static str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.remote_execution().is_some()
            || self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
                .is_some()
            // An existing backlog is prepared by detached queue promotion,
            // including retries. Never await its Project/Vault inputs here.
            || self.owned.prompt_state_owner
                .peek_next_queued_prompt(&session, agent_id).is_some()
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
        self.with_project_prompt_environment(session_id, agent_id, vault_use, |app| {
            app.ensure_prompt_provider_run_for_agent(session_id, agent_id)
                .map(|_| ())
        })
        .await
    }

    // Keep an Always-policy unlock alive through resolution, replacement and queue activation;
    // failures leave the unactivated queue intact. A launch that turns out to need the locked
    // Vault (a Claude token, no Project Secret to unlock for up front) asks the owner and runs
    // once more, so a queued prompt never waits on a popup nobody was shown. `vault_use` says
    // how long the caller waits on the unlock card: a submit opens it and goes on
    // (`VAULT_ADMIT_PROMPT`), a queued prompt's start waits until it ends
    // (`VAULT_START_QUEUED_PROMPT`).
    pub(super) async fn with_project_prompt_environment<T>(
        &self,
        session_id: &str,
        agent_id: &str,
        vault_use: &'static str,
        mut operation: impl FnMut(&mut DaemonApp) -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let has_secrets = self
            .environment_store()
            .load(session.project_id())?
            .is_some_and(|record| !record.secrets.is_empty());
        let _vault = if has_secrets {
            Some(
                self.ensure_vault_unlocked_for_agent(session_id, agent_id, vault_use)
                    .await?,
            )
        } else {
            None
        };
        match self.with_authorized_app_side_effect(&mut operation).await {
            Err(error)
                if _vault.is_none() && crate::secret::is_chariox_vault_locked_error(&error) =>
            {
                let _vault = self
                    .ensure_vault_unlocked_for_agent(session_id, agent_id, vault_use)
                    .await?;
                self.with_authorized_app_side_effect(operation).await
            }
            result => result,
        }
    }

    /// The Project's Variables and Secret values for a terminal the Room opens.
    pub(crate) fn project_environment_for_terminal(
        &self,
        session_id: &str,
    ) -> Result<std::collections::BTreeMap<String, String>, DaemonError> {
        Ok(self
            .project_environment_for_shell(session_id)?
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect())
    }

    /// A locked Vault leaves the Secrets out rather than blocking a shell or a terminal; the
    /// provider launch is the one place that asks the owner to unlock it.
    pub(crate) fn project_environment_for_shell(
        &self,
        session_id: &str,
    ) -> Result<crate::provider::ProviderCredentialEnvironment, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(project) = self.owned.session_store.environment_project(&session) else {
            return Ok(Default::default());
        };
        let Some(record) = self.environment_store().load(project.id())? else {
            return Ok(Default::default());
        };
        match crate::project_environment::project_launch_environment(
            &self.owned.config_projection.snapshot(),
            project.id(),
            &record,
        ) {
            Ok(launch) => Ok(launch.environment),
            Err(error) if crate::secret::is_chariox_vault_locked_error(&error) => {
                let mut environment = crate::provider::ProviderCredentialEnvironment::default();
                for (name, value) in crate::project_environment::parse_variables(&record.variables)
                    .map_err(environment_error)?
                {
                    environment.insert(name, zeroize::Zeroizing::new(value));
                }
                Ok(environment)
            }
            Err(error) => Err(error),
        }
    }
}
