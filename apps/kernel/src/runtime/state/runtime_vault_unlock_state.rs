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

    fn unlocked_until_expiry() -> Self {
        Self {
            path: None,
            lock_on_drop: false,
        }
    }

    fn not_required() -> Self {
        Self {
            path: None,
            lock_on_drop: false,
        }
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

impl KernelRuntimeState {
    pub(super) async fn prepare_provider_launch_request_with_vault(
        &self,
        request: crate::provider::LaunchProviderRequest,
        operation: &'static str,
    ) -> Result<crate::provider::LaunchProviderRequest, DaemonError> {
        let _vault_unlock = self
            .ensure_provider_account_vault_unlocked_for_launch(&request, operation)
            .await?;
        let config = self.owned.config_projection.snapshot();
        self.owned
            .prepare_provider_launch_request(request, config.runtime_mcp_url())
    }

    async fn ensure_provider_account_vault_unlocked_for_launch(
        &self,
        request: &crate::provider::LaunchProviderRequest,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
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
        let account_owner_user_id =
            crate::account_profile::provider_account_authority_owner_user_id(
                &config,
                runtime_owner_user_id,
            );
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
        if let Some(agent_id) = run.agent_instance_id() {
            request = request.with_agent_id(agent_id.to_string());
        }
        let _vault_unlock = self
            .ensure_provider_account_vault_unlocked_for_launch(&request, operation)
            .await?;
        let config = self.owned.config_projection.snapshot();
        let account_owner_user_id =
            crate::account_profile::provider_account_authority_owner_user_id(
                &config,
                run.owner_user_id(),
            );
        crate::provider::resolve_provider_account_credentials(
            &config,
            &account_owner_user_id,
            run.provider(),
            run.account_profile(),
        )
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
        let account_owner_user_id =
            crate::account_profile::provider_account_authority_owner_user_id(
                &config,
                agent.owner_user_id(),
            );
        let profile = self.owned.provider_account_profiles.get(
            &account_owner_user_id,
            agent.provider(),
            agent.provider_account_profile(),
        )?;
        let request = crate::provider::LaunchProviderRequest::new(
            agent.session_id(),
            crate::provider::adapter_key_for_provider(agent.provider()),
            agent.provider(),
            &profile.profile_id,
            agent.model().unwrap_or_default(),
        )
        .with_owner_user_id(agent.owner_user_id().to_string())
        .with_agent_id(agent.id().to_string());
        let _vault_unlock = self
            .ensure_provider_account_vault_unlocked_for_launch(&request, operation)
            .await?;
        let mut environment = crate::provider::resolve_provider_account_credentials(
            &config,
            &account_owner_user_id,
            agent.provider(),
            &profile.profile_id,
        )?;
        let token = environment
            .remove(crate::provider::CLAUDE_OAUTH_TOKEN_ENV)
            .filter(|value| !value.trim().is_empty())
            .ok_or(DaemonError::InvalidConfig {
                field: "provider account credential",
                message: "remote Claude launch requires a Chariox-vault setup token; use `provider setup-token claude <account-profile>` on the home kernel",
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

    async fn await_vault_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        operation: &'static str,
        interaction_name: &'static str,
    ) -> Result<super::PendingInteractionResolution, DaemonError> {
        let interaction_id = interaction.id().to_string();
        let timeout_sec = interaction.timeout_sec();
        let resolution_rx = self.create_terminal_credential_interaction(session_id, interaction)?;
        if let Some(timeout_sec) = timeout_sec {
            let state = self.clone();
            let timeout_session_id = session_id.to_string();
            let timeout_interaction_id = interaction_id;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(timeout_sec)).await;
                let _ = state
                    .timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id)
                    .await;
            });
        }
        resolution_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation,
                message: format!(
                    "{interaction_name} interaction dropped before resolution: {error}"
                ),
            })
    }

    pub(crate) async fn ensure_vault_unlocked_for_command_context(
        &self,
        command: &crate::runtime::command::KernelCommand,
        session_id: Option<&str>,
        agent_id: Option<&str>,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        if user_config.credential_vault.backend
            != crate::config::CredentialVaultBackend::CharioxEncrypted
        {
            return Ok(VaultUnlockGuard::not_required());
        }
        let vault_path = expand_vault_path(&user_config.credential_vault.path);
        if user_config.credential_vault.unlock_policy
            != crate::config::CredentialVaultUnlockPolicy::Always
            && crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked
        {
            return Ok(VaultUnlockGuard::unlocked_until_expiry());
        }
        let session_id = session_id
            .or(command.session_id.as_deref())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message:
                    "encrypted Chariox vault access requires a session_id so the unlock popup can be shown"
                        .to_string(),
            })?;
        let agent_id = self
            .vault_prompt_agent(
                session_id,
                agent_id.or(command.agent_id.as_deref()),
                operation,
            )
            .await?;
        self.ensure_vault_unlocked_for_agent(session_id, &agent_id, operation)
            .await
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
        let force_prompt = matches!(
            vault_config.unlock_policy,
            crate::config::CredentialVaultUnlockPolicy::Always
        );
        if !force_prompt && crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
            return Ok(VaultUnlockGuard::unlocked_until_expiry());
        }
        let unlock_request_lock = vault_unlock_request_lock(&vault_path);
        let _dedupe_guard = unlock_request_lock.lock().await;
        if !force_prompt && crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
            return Ok(VaultUnlockGuard::unlocked_until_expiry());
        }

        let resolution = self
            .await_vault_interaction(
                session_id,
                vault_passphrase_interaction(session_id, agent_id, operation),
                operation,
                "vault unlock",
            )
            .await?;
        let passphrase = vault_passphrase_from_resolution(resolution, operation)?;
        let choice_id = if matches!(
            vault_config.unlock_policy,
            crate::config::CredentialVaultUnlockPolicy::Ttl
        ) {
            vault_lease_choice_from_resolution(
                self.await_vault_interaction(
                    session_id,
                    vault_unlock_lease_interaction(session_id, agent_id, &vault_config),
                    operation,
                    "vault unlock lease",
                )
                .await?,
                operation,
            )?
        } else if matches!(
            vault_config.unlock_policy,
            crate::config::CredentialVaultUnlockPolicy::KernelInit
        ) {
            "unlock_kernel".to_string()
        } else {
            "unlock_operation".to_string()
        };
        let (lease, lock_after_operation) =
            unlock_lease_for_choice(&choice_id, &vault_config, force_prompt);
        let status =
            crate::secret::unlock_chariox_encrypted_vault(&vault_path, passphrase.as_str(), lease)?;
        if let Err(error) = self.pin_critical_approval_verifier_after_unlock(&vault_path) {
            crate::secret::lock_chariox_encrypted_vault(&vault_path)?;
            crate::secret::clear_vault_secret_process_cache()?;
            return Err(error);
        }
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
        if lock_after_operation {
            Ok(VaultUnlockGuard::unlocked_for_operation(vault_path))
        } else {
            Ok(VaultUnlockGuard::unlocked_until_expiry())
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
                vault_manage_interaction(session_id, agent_id),
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
                vault_passphrase_change_interaction(session_id, agent_id, step),
                VAULT_PASSPHRASE_CHANGE,
                "vault passphrase change",
            )
            .await?;
        vault_secret_from_resolution(resolution, VAULT_PASSPHRASE_CHANGE, "passphrase change")
    }
}

const VAULT_PASSPHRASE_CHANGE: &str = "credential_vault_change_passphrase";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VaultPassphraseChangeStep {
    Current,
    New,
    Repeat,
}

fn vault_passphrase_change_interaction(
    session_id: &str,
    agent_id: &str,
    step: VaultPassphraseChangeStep,
) -> crate::session::RuntimeInteraction {
    let (slug, label, message) = match step {
        VaultPassphraseChangeStep::Current => (
            "current",
            "Current passphrase",
            "Enter the current vault passphrase (your Chariox passkey). If you set it in the terminal before the case-preserving input fix, it is your passphrase with A-Z in lower case and without spaces or emoji.",
        ),
        VaultPassphraseChangeStep::New => (
            "new",
            "New passphrase",
            "Enter the new vault passphrase.",
        ),
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
        format!("Session `{session_id}`: {message}"),
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

fn vault_passphrase_interaction(
    session_id: &str,
    agent_id: &str,
    operation: &'static str,
) -> crate::session::RuntimeInteraction {
    let choices = vec![crate::session::RuntimeInteractionChoice::new(
        "cancel",
        "Cancel",
        "cancel",
        Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
    )];
    crate::session::RuntimeInteraction::new(
        format!(
            "vault-unlock-{}-{}",
            agent_id,
            crate::session::unix_epoch_ms()
        ),
        agent_id,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Critical,
        Some("Unlock Chariox Vault".to_string()),
        format!(
            "Agent `{agent_id}` in session `{session_id}` needs Chariox Vault access for `{operation}`. Enter the vault passphrase."
        ),
        choices,
        Some(crate::session::RuntimeInteractionCustomChoice::secret(
            "passphrase",
            "Vault passphrase",
            Some("Passphrase".to_string()),
            Some(1),
            Some(512),
        )),
        Some(300),
        Some("cancel".to_string()),
    )
}

fn vault_unlock_lease_interaction(
    session_id: &str,
    agent_id: &str,
    config: &crate::config::UserCredentialVaultConfig,
) -> crate::session::RuntimeInteraction {
    crate::session::RuntimeInteraction::new(
        format!(
            "vault-unlock-lease-{}-{}",
            agent_id,
            crate::session::unix_epoch_ms()
        ),
        agent_id,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Critical,
        Some("Choose Vault Unlock Duration".to_string()),
        format!("Choose how long to keep the Chariox Vault unlocked for session `{session_id}`."),
        vec![
            crate::session::RuntimeInteractionChoice::new(
                "unlock_operation",
                "This operation",
                "operation",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "unlock_default_ttl",
                format!(
                    "{} minutes",
                    config.default_ttl_minutes.min(config.max_ttl_minutes)
                ),
                "ttl_default",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "unlock_60m",
                "1 hour",
                "ttl_60",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "unlock_kernel",
                "Until kernel shutdown",
                "kernel_shutdown",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "cancel",
                "Cancel",
                "cancel",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        Some(300),
        Some("cancel".to_string()),
    )
}

fn vault_passphrase_from_resolution(
    resolution: super::PendingInteractionResolution,
    operation: &'static str,
) -> Result<zeroize::Zeroizing<String>, DaemonError> {
    vault_secret_from_resolution(resolution, operation, "unlock")
}

fn vault_secret_from_resolution(
    resolution: super::PendingInteractionResolution,
    operation: &'static str,
    action: &str,
) -> Result<zeroize::Zeroizing<String>, DaemonError> {
    if resolution.status == "timed_out" {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("Chariox vault {action} timed out"),
        });
    }
    match resolution.choice_id.as_deref() {
        None | Some("cancel") => Err(DaemonError::LocalTransport {
            operation,
            message: format!("Chariox vault {action} was cancelled"),
        }),
        Some("passphrase") => resolution
            .reply
            .map(zeroize::Zeroizing::new)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: format!("Chariox vault {action} resolved without a passphrase"),
            }),
        Some(_) => Err(DaemonError::LocalTransport {
            operation,
            message: format!("Chariox vault {action} resolved without a passphrase"),
        }),
    }
}

fn vault_lease_choice_from_resolution(
    resolution: super::PendingInteractionResolution,
    operation: &'static str,
) -> Result<String, DaemonError> {
    if resolution.status == "timed_out" {
        return Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock duration selection timed out".to_string(),
        });
    }
    match resolution.choice_id.as_deref() {
        None | Some("cancel") => Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock was cancelled".to_string(),
        }),
        Some(
            choice_id
            @ ("unlock_operation" | "unlock_default_ttl" | "unlock_60m" | "unlock_kernel"),
        ) => Ok(choice_id.to_string()),
        Some(_) => Err(DaemonError::LocalTransport {
            operation,
            message: "Chariox vault unlock resolved with an invalid duration".to_string(),
        }),
    }
}

fn unlock_lease_for_choice(
    choice_id: &str,
    config: &crate::config::UserCredentialVaultConfig,
    force_operation: bool,
) -> (crate::secret::VaultUnlockLease, bool) {
    if force_operation {
        return (crate::secret::VaultUnlockLease::Operation, true);
    }
    if matches!(
        config.unlock_policy,
        crate::config::CredentialVaultUnlockPolicy::KernelInit
    ) {
        return (crate::secret::VaultUnlockLease::KernelShutdown, false);
    }
    match choice_id {
        "unlock_operation" => (crate::secret::VaultUnlockLease::Operation, true),
        "unlock_60m" => (
            crate::secret::VaultUnlockLease::TtlMinutes(60.min(config.max_ttl_minutes)),
            false,
        ),
        "unlock_kernel" => (crate::secret::VaultUnlockLease::KernelShutdown, false),
        "passphrase" | "unlock_default_ttl" => (
            crate::secret::VaultUnlockLease::TtlMinutes(
                config
                    .default_ttl_minutes
                    .min(config.max_ttl_minutes)
                    .max(1),
            ),
            false,
        ),
        _ => (crate::secret::VaultUnlockLease::Operation, true),
    }
}

fn vault_manage_interaction(
    session_id: &str,
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
            "The Chariox Vault is unlocked for session `{session_id}`. Extend the unlock window, change the passphrase, or lock it now."
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
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<
            std::collections::BTreeMap<std::path::PathBuf, std::sync::Arc<tokio::sync::Mutex<()>>>,
        >,
    > = std::sync::OnceLock::new();
    let path = expand_vault_path(&path.to_string_lossy());
    let mut locks = LOCKS
        .get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()))
        .lock()
        .expect("vault unlock request lock map poisoned");
    locks
        .entry(path)
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone()
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

    #[test]
    fn vault_passphrase_prompt_cannot_encode_an_unlock_lease_as_the_secret() {
        let interaction = vault_passphrase_interaction("session-1", "agent-1", "test");

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
            .expect("vault passphrase prompt should expose secret input");
        assert_eq!(custom_choice.id(), "passphrase");
        assert_eq!(
            custom_choice.input_kind(),
            crate::session::RuntimeInteractionInputKind::Secret
        );

        let error = vault_passphrase_from_resolution(
            resolution(Some("unlock_60m"), Some("ttl_60")),
            "test",
        )
        .expect_err("a lease choice must never become a vault passphrase");
        assert!(matches!(
            error,
            DaemonError::LocalTransport { message, .. }
                if message == "Chariox vault unlock resolved without a passphrase"
        ));
    }

    #[test]
    fn vault_passphrase_prompt_accepts_only_the_secret_custom_choice() {
        let passphrase = vault_passphrase_from_resolution(
            resolution(Some("passphrase"), Some("correct horse battery staple")),
            "test",
        )
        .expect("secret custom choice should resolve the vault passphrase");

        assert_eq!(passphrase.as_str(), "correct horse battery staple");
    }

    #[test]
    fn vault_manage_offers_a_passphrase_change_through_secret_prompts() {
        let manage = vault_manage_interaction("session-1", "agent-1");
        assert!(manage
            .choices()
            .iter()
            .any(|choice| choice.id() == "change_passphrase"));

        for step in [
            VaultPassphraseChangeStep::Current,
            VaultPassphraseChangeStep::New,
            VaultPassphraseChangeStep::Repeat,
        ] {
            let interaction = vault_passphrase_change_interaction("session-1", "agent-1", step);
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
        let error = vault_secret_from_resolution(
            resolution(Some("cancel"), Some("cancel")),
            "test",
            "passphrase change",
        )
        .expect_err("cancel should stop the change");
        assert!(matches!(
            error,
            DaemonError::LocalTransport { message, .. }
                if message == "Chariox vault passphrase change was cancelled"
        ));
    }

    #[test]
    fn vault_duration_prompt_is_a_separate_non_secret_interaction() {
        let config = crate::config::UserCredentialVaultConfig::default();
        let interaction = vault_unlock_lease_interaction("session-1", "agent-1", &config);

        assert!(interaction.custom_choice().is_none());
        assert_eq!(
            interaction
                .choices()
                .iter()
                .map(|choice| choice.id())
                .collect::<Vec<_>>(),
            vec![
                "unlock_operation",
                "unlock_default_ttl",
                "unlock_60m",
                "unlock_kernel",
                "cancel",
            ]
        );
        assert_eq!(
            vault_lease_choice_from_resolution(
                resolution(Some("unlock_60m"), Some("ttl_60")),
                "test",
            )
            .expect("known duration should resolve"),
            "unlock_60m"
        );
        assert!(vault_lease_choice_from_resolution(
            resolution(Some("passphrase"), Some("secret")),
            "test",
        )
        .is_err());
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
mod mp11_always_tests {
    use super::*;
    use crate::DaemonApp;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    async fn runtime(config: crate::config::DaemonConfig) -> KernelRuntimeState {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed"),
        ));
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(&app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }
    #[tokio::test]
    async fn mp11_always_policy_cannot_use_a_live_lease_in_any_credential_command() {
        crate::test_support::isolated_env_test!();
        let _environment = crate::env_lock::lock();
        for initial_policy in [
            crate::config::CredentialVaultUnlockPolicy::Ttl,
            crate::config::CredentialVaultUnlockPolicy::KernelInit,
            crate::config::CredentialVaultUnlockPolicy::Always,
        ] {
            let root =
                std::env::temp_dir().join(format!("mp11-always-{:016x}", rand::random::<u64>()));
            std::fs::create_dir_all(&root).unwrap();
            let vault = root.join("vault.json");
            crate::secret::create_chariox_encrypted_vault_for_test(&vault, "synthetic-passphrase")
                .unwrap();
            crate::secret::unlock_chariox_encrypted_vault(
                &vault,
                "synthetic-passphrase",
                if initial_policy == crate::config::CredentialVaultUnlockPolicy::Always {
                    crate::secret::VaultUnlockLease::Operation
                } else {
                    crate::secret::VaultUnlockLease::KernelShutdown
                },
            )
            .unwrap();
            let mut config = crate::config::DaemonConfig::for_tests();
            config.user_config_path = root.join("config.toml");
            config = config.with_session_history_root(root.join("history"));
            config.user_config.history.operational.path =
                Some(root.join("operational.db").display().to_string());
            config.user_config.artifacts.operational.root =
                Some(root.join("artifacts").display().to_string());
            config.user_config.artifacts.operational.index_path =
                Some(root.join("artifacts.db").display().to_string());
            config.user_config.state.path = Some(root.join("state.db").display().to_string());
            config.user_config.credential_vault.backend =
                crate::config::CredentialVaultBackend::CharioxEncrypted;
            config.user_config.credential_vault.path = vault.display().to_string();
            config.user_config.credential_vault.unlock_policy = initial_policy;
            let state = runtime(config).await;
            let command = crate::runtime::command::KernelCommand::from_local_request(
                "synthetic",
                None,
                None,
                &crate::local::LocalDaemonRequest::GetCredentialVaultStatus(
                    crate::local::GetCredentialVaultStatusRequest,
                ),
            );
            let owner = state.provider_account_authority_owner_user_id(
                &crate::runtime::command::command_caller_user_id(&command),
            );
            let profile = state
                .provider_account_profile_registry()
                .create_managed(&owner, "claude", "Synthetic")
                .unwrap();
            state
                .set_user_config_value("credential_vault.unlock_policy".into(), "always".into())
                .await
                .unwrap();
            let projection = &state.owned.config_projection;
            let set = crate::runtime::user_config_executor::execute_set_credential_secret_request(
                projection,
                &state,
                &command,
                crate::local::SetCredentialSecretRequest {
                    session_id: None,
                    agent_id: None,
                    key: "synthetic-key".into(),
                    value: "synthetic".into(),
                },
            )
            .await;
            let delete =
                crate::runtime::user_config_executor::execute_delete_credential_secret_request(
                    projection,
                    &state,
                    &command,
                    crate::local::DeleteCredentialSecretRequest {
                        session_id: None,
                        agent_id: None,
                        key: "synthetic-key".into(),
                    },
                )
                .await;
            let provider = crate::runtime::user_config_executor::execute_set_provider_account_credential_request(projection, &state, &command,
                crate::local::SetProviderAccountCredentialRequest { session_id: None, agent_id: None, provider: "claude".into(), account_profile: profile.profile_id,
                    value: "synthetic-value".into(), run: false, overwrite: true }).await;
            let refused = [set, delete, provider].into_iter().all(|result| {
                result.is_err_and(|error| error.to_string().contains("requires a session_id"))
            });
            crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
            drop(state);
            std::fs::remove_dir_all(root).unwrap();
            assert!(
                refused,
                "Always policy reused an existing lease through a command caller"
            );
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn mp11_manual_claude_token_verifies_before_authentication_and_replacement() {
        crate::test_support::isolated_env_test!();
        let _environment = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "envp03-manual-claude-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::env::remove_var("CHARIOX_HOME");
                std::env::remove_var("CHARIOX_CLAUDE_BIN");
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        std::env::set_var("CHARIOX_HOME", &root);
        let executable = root.join("claude");
        std::fs::write(&executable, r##"#!/bin/sh
if [ "$1" = "-p" ]; then
  if [ "$CLAUDE_CODE_OAUTH_TOKEN" != "synthetic-valid" ]; then exit 1; fi
  printf '%s' '{"type":"result","subtype":"success","is_error":false,"result":"Current session: 17% used\nCurrent week (all models): 41% used","usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0},"num_turns":0,"total_cost_usd":0,"duration_api_ms":0}'
else
  printf '%s' '{"loggedIn":false}'
fi
"##).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("CHARIOX_CLAUDE_BIN", &executable);
        let vault = root.join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&vault, "synthetic-passphrase")
            .unwrap();
        crate::secret::unlock_chariox_encrypted_vault(
            &vault,
            "synthetic-passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config_path = root.join("config.toml");
        config = config.with_session_history_root(root.join("history"));
        config.user_config.state.path = Some(root.join("state.db").display().to_string());
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = vault.display().to_string();
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::KernelInit;
        let state = runtime(config).await;
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "synthetic",
            None,
            None,
            &crate::local::LocalDaemonRequest::GetCredentialVaultStatus(
                crate::local::GetCredentialVaultStatusRequest,
            ),
        );
        let owner = state.provider_account_authority_owner_user_id(
            &crate::runtime::command::command_caller_user_id(&command),
        );
        let registry = state.provider_account_profile_registry();
        let profile = registry
            .create_managed(&owner, "claude", "Synthetic")
            .unwrap();
        let request = |value: &str, overwrite| crate::local::SetProviderAccountCredentialRequest {
            session_id: None,
            agent_id: None,
            provider: "claude".into(),
            account_profile: profile.profile_id.clone(),
            value: value.into(),
            run: false,
            overwrite,
        };
        let result =
            crate::runtime::user_config_executor::execute_set_provider_account_credential_request(
                &state.owned.config_projection,
                &state,
                &command,
                request("synthetic-valid", false),
            )
            .await
            .unwrap();
        assert!(matches!(
            result,
            crate::local::LocalDaemonResponse::ProviderAccountCredentialStored { .. }
        ));
        assert_eq!(
            registry
                .get(&owner, "claude", &profile.profile_id)
                .unwrap()
                .auth_state,
            crate::account_profile::ProviderAccountAuthState::Authenticated,
            "manual enrollment must verify and authenticate through the official CLI"
        );
        let bad =
            crate::runtime::user_config_executor::execute_set_provider_account_credential_request(
                &state.owned.config_projection,
                &state,
                &command,
                request("synthetic-invalid", true),
            )
            .await;
        assert!(
            bad.is_err(),
            "a rejected token must not replace the registered credential"
        );
        let values = crate::provider::resolve_provider_account_credentials(
            &state.owned.config_projection.snapshot(),
            &owner,
            "claude",
            &profile.profile_id,
        )
        .unwrap();
        assert!(values.iter().any(
            |(name, value)| name == crate::provider::CLAUDE_OAUTH_TOKEN_ENV
                && value == "synthetic-valid"
        ));
        crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
        drop(state);
    }
}
