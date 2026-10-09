//! MP-08/MP-10/MP-11: receiving-kernel login; no credential synchronization.
use super::*;
use crate::local::{LocalDaemonResponse, ProviderLoginProcessState};
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
    RuntimeProviderLogin,
};

struct RecoveryClaim {
    run_id: String,
    claims: Arc<std::sync::Mutex<BTreeSet<String>>>,
}
impl Drop for RecoveryClaim {
    fn drop(&mut self) {
        self.claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.run_id);
    }
}

impl KernelRuntimeState {
    pub(super) fn publish_credential_copy_notices(&self, session_id: &str) {
        let config = self.owned.config_projection.snapshot();
        for agent in self
            .owned
            .agent_store
            .list_agents()
            .into_iter()
            .filter(|agent| agent.session_id() == session_id && agent.remote_execution().is_none())
        {
            let Ok(owner) = crate::account_profile::provider_account_authority_owner_for_profile(
                &config,
                &self.owned.provider_account_profiles,
                agent.owner_user_id(),
                agent.provider(),
                agent.provider_account_profile(),
            ) else {
                continue;
            };
            if let Ok(Some(message)) = self
                .owned
                .provider_account_profiles
                .take_credential_copy_notice(
                    &owner,
                    agent.provider(),
                    agent.provider_account_profile(),
                    &config.host_machine_id,
                )
            {
                self.owned.record_notice_for_agent(
                    session_id,
                    None,
                    Some(agent.id()),
                    self.owned
                        .attachment_store
                        .list_session_attachment_ids(session_id),
                    message,
                );
            }
        }
    }

    pub(super) fn try_provider_auth_recovery<'a>(
        &'a self,
        run: &'a crate::provider::RuntimeProviderRun,
        message: &'a str,
        expected_prompt_id: Option<&'a str>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, DaemonError>> + Send + 'a>>
    {
        self.start_provider_auth_recovery(run, message, expected_prompt_id, None)
    }

    pub(super) fn try_provider_launch_auth_recovery<'a>(
        &'a self,
        started: &'a crate::app::StartedProviderLaunch,
        message: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, DaemonError>> + Send + 'a>>
    {
        self.start_provider_auth_recovery(&started.run, message, None, Some(started.clone()))
    }

    fn start_provider_auth_recovery<'a>(
        &'a self,
        run: &'a crate::provider::RuntimeProviderRun,
        message: &'a str,
        expected_prompt_id: Option<&'a str>,
        failed_launch: Option<crate::app::StartedProviderLaunch>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, DaemonError>> + Send + 'a>>
    {
        Box::pin(async move {
            if !crate::provider::renewal_failure::renewal_failed(run.adapter_key(), message) {
                return Ok(false);
            }
            let Some(agent_id) = run.agent_instance_id() else {
                return Ok(false);
            };
            let owner = crate::account_profile::provider_account_authority_owner_for_profile(
                &self.owned.config_projection.snapshot(),
                &self.owned.provider_account_profiles,
                run.owner_user_id(),
                run.provider(),
                run.account_profile(),
            )?;
            if !crate::provider::renewal_failure::oauth_renewal_evidence(message)
                && !self
                    .owned
                    .provider_account_profiles
                    .has_renewable_login(&owner, run.adapter_key(), run.account_profile())
                    .unwrap_or(false)
            {
                return Ok(false);
            }
            let claims = Arc::clone(&self.owned.provider_auth_recovery_runs);
            if !claims
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(run.id().into())
            {
                return Ok(true);
            }
            let claim = RecoveryClaim {
                run_id: run.id().into(),
                claims,
            };
            let session = self.owned.session_store.get_session(run.session_id())?;
            let prompt = self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id);
            if expected_prompt_id
                .is_some_and(|id| prompt.as_ref().is_none_or(|prompt| id != prompt.id()))
                || (prompt.is_none() && failed_launch.is_none())
            {
                return Ok(false);
            }
            let id = format!("provider-auth-recovery:{}", run.id());
            if session
                .active_interactions()
                .iter()
                .any(|interaction| interaction.id().starts_with(&id))
            {
                return Ok(true);
            }
            // Never replace a provider permission or another kernel interaction.
            if session.active_interaction_for_agent(agent_id).is_some() {
                return Ok(false);
            }
            let provider = provider_label(run.adapter_key());
            let receiver = self.create_runtime_interaction(run.session_id(), RuntimeInteraction::new(
            &id, agent_id, RuntimeInteractionKind::Choice, RuntimeInteractionLevel::Warning,
            Some(format!("Log in to {provider} on this machine")),
            "The provider could not renew its login. Credentials will stay on this machine.",
            vec![choice("login", "Log in", RuntimeInteractionChoiceStyle::Primary),
                choice("cancel", "Cancel", RuntimeInteractionChoiceStyle::Secondary)],
            None, Some(600), None,
        )).await?;
            self.owned.clear_prompt_activity(run.id());
            let state = self.clone();
            let run = run.clone();
            tokio::spawn(async move {
                let _claim = claim;
                let outcome = state.recover_provider_login(&run, &id, receiver).await;
                if outcome.is_ok_and(|succeeded| succeeded) {
                    let _permit = state.provider_runtime_lanes.acquire(run.id()).await;
                    let current = state
                        .owned
                        .provider_store
                        .get_run_for_agent(run.session_id(), agent_id_for(&run));
                    if current
                        .as_ref()
                        .is_none_or(|current| current.id() != run.id())
                    {
                        return;
                    }
                    // Reload the official harness so its in-memory auth cannot retain the invalidated copy.
                    if state
                        .restart_provider_after_login(&run, prompt.as_ref())
                        .await
                        .is_ok()
                    {
                        return;
                    }
                }
                // Settle through the ordinary failure path without triggering recovery again.
                let _permit = state.provider_runtime_lanes.acquire(run.id()).await;
                let message =
                    "Provider login did not complete; retry after logging in on this machine.";
                if let Some(started) = failed_launch.as_ref() {
                    state
                        .fail_provider_launch_in_lane(
                            started,
                            &DaemonError::LocalTransport {
                                operation: "provider login recovery",
                                message: message.into(),
                            },
                        )
                        .await;
                } else if let Some(prompt) = prompt.as_ref() {
                    let _ = state
                        .fail_owned_provider_prompt_with_termination_if_matches(
                            run.session_id(),
                            run.id(),
                            message,
                            true,
                            Some(prompt.id()),
                            None,
                        )
                        .await;
                }
            });
            Ok(true)
        })
    }

    async fn redispatch_after_provider_login(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<bool, DaemonError> {
        let session = self.owned.session_store.get_session(run.session_id())?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id_for(run))
            .is_none_or(|active| active.id() != prompt.id())
        {
            return Ok(false);
        }
        let dispatch = crate::app::KernelPromptDispatch {
            session_id: run.session_id().into(),
            provider_run_id: run.id().into(),
            agent_id: agent_id_for(run).into(),
            prompt_id: prompt.id().into(),
            target_active_prompt_id: None,
            source_attachment_id: prompt.source_attachment_id().into(),
            prompt: prompt.prompt().into(),
            hidden_system_context: prompt.hidden_system_context().into(),
            attachments: prompt.attachments().to_vec(),
            prompt_origin: prompt.prompt_origin(),
            external_provider: prompt.external_provider().map(str::to_string),
            external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
            external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
            steering: false,
        };
        self.enqueue_prompt_dispatch_after_liveness_with_acceptance(&dispatch, &self.owned)
            .await
    }

    async fn restart_provider_after_login(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<(), DaemonError> {
        let agent_id = agent_id_for(run);
        let config = self.owned.config_projection.snapshot();
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let request = super::provider_reload::policy_reload_launch_request(
            run,
            agent_id,
            agent.provider_resume_state().clone(),
        )
        .with_client_interface(run.client_interface())
        .with_workspace_live_sync_mode(
            crate::provider::provider_workspace_live_sync_mode_for_session(
                run.provider(),
                &config,
                self.owned
                    .session_store
                    .get_session(run.session_id())
                    .ok()
                    .as_ref(),
            ),
        );
        let request = self
            .prepare_provider_launch_request_with_vault(request, "resume after provider login")
            .await?;
        self.retire_owned_provider_run_after_terminal_failure(run.session_id(), run.id())
            .await;
        self.spawn_provider_relaunch(
            request,
            config.provider_runtime_init_delay_ms,
            Some(run.id().into()),
            0,
        );
        if let Some(prompt) = prompt.cloned() {
            let state = self.clone();
            let session_id = run.session_id().to_string();
            let agent_id = agent_id.to_string();
            let previous_run = run.id().to_string();
            tokio::spawn(async move {
                // Relaunch retains the admitted prompt; dispatch only when the replacement is ready.
                let outcome = tokio::time::timeout(Duration::from_secs(60), async {
                    loop {
                        if let Some(run) = state
                            .owned
                            .provider_store
                            .get_run_for_agent(&session_id, &agent_id)
                        {
                            if run.id() != previous_run
                                && run.state() == crate::provider::ProviderRunState::Running
                            {
                                let _permit = state.provider_runtime_lanes.acquire(run.id()).await;
                                return state.redispatch_after_provider_login(&run, &prompt).await;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                })
                .await;
                if !matches!(outcome, Ok(Ok(true))) {
                    let _ = state
                        .fail_provider_login_redispatch(
                            &session_id,
                            &agent_id,
                            &previous_run,
                            &prompt,
                        )
                        .await;
                }
            });
        }
        Ok(())
    }

    async fn fail_provider_login_redispatch(
        &self,
        session_id: &str,
        agent_id: &str,
        previous_run_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<bool, DaemonError> {
        let run = match self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
        {
            Some(run) => run,
            None => self.owned.provider_store.get_run(previous_run_id)?,
        };
        let _permit = self.provider_runtime_lanes.acquire(run.id()).await;
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(active) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(false);
        };
        if active.id() != prompt.id() {
            return Ok(false);
        }
        // MP-08/MP-10/MP-11: transfer the retained turn's failure ownership to
        // its replacement, including a run still initializing at the deadline.
        // The shared failure path then retires it and advances the backlog once.
        if active
            .durable_delivery_provider_run_id()
            .is_some_and(|id| id != previous_run_id && id != run.id())
        {
            return Ok(false);
        }
        self.owned.mark_active_prompt_delivery(
            session_id,
            agent_id,
            prompt.id(),
            crate::session::DurablePromptDeliveryPhase::Accepted,
            Some(run.id().to_string()),
            None,
        )?;
        self.fail_owned_provider_prompt_with_termination_if_matches(
            session_id,
            run.id(),
            "Provider could not resume the turn after login; retry on this machine.",
            true,
            Some(prompt.id()),
            None,
        )
        .await
    }

    async fn recover_provider_login(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        id: &str,
        receiver: tokio::sync::oneshot::Receiver<PendingInteractionResolution>,
    ) -> Result<bool, DaemonError> {
        let resolution = tokio::time::timeout(Duration::from_secs(600), receiver).await;
        let _ = self.timeout_runtime_interaction(run.session_id(), id).await;
        if !resolution
            .is_ok_and(|reply| reply.is_ok_and(|reply| reply.choice_id.as_deref() == Some("login")))
        {
            return Ok(false);
        }
        let owner = crate::account_profile::provider_account_authority_owner_for_profile(
            &self.owned.config_projection.snapshot(),
            &self.owned.provider_account_profiles,
            run.owner_user_id(),
            run.provider(),
            run.account_profile(),
        )?;
        let response = crate::runtime::provider_auth_control::execute_start_provider_login_request(
            self,
            &owner,
            crate::local::StartProviderLoginRequest {
                provider: run.adapter_key().into(),
                account_profile: run.account_profile().into(),
                method: None,
            },
        )
        .await?;
        let LocalDaemonResponse::ProviderLoginStarted { login } = response else {
            return Ok(false);
        };
        let Some(login_id) = login.login_id.clone() else {
            return Ok(false);
        };
        let progress_id = format!("{id}:login");
        let result = self
            .wait_for_provider_login(run, &owner, &progress_id, &login)
            .await;
        let _ = self
            .update_provider_login_interaction(
                run.session_id(),
                agent_id_for(run),
                &progress_id,
                None,
            )
            .await;
        if !result.as_ref().is_ok_and(|succeeded| *succeeded) {
            let _ = crate::runtime::provider_auth_control::execute_cancel_provider_login_request(
                self,
                &owner,
                crate::local::CancelProviderLoginRequest { login_id },
            )
            .await;
        }
        result
    }

    async fn wait_for_provider_login(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        owner: &str,
        id: &str,
        login: &crate::provider::ProviderLoginStart,
    ) -> Result<bool, DaemonError> {
        let projection = RuntimeProviderLogin {
            kernel_id: self.owned.config_projection.snapshot().daemon_id,
            login: login.clone(),
            terminal_output_base64: String::new(),
        };
        let mut receiver = self
            .create_runtime_interaction(
                run.session_id(),
                login_interaction(run, id, projection.clone()),
            )
            .await?;
        loop {
            tokio::select! {
                reply = &mut receiver => {
                    let Ok(reply) = reply else { return Ok(false); };
                    if reply.choice_id.as_deref() != Some("provider-response") { return Ok(false); }
                    let Some(mut input) = reply.reply else { return Ok(false); };
                    input.push('\r');
                    let data_base64 = base64::engine::general_purpose::STANDARD.encode(input.as_bytes());
                    use zeroize::Zeroize;
                    input.zeroize();
                    crate::runtime::provider_auth_control::execute_send_provider_login_input_request(
                        self, owner, crate::local::SendProviderLoginInputRequest {
                            login_id: login.login_id.clone().unwrap(), data_base64,
                        },
                    ).await?;
                    receiver = self.create_runtime_interaction(run.session_id(), login_interaction(run, id, projection.clone())).await?;
                }
                _ = tokio::time::sleep(Duration::from_millis(1500)) => {
                    let response = crate::runtime::provider_auth_control::execute_get_provider_login_status_request(
                        self, owner, crate::local::GetProviderLoginStatusRequest { login_id: login.login_id.clone().unwrap() },
                    ).await?;
                    let LocalDaemonResponse::ProviderLoginStatus { login: status } = response else { return Ok(false); };
                    match status.state {
                        ProviderLoginProcessState::Succeeded => return Ok(true),
                        ProviderLoginProcessState::Failed | ProviderLoginProcessState::Cancelled => return Ok(false),
                        ProviderLoginProcessState::Running => {}
                    }
                    self.update_provider_login_interaction(run.session_id(), agent_id_for(run), id, Some(RuntimeProviderLogin {
                        terminal_output_base64: status.terminal_output_base64, ..projection.clone()
                    })).await?;
                }
            }
        }
    }
}

fn agent_id_for(run: &crate::provider::RuntimeProviderRun) -> &str {
    run.agent_instance_id().unwrap_or_default()
}
fn provider_label(provider: &str) -> &str {
    match provider {
        "codex" => "Codex",
        "claude" => "Claude",
        "opencode" => "OpenCode",
        other => other,
    }
}
fn choice(id: &str, label: &str, style: RuntimeInteractionChoiceStyle) -> RuntimeInteractionChoice {
    RuntimeInteractionChoice::new(id, label, id, Some(style))
}
fn login_interaction(
    run: &crate::provider::RuntimeProviderRun,
    id: &str,
    login: RuntimeProviderLogin,
) -> RuntimeInteraction {
    RuntimeInteraction::new(
        id,
        agent_id_for(run),
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Warning,
        Some(format!(
            "Log in to {} on this machine",
            provider_label(run.adapter_key())
        )),
        "Complete the provider's official login. The agent will resume when login succeeds.",
        vec![choice(
            "cancel",
            "Cancel login",
            RuntimeInteractionChoiceStyle::Secondary,
        )],
        (login.login.login_kind == "terminal").then(|| {
            RuntimeInteractionCustomChoice::secret(
                "provider-response",
                "Send response",
                Some("Provider login response".into()),
                Some(1),
                Some(8192),
            )
        }),
        Some(600),
        None,
    )
    .with_provider_login(login)
}
