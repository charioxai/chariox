//! MP-08/MP-10/MP-11: upgrade-safe, bounded first-use verification. Network
//! failure is not evidence that a token is invalid and never triggers OAuth.
use super::*;
use crate::account_profile::ProviderAccountAuthState;
use crate::provider::{ClaudeCredentialCheckError, ProviderCredentialEnvironment};

type ClaudeTokenCheck = Mutex<Option<Result<(), ClaudeCredentialCheckError>>>;

#[derive(Default)]
pub(super) struct ClaudeTokenChecks(Mutex<BTreeMap<String, Arc<ClaudeTokenCheck>>>);

impl KernelRuntimeState {
    pub(in crate::runtime) async fn invalidate_claude_token_check(
        &self,
        owner: &str,
        profile: &str,
    ) {
        let id = crate::provider::provider_account_credential_id(owner, "claude", profile);
        self.owned.claude_token_checks.0.lock().await.remove(&id);
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
        let owner = self.provider_account_authority_owner_user_id(&request.owner_user_id);
        let profile =
            self.owned
                .provider_account_profiles
                .get(&owner, "claude", &request.account_profile)?;
        let verification = crate::provider::provider_account_credential_verification(
            &owner,
            "claude",
            &profile.profile_id,
        )?;
        if !verification.registered
            || (verification.verified
                && profile.auth_state == ProviderAccountAuthState::Authenticated)
        {
            return Ok(credentials);
        }
        let credential_id =
            crate::provider::provider_account_credential_id(&owner, "claude", &profile.profile_id);
        // The map lock only locates an account's check lane. A slow official
        // CLI must not hold unrelated launches or completed replacements.
        let lane = self
            .owned
            .claude_token_checks
            .0
            .lock()
            .await
            .entry(credential_id.clone())
            .or_default()
            .clone();
        let mut checks = lane.lock().await;
        // Cache verdicts only. Invalidation removes the lane from the map, so
        // an in-flight old check cannot repopulate a replacement's cache.
        let checked =
            if verification.verified && profile.auth_state == ProviderAccountAuthState::Expired {
                Err(ClaudeCredentialCheckError::Rejected)
            } else if let Some(checked) = checks.as_ref() {
                checked.clone()
            } else {
                let checked = crate::runtime::claude_setup_token_login::check(
                    self,
                    &owner,
                    &profile.profile_id,
                    &zeroize::Zeroizing::new(token.to_string()),
                )
                .await;
                *checks = Some(checked.clone());
                checked
            };
        drop(checks);
        match checked {
            Ok(()) => {
                let _login_lane = self
                    .provider_runtime_lanes
                    .acquire(&format!("claude-account-login:{credential_id}"))
                    .await;
                if !crate::provider::mark_provider_account_credential_verified(
                    &owner,
                    &profile.profile_id,
                    verification.revision,
                )? {
                    return self
                        .resolve_signed_in_claude_credentials(request, &owner, &profile.profile_id)
                        .await;
                }
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
            Err(ClaudeCredentialCheckError::Inconclusive(_)) => Ok(credentials),
            Err(ClaudeCredentialCheckError::Rejected) => {
                drop(credentials);
                let _login_lane = self
                    .provider_runtime_lanes
                    .acquire(&format!("claude-account-login:{credential_id}"))
                    .await;
                if crate::provider::provider_account_credential_verification(
                    &owner,
                    "claude",
                    &profile.profile_id,
                )?
                .verified
                    && self
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
                drop(_login_lane);
                let response = crate::runtime::provider_auth_control::start_claude_login_for_rejected_revision(
                    self, &owner, &profile.profile_id, verification.revision,
                ).await?;
                let Some(LocalDaemonResponse::ProviderLoginStarted { login }) = response else {
                    return self
                        .resolve_signed_in_claude_credentials(request, &owner, &profile.profile_id)
                        .await;
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
