use super::vault_unlock_requests::{
    notify_vault_unlocked, vault_unlock_notifications, vault_unlock_session_request_lock,
};
use super::*;

pub(crate) struct VaultUnlockGuard {
    path: Option<std::path::PathBuf>,
    lock_on_drop: bool,
}

impl VaultUnlockGuard {
    fn unlocked_for_operation(path: std::path::PathBuf) -> Self {
        Self {
            path: Some(path),
            lock_on_drop: true,
        }
    }

    fn unlocked_until_expiry(path: std::path::PathBuf) -> Self {
        Self {
            path: Some(path),
            lock_on_drop: false,
        }
    }

    fn for_existing_unlock(path: std::path::PathBuf) -> Result<Self, DaemonError> {
        // A client "once" unlock belongs to the next operation, including an
        // agent call resumed by another window's unlock notification.
        if crate::secret::take_chariox_vault_single_use_unlock(&path)? {
            Ok(Self::unlocked_for_operation(path))
        } else {
            Ok(Self::unlocked_until_expiry(path))
        }
    }

    fn not_required() -> Self {
        Self {
            path: None,
            lock_on_drop: false,
        }
    }

    /// No guard keeps the vault open: a lease can expire and another
    /// operation's guard locks it for everyone. Re-check before reuse.
    pub(crate) fn still_unlocked(&self) -> bool {
        self.path.as_ref().is_none_or(|path| {
            crate::secret::chariox_encrypted_vault_status(path).is_ok_and(|status| status.unlocked)
        })
    }
}

impl Drop for VaultUnlockGuard {
    fn drop(&mut self) {
        if !self.lock_on_drop {
            return;
        }
        if let Some(path) = self.path.as_ref() {
            let _ = crate::secret::lock_chariox_encrypted_vault(path);
            let _ = crate::secret::clear_vault_secret_process_cache();
        }
    }
}

pub(super) struct CloseInteractionOnDrop {
    pub(super) state: KernelRuntimeState,
    pub(super) session_id: String,
    pub(super) interaction_id: String,
}

impl Drop for CloseInteractionOnDrop {
    fn drop(&mut self) {
        // No-op once the interaction was answered or timed out.
        let _ = self
            .state
            .owned
            .timeout_runtime_interaction(&self.session_id, &self.interaction_id);
    }
}

impl KernelRuntimeState {
    pub(in crate::runtime) async fn prepare_provider_launch_request_with_vault(
        &self,
        request: crate::provider::LaunchProviderRequest,
        operation: &'static str,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let request = self
            .resolve_provider_launch_account_authority(request)
            .await?;
        let request = self.owned.prepare_provider_launch_account(request)?;
        let _vault_unlock = self
            .ensure_provider_account_vault_unlocked_for_launch(&request, operation)
            .await?;
        let mut request = self
            .owned
            .prepare_provider_launch_request(request, config.runtime_mcp_url())?;
        let credentials = std::mem::take(&mut request.provider_credential_env);
        drop(_vault_unlock);
        let credentials = self
            .checked_claude_credentials(&request, credentials)
            .await?;
        Ok(request.with_provider_credential_env(credentials))
    }

    /// The human part of an idle provider reload: hold the returned guard
    /// across `reload_agent_provider_if_idle` so the reload never prompts.
    pub(super) async fn unlock_vault_for_agent_reload(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let Some(run) = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
        else {
            return Ok(VaultUnlockGuard::not_required());
        };
        let request = super::provider_reload::policy_reload_launch_request(
            &run,
            agent_id,
            Default::default(),
        );
        let request = self
            .resolve_provider_launch_account_authority(request)
            .await?;
        self.ensure_provider_account_vault_unlocked_for_launch(&request, "reload provider run")
            .await
    }

    pub(super) async fn prepare_leased_provider_launch_request_with_vault(
        &self,
        request: crate::provider::LaunchProviderRequest,
        operation: &'static str,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let request = self.owned.prepare_leased_provider_launch_account(request)?;
        self.prepare_provider_launch_request_with_vault(request, operation)
            .await
    }

    async fn ensure_provider_account_vault_unlocked_for_launch(
        &self,
        request: &crate::provider::LaunchProviderRequest,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        if self.owned.claude_enrollment_is_rejected(request)? {
            return Ok(VaultUnlockGuard::not_required());
        }
        let config = self.owned.config_projection.snapshot();
        if config.user_config.credential_vault.backend
            != crate::config::CredentialVaultBackend::CharioxEncrypted
            || crate::provider::canonical_provider_family(&request.provider) != Some("claude")
        {
            return Ok(VaultUnlockGuard::not_required());
        }
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let agent = request
            .agent_id
            .as_deref()
            .and_then(|agent_id| self.owned.agent_store.get_agent(agent_id).ok())
            .or_else(|| {
                session
                    .focused_agent_id()
                    .and_then(|agent_id| self.owned.agent_store.get_agent(agent_id).ok())
            });
        let runtime_owner_user_id = agent
            .as_ref()
            .map(|agent| agent.owner_user_id())
            .unwrap_or_else(|| session.owner_user_id());
        let mut owned_request = request.clone();
        owned_request.owner_user_id = runtime_owner_user_id.to_string();
        let account_owner_user_id = match request.provider_account_owner_user_id.clone() {
            Some(owner)
                if owner
                    != crate::account_profile::provider_account_authority_owner_user_id(
                        &config,
                        runtime_owner_user_id,
                    ) =>
            {
                owner
            }
            _ => crate::account_profile::provider_account_authority_for_launch(
                &config,
                &self.owned.provider_account_profiles,
                &owned_request,
            )?,
        };
        let profile = self.owned.provider_account_profiles.get(
            &account_owner_user_id,
            &request.provider,
            &request.account_profile,
        )?;
        if !crate::provider::launch_uses_vault_credential(
            &self.owned.provider_account_profiles,
            &account_owner_user_id,
            &request.provider,
            &profile.profile_id,
            request.client_interface,
        )? {
            return Ok(VaultUnlockGuard::not_required());
        }
        let agent_id = agent
            .as_ref()
            .map(|agent| agent.id())
            .or(request.agent_id.as_deref())
            .unwrap_or("provider");
        self.ensure_vault_unlocked_for_agent(session.id(), agent_id, operation)
            .await
    }

    pub(super) async fn resolve_provider_account_credentials_for_run_with_vault(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        operation: &'static str,
    ) -> Result<crate::provider::ProviderCredentialEnvironment, DaemonError> {
        let mut request = crate::provider::LaunchProviderRequest::new(
            run.session_id(),
            run.adapter_key(),
            run.provider(),
            run.account_profile(),
            run.model(),
        )
        .with_owner_user_id(run.owner_user_id().to_string());
        crate::account_profile::copy_provider_account_selection(run, &mut request);
        request.client_interface = run.client_interface();
        if let Some(agent_id) = run.agent_instance_id() {
            request = request.with_agent_id(agent_id.to_string());
        }
        let request = self
            .resolve_provider_launch_account_authority(request)
            .await?;
        let _vault_unlock = self
            .ensure_provider_account_vault_unlocked_for_launch(&request, operation)
            .await?;
        // MP-08 / MP-10 / MP-11: Vault UI is an await boundary. A retired
        // starting run must not verify credentials or begin account recovery.
        if run.state() == crate::provider::ProviderRunState::Starting
            && self
                .owned
                .provider_store
                .get_run(run.id())
                .ok()
                .is_none_or(|current| {
                    current.state() != crate::provider::ProviderRunState::Starting
                        || current.started_at_ms() != run.started_at_ms()
                })
        {
            return Err(DaemonError::LocalTransport {
                operation,
                message: "provider run retired while awaiting Vault unlock".to_string(),
            });
        }
        let config = self.owned.config_projection.snapshot();
        let account_owner_user_id = self.resolved_provider_account_owner_for_run(run).await?;
        let credentials = if self.owned.claude_enrollment_is_rejected(&request)? {
            crate::provider::ProviderCredentialEnvironment::default()
        } else {
            crate::provider::resolve_provider_account_credentials(
                &config,
                &account_owner_user_id,
                run.provider(),
                run.account_profile(),
            )?
        };
        drop(_vault_unlock);
        self.checked_claude_credentials(&request, credentials).await
    }

    pub(super) async fn resolve_remote_provider_launch_credential(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
    ) -> Result<Option<crate::transport::relay_peer::RemoteProviderLaunchCredential>, DaemonError>
    {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        if crate::provider::canonical_provider_family(agent.provider()) != Some("claude") {
            return Ok(None);
        }

        let config = self.owned.config_projection.snapshot();
        let execution_owner_user_id = self
            .with_app_side_effect(|app| {
                app.provider_account_owner_for_execution(session_id, Some(agent_id))
            })
            .await?;
        let account_owner_user_id = if execution_owner_user_id
            != crate::account_profile::provider_account_authority_owner_user_id(
                &config,
                agent.owner_user_id(),
            ) {
            execution_owner_user_id
        } else {
            crate::account_profile::provider_account_authority_owner_for_profile(
                &config,
                &self.owned.provider_account_profiles,
                agent.owner_user_id(),
                agent.provider(),
                agent.provider_account_profile(),
            )?
        };
        let profile = self.owned.provider_account_profiles.get(
            &account_owner_user_id,
            agent.provider(),
            agent.provider_account_profile(),
        )?;
        let mut request = crate::provider::LaunchProviderRequest::new(
            agent.session_id(),
            crate::provider::adapter_key_for_provider(agent.provider()),
            agent.provider(),
            &profile.profile_id,
            agent.model().unwrap_or_default(),
        )
        .with_owner_user_id(agent.owner_user_id().to_string())
        .with_agent_id(agent.id().to_string());
        crate::account_profile::bind_provider_account_authority(
            &mut request,
            account_owner_user_id.clone(),
            agent.provider_account_profile().to_string(),
        );
        let request = self
            .resolve_provider_launch_account_authority(request)
            .await?;
        if self.owned.claude_enrollment_is_rejected(&request)? {
            return Err(DaemonError::InvalidConfig {
                field: "provider account credential",
                message: "Remote Claude execution needs a renewed portable credential; open Provider Accounts on the home kernel to renew it.",
            });
        }
        let _vault_unlock = self
            .ensure_provider_account_vault_unlocked_for_launch(&request, operation)
            .await?;
        let mut environment = crate::provider::resolve_provider_account_credentials(
            &config,
            &account_owner_user_id,
            agent.provider(),
            &profile.profile_id,
        )?;
        drop(_vault_unlock);
        environment = self
            .checked_claude_credentials(&request, environment)
            .await?;
        let token = environment
            .remove(crate::provider::CLAUDE_OAUTH_TOKEN_ENV)
            .filter(|value| !value.trim().is_empty())
            .ok_or(DaemonError::InvalidConfig {
                field: "provider account credential",
                message: "remote Claude launch requires sign-in; choose Log in in Provider Accounts on the home kernel",
            })?;
        Ok(Some(
            crate::transport::relay_peer::RemoteProviderLaunchCredential {
                provider: "claude".to_string(),
                account_profile: profile.profile_id,
                secret_input:
                    crate::transport::relay_peer::RemoteCredentialSecretInput::from_zeroizing(token),
            },
        ))
    }

    pub(super) async fn await_vault_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        operation: &'static str,
        interaction_name: &'static str,
    ) -> Result<super::PendingInteractionResolution, DaemonError> {
        let deadline = interaction
            .timeout_sec()
            .map(|seconds| tokio::time::Instant::now() + std::time::Duration::from_secs(seconds));
        self.await_vault_interaction_until(
            session_id,
            interaction,
            operation,
            interaction_name,
            deadline,
        )
        .await
    }

    pub(super) async fn await_vault_interaction_until(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        operation: &'static str,
        interaction_name: &'static str,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<super::PendingInteractionResolution, DaemonError> {
        let interaction_id = interaction.id().to_string();
        let mut resolution_rx =
            self.create_terminal_credential_interaction(session_id, interaction)?;
        // A cancelled waiter closes its popup instead of leaving it open to
        // silently discard a passphrase.
        let _close = CloseInteractionOnDrop {
            state: self.clone(),
            session_id: session_id.to_string(),
            interaction_id: interaction_id.clone(),
        };
        let resolution = match deadline {
            Some(deadline) => tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline) => {
                    self.timeout_runtime_interaction(session_id, &interaction_id).await?;
                    // A queued late answer cannot extend an expired operation.
                    Ok(super::PendingInteractionResolution {
                        status: "timed_out", choice_id: None, reply: None,
                    })
                }
                result = &mut resolution_rx => result,
            },
            None => resolution_rx.await,
        };
        resolution.map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!("{interaction_name} interaction dropped before resolution: {error}"),
        })
    }

    pub(crate) async fn ensure_vault_unlocked_for_command_context(
        &self,
        command: &crate::runtime::command::KernelCommand,
        session_id: Option<&str>,
        agent_id: Option<&str>,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        if let Some(path) = self.client_single_use_unlock_path()? {
            // "Ask every time": the owner's client unlocked for this one operation.
            return VaultUnlockGuard::for_existing_unlock(path);
        }
        let Some((session_id, agent_id)) = self
            .preflight_vault_unlock_for_command_context(command, session_id, agent_id, operation)
            .await?
        else {
            return Ok(VaultUnlockGuard::not_required());
        };
        self.ensure_vault_unlocked_for_agent(&session_id, &agent_id, operation)
            .await
    }

    /// The Vault's path while the owner's client holds it open on a single-use
    /// unlock ("ask every time"), which the next operation uses up.
    fn client_single_use_unlock_path(&self) -> Result<Option<std::path::PathBuf>, DaemonError> {
        let vault = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        if vault.backend != crate::config::CredentialVaultBackend::CharioxEncrypted {
            return Ok(None);
        }
        let path = expand_vault_path(&vault.path);
        Ok(crate::secret::chariox_vault_single_use_unlocked(&path)?.then_some(path))
    }

    /// Validate where an unlock interaction can be shown without acquiring a
    /// Vault lease. Verification of caller-supplied input can then run while
    /// the Vault stays locked; the actual write repeats this same preflight.
    pub(crate) async fn preflight_vault_unlock_for_command_context(
        &self,
        command: &crate::runtime::command::KernelCommand,
        session_id: Option<&str>,
        agent_id: Option<&str>,
        operation: &'static str,
    ) -> Result<Option<(String, String)>, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        if user_config.credential_vault.backend
            != crate::config::CredentialVaultBackend::CharioxEncrypted
        {
            return Ok(None);
        }
        let vault_path = expand_vault_path(&user_config.credential_vault.path);
        if crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
            return Ok(None);
        }
        if self.client_single_use_unlock_path()?.is_some() {
            return Ok(None);
        }
        let session_id = session_id
            .or(command.session_id.as_deref())
            .ok_or_else(super::vault_unlock_request::vault_locked_error)?;
        let agent_id = self
            .vault_prompt_agent(
                session_id,
                agent_id.or(command.agent_id.as_deref()),
                operation,
            )
            .await?;
        Ok(Some((session_id.to_owned(), agent_id)))
    }

    /// The unlock popup is an agent's runtime interaction. Without a named
    /// agent it goes to the session's focus agent, the terminal the person is
    /// using; a session without agents cannot show it.
    pub(crate) async fn vault_prompt_agent(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
        operation: &'static str,
    ) -> Result<String, DaemonError> {
        if let Some(agent_id) = agent_id {
            return Ok(agent_id.to_owned());
        }
        self.focused_agent_id(session_id)
            .await?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: "encrypted Chariox vault access requires an agent in the session so the unlock popup can be shown".to_string(),
            })
    }

    pub(super) async fn ensure_vault_unlocked_for_provider_run(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let agent_id =
            provider_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation,
                    message: "provider run is not bound to an agent for vault unlock".to_string(),
                })?;
        self.ensure_vault_unlocked_for_agent(provider_run.session_id(), agent_id, operation)
            .await
    }

    pub(super) async fn ensure_vault_unlocked_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let vault_config = user_config.credential_vault;
        if vault_config.backend != crate::config::CredentialVaultBackend::CharioxEncrypted {
            return Ok(VaultUnlockGuard::not_required());
        }
        let vault_path = expand_vault_path(&vault_config.path);
        if crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
            return VaultUnlockGuard::for_existing_unlock(vault_path);
        }
        // The Room queue and passphrase retries share the provider tool's budget.
        // No card may outlive the call that is waiting for it.
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(vault_unlock_card_seconds(operation));
        let session_request_lock = vault_unlock_session_request_lock(&vault_path, session_id);
        let _dedupe_guard = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => return Err(vault_unlock_timed_out()),
            guard = session_request_lock.lock() => guard,
        };
        let mut unlocks = vault_unlock_notifications(&vault_path);
        if crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
            return VaultUnlockGuard::for_existing_unlock(vault_path);
        }

        let mut problem: Option<String> = None;
        let (status, lock_after_operation) = loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let seconds = remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
            let mut interaction = vault_passphrase_interaction(
                &self.vault_requester(session_id, agent_id),
                agent_id,
                &vault_config,
                seconds.max(1),
            );
            if let Some(problem) = problem.as_deref() {
                interaction = interaction.noting_problem(problem);
            }
            let Some(resolution) = self
                .await_vault_unlock_interaction(
                    session_id,
                    interaction.clone(),
                    operation,
                    &vault_path,
                    &mut unlocks,
                    deadline,
                )
                .await?
            else {
                return VaultUnlockGuard::for_existing_unlock(vault_path.clone());
            };
            let (passphrase, duration) = vault_unlock_answer(&interaction, resolution, operation)?;
            let lease = duration.lease(&vault_config, true);
            let lock_after_operation = lease == crate::secret::VaultUnlockLease::Operation;
            // Human input must never hold the mutation lock across sessions.
            let unlock_request_lock = vault_unlock_request_lock(&vault_path);
            let _mutation = unlock_request_lock.lock().await;
            if crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
                return VaultUnlockGuard::for_existing_unlock(vault_path.clone());
            }
            let owner = self
                .owned
                .session_store
                .get_session(session_id)?
                .owner_user_id()
                .to_owned();
            let status = match self
                .unlock_vault_counted(&owner, &vault_path, passphrase.as_str(), lease, true)
                .await
            {
                Ok(status) => status,
                Err(DaemonError::VaultUnlock {
                    message,
                    retryable: true,
                    ..
                }) => {
                    problem = Some(message);
                    continue;
                }
                Err(error) => return Err(error),
            };
            break (status, lock_after_operation);
        };
        crate::logging::info_with_fields(
            "credential_vault",
            "Chariox vault unlocked",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "operation": operation,
                "path": status.path.display().to_string(),
                "expires_at_ms": status.expires_at_ms,
                "lock_after_operation": lock_after_operation,
            }),
        );
        if !lock_after_operation {
            notify_vault_unlocked(&vault_path, crate::local::VaultUnlockSurface::Card);
        }
        if lock_after_operation {
            Ok(VaultUnlockGuard::unlocked_for_operation(vault_path))
        } else {
            Ok(VaultUnlockGuard::unlocked_until_expiry(vault_path))
        }
    }

    pub(crate) async fn manage_credential_vault_unlock(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(crate::secret::CharioxVaultUnlockStatus, String), DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let vault_config = user_config.credential_vault;
        if vault_config.backend != crate::config::CredentialVaultBackend::CharioxEncrypted {
            let status = crate::secret::chariox_encrypted_vault_status(expand_vault_path(
                &vault_config.path,
            ))?;
            return Ok((status, "not_required".to_string()));
        }
        let vault_path = expand_vault_path(&vault_config.path);
        let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
        if !status.unlocked {
            let _guard = self
                .ensure_vault_unlocked_for_agent(session_id, agent_id, "credential_vault_manage")
                .await?;
            let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
            return Ok((status, "unlocked".to_string()));
        }

        let resolution = self
            .await_vault_interaction(
                session_id,
                vault_manage_interaction(&self.vault_requester(session_id, agent_id), agent_id),
                "credential_vault_manage",
                "vault management",
            )
            .await?;
        let choice_id = resolution.choice_id.as_deref().unwrap_or("dismiss");
        if choice_id == "change_passphrase" {
            return self
                .change_credential_vault_passphrase(session_id, agent_id, &vault_path)
                .await;
        }
        apply_vault_manage_choice(&vault_path, choice_id, &vault_config)
    }

    /// Asks for the current passphrase and the new one twice, each in its own
    /// secret interaction, then changes it (`change_vault_passphrase`, which
    /// also rotates the passkey). The current passphrase is checked exactly
    /// (no case folding), and nothing changes unless both new entries match.
    async fn change_credential_vault_passphrase(
        &self,
        session_id: &str,
        agent_id: &str,
        vault_path: &std::path::Path,
    ) -> Result<(crate::secret::CharioxVaultUnlockStatus, String), DaemonError> {
        let unlock_request_lock = vault_unlock_request_lock(vault_path);
        let _dedupe_guard = unlock_request_lock.lock().await;
        let owner = self
            .owned
            .session_store
            .get_session(session_id)?
            .owner_user_id()
            .to_owned();
        let current = self
            .vault_passphrase_change_entry(session_id, agent_id, VaultPassphraseChangeStep::Current)
            .await?;
        let new = self
            .vault_passphrase_change_entry(session_id, agent_id, VaultPassphraseChangeStep::New)
            .await?;
        let repeat = self
            .vault_passphrase_change_entry(session_id, agent_id, VaultPassphraseChangeStep::Repeat)
            .await?;
        if new.as_str() != repeat.as_str() {
            return Err(DaemonError::LocalTransport {
                operation: VAULT_PASSPHRASE_CHANGE,
                message:
                    "the two new Chariox vault passphrases differ; the passphrase is unchanged"
                        .to_string(),
            });
        }
        let status = self
            .change_vault_passphrase(&owner, vault_path, current, new)
            .await?;
        crate::logging::info_with_fields(
            "credential_vault",
            "Chariox vault passphrase changed",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "path": status.path.display().to_string(),
            }),
        );
        Ok((status, "passphrase_changed".to_string()))
    }

    async fn vault_passphrase_change_entry(
        &self,
        session_id: &str,
        agent_id: &str,
        step: VaultPassphraseChangeStep,
    ) -> Result<zeroize::Zeroizing<String>, DaemonError> {
        let resolution = self
            .await_vault_interaction(
                session_id,
                vault_passphrase_change_interaction(
                    &self.vault_requester(session_id, agent_id),
                    agent_id,
                    step,
                ),
                VAULT_PASSPHRASE_CHANGE,
                "vault passphrase change",
            )
            .await?;
        vault_secret_from_resolution(resolution, VAULT_PASSPHRASE_CHANGE)
    }
}

/// How a Vault prompt names who is asking: the agent's alias and the Room's
/// name when the kernel has them, never an id.
struct VaultRequester {
    agent: Option<String>,
    room: Option<String>,
}

impl VaultRequester {
    fn subject(&self) -> String {
        match (&self.agent, &self.room) {
            (Some(agent), Some(room)) => format!("{agent} in {room}"),
            (Some(agent), None) => agent.clone(),
            (None, Some(room)) => format!("An agent in {room}"),
            (None, None) => "An agent".to_string(),
        }
    }

    fn room_prefix(&self) -> String {
        self.room
            .as_ref()
            .map(|room| format!("{room}: "))
            .unwrap_or_default()
    }
}

impl KernelRuntimeState {
    fn vault_requester(&self, session_id: &str, agent_id: &str) -> VaultRequester {
        VaultRequester {
            agent: self
                .owned
                .agent_store
                .get_agent(agent_id)
                .ok()
                .and_then(|agent| agent.alias().map(str::to_owned)),
            room: self
                .owned
                .session_store
                .get_session(session_id)
                .ok()
                .and_then(|session| session.alias().map(str::to_owned)),
        }
    }
}

/// A person has to find and fill the card; the waiting tool call is cut by the provider
/// at 300 s, so the kernel ends the card itself just before that with a clear error.
const VAULT_UNLOCK_TOOL_CARD_SECONDS: u64 = 280;
const VAULT_UNLOCK_LAUNCH_CARD_SECONDS: u64 = 60;
const VAULT_PASSPHRASE_CHANGE: &str = "credential_vault_change_passphrase";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VaultPassphraseChangeStep {
    Current,
    New,
    Repeat,
}

fn vault_passphrase_change_interaction(
    requester: &VaultRequester,
    agent_id: &str,
    step: VaultPassphraseChangeStep,
) -> crate::session::RuntimeInteraction {
    let (slug, label, message) = match step {
        VaultPassphraseChangeStep::Current => (
            "current",
            "Current passphrase",
            "Enter the current vault passphrase (your Chariox passkey).",
        ),
        VaultPassphraseChangeStep::New => {
            ("new", "New passphrase", "Enter the new vault passphrase.")
        }
        VaultPassphraseChangeStep::Repeat => (
            "repeat",
            "Repeat new passphrase",
            "Enter the new vault passphrase again.",
        ),
    };
    crate::session::RuntimeInteraction::new(
        format!(
            "vault-passphrase-{slug}-{}-{}",
            agent_id,
            crate::session::unix_epoch_ms()
        ),
        agent_id,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Critical,
        Some("Change Chariox Vault Passphrase".to_string()),
        format!("{}{message}", requester.room_prefix()),
        vec![crate::session::RuntimeInteractionChoice::new(
            "cancel",
            "Cancel",
            "cancel",
            Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
        )],
        Some(crate::session::RuntimeInteractionCustomChoice::secret(
            "passphrase",
            label,
            Some("Passphrase".to_string()),
            Some(1),
            Some(512),
        )),
        Some(300),
        Some("cancel".to_string()),
    )
}

/// A launch must end quickly with retry guidance; an agent's tool call can wait for a person.
fn vault_unlock_card_seconds(operation: &str) -> u64 {
    let agent_tool = operation.starts_with("runtime_tool_")
        || matches!(
            operation,
            "vault_generate"
                | "credential_vault_manage"
                | "kernel_browser_paste_secret"
                | "vault_capture_browser"
                | "web_bot_auth"
        );
    if agent_tool {
        VAULT_UNLOCK_TOOL_CARD_SECONDS
    } else {
        VAULT_UNLOCK_LAUNCH_CARD_SECONDS
    }
}

fn vault_unlock_choice_label(duration: crate::local::VaultUnlockDuration) -> String {
    use crate::local::VaultUnlockDuration;
    match duration {
        VaultUnlockDuration::Once => "Just this once".to_string(),
        VaultUnlockDuration::Minutes { minutes } if minutes % 60 == 0 && minutes > 60 => {
            format!("{} hours", minutes / 60)
        }
        VaultUnlockDuration::Minutes { minutes: 60 } => "1 hour".to_string(),
        VaultUnlockDuration::Minutes { minutes: 1 } => "1 minute".to_string(),
        VaultUnlockDuration::Minutes { minutes } => format!("{minutes} minutes"),
        VaultUnlockDuration::UntilKernelStops => "Until the kernel stops".to_string(),
    }
}

fn vault_unlock_choice_id(duration: crate::local::VaultUnlockDuration) -> String {
    use crate::local::VaultUnlockDuration;
    match duration {
        VaultUnlockDuration::Once => "unlock_once".to_string(),
        VaultUnlockDuration::Minutes { minutes } => format!("unlock_{minutes}m"),
        VaultUnlockDuration::UntilKernelStops => "unlock_kernel".to_string(),
    }
}

fn vault_passphrase_interaction(
    requester: &VaultRequester,
    agent_id: &str,
    config: &crate::config::UserCredentialVaultConfig,
    seconds: u64,
) -> crate::session::RuntimeInteraction {
    let options = crate::local::VaultUnlockOptions::for_config(config);
    let mut choices = options
        .durations
        .iter()
        .map(|&duration| {
            let label = vault_unlock_choice_label(duration);
            crate::session::RuntimeInteractionChoice::new(
                vault_unlock_choice_id(duration),
                label.clone(),
                label,
                Some(if duration == options.default {
                    crate::session::RuntimeInteractionChoiceStyle::Primary
                } else {
                    crate::session::RuntimeInteractionChoiceStyle::Secondary
                }),
            )
            .unlocking_vault(duration)
        })
        .collect::<Vec<_>>();
    choices.push(crate::session::RuntimeInteractionChoice::new(
        "cancel",
        "Cancel",
        "cancel",
        Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
    ));
    crate::session::RuntimeInteraction::new(
        format!("vault-unlock-{}-{}", agent_id, rand::random::<u64>()),
        agent_id,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Critical,
        Some("Unlock Chariox Vault".to_string()),
        format!(
            "{} needs the Vault. Enter the vault passphrase.",
            requester.subject()
        ),
        choices,
        Some(crate::session::RuntimeInteractionCustomChoice::secret(
            "passphrase",
            "Vault passphrase",
            Some("Passphrase".to_string()),
            Some(1),
            Some(512),
        )),
        Some(seconds),
        None,
    )
}

fn vault_unlock_timed_out() -> DaemonError {
    DaemonError::VaultUnlock {
        code: "vault_unlock_timed_out",
        message: "Chariox vault unlock timed out. Retry to unlock the vault.".into(),
        retryable: false,
    }
}

/// The passphrase and duration the owner answered a Vault unlock question with.
fn vault_unlock_answer(
    interaction: &crate::session::RuntimeInteraction,
    resolution: super::PendingInteractionResolution,
    operation: &'static str,
) -> Result<
    (
        zeroize::Zeroizing<String>,
        crate::local::VaultUnlockDuration,
    ),
    DaemonError,
> {
    // A client without a duration control (the terminal) answers with the typed
    // passphrase alone and gets the duration the card preselects.
    let preselected = || {
        interaction
            .choices()
            .iter()
            .find(|choice| {
                choice.style() == Some(crate::session::RuntimeInteractionChoiceStyle::Primary)
            })
            .and_then(crate::session::RuntimeInteractionChoice::vault_unlock)
    };
    let duration = match resolution.choice_id.as_deref() {
        Some("passphrase") => preselected(),
        Some(id) => interaction
            .choice(id)
            .and_then(crate::session::RuntimeInteractionChoice::vault_unlock),
        None => None,
    };
    match (resolution.status, duration, resolution.reply) {
        ("timed_out", ..) => Err(vault_unlock_timed_out()),
        ("answered", Some(duration), Some(passphrase)) => {
            Ok((zeroize::Zeroizing::new(passphrase), duration))
        }
        (_, None, _) => Err(DaemonError::VaultUnlock {
            code: "vault_unlock_declined",
            message: "Chariox vault unlock was cancelled. Retry to unlock the vault.".into(),
            retryable: false,
        }),
        _ => Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock resolved without a passphrase".into(),
        }),
    }
}

fn vault_secret_from_resolution(
    resolution: super::PendingInteractionResolution,
    operation: &'static str,
) -> Result<zeroize::Zeroizing<String>, DaemonError> {
    if resolution.status == "timed_out" {
        return Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault passphrase change timed out".into(),
        });
    }
    match resolution.choice_id.as_deref() {
        None | Some("cancel") => Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault passphrase change was cancelled".into(),
        }),
        Some("passphrase") => resolution
            .reply
            .map(zeroize::Zeroizing::new)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: "Chariox vault passphrase change resolved without a passphrase".into(),
            }),
        Some(_) => Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault passphrase change resolved without a passphrase".into(),
        }),
    }
}

fn vault_manage_interaction(
    requester: &VaultRequester,
    agent_id: &str,
) -> crate::session::RuntimeInteraction {
    crate::session::RuntimeInteraction::new(
        format!(
            "vault-manage-{}-{}",
            agent_id,
            crate::session::unix_epoch_ms()
        ),
        agent_id,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        Some("Chariox Vault Unlocked".to_string()),
        format!(
            "{}The Vault is unlocked. Extend the unlock window, change the passphrase, or lock it now.",
            requester.room_prefix()
        ),
        vec![
            crate::session::RuntimeInteractionChoice::new(
                "extend_30m",
                "Extend 30 minutes",
                "extend_30m",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "extend_60m",
                "Extend 1 hour",
                "extend_60m",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "change_passphrase",
                "Change passphrase",
                "change_passphrase",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "lock_now",
                "Lock now",
                "lock_now",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "dismiss",
                "Dismiss",
                "dismiss",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
        ],
        None,
        Some(120),
        Some("dismiss".to_string()),
    )
}

fn apply_vault_manage_choice(
    vault_path: &std::path::Path,
    choice_id: &str,
    config: &crate::config::UserCredentialVaultConfig,
) -> Result<(crate::secret::CharioxVaultUnlockStatus, String), DaemonError> {
    match choice_id {
        "extend_30m" => {
            let status = crate::secret::extend_chariox_encrypted_vault(
                vault_path,
                crate::secret::VaultUnlockLease::TtlMinutes(30.min(config.max_ttl_minutes)),
            )?;
            Ok((status, "extended_30m".to_string()))
        }
        "extend_60m" => {
            let status = crate::secret::extend_chariox_encrypted_vault(
                vault_path,
                crate::secret::VaultUnlockLease::TtlMinutes(60.min(config.max_ttl_minutes)),
            )?;
            Ok((status, "extended_60m".to_string()))
        }
        "lock_now" => {
            crate::secret::lock_chariox_encrypted_vault(vault_path)?;
            crate::secret::clear_vault_secret_process_cache()?;
            let status = crate::secret::chariox_encrypted_vault_status(vault_path)?;
            Ok((status, "locked".to_string()))
        }
        _ => {
            let status = crate::secret::chariox_encrypted_vault_status(vault_path)?;
            Ok((status, "dismissed".to_string()))
        }
    }
}

pub(super) fn vault_unlock_request_lock(
    path: &std::path::Path,
) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    super::vault_unlock_requests::vault_unlock_request_lock(path)
}

pub(super) fn expand_vault_path(path: &str) -> std::path::PathBuf {
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
            return home;
        }
    }
    if let Some(suffix) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
            return home.join(suffix);
        }
    }
    std::path::PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolution(
        choice_id: Option<&str>,
        reply: Option<&str>,
    ) -> super::super::PendingInteractionResolution {
        super::super::PendingInteractionResolution {
            status: "answered",
            choice_id: choice_id.map(ToOwned::to_owned),
            reply: reply.map(ToOwned::to_owned),
        }
    }

    // MP-11 F1: exercise the actual wait and shared management reply path.
    #[tokio::test]
    async fn vault_management_reply_after_expiry_requires_unlock() {
        let workspace = crate::test_support::TestWorktree::new("vault-expired-reply");
        let private_state = crate::test_support::TestWorktree::new("vault-expired-reply-state");
        let path = private_state.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "test passphrase").unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        for choice in ["extend_30m", "extend_60m"] {
            crate::secret::unlock_chariox_encrypted_vault(
                &path,
                "test passphrase",
                crate::secret::VaultUnlockLease::KernelShutdown,
            )
            .unwrap();
            let call = state.manage_credential_vault_unlock(session.id(), agent.id());
            tokio::pin!(call);
            let interaction = tokio::select! {
                result = &mut call => panic!("management must await reply: {result:?}"),
                result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        let snapshot = state.owned.session_store.get_session(session.id()).unwrap();
                        if let Some(interaction) = snapshot.active_interaction_for_agent(agent.id()) {
                            break interaction.clone();
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                }) => result.expect("management interaction"),
            };
            crate::secret::expire_chariox_encrypted_vault_for_test(&path);
            state
                .owned
                .resolve_runtime_interaction(
                    session.id(),
                    interaction.id(),
                    choice,
                    None,
                    Some(session.owner_user_id()),
                    false,
                )
                .unwrap();
            let result = call.await;
            crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            assert!(result.is_err(), "reply must not revive an expired key");
            assert!(crate::secret::is_chariox_vault_locked_error(
                &result.unwrap_err()
            ));
            if choice == "extend_30m" {
                crate::secret::create_chariox_encrypted_vault_for_test(&path, "test passphrase")
                    .unwrap();
            }
        }
    }

    // A session created before this kernel was enrolled is owned by `local`;
    // after a restart its Vault decision must still be answered by the person
    // the kernel is enrolled for, and by nobody else.
    // VUI-2: an unlock somewhere else answers the open card "unlocked" and names where.
    #[tokio::test]
    async fn an_unlock_elsewhere_answers_the_open_card_as_unlocked_naming_where() {
        let workspace = crate::test_support::TestWorktree::new("vault-unlocked-elsewhere");
        let private_state =
            crate::test_support::TestWorktree::new("vault-unlocked-elsewhere-state");
        let path = private_state.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "test passphrase").unwrap();
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        let mut app = crate::app::DaemonApp::bootstrap(config.clone()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        let card = vault_passphrase_interaction(
            &requester(None, None),
            agent.id(),
            &config.user_config.credential_vault,
            60,
        );
        let answer = state
            .create_terminal_credential_interaction(session.id(), card.clone())
            .unwrap();
        state
            .owned
            .settle_vault_unlocked(
                session.id(),
                card.id(),
                crate::local::VaultUnlockSurface::VaultWindow,
            )
            .unwrap();
        let resolution = answer.await.unwrap();
        assert_eq!(
            (
                resolution.status,
                resolution.choice_id.as_deref(),
                resolution.reply.as_deref()
            ),
            ("answered", Some("unlocked"), Some("vault_window"))
        );
        assert!(state
            .owned
            .session_store
            .get_session(session.id())
            .unwrap()
            .active_interaction_for_agent(agent.id())
            .is_none());
        std::fs::remove_file(&path).unwrap();
    }

    // PCD-04: a wrong passphrase asks again in a card that says so; the agent's operation waits and then goes on.
    #[tokio::test(start_paused = true)]
    async fn a_wrong_passphrase_asks_again_instead_of_failing_the_agents_operation() {
        wrong_passphrase_retry(false).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_wrong_passphrase_keeps_the_original_tool_deadline() {
        wrong_passphrase_retry(true).await;
    }

    // PCD-04: clients never see the card leave while the passphrase is checked; the refusal arrives in the same card.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_wrong_passphrase_keeps_one_card_on_screen() {
        let workspace = crate::test_support::TestWorktree::new("vault-one-card");
        let private_state = crate::test_support::TestWorktree::new("vault-one-card-state");
        let path = private_state.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "test passphrase").unwrap();
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        let (session_id, agent_id) = (session.id().to_owned(), agent.id().to_owned());
        let asking = {
            let (state, session_id, agent_id) =
                (state.clone(), session_id.clone(), agent_id.clone());
            tokio::spawn(async move {
                state
                    .ensure_vault_unlocked_for_agent(&session_id, &agent_id, "runtime_tool_test")
                    .await
                    .map(|_| ())
            })
        };
        let visible = {
            let (state, session_id, agent_id) =
                (state.clone(), session_id.clone(), agent_id.clone());
            move || {
                state
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .unwrap()
                    .active_interactions()
                    .iter()
                    .filter(|card| card.agent_id() == Some(agent_id.as_str()))
                    .cloned()
                    .collect::<Vec<_>>()
            }
        };
        let first = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                if let Some(card) = visible().pop() {
                    return card;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the Vault card is on screen");
        let sampling = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let sampler = {
            let (visible, sampling) = (visible.clone(), sampling.clone());
            tokio::spawn(async move {
                let mut seen: Vec<Vec<(String, bool)>> = Vec::new();
                while sampling.load(std::sync::atomic::Ordering::SeqCst) {
                    let now = visible()
                        .iter()
                        .map(|card| {
                            (
                                card.id().to_owned(),
                                card.message().starts_with("That is not the Vault passphrase"),
                            )
                        })
                        .collect::<Vec<_>>();
                    if seen.last() != Some(&now) {
                        seen.push(now);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                seen
            })
        };
        let answer = |passphrase: &'static str| {
            let (state, session_id, id) = (state.clone(), session_id.clone(), first.id().to_owned());
            async move {
                state
                    .resolve_terminal_runtime_interaction(
                        &session_id,
                        &id,
                        "unlock_kernel",
                        Some(passphrase),
                        Some(crate::session::DEFAULT_LOCAL_USER_ID),
                    )
                    .await
            }
        };
        answer("not the passphrase").await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            while !visible()
                .pop()
                .is_some_and(|card| card.message().starts_with("That is not the Vault passphrase"))
            {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the refusal shows in the card");
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        sampling.store(false, std::sync::atomic::Ordering::SeqCst);
        let seen = sampler.await.unwrap();
        assert!(
            seen.iter().all(|cards| cards.len() == 1),
            "a client saw no card or two cards: {seen:?}"
        );
        assert!(
            seen.iter().all(|cards| cards[0].0 == first.id()),
            "the refusal came in another card: {seen:?}"
        );
        assert_eq!(visible().len(), 1);
        assert!(!asking.is_finished(), "the agent's operation still waits");
        answer("test passphrase").await.unwrap();
        asking
            .await
            .unwrap()
            .expect("the right passphrase after a wrong one still unlocks");
        assert!(visible().is_empty(), "the unlock closes the card");
        assert!(
            crate::secret::chariox_encrypted_vault_status(&path)
                .unwrap()
                .unlocked
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn an_expired_vault_wait_ignores_a_queued_answer() {
        let workspace = crate::test_support::TestWorktree::new("vault-queued-answer");
        let mut app =
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        let config = state
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        let interaction = vault_passphrase_interaction(
            &state.vault_requester(session.id(), agent.id()),
            agent.id(),
            &config,
            60,
        );
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let waiting = state.await_vault_interaction_until(
            session.id(),
            interaction.clone(),
            "runtime_tool_test",
            "vault unlock",
            Some(deadline),
        );
        tokio::pin!(waiting);
        tokio::select! {
            result = &mut waiting => panic!("must await a reply: {result:?}"),
            () = async {
                loop {
                    if state.owned.session_store.get_session(session.id()).unwrap()
                        .active_interaction_for_agent(agent.id()).is_some() { break }
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            } => {}
        }
        // Hold the waiter unpolled while the owner reply is queued after its budget.
        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        state
            .resolve_terminal_runtime_interaction(
                session.id(),
                interaction.id(),
                "passphrase",
                Some("fixture reply"),
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .unwrap();
        let resolution = waiting.await.unwrap();
        assert_eq!(resolution.status, "timed_out");
        assert!(
            resolution.reply.is_none(),
            "an expired waiter must discard the queued passphrase"
        );
    }

    // MP-08 / MP-09 / MP-10 / MP-11: queue time belongs to the waiting call.
    #[tokio::test(start_paused = true)]
    async fn a_queued_vault_unlock_card_gets_only_the_calls_remaining_budget() {
        queued_vault_unlock(false).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_queued_vault_unlock_times_out_without_acquiring_the_room_lock() {
        queued_vault_unlock(true).await;
    }

    // Two agents share one Room queue. Keep the first waiter under test control
    // so the second must expire even if the lock holder has not been scheduled.
    async fn queued_vault_unlock(hold_lock_past_deadline: bool) {
        let workspace = crate::test_support::TestWorktree::new("vault-room-queue");
        let private_state = crate::test_support::TestWorktree::new("vault-room-queue-state");
        let path = private_state.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "test passphrase").unwrap();
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let second_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(
                session.id(),
                "dev-stub",
            ))
            .unwrap();
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        let first =
            state.ensure_vault_unlocked_for_agent(session.id(), first_agent.id(), "vault_generate");
        tokio::pin!(first);
        let next_card = |agent_id: String| {
            let state = state.clone();
            let session_id = session.id().to_owned();
            async move {
                tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    loop {
                        if let Some(card) = state
                            .owned
                            .session_store
                            .get_session(&session_id)
                            .unwrap()
                            .active_interaction_for_agent(&agent_id)
                            .cloned()
                        {
                            return card;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                })
                .await
                .expect("the agent has an unlock card")
            }
        };
        let first_card = tokio::select! {
            result = &mut first => panic!("first agent must wait: {:?}", result.err()),
            card = next_card(first_agent.id().to_owned()) => card,
        };
        let second = state.ensure_vault_unlocked_for_agent(
            session.id(),
            second_agent.id(),
            "vault_generate",
        );
        tokio::pin!(second);
        let started = tokio::time::Instant::now();
        tokio::select! {
            result = &mut second => panic!("second agent must queue: {:?}", result.err()),
            () = tokio::time::sleep(std::time::Duration::from_millis(1)) => {},
        }
        assert!(state
            .owned
            .session_store
            .get_session(session.id())
            .unwrap()
            .active_interaction_for_agent(second_agent.id())
            .is_none());
        let second_card = if hold_lock_past_deadline {
            tokio::time::advance(std::time::Duration::from_secs(281)).await;
            None
        } else {
            tokio::time::advance(std::time::Duration::from_secs(200)).await;
            state
                .resolve_terminal_runtime_interaction(
                    session.id(),
                    first_card.id(),
                    "cancel",
                    None,
                    Some(crate::session::DEFAULT_LOCAL_USER_ID),
                )
                .await
                .unwrap();
            assert!(matches!(
                first.as_mut().await,
                Err(DaemonError::VaultUnlock {
                    code: "vault_unlock_declined",
                    ..
                })
            ));
            let card = tokio::select! {
                result = &mut second => panic!("second agent must await its card: {:?}", result.err()),
                card = next_card(second_agent.id().to_owned()) => card,
            };
            assert!(
                card.timeout_sec().unwrap() <= 80,
                "queue time must count against the original tool budget, got {:?}",
                card.timeout_sec()
            );
            Some(card)
        };
        let result = tokio::time::timeout_at(
            started + std::time::Duration::from_secs(282),
            second.as_mut(),
        )
        .await
        .expect("queued agent settles on its own deadline");
        assert!(matches!(
            result,
            Err(DaemonError::VaultUnlock {
                code: "vault_unlock_timed_out",
                ..
            })
        ));
        assert!(state
            .owned
            .session_store
            .get_session(session.id())
            .unwrap()
            .active_interaction_for_agent(second_agent.id())
            .is_none());
        if hold_lock_past_deadline {
            assert!(matches!(
                first.await,
                Err(DaemonError::VaultUnlock {
                    code: "vault_unlock_timed_out",
                    ..
                })
            ));
        }
        if let Some(card) = second_card {
            assert!(
                state
                    .resolve_terminal_runtime_interaction(
                        session.id(),
                        card.id(),
                        "unlock_kernel",
                        Some("test passphrase"),
                        Some(crate::session::DEFAULT_LOCAL_USER_ID),
                    )
                    .await
                    .is_err(),
                "a late answer must not revive the queued call"
            );
        }
        assert!(
            !crate::secret::chariox_encrypted_vault_status(&path)
                .unwrap()
                .unlocked
        );
        std::fs::remove_file(&path).unwrap();
    }

    async fn wrong_passphrase_retry(expire: bool) {
        let workspace = crate::test_support::TestWorktree::new("vault-wrong-passphrase");
        let private_state = crate::test_support::TestWorktree::new("vault-wrong-passphrase-state");
        let path = private_state.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "test passphrase").unwrap();
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        let (session_id, agent_id) = (session.id().to_owned(), agent.id().to_owned());
        let started = tokio::time::Instant::now();
        let asking = {
            let (state, session_id, agent_id) =
                (state.clone(), session_id.clone(), agent_id.clone());
            tokio::spawn(async move {
                state
                    .ensure_vault_unlocked_for_agent(&session_id, &agent_id, "runtime_tool_test")
                    .await
                    .map(|_| ())
            })
        };
        let next_card = |after: Option<String>| {
            let (state, session_id, agent_id) =
                (state.clone(), session_id.clone(), agent_id.clone());
            async move {
                tokio::time::timeout(std::time::Duration::from_secs(20), async {
                    loop {
                        if let Some(card) = state
                            .owned
                            .session_store
                            .get_session(&session_id)
                            .unwrap()
                            .active_interaction_for_agent(&agent_id)
                            .cloned()
                            .filter(|card| Some(card.id().to_owned()) != after)
                        {
                            return card;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                })
                .await
                .expect("the Vault card is on screen")
            }
        };
        let answer = |card: &crate::session::RuntimeInteraction, passphrase: &'static str| {
            let choice = card
                .choices()
                .iter()
                .find(|choice| {
                    choice.vault_unlock()
                        == Some(crate::local::VaultUnlockDuration::UntilKernelStops)
                })
                .expect("a duration choice")
                .id()
                .to_owned();
            let (state, session_id, id) = (state.clone(), session_id.clone(), card.id().to_owned());
            async move {
                state
                    .resolve_terminal_runtime_interaction(
                        &session_id,
                        &id,
                        &choice,
                        Some(passphrase),
                        Some(crate::session::DEFAULT_LOCAL_USER_ID),
                    )
                    .await
                    .unwrap()
            }
        };
        let first = next_card(None).await;
        tokio::time::advance(std::time::Duration::from_secs(200)).await;
        answer(&first, "not the passphrase").await;
        let again = next_card(Some(first.id().to_owned())).await;
        assert!(
            again
                .message()
                .starts_with("That is not the Vault passphrase"),
            "{}",
            again.message()
        );
        assert!(!asking.is_finished(), "the agent's operation still waits");
        if expire {
            assert!(
                again.timeout_sec().unwrap() <= 80,
                "the replacement gets only the original budget's remainder"
            );
            let result =
                tokio::time::timeout_at(started + std::time::Duration::from_secs(281), asking)
                    .await
                    .expect("the operation settles before its provider tool deadline")
                    .unwrap();
            assert!(matches!(
                result,
                Err(DaemonError::VaultUnlock {
                    code: "vault_unlock_timed_out",
                    ..
                })
            ));
            assert!(state
                .owned
                .session_store
                .get_session(&session_id)
                .unwrap()
                .active_interaction_for_agent(&agent_id)
                .is_none());
            assert!(
                !crate::secret::chariox_encrypted_vault_status(&path)
                    .unwrap()
                    .unlocked
            );
            assert!(
                state
                    .resolve_terminal_runtime_interaction(
                        &session_id,
                        again.id(),
                        "unlock_kernel",
                        Some("test passphrase"),
                        Some(crate::session::DEFAULT_LOCAL_USER_ID)
                    )
                    .await
                    .is_err(),
                "a late answer cannot resume the expired operation"
            );
            std::fs::remove_file(&path).unwrap();
            return;
        }
        answer(&again, "test passphrase").await;
        asking
            .await
            .unwrap()
            .expect("the right passphrase lets it go on");
        assert!(
            crate::secret::chariox_encrypted_vault_status(&path)
                .unwrap()
                .unlocked
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[tokio::test]
    async fn vault_decision_of_a_pre_enrollment_session_is_answered_by_the_enrolled_owner() {
        let workspace = crate::test_support::TestWorktree::new("vault-legacy-owner");
        let private_state = crate::test_support::TestWorktree::new("vault-legacy-owner-state");
        let path = private_state.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "test passphrase").unwrap();
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "enrolled-owner".to_string(),
            ..Default::default()
        });
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        assert_eq!(
            session.owner_user_id(),
            crate::session::DEFAULT_LOCAL_USER_ID
        );
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        let call = state.manage_credential_vault_unlock(session.id(), agent.id());
        tokio::pin!(call);
        let interaction = tokio::select! {
            result = &mut call => panic!("unlock must await reply: {result:?}"),
            result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let snapshot = state.owned.session_store.get_session(session.id()).unwrap();
                    if let Some(interaction) = snapshot.active_interaction_for_agent(agent.id()) {
                        break interaction.clone();
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            }) => result.expect("unlock interaction"),
        };
        assert!(interaction.id().starts_with("vault-unlock-"));
        let answer = |caller: &str| {
            state.owned.resolve_runtime_interaction(
                session.id(),
                interaction.id(),
                "cancel",
                None,
                Some(caller),
                false,
            )
        };
        let refused = answer("intruder").expect_err("another identity cannot answer");
        assert!(
            refused.to_string().contains("Only the operation owner"),
            "{refused}"
        );
        answer("enrolled-owner").expect("the enrolled owner answers the pre-enrollment decision");
        let _ = call.await;
        std::fs::remove_file(&path).unwrap();
    }

    fn unlock_card(
        policy: crate::config::CredentialVaultUnlockPolicy,
    ) -> crate::session::RuntimeInteraction {
        let config = crate::config::UserCredentialVaultConfig {
            unlock_policy: policy,
            ..Default::default()
        };
        vault_passphrase_interaction(&requester(None, None), "agent-1", &config, 60)
    }

    #[test]
    fn vault_unlock_card_offers_every_duration_with_the_passphrase_inline() {
        let interaction = unlock_card(crate::config::CredentialVaultUnlockPolicy::Ttl);
        let ids = interaction
            .choices()
            .iter()
            .map(|choice| choice.id())
            .collect::<Vec<_>>();
        assert_eq!(ids.first(), Some(&"unlock_once"));
        assert!(ids.contains(&"unlock_kernel"), "{ids:?}");
        assert_eq!(ids.last(), Some(&"cancel"));
        assert!(interaction
            .choice("cancel")
            .unwrap()
            .vault_unlock()
            .is_none());
        assert_eq!(
            interaction.choice("unlock_kernel").unwrap().vault_unlock(),
            Some(crate::local::VaultUnlockDuration::UntilKernelStops)
        );
        let custom_choice = interaction
            .custom_choice()
            .expect("vault passphrase prompt should expose secret input");
        assert_eq!(custom_choice.id(), "passphrase");
        assert_eq!(
            custom_choice.input_kind(),
            crate::session::RuntimeInteractionInputKind::Secret
        );
    }

    #[test]
    fn the_unlock_policy_only_preselects_a_duration() {
        use crate::config::CredentialVaultUnlockPolicy::{Always, KernelInit, Ttl};
        for (policy, preselected) in [
            (Always, "unlock_once"),
            (KernelInit, "unlock_kernel"),
            (Ttl, "unlock_30m"),
        ] {
            let interaction = unlock_card(policy);
            let primary = interaction
                .choices()
                .iter()
                .filter(|choice| {
                    choice.style() == Some(crate::session::RuntimeInteractionChoiceStyle::Primary)
                })
                .map(|choice| choice.id())
                .collect::<Vec<_>>();
            assert_eq!(primary, vec![preselected], "{policy:?}");
            assert_eq!(
                interaction.choices().len(),
                unlock_card(Ttl).choices().len(),
                "the policy must not change what is offered"
            );
        }
    }

    #[test]
    fn vault_unlock_answer_takes_the_passphrase_and_the_chosen_duration() {
        let interaction = unlock_card(crate::config::CredentialVaultUnlockPolicy::Ttl);
        let (passphrase, duration) = vault_unlock_answer(
            &interaction,
            resolution(Some("unlock_kernel"), Some("correct horse battery staple")),
            "test",
        )
        .expect("a duration choice with the passphrase unlocks");
        assert_eq!(passphrase.as_str(), "correct horse battery staple");
        assert_eq!(
            duration,
            crate::local::VaultUnlockDuration::UntilKernelStops
        );

        // The terminal answers with the typed passphrase alone: the preselected duration.
        let (_, duration) = vault_unlock_answer(
            &interaction,
            resolution(Some("passphrase"), Some("typed")),
            "test",
        )
        .expect("a typed passphrase unlocks for the preselected duration");
        assert_eq!(
            duration,
            crate::local::VaultUnlockDuration::Minutes { minutes: 30 }
        );

        for (choice, reply) in [
            (Some("cancel"), Some("cancel")),
            (Some("passphrase"), None),
            (Some("unlock_kernel"), None),
            (None, None),
        ] {
            assert!(
                vault_unlock_answer(&interaction, resolution(choice, reply), "test").is_err(),
                "{choice:?} must not unlock"
            );
        }
    }

    #[test]
    fn vault_manage_offers_a_passphrase_change_through_secret_prompts() {
        let manage = vault_manage_interaction(&requester(None, None), "agent-1");
        assert!(manage
            .choices()
            .iter()
            .any(|choice| choice.id() == "change_passphrase"));

        for step in [
            VaultPassphraseChangeStep::Current,
            VaultPassphraseChangeStep::New,
            VaultPassphraseChangeStep::Repeat,
        ] {
            let interaction =
                vault_passphrase_change_interaction(&requester(None, None), "agent-1", step);
            assert_eq!(
                interaction
                    .choices()
                    .iter()
                    .map(|choice| choice.id())
                    .collect::<Vec<_>>(),
                vec!["cancel"]
            );
            let custom_choice = interaction
                .custom_choice()
                .expect("each change step should take secret input");
            assert_eq!(custom_choice.id(), "passphrase");
            assert_eq!(
                custom_choice.input_kind(),
                crate::session::RuntimeInteractionInputKind::Secret
            );
        }
        let error =
            vault_secret_from_resolution(resolution(Some("cancel"), Some("cancel")), "test")
                .expect_err("cancel should stop the change");
        assert!(matches!(
            error,
            DaemonError::LocalTransport { message, .. }
                if message == "Chariox vault passphrase change was cancelled"
        ));
    }

    fn requester(agent: Option<&str>, room: Option<&str>) -> VaultRequester {
        VaultRequester {
            agent: agent.map(str::to_owned),
            room: room.map(str::to_owned),
        }
    }

    #[test]
    fn vault_prompts_name_alias_and_room_never_ids() {
        let agent_id = "agent-bfd2428adc43df51a0ffd9d68bcb09b7";
        for (agent, room) in [
            (None, None),
            (Some("Reviewer"), None),
            (None, Some("Launch room")),
            (Some("Reviewer"), Some("Launch room")),
        ] {
            let requester = requester(agent, room);
            let mut interactions = vec![
                vault_passphrase_interaction(
                    &requester,
                    agent_id,
                    &crate::config::UserCredentialVaultConfig::default(),
                    60,
                ),
                vault_manage_interaction(&requester, agent_id),
            ];
            for step in [
                VaultPassphraseChangeStep::Current,
                VaultPassphraseChangeStep::New,
                VaultPassphraseChangeStep::Repeat,
            ] {
                interactions.push(vault_passphrase_change_interaction(
                    &requester, agent_id, step,
                ));
            }
            for interaction in &interactions {
                let text = format!(
                    "{} {}",
                    interaction.title().unwrap_or_default(),
                    interaction.message()
                );
                assert!(
                    !["agent-", "runtime_tool_", "case-preserving", "`"]
                        .iter()
                        .any(|internal| text.contains(internal))
                        && !text
                            .split(|c: char| !c.is_ascii_hexdigit())
                            .any(|run| run.len() >= 16),
                    "raw id or internal wording in: {text}"
                );
                if let Some(room) = room {
                    assert!(text.contains(room), "room name missing in: {text}");
                }
            }
            if let Some(agent) = agent {
                assert!(interactions[0].message().contains(agent));
            }
        }
    }

    #[test]
    fn the_chosen_duration_decides_the_lease() {
        use crate::local::VaultUnlockDuration::{Minutes, Once, UntilKernelStops};
        use crate::secret::VaultUnlockLease;
        let config = crate::config::UserCredentialVaultConfig::default();
        assert_eq!(Once.lease(&config, true), VaultUnlockLease::Operation);
        assert_eq!(Once.lease(&config, false), VaultUnlockLease::SingleUse);
        assert_eq!(
            UntilKernelStops.lease(&config, true),
            VaultUnlockLease::KernelShutdown
        );
        assert_eq!(
            Minutes { minutes: 30 }.lease(&config, true),
            VaultUnlockLease::TtlMinutes(30)
        );
        assert_eq!(
            Minutes { minutes: u64::MAX }.lease(&config, true),
            VaultUnlockLease::TtlMinutes(config.max_ttl_minutes)
        );
    }

    #[tokio::test]
    async fn vault_prompt_goes_to_the_named_agent_else_the_focus_agent() {
        let root = std::env::temp_dir().join(format!(
            "chariox-vault-prompt-agent-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(&root).expect("test root should be created");
        let root_path = root.to_string_lossy().to_string();
        let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                &root_path, &root_path,
            ))
            .expect("session should be created");
        let (unfocused, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                &root_path, &root_path,
            ))
            .expect("second session should be created");
        app.sessions_mut()
            .set_focused_agent(unfocused.id(), None)
            .expect("focus should clear");
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(
                session.id(),
                "dev-stub",
            ))
            .expect("agent should be created");
        crate::app::KernelSessionService::new(&mut app)
            .focus_agent(session.id(), agent.id())
            .expect("agent should take focus");
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);

        assert_eq!(
            state
                .vault_prompt_agent(session.id(), Some("agent-9"), "test")
                .await
                .expect("named agent"),
            "agent-9"
        );
        assert_eq!(
            state
                .vault_prompt_agent(session.id(), None, "test")
                .await
                .expect("focus agent"),
            agent.id()
        );
        let error = state
            .vault_prompt_agent(unfocused.id(), None, "test")
            .await
            .expect_err("a session without a focus agent cannot show the popup");
        assert!(matches!(
            error,
            DaemonError::LocalTransport { message, .. }
                if message.contains("requires an agent in the session")
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
#[path = "vault_restart_tests.rs"]
mod restart_tests;
