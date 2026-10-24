//! MP-08/MP-10/MP-11: upgrade-safe, bounded first-use verification. Network
//! failure is not evidence that a token is invalid and never triggers OAuth.
use super::*;
use crate::account_profile::ProviderAccountAuthState;
use crate::provider::{ClaudeCredentialCheckError, ProviderCredentialEnvironment};

impl KernelRuntimeState {
    pub(in crate::runtime) async fn invalidate_claude_token_check(
        &self,
        owner: &str,
        profile: &str,
    ) {
        let id = crate::provider::provider_account_credential_id(owner, "claude", profile);
        self.owned.claude_token_checks.invalidate(&id).await;
    }

    pub(super) async fn checked_claude_credentials(
        &self,
        request: &crate::provider::LaunchProviderRequest,
        credentials: ProviderCredentialEnvironment,
    ) -> Result<ProviderCredentialEnvironment, DaemonError> {
        let Some((_, token)) = credentials
            .iter()
            .find(|(key, _)| *key == crate::provider::CLAUDE_OAUTH_TOKEN_ENV)
        else {
            return Ok(credentials);
        };
        let owner = crate::account_profile::provider_account_authority_for_launch(
            &self.owned.config_projection.snapshot(),
            &self.owned.provider_account_profiles,
            request,
        )?;
        let profile =
            self.owned
                .provider_account_profiles
                .get(&owner, "claude", &request.account_profile)?;
        if profile.auth_state == ProviderAccountAuthState::Authenticated {
            return Ok(credentials);
        }
        let credential_id =
            crate::provider::provider_account_credential_id(&owner, "claude", &profile.profile_id);
        let checked = if profile.auth_state == ProviderAccountAuthState::Expired {
            Err(ClaudeCredentialCheckError::Rejected)
        } else {
            let checked = self
                .owned
                .claude_token_checks
                .check(&credential_id, || async {
                    crate::runtime::claude_setup_token_login::check(
                        self,
                        &owner,
                        &profile.profile_id,
                        &zeroize::Zeroizing::new(token.to_string()),
                    )
                    .await
                })
                .await;
            let Some(checked) = checked else {
                // A replacement invalidated this in-flight credential generation.
                // Resolve it through the ordinary Vault path before retrying.
                drop(credentials);
                let fresh = self
                    .resolve_signed_in_claude_credentials(request, &owner, &profile.profile_id)
                    .await?;
                return Box::pin(self.checked_claude_credentials(request, fresh)).await;
            };
            checked
        };
        match checked {
            Ok(()) => {
                crate::runtime::claude_setup_token_login::record_verified_token(
                    self,
                    &owner,
                    &profile.profile_id,
                )
                .await;
                Ok(credentials)
            }
            // A network/launcher failure does not establish rejection. Let
            // the ordinary provider run make its own request; any later auth
            // refusal follows the same shared recovery interaction.
            Err(ClaudeCredentialCheckError::Inconclusive(_)) => {
                crate::logging::warn_with_fields(
                    "daemon.provider_account",
                    "claude token first-use check was inconclusive",
                    serde_json::json!({"profile_id": profile.profile_id}),
                );
                Ok(credentials)
            }
            Err(ClaudeCredentialCheckError::Rejected) => {
                drop(credentials);
                let _login_lane = self
                    .provider_runtime_lanes
                    .acquire(&format!("claude-account-login:{credential_id}"))
                    .await;
                if self
                    .owned
                    .provider_account_profiles
                    .get(&owner, "claude", &profile.profile_id)?
                    .auth_state
                    == ProviderAccountAuthState::Authenticated
                {
                    return self
                        .resolve_signed_in_claude_credentials(request, &owner, &profile.profile_id)
                        .await;
                }
                let _ = self.owned.provider_account_profiles.update_observation(
                    &owner,
                    "claude",
                    &profile.profile_id,
                    ProviderAccountAuthState::Expired,
                    None,
                    None,
                    None,
                    None,
                );
                let agent_id = self
                    .vault_prompt_agent(
                        &request.session_id,
                        request.agent_id.as_deref(),
                        "Claude authorization",
                    )
                    .await?;
                let response =
                    crate::runtime::provider_auth_control::execute_start_provider_login_request(
                        self,
                        &owner,
                        crate::local::StartProviderLoginRequest {
                            provider: "claude".into(),
                            account_profile: profile.profile_id.clone(),
                            method: Some("setup_token".into()),
                        },
                    )
                    .await?;
                let LocalDaemonResponse::ProviderLoginStarted { login } = response else {
                    return Err(login_cancelled());
                };
                let id = format!(
                    "provider-auth-recovery:{}",
                    login.login_id.as_deref().unwrap_or_default()
                );
                if !self
                    .wait_for_account_login(&request.session_id, &agent_id, &owner, &id, &login)
                    .await?
                {
                    return Err(login_cancelled());
                }
                self.resolve_signed_in_claude_credentials(request, &owner, &profile.profile_id)
                    .await
            }
        }
    }
    async fn resolve_signed_in_claude_credentials(
        &self,
        request: &crate::provider::LaunchProviderRequest,
        owner: &str,
        profile: &str,
    ) -> Result<ProviderCredentialEnvironment, DaemonError> {
        // Storage relocks operation-scoped Vaults. Launch reads get their own
        // ordinary unlock; the prior lease never spans human OAuth consent.
        let agent_id = self
            .vault_prompt_agent(
                &request.session_id,
                request.agent_id.as_deref(),
                "use signed-in Claude account",
            )
            .await?;
        let _unlock = self
            .ensure_vault_unlocked_for_agent(
                &request.session_id,
                &agent_id,
                "use signed-in Claude account",
            )
            .await?;
        crate::provider::resolve_provider_account_credentials(
            &self.owned.config_projection.snapshot(),
            owner,
            "claude",
            profile,
        )
    }
}

fn login_cancelled() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "Claude authorization",
        message:
            "Claude sign-in did not complete. Choose Log in in Provider Accounts to try again."
                .into(),
    }
}
