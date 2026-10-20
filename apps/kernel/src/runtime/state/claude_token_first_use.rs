//! MP-08/MP-10/MP-11: upgrade-safe, bounded first-use verification. Network
//! failure is not evidence that a token is invalid and never triggers OAuth.
use super::*;
use crate::account_profile::ProviderAccountAuthState;
use crate::provider::{ClaudeCredentialCheckError, ProviderCredentialEnvironment};
use sha2::{Digest, Sha256};

const ENROLLED_TOKEN_RENEWAL_NOTICE: &str = "The enrolled Claude token was refused. This run is using your native Claude login. Renew the enrolled token in Provider Accounts.";
const ENROLLED_TOKEN_RECOVERY: &str = "The enrolled Claude token is expired or refused, and no working native Claude login could be verified. Open Provider Accounts to renew the token or sign in, then retry.";

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
        if crate::provider::canonical_provider_family(&request.provider) != Some("claude") {
            return Ok(credentials);
        }
        let owner = crate::account_profile::provider_account_authority_for_launch(
            &self.owned.config_projection.snapshot(),
            &self.owned.provider_account_profiles,
            request,
        )?;
        let profile =
            self.owned
                .provider_account_profiles
                .get(&owner, "claude", &request.account_profile)?;
        let credential_id =
            crate::provider::provider_account_credential_id(&owner, "claude", &profile.profile_id);
        let registered = crate::provider::provider_account_credential_registered(
            &owner,
            "claude",
            &profile.profile_id,
        )?;
        let Some((_, token)) = credentials
            .iter()
            .find(|(key, _)| *key == crate::provider::CLAUDE_OAUTH_TOKEN_ENV)
        else {
            if self.owned.claude_enrollment_is_rejected(request)? {
                return self
                    .native_claude_fallback(request, &owner, &profile.profile_id)
                    .await;
            }
            return Ok(credentials);
        };
        // Transferred credentials are generation-specific: renewal on home must
        // not reuse a worker's rejection of an older per-launch token.
        let check_key = if registered {
            credential_id.clone()
        } else {
            format!("{credential_id}:{:x}", Sha256::digest(token.as_bytes()))
        };
        let checked = if registered && profile.auth_state == ProviderAccountAuthState::Expired {
            Err(ClaudeCredentialCheckError::Rejected)
        } else {
            let checked = self
                .owned
                .claude_token_checks
                .check(&check_key, || async {
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
                if !registered {
                    return Err(DaemonError::LocalTransport {
                        operation: "verify Claude credential",
                        message: "Claude account changed during verification. Retry the launch with the current account.".into(),
                    });
                }
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
            // the ordinary provider run surface any later authentication refusal.
            Err(ClaudeCredentialCheckError::Inconclusive(_)) => Ok(credentials),
            Err(ClaudeCredentialCheckError::Rejected) => {
                drop(credentials);
                self.native_claude_fallback(request, &owner, &profile.profile_id)
                    .await
            }
        }
    }
    async fn native_claude_fallback(
        &self,
        request: &crate::provider::LaunchProviderRequest,
        owner: &str,
        profile: &str,
    ) -> Result<ProviderCredentialEnvironment, DaemonError> {
        let _ = self.owned.provider_account_profiles.update_observation(
            owner,
            "claude",
            profile,
            ProviderAccountAuthState::Expired,
            None,
            None,
            None,
            None,
        );
        // A home-native login is not a portable worker credential. Keep the
        // existing leased-launch authority contract instead of silently using
        // a different worker account or claiming a fallback that cannot run.
        if request
            .agent_id
            .as_deref()
            .and_then(|id| self.owned.agent_store.get_agent(id).ok())
            .is_some_and(|agent| agent.remote_execution().is_some())
        {
            return Err(DaemonError::InvalidConfig {
                field: "provider account credential",
                message: "The enrolled Claude token was refused. Remote Claude execution needs a renewed portable credential; open Provider Accounts on the home kernel to renew it.",
            });
        }
        let native =
            crate::runtime::claude_setup_token_login::check_native(self, owner, profile).await;
        if native.is_ok() {
            self.owned.record_notice_for_agent(
                &request.session_id,
                None,
                request.agent_id.as_deref(),
                Vec::new(),
                ENROLLED_TOKEN_RENEWAL_NOTICE,
            );
            return Ok(ProviderCredentialEnvironment::default());
        }
        Err(DaemonError::LocalTransport {
            operation: "Claude authorization",
            message: ENROLLED_TOKEN_RECOVERY.into(),
        })
    }

    async fn resolve_signed_in_claude_credentials(
        &self,
        request: &crate::provider::LaunchProviderRequest,
        owner: &str,
        profile: &str,
    ) -> Result<ProviderCredentialEnvironment, DaemonError> {
        // Replacement credentials use their own ordinary scoped Vault unlock.
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
