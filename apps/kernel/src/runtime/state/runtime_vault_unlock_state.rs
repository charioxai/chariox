use super::vault_unlock_requests::{
    notify_vault_unlocked, room_unlock, vault_unlock_notifications, RoomUnlock, RoomUnlockEnd,
    VaultUnlockWait,
};
use super::*;

pub(crate) struct VaultUnlockGuard {
    path: Option<std::path::PathBuf>,
    lock_on_drop: bool,
    /// Where the owner unlocked, when the Vault was opened by someone else's window while this call waited.
    unlocked_via: Option<crate::local::VaultUnlockSurface>,
}

/// The answer to a Vault management request: the Vault status, what was done, and where
/// a person unlocked it when the call only waited for that.
#[derive(Debug)]
pub(crate) struct VaultManaged {
    pub(crate) status: crate::secret::CharioxVaultUnlockStatus,
    pub(crate) action: String,
    pub(crate) unlocked_via: Option<crate::local::VaultUnlockSurface>,
}

impl VaultManaged {
    /// What the agent's tool call returns, naming where a person unlocked when it only waited for that.
    pub(crate) fn tool_payload(&self) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "action": self.action,
            "status": self.status,
        });
        if let Some(surface) = self.unlocked_via {
            payload["via"] = surface.words().into();
        }
        payload
    }

    fn done(status: crate::secret::CharioxVaultUnlockStatus, action: String) -> Self {
        Self {
            status,
            action,
            unlocked_via: None,
        }
    }
}

impl VaultUnlockGuard {
    fn unlocked_for_operation(path: std::path::PathBuf) -> Self {
        Self {
            path: Some(path),
            lock_on_drop: true,
            unlocked_via: None,
        }
    }

    fn unlocked_until_expiry(path: std::path::PathBuf) -> Self {
        Self {
            path: Some(path),
            lock_on_drop: false,
            unlocked_via: None,
        }
    }

    /// The open Vault for this operation of `holder`; `None` when it is open only for another
    /// holder's operation that took its single-use ("once") unlock.
    fn for_existing_unlock(
        path: std::path::PathBuf,
        holder: &str,
    ) -> Result<Option<Self>, DaemonError> {
        // A client "once" unlock belongs to the next operation, including an
        // agent call resumed by another window's unlock notification.
        if crate::secret::take_chariox_vault_single_use_unlock(&path, holder)? {
            Ok(Some(Self::unlocked_for_operation(path)))
        } else if crate::secret::chariox_vault_held_for_another(&path, Some(holder))? {
            Ok(None)
        } else {
            Ok(Some(Self::unlocked_until_expiry(path)))
        }
    }

    pub(super) fn not_required() -> Self {
        Self {
            path: None,
            lock_on_drop: false,
            unlocked_via: None,
        }
    }

    pub(super) fn unlocked_via(&self) -> Option<crate::local::VaultUnlockSurface> {
        self.unlocked_via
    }

    /// Whether dropping this guard locks the Vault: it was opened for this one operation.
    pub(super) fn locks_on_drop(&self) -> bool {
        self.lock_on_drop
    }

    /// Keeps the unlock this guard owns open for `hold`, so a "just this once" unlock outlasts
    /// the operation's longest step instead of running out before it.
    pub(super) fn keep_open_for(&self, hold: std::time::Duration) -> Result<(), DaemonError> {
        match self.path.as_ref().filter(|_| self.lock_on_drop) {
            Some(path) => crate::secret::hold_chariox_vault_unlock_until(
                path,
                crate::session::unix_epoch_ms()
                    .saturating_add(u64::try_from(hold.as_millis()).unwrap_or(u64::MAX)),
            ),
            None => Ok(()),
        }
    }

    /// Ends the guard without locking, for a hold whose unlock runs out on its own.
    pub(super) fn disarm(mut self) {
        self.lock_on_drop = false;
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
        let resolution_rx = self.create_terminal_credential_interaction(session_id, interaction)?;
        // A cancelled waiter closes its popup instead of leaving it open to
        // silently discard a passphrase.
        let _close = CloseInteractionOnDrop {
            state: self.clone(),
            session_id: session_id.to_string(),
            interaction_id: interaction_id.clone(),
        };
        self.wait_for_vault_resolution(
            session_id,
            &interaction_id,
            resolution_rx,
            operation,
            interaction_name,
            deadline,
        )
        .await
    }

    /// Waits for the answer to a registered Vault question until its deadline;
    /// an expired wait closes the card and ignores a queued late answer.
    pub(super) async fn wait_for_vault_resolution(
        &self,
        session_id: &str,
        interaction_id: &str,
        mut resolution_rx: tokio::sync::oneshot::Receiver<super::PendingInteractionResolution>,
        operation: &'static str,
        interaction_name: &'static str,
        deadline: Option<tokio::time::Instant>,
    ) -> Result<super::PendingInteractionResolution, DaemonError> {
        let resolution = match deadline {
            Some(deadline) => tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline) => {
                    self.timeout_runtime_interaction(session_id, interaction_id).await?;
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
            if let Some(guard) =
                VaultUnlockGuard::for_existing_unlock(path, agent_id.unwrap_or_default())?
            {
                return Ok(guard);
            }
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
                message: "The Vault is locked and this Room has no agent to show its unlock card: unlock it from the Vault window, then try again.".to_string(),
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

    /// The one way a Vault use waits for a person. The Room's unlock card is a kernel wait that
    /// lasts until the person answers it, cancels it or unlocks elsewhere; every use in the Room
    /// joins it. The use itself waits only as long as its caller can (`vault_unlock_wait`), and
    /// leaves the card open when it stops waiting.
    pub(super) async fn ensure_vault_unlocked_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let vault_config = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        if vault_config.backend != crate::config::CredentialVaultBackend::CharioxEncrypted {
            return Ok(VaultUnlockGuard::not_required());
        }
        let vault_path = expand_vault_path(&vault_config.path);
        let deadline = vault_unlock_wait(operation).map(|wait| tokio::time::Instant::now() + wait);
        // The card belongs to the Room; the turn that asked keeps its own lifetime, so a use
        // whose turn ended while it waited does not go on after the unlock.
        let asking_turn = self.active_turn(session_id, agent_id);
        let mut unlocked_via = None;
        loop {
            if crate::secret::chariox_encrypted_vault_status(&vault_path)?.unlocked {
                if let Some(mut guard) =
                    VaultUnlockGuard::for_existing_unlock(vault_path.clone(), agent_id)?
                {
                    guard.unlocked_via = unlocked_via;
                    return Ok(guard);
                }
            }
            let mut end = match room_unlock(&vault_path, session_id) {
                RoomUnlock::Joined(end) => end,
                RoomUnlock::Opened(sender, end) => {
                    let state = self.clone();
                    let (session_id, agent_id) = (session_id.to_owned(), agent_id.to_owned());
                    let (vault_path, vault_config) = (vault_path.clone(), vault_config.clone());
                    tokio::spawn(async move {
                        let ended = match Box::pin(state.hold_vault_unlock_card(
                            &session_id,
                            &agent_id,
                            operation,
                            &vault_path,
                            &vault_config,
                        ))
                        .await
                        {
                            Ok(via) => RoomUnlockEnd::Unlocked(via),
                            Err(error) => RoomUnlockEnd::Refused(std::sync::Arc::new(error)),
                        };
                        sender.send_replace(Some(ended));
                    });
                    end
                }
            };
            let ended = end.wait_for(Option::is_some);
            let ended = match deadline {
                None => ended.await,
                Some(deadline) => tokio::select! {
                    biased;
                    ended = ended => ended,
                    () = tokio::time::sleep_until(deadline) => {
                        return Err(vault_unlock_card_open(operation));
                    }
                },
            };
            let ended = ended.map_err(|_| DaemonError::LocalTransport {
                operation,
                message: "the Vault unlock card ended without an answer".into(),
            })?;
            match (*ended).clone() {
                Some(RoomUnlockEnd::Unlocked(via)) => {
                    if turn_ended(&asking_turn, &self.active_turn(session_id, agent_id)) {
                        return Err(DaemonError::LocalTransport {
                            operation,
                            message: "the turn that needed the Vault ended while it waited for the unlock".into(),
                        });
                    }
                    unlocked_via = via;
                }
                Some(RoomUnlockEnd::Refused(error)) => {
                    return Err(refusal_for_waiter(&error, operation))
                }
                None => unreachable!("wait_for returns an ended card"),
            }
        }
    }

    /// The agent's turn: its running prompt and its running provider run, when it has them. A
    /// prompt being cancelled is no longer the turn, as for a card bound to that turn.
    fn active_turn(&self, session_id: &str, agent_id: &str) -> (Option<String>, Option<String>) {
        let prompt = self
            .owned
            .session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                self.owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent_id)
            })
            .filter(|prompt| {
                matches!(
                    prompt.status(),
                    crate::session::PromptStatus::Running
                        | crate::session::PromptStatus::Dispatching
                )
            })
            .map(|prompt| prompt.id().to_owned());
        let run = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
            .filter(|run| run.state() == crate::provider::ProviderRunState::Running)
            .map(|run| run.id().to_owned());
        (prompt, run)
    }

    /// The Room's unlock card, from its first question to the unlock: a wrong passphrase asks
    /// again in the same card, and a person unlocking elsewhere ends it, naming that window. It
    /// has no deadline.
    async fn hold_vault_unlock_card(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
        vault_path: &std::path::Path,
        vault_config: &crate::config::UserCredentialVaultConfig,
    ) -> Result<Option<crate::local::VaultUnlockSurface>, DaemonError> {
        let mut unlocks = vault_unlock_notifications(vault_path);
        if vault_open_to_every_use(vault_path)? {
            return Ok(None);
        }
        let mut problem: Option<String> = None;
        // One card serves every attempt; it closes with this wait, whichever way it ends.
        let card_id = format!("vault-unlock-{}-{}", agent_id, rand::random::<u64>());
        let mut card: Option<CloseInteractionOnDrop> = None;
        loop {
            let mut interaction = vault_passphrase_interaction(
                &self.vault_requester(session_id, agent_id),
                agent_id,
                &card_id,
                vault_config,
            );
            if let Some(problem) = problem.as_deref() {
                interaction = interaction.showing_problem(problem);
            }
            let resolution = match self
                .await_vault_unlock_interaction(
                    session_id,
                    interaction.clone(),
                    operation,
                    vault_path,
                    &mut unlocks,
                    &mut card,
                )
                .await?
            {
                VaultUnlockWait::Answered(resolution) => resolution,
                VaultUnlockWait::UnlockedElsewhere(surface) => return Ok(Some(surface)),
            };
            let (passphrase, duration) = vault_unlock_answer(&interaction, resolution, operation)?;
            // "Just this once" belongs to the next use, whichever of the waiting uses takes it.
            let lease = match duration.lease(vault_config, true) {
                crate::secret::VaultUnlockLease::Operation => {
                    crate::secret::VaultUnlockLease::SingleUse
                }
                lease => lease,
            };
            // Human input must never hold the mutation lock across sessions.
            let unlock_request_lock = vault_unlock_request_lock(vault_path);
            let _mutation = unlock_request_lock.lock().await;
            if vault_open_to_every_use(vault_path)? {
                self.owned.settle_vault_unlocked(
                    session_id,
                    &card_id,
                    crate::local::VaultUnlockSurface::Card,
                )?;
                return Ok(None);
            }
            let owner = self
                .owned
                .session_store
                .get_session(session_id)?
                .owner_user_id()
                .to_owned();
            let status = match self
                .unlock_vault_counted(&owner, vault_path, passphrase.as_str(), lease, true)
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
            self.owned.settle_vault_unlocked(
                session_id,
                &card_id,
                crate::local::VaultUnlockSurface::Card,
            )?;
            crate::logging::info_with_fields(
                "credential_vault",
                "Chariox vault unlocked",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "operation": operation,
                    "path": status.path.display().to_string(),
                    "expires_at_ms": status.expires_at_ms,
                    "single_use": lease == crate::secret::VaultUnlockLease::SingleUse,
                }),
            );
            if lease != crate::secret::VaultUnlockLease::SingleUse {
                notify_vault_unlocked(vault_path, crate::local::VaultUnlockSurface::Card);
            }
            return Ok(None);
        }
    }

    /// An unlocked Vault raises no card: the answer is its status ("Vault unlocked until <time>"),
    /// and extending or locking it is the agent's own action. Only a passphrase change asks the
    /// person, through the passphrase cards. A locked Vault asks for the passphrase as before.
    pub(crate) async fn manage_credential_vault_unlock(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<VaultManaged, DaemonError> {
        self.manage_credential_vault(session_id, agent_id, None)
            .await
    }

    pub(crate) async fn manage_credential_vault(
        &self,
        session_id: &str,
        agent_id: &str,
        action: Option<&str>,
    ) -> Result<VaultManaged, DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let vault_config = user_config.credential_vault;
        if vault_config.backend != crate::config::CredentialVaultBackend::CharioxEncrypted {
            let status = crate::secret::chariox_encrypted_vault_status(expand_vault_path(
                &vault_config.path,
            ))?;
            return Ok(VaultManaged::done(status, "not_required".to_string()));
        }
        let vault_path = expand_vault_path(&vault_config.path);
        let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
        if !status.unlocked {
            let guard = self
                .ensure_vault_unlocked_for_agent(session_id, agent_id, "credential_vault_manage")
                .await?;
            let status = crate::secret::chariox_encrypted_vault_status(&vault_path)?;
            return Ok(VaultManaged {
                status,
                action: "unlocked".to_string(),
                unlocked_via: guard.unlocked_via(),
            });
        }

        match action.unwrap_or("popup") {
            "popup" | "status" => Ok(VaultManaged::done(status, "status".to_string())),
            "change_passphrase" => {
                let (status, action) = self
                    .change_credential_vault_passphrase(session_id, agent_id, &vault_path)
                    .await?;
                Ok(VaultManaged::done(status, action))
            }
            choice @ ("extend" | "extend_30m" | "extend_60m") => {
                let choice = if choice == "extend_60m" {
                    "extend_60m"
                } else {
                    "extend_30m"
                };
                let (status, action) = apply_vault_manage_choice(&vault_path, choice, &vault_config)?;
                Ok(VaultManaged::done(status, action))
            }
            other => Err(DaemonError::LocalTransport {
                operation: "credential_vault_manage",
                message: format!(
                    "unsupported vault action `{other}`; expected status, extend, extend_60m, lock, or change_passphrase"
                ),
            }),
        }
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

/// The provider cuts an agent's tool call at 300 s; the call stops waiting on the card just
/// before that, and the card stays open for the person.
const VAULT_UNLOCK_TOOL_WAIT: std::time::Duration = std::time::Duration::from_secs(280);
/// A launch or recovery a person may be watching ends quickly with the card left open.
const VAULT_UNLOCK_LAUNCH_WAIT: std::time::Duration = std::time::Duration::from_secs(60);
/// A prompt submit never waits for a person: it opens the card and queues the prompt.
pub(super) const VAULT_ADMIT_PROMPT: &str = "admit prompt";
/// A queued prompt's start waits on the card for as long as the person needs.
pub(super) const VAULT_START_QUEUED_PROMPT: &str = "start queued prompt";
const VAULT_UNLOCK_CARD_OPEN: &str = "vault_unlock_card_open";
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

/// The asking turn ended when its prompt or its provider run is no longer the agent's; a turn
/// that started while the use waited (none was running when it asked) is not an end.
fn turn_ended(
    asking: &(Option<String>, Option<String>),
    now: &(Option<String>, Option<String>),
) -> bool {
    (asking.0.is_some() && asking.0 != now.0) || (asking.1.is_some() && asking.1 != now.1)
}

fn is_agent_vault_tool(operation: &str) -> bool {
    operation.starts_with("runtime_tool_")
        || operation.starts_with("vault_")
        || matches!(
            operation,
            "credential_vault_manage"
                | "kernel_browser_paste_secret"
                | "web_bot_auth"
                | "agent_auth_register"
                | "agent_auth_store"
        )
}

/// The Vault is open, and not only for one operation's "once" unlock.
pub(super) fn vault_open_to_every_use(path: &std::path::Path) -> Result<bool, DaemonError> {
    Ok(
        crate::secret::chariox_encrypted_vault_status(path)?.unlocked
            && !crate::secret::chariox_vault_held_for_another(path, None)?,
    )
}

/// How long a Vault use waits on the Room's unlock card; `None` waits until it ends.
fn vault_unlock_wait(operation: &str) -> Option<std::time::Duration> {
    match operation {
        VAULT_START_QUEUED_PROMPT => None,
        VAULT_ADMIT_PROMPT => Some(std::time::Duration::ZERO),
        tool if is_agent_vault_tool(tool) => Some(VAULT_UNLOCK_TOOL_WAIT),
        _ => Some(VAULT_UNLOCK_LAUNCH_WAIT),
    }
}

/// A use that stopped waiting while the Room's unlock card stays open for the person.
fn vault_unlock_card_open(operation: &str) -> DaemonError {
    let next = if is_agent_vault_tool(operation) {
        "call this tool again once the Vault is unlocked"
    } else {
        "try again once the Vault is unlocked"
    };
    DaemonError::VaultUnlock {
        code: VAULT_UNLOCK_CARD_OPEN,
        message: format!("The Vault is locked. The unlock card is open in this Room: {next}."),
        retryable: true,
    }
}

pub(super) fn is_vault_unlock_card_open(error: &DaemonError) -> bool {
    matches!(error, DaemonError::VaultUnlock { code, .. } if *code == VAULT_UNLOCK_CARD_OPEN)
}

/// Every use that waited on a card gets the reason it ended without an unlock.
fn refusal_for_waiter(error: &DaemonError, operation: &'static str) -> DaemonError {
    match *error {
        DaemonError::VaultUnlock {
            code,
            ref message,
            retryable,
        } => DaemonError::VaultUnlock {
            code,
            message: message.clone(),
            retryable,
        },
        ref other => DaemonError::LocalTransport {
            operation,
            message: other.to_string(),
        },
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
    id: &str,
    config: &crate::config::UserCredentialVaultConfig,
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
        id,
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
        None,
        None,
    )
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
        ("answered", Some(duration), Some(passphrase)) => {
            Ok((zeroize::Zeroizing::new(passphrase), duration))
        }
        // The kernel withdrew the card (it is stopping, or the Room ended): nobody declined, so
        // what waited on it stays to ask again.
        ("timed_out", ..) => Err(DaemonError::VaultUnlock {
            code: "vault_unlock_card_closed",
            message: "The Vault unlock card closed without an answer.".into(),
            retryable: true,
        }),
        (_, None, _) => Err(DaemonError::VaultUnlock {
            code: "vault_unlock_declined",
            message:
                "The Vault unlock was cancelled: unlock it from the Vault window, then try again."
                    .into(),
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

    // An unlocked Vault raises no card: managing it returns its state, and extending it is the
    // agent's own action that returns the new state.
    #[tokio::test]
    async fn managing_an_unlocked_vault_raises_no_card_and_returns_its_state() {
        let workspace = crate::test_support::TestWorktree::new("vault-manage-no-card");
        let private_state = crate::test_support::TestWorktree::new("vault-manage-no-card-state");
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
        for (action, expected) in [
            (None, "status"),
            (Some("status"), "status"),
            (Some("extend"), "extended_30m"),
            (Some("extend_60m"), "extended_60m"),
        ] {
            crate::secret::unlock_chariox_encrypted_vault(
                &path,
                "test passphrase",
                crate::secret::VaultUnlockLease::KernelShutdown,
            )
            .unwrap();
            let managed = state
                .manage_credential_vault(session.id(), agent.id(), action)
                .await
                .unwrap_or_else(|error| panic!("{action:?}: {error}"));
            assert_eq!(managed.action, expected);
            assert!(managed.status.unlocked, "{action:?}");
            assert!(
                state
                    .owned
                    .session_store
                    .get_session(session.id())
                    .unwrap()
                    .active_interaction_for_agent(agent.id())
                    .is_none(),
                "{action:?} raised a card"
            );
        }
        let refused = state
            .manage_credential_vault(session.id(), agent.id(), Some("dance"))
            .await
            .expect_err("an unknown action is refused");
        assert!(
            refused.to_string().contains("unsupported vault action"),
            "{refused}"
        );
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
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
            "vault-unlock-test",
            &config.user_config.credential_vault,
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

    // An agent's Vault tool waits on the card for the whole tool budget. When nobody answers in
    // time the card stays open in the Room; the call says so, and the card still unlocks later.
    #[tokio::test(start_paused = true)]
    async fn a_vault_tool_waits_its_budget_and_leaves_the_card_open_for_the_person() {
        let workspace = crate::test_support::TestWorktree::new("vault-tool-budget");
        let private_state = crate::test_support::TestWorktree::new("vault-tool-budget-state");
        let path = private_state.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "test passphrase").unwrap();
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::Ttl;
        config.user_config.credential_vault.path = path.display().to_string();
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace.session_request())
            .unwrap();
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        let card_of = |state: &KernelRuntimeState| {
            state
                .owned
                .session_store
                .get_session(session.id())
                .unwrap()
                .active_interaction_for_agent(agent.id())
                .cloned()
        };
        for operation in ["vault_use_env", "vault_write_file", "vault_copy"] {
            let call = state.ensure_vault_unlocked_for_agent(session.id(), agent.id(), operation);
            tokio::pin!(call);
            tokio::select! {
                result = &mut call => panic!("{operation} must wait for the person: {:?}", result.err()),
                () = tokio::time::sleep(std::time::Duration::from_secs(61)) => {},
            }
            assert!(card_of(&state).is_some(), "{operation} shows the card");
            let refusal = tokio::time::timeout(std::time::Duration::from_secs(230), &mut call)
                .await
                .expect("the call ends within its tool budget")
                .err()
                .expect("an unanswered unlock does not open the Vault");
            let message = refusal.to_string();
            assert!(
                message.contains("The Vault is locked")
                    && message.contains("unlock card is open in this Room")
                    && message.contains("again"),
                "{operation}: {message}"
            );
            assert!(
                card_of(&state).is_some(),
                "{operation} leaves the card open"
            );
        }
        // One card served all three calls, and it still unlocks the Vault.
        let card = card_of(&state).unwrap();
        state
            .resolve_terminal_runtime_interaction(
                session.id(),
                card.id(),
                "passphrase",
                Some("test passphrase"),
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !crate::secret::chariox_encrypted_vault_status(&path)
                .unwrap()
                .unlocked
                || card_of(&state).is_some()
            {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the open card unlocks the Vault after its callers gave up");
        let _ = state
            .ensure_vault_unlocked_for_agent(session.id(), agent.id(), "vault_use_env")
            .await
            .expect("the next call finds the Vault unlocked");
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
    }

    // The kernel withdrawing the card (it stops, or the Room ends) is not the person's cancel:
    // what waited on the card stays to ask again.
    #[test]
    fn a_withdrawn_unlock_card_is_not_a_cancel() {
        let card = unlock_card(crate::config::CredentialVaultUnlockPolicy::Ttl);
        let withdrawn = super::super::PendingInteractionResolution {
            status: "timed_out",
            choice_id: None,
            reply: None,
        };
        assert!(matches!(
            vault_unlock_answer(&card, withdrawn, "test"),
            Err(DaemonError::VaultUnlock {
                code: "vault_unlock_card_closed",
                retryable: true,
                ..
            })
        ));
        assert!(matches!(
            vault_unlock_answer(&card, resolution(Some("cancel"), None), "test"),
            Err(DaemonError::VaultUnlock {
                code: "vault_unlock_declined",
                ..
            })
        ));
    }

    // The card belongs to the Room, not to the turn that first needed the Vault: the end of that
    // turn leaves it waiting for the person.
    #[tokio::test]
    async fn the_unlock_card_outlives_the_turn_that_raised_it() {
        let workspace = crate::test_support::TestWorktree::new("vault-card-turn");
        let private_state = crate::test_support::TestWorktree::new("vault-card-turn-state");
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
        let room = state.owned.session_store.get_session(session.id()).unwrap();
        state
            .owned
            .prompt_state_owner
            .submit_prepared_prompt(
                &room,
                crate::session::PromptQueueItem::new(
                    "turn-needing-the-vault",
                    "attachment-1",
                    agent.id(),
                    "use the Vault",
                    crate::session::PromptStatus::Running,
                ),
                false,
            )
            .unwrap();
        let (active, queued) = state
            .owned
            .prompt_state_owner
            .state_parts(&room, agent.id());
        assert!(active.is_some(), "the agent is in a turn");
        state
            .owned
            .mirror_prompt_owner_agent_state(session.id(), agent.id(), active, queued)
            .unwrap();
        let call = {
            let (state, session_id, agent_id) = (
                state.clone(),
                session.id().to_owned(),
                agent.id().to_owned(),
            );
            tokio::spawn(async move {
                state
                    .ensure_vault_unlocked_for_agent(&session_id, &agent_id, "vault_use_env")
                    .await
                    .map(drop)
            })
        };
        let card = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(card) = state
                    .owned
                    .session_store
                    .get_session(session.id())
                    .unwrap()
                    .active_interaction_for_agent(agent.id())
                    .cloned()
                {
                    return card;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the use raises the Room's card");
        let lifetime = state
            .owned
            .pending_interactions
            .write()
            .get(card.id())
            .and_then(|pending| pending.agent_lifetime.clone())
            .expect("the card names its agent");
        assert_eq!(
            (
                lifetime.prompt_id,
                lifetime.provider_run_id,
                lifetime.native_turn_id
            ),
            (None, None, None),
            "the card is not bound to the agent's turn"
        );
        call.abort();
        std::fs::remove_file(&path).unwrap();
    }

    // Vault "ask every time": two Rooms each have an agent start waiting on its own unlock
    // card. A "just this once" unlock serves exactly one of those starts, whether the owner
    // answers one card or unlocks once from a window; the other Room's card stays open.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_once_unlock_serves_one_room_and_leaves_the_other_rooms_card_open() {
        let workspace_a = crate::test_support::TestWorktree::new("vault-once-room-a");
        let workspace_b = crate::test_support::TestWorktree::new("vault-once-room-b");
        let private_state = crate::test_support::TestWorktree::new("vault-once-rooms-state");
        let path = private_state.path().join("vault.json");
        crate::secret::create_chariox_encrypted_vault_for_test(&path, "test passphrase").unwrap();
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = path.display().to_string();
        config.user_config.credential_vault.unlock_policy =
            crate::config::CredentialVaultUnlockPolicy::Always;
        let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
        let (room_a, agent_a) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace_a.session_request())
            .unwrap();
        let (room_b, agent_b) = crate::app::KernelSessionService::new(&mut app)
            .create_session(workspace_b.session_request())
            .unwrap();
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        let owner = room_a.owner_user_id().to_owned();
        let rooms = [
            (room_a.id().to_owned(), agent_a.id().to_owned()),
            (room_b.id().to_owned(), agent_b.id().to_owned()),
        ];
        // An agent start in a Room that needs the Vault.
        let start = |room: usize| {
            let state = state.clone();
            let (session_id, agent_id) = rooms[room].clone();
            tokio::spawn(async move {
                state
                    .ensure_vault_unlocked_for_agent(&session_id, &agent_id, "launch provider")
                    .await
            })
        };
        let card = |room: usize| {
            let (session_id, agent_id) = &rooms[room];
            state
                .owned
                .session_store
                .get_session(session_id)
                .unwrap()
                .active_interaction_for_agent(agent_id)
                .map(|card| card.id().to_owned())
        };
        let open_card = |room: usize| async move {
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
                loop {
                    if let Some(id) = card(room) {
                        return id;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("the Room shows its unlock card")
        };
        let settle = || tokio::time::sleep(std::time::Duration::from_millis(500));
        let mut wrong = Vec::new();

        // 1. The owner answers Room A's card with "Just this once".
        let mut starts = [start(0), start(1)];
        let card_a = open_card(0).await;
        open_card(1).await;
        state
            .resolve_terminal_runtime_interaction(
                &rooms[0].0,
                &card_a,
                "unlock_once",
                Some("test passphrase"),
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .unwrap();
        let guard_a = tokio::time::timeout(std::time::Duration::from_secs(20), &mut starts[0])
            .await
            .expect("Room A's start goes on after its card is answered")
            .unwrap()
            .expect("Room A's start has the Vault");
        settle().await;
        if starts[1].is_finished() {
            wrong.push("card answer: Room A's \"once\" let Room B's start go on too".to_owned());
        }
        if card(1).is_none() {
            wrong.push("card answer: Room A's \"once\" closed Room B's card".to_owned());
        }
        drop(guard_a);
        settle().await;
        if starts[1].is_finished() {
            starts[1] = start(1);
        }

        // 2. Both Rooms wait again; the owner unlocks once from a window (the staging probe's
        //    `UnlockCredentialVault { duration: once, surface: terminal }`).
        starts[0] = start(0);
        open_card(0).await;
        open_card(1).await;
        state
            .unlock_credential_vault_for_owner(
                &owner,
                crate::local::UnlockCredentialVaultRequest {
                    passphrase: crate::local::VaultPassphrase::new("test passphrase"),
                    duration: crate::local::VaultUnlockDuration::Once,
                    surface: crate::local::VaultUnlockSurface::Terminal,
                },
            )
            .await
            .unwrap();
        settle().await;
        let still_asking = (0..2).filter(|&room| card(room).is_some()).count();
        let mut waiting = Vec::new();
        let mut went_on = 0;
        for (room, start) in starts.into_iter().enumerate() {
            if start.is_finished() {
                // The guard of a start that has the Vault stays held until both are counted.
                waiting.push((room, None, Some(start.await.unwrap())));
            } else {
                waiting.push((room, Some(start), None));
            }
        }
        for (_, _, result) in &waiting {
            if matches!(result, Some(Ok(_))) {
                went_on += 1;
            }
        }
        if (went_on, still_asking) != (1, 1) {
            wrong.push(format!(
                "window unlock: one \"once\" unlock gave the Vault to {went_on} of 2 Rooms' \
                 starts and left {still_asking} of 2 cards open (expected 1 and 1)"
            ));
        }

        for (room, start, result) in waiting {
            drop(result);
            let Some(start) = start else { continue };
            if let Some(id) = card(room) {
                let _ = state
                    .resolve_terminal_runtime_interaction(
                        &rooms[room].0,
                        &id,
                        "cancel",
                        None,
                        Some(crate::session::DEFAULT_LOCAL_USER_ID),
                    )
                    .await;
            }
            let _ = tokio::time::timeout(std::time::Duration::from_secs(20), start).await;
        }
        let _ = crate::secret::lock_chariox_encrypted_vault(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    // PCD-04: a wrong passphrase asks again in a card that says so; the agent's operation waits and then goes on.
    #[tokio::test(start_paused = true)]
    async fn a_wrong_passphrase_asks_again_instead_of_failing_the_agents_operation() {
        wrong_passphrase_retry(false).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_wrong_passphrase_keeps_the_card_open_past_the_tool_budget() {
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
                let mut seen: Vec<Vec<(String, bool, usize)>> = Vec::new();
                while sampling.load(std::sync::atomic::Ordering::SeqCst) {
                    let now = visible()
                        .iter()
                        .map(|card| {
                            (
                                card.id().to_owned(),
                                card.message()
                                    .starts_with("That is not the Vault passphrase"),
                                card.message().lines().count(),
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
            let (state, session_id, id) =
                (state.clone(), session_id.clone(), first.id().to_owned());
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
            while !visible().pop().is_some_and(|card| {
                card.message()
                    .starts_with("That is not the Vault passphrase")
            }) {
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
        assert!(
            seen.iter()
                .all(|cards| cards[0].2 == first.message().lines().count()),
            "the refusal changed the card's line count, so the card moves: {seen:?}"
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
            "vault-unlock-test",
            &config,
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

    // Two agents of one Room need the locked Vault: the second joins the first one's card, and
    // the card's end answers both.
    #[tokio::test]
    async fn a_second_use_in_the_room_joins_the_open_card() {
        joined_vault_unlock(true).await;
    }

    #[tokio::test]
    async fn a_cancelled_card_refuses_every_use_that_joined_it() {
        joined_vault_unlock(false).await;
    }

    async fn joined_vault_unlock(unlock: bool) {
        let workspace = crate::test_support::TestWorktree::new("vault-room-join");
        let private_state = crate::test_support::TestWorktree::new("vault-room-join-state");
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
        let cards = || {
            state
                .owned
                .session_store
                .get_session(session.id())
                .unwrap()
                .active_interactions()
                .iter()
                .filter(|card| card.id().starts_with("vault-unlock-"))
                .cloned()
                .collect::<Vec<_>>()
        };
        let use_vault = |agent_id: String| {
            let state = state.clone();
            let session_id = session.id().to_owned();
            tokio::spawn(async move {
                state
                    .ensure_vault_unlocked_for_agent(&session_id, &agent_id, "vault_generate")
                    .await
                    .map(drop)
            })
        };
        let first = use_vault(first_agent.id().to_owned());
        let card = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(card) = cards().into_iter().next() {
                    return card;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the first use raises the Room's card");
        let second = use_vault(second_agent.id().to_owned());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            cards()
                .iter()
                .map(|card| card.id().to_owned())
                .collect::<Vec<_>>(),
            vec![card.id().to_owned()],
            "the second use joins the open card"
        );
        assert!(!first.is_finished() && !second.is_finished());
        let (choice, reply) = if unlock {
            ("passphrase", Some("test passphrase"))
        } else {
            ("cancel", None)
        };
        state
            .resolve_terminal_runtime_interaction(
                session.id(),
                card.id(),
                choice,
                reply,
                Some(crate::session::DEFAULT_LOCAL_USER_ID),
            )
            .await
            .unwrap();
        for call in [first, second] {
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), call)
                .await
                .expect("the card's end answers every use that joined it")
                .unwrap();
            if unlock {
                result.expect("both uses go on after the unlock");
            } else {
                assert!(matches!(
                    result,
                    Err(DaemonError::VaultUnlock {
                        code: "vault_unlock_declined",
                        ..
                    })
                ));
            }
        }
        assert!(cards().is_empty());
        assert_eq!(
            crate::secret::chariox_encrypted_vault_status(&path)
                .unwrap()
                .unlocked,
            unlock
        );
        crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
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
        let next_card = |refused: bool| {
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
                            .filter(|card| {
                                !refused
                                    || card
                                        .message()
                                        .starts_with("That is not the Vault passphrase")
                            })
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
        let first = next_card(false).await;
        tokio::time::advance(std::time::Duration::from_secs(200)).await;
        answer(&first, "not the passphrase").await;
        let again = next_card(true).await;
        assert_eq!(again.id(), first.id(), "the refusal shows in the same card");
        assert!(
            again
                .message()
                .starts_with("That is not the Vault passphrase"),
            "{}",
            again.message()
        );
        assert!(!asking.is_finished(), "the agent's operation still waits");
        if expire {
            let result =
                tokio::time::timeout_at(started + std::time::Duration::from_secs(281), asking)
                    .await
                    .expect("the operation settles before its provider tool deadline")
                    .unwrap();
            assert!(
                matches!(&result, Err(error) if is_vault_unlock_card_open(error)),
                "{result:?}"
            );
            let open = next_card(true).await;
            assert_eq!(open.id(), first.id(), "the card stays open for the person");
            answer(&open, "test passphrase").await;
            tokio::time::timeout(std::time::Duration::from_secs(20), async {
                while !crate::secret::chariox_encrypted_vault_status(&path)
                    .unwrap()
                    .unlocked
                {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("a later answer still unlocks the Vault");
            crate::secret::lock_chariox_encrypted_vault(&path).unwrap();
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
        vault_passphrase_interaction(
            &requester(None, None),
            "agent-1",
            "vault-unlock-test",
            &config,
        )
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
    fn a_passphrase_change_asks_the_person_through_secret_prompts() {
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
            let mut interactions = vec![vault_passphrase_interaction(
                &requester,
                agent_id,
                "vault-unlock-test",
                &crate::config::UserCredentialVaultConfig::default(),
            )];
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
                if message.contains("no agent to show its unlock card")
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
#[path = "vault_restart_tests.rs"]
mod restart_tests;
