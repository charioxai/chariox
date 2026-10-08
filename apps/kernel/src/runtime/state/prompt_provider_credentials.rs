//! MP-08 / MP-10 / MP-11: ordinary cold/refresh launches use the shared account check.
use super::runtime_vault_unlock_state::VaultUnlockGuard;
use super::*;
use crate::account_profile::ProviderAccountAuthState;

pub(super) struct PromptProviderCredentialGuard {
    _unlock: Option<VaultUnlockGuard>,
    selected: Option<crate::provider::LaunchProviderRequest>,
    revision: Option<u64>,
}

impl PromptProviderCredentialGuard {
    // Human unlock or sign-in can outlive a provider/account change. Recheck
    // the selected identity under the app lock before the ordinary launcher.
    pub(super) fn validate(&self, app: &DaemonApp) -> Result<(), DaemonError> {
        let Some(selected) = &self.selected else {
            return Ok(());
        };
        let agent = app
            .agent_launch_profile(
                app.agents().get_agent(
                    selected
                        .agent_id
                        .as_deref()
                        .expect("prompt guard has an agent"),
                )?,
            )
            .0;
        let owner = crate::account_profile::provider_account_authority_owner_user_id(
            app.config(),
            agent.owner_user_id(),
        );
        let profile = app.provider_account_profile_registry().get(
            &owner,
            agent.provider(),
            agent.provider_account_profile(),
        )?;
        if agent.session_id() != selected.session_id
            || agent.owner_user_id() != selected.owner_user_id
            || crate::provider::canonical_provider_family(agent.provider()) != Some("claude")
            || profile.profile_id != selected.account_profile
            || crate::provider::provider_account_credential_verification(
                &owner,
                "claude",
                &profile.profile_id,
            )?
            .revision
                != self.revision
        {
            return Err(DaemonError::LocalTransport {
                operation: "prepare prompt provider credentials",
                message: "Selected provider account changed while preparing this prompt; submit it again.".into(),
            });
        }
        Ok(())
    }
}

impl KernelRuntimeState {
    pub(super) async fn prepare_prompt_provider_credentials(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<PromptProviderCredentialGuard, DaemonError> {
        let request = self
            .with_authorized_app_side_effect(|app| {
                let agent = app
                    .agent_launch_profile(app.agents().get_agent(agent_id)?)
                    .0;
                // Remote agents resolve their selected account on the worker through
                // the existing leased launch path; this guard covers local execution.
                if agent.remote_execution().is_some()
                    || crate::provider::canonical_provider_family(agent.provider())
                        != Some("claude")
                {
                    return Ok(None);
                }
                let owner = crate::account_profile::provider_account_authority_owner_user_id(
                    app.config(),
                    agent.owner_user_id(),
                );
                let profile = app.provider_account_profile_registry().get(
                    &owner,
                    "claude",
                    agent.provider_account_profile(),
                )?;
                let verification = crate::provider::provider_account_credential_verification(
                    &owner,
                    "claude",
                    &profile.profile_id,
                )?;
                if !verification.registered {
                    return Ok(None);
                }
                Ok(Some((
                    crate::provider::LaunchProviderRequest::new(
                        session_id,
                        "claude",
                        agent.provider(),
                        &profile.profile_id,
                        agent.model().unwrap_or("default"),
                    )
                    .with_agent_id(agent_id.to_string())
                    .with_owner_user_id(agent.owner_user_id().to_string()),
                    verification,
                    profile.auth_state,
                )))
            })
            .await?;
        let Some((request, verification, auth_state)) = request else {
            return Ok(PromptProviderCredentialGuard {
                _unlock: None,
                selected: None,
                revision: None,
            });
        };
        // Verified accounts resolve their credential once at launch. Always
        // policy must not ask twice just to revisit a durable verification.
        if verification.verified && auth_state == ProviderAccountAuthState::Authenticated {
            let unlock = self
                .ensure_provider_account_vault_unlocked_for_launch(
                    &request,
                    "launch prompt provider account",
                )
                .await?;
            return Ok(PromptProviderCredentialGuard {
                _unlock: Some(unlock),
                selected: Some(request),
                revision: verification.revision,
            });
        }
        let unlock = self
            .ensure_provider_account_vault_unlocked_for_launch(
                &request,
                "check prompt provider account",
            )
            .await?;
        self.authorize_current_external_command()?;
        let owner = self.provider_account_authority_owner_user_id(&request.owner_user_id);
        let credentials = crate::provider::resolve_provider_account_credentials(
            &self.owned.config_projection.snapshot(),
            &owner,
            "claude",
            &request.account_profile,
        )?;
        // An operation lease never spans human OAuth consent. A replacement
        // resolves through checked_claude_credentials' normal shared sign-in.
        drop(unlock);
        let _ = self
            .checked_claude_credentials(&request, credentials)
            .await?;
        self.authorize_current_external_command()?;
        let revision = crate::provider::provider_account_credential_verification(
            &owner,
            "claude",
            &request.account_profile,
        )?
        .revision;
        let unlock = self
            .ensure_provider_account_vault_unlocked_for_launch(
                &request,
                "launch prompt provider account",
            )
            .await?;
        Ok(PromptProviderCredentialGuard {
            _unlock: Some(unlock),
            selected: Some(request),
            revision,
        })
    }
}
