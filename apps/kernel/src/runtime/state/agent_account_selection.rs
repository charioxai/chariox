//! Stable provider account selection shared by session and agent admission.
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn resolve_agent_account_profile(
        &self,
        owner_user_id: &str,
        provider: &str,
        account_profile: Option<&str>,
    ) -> Result<Option<String>, DaemonError> {
        if crate::provider::canonical_provider_family(provider).is_none() {
            return Ok(account_profile.map(str::to_string));
        }
        let owner = crate::account_profile::provider_account_authority_owner_user_id(
            &self.config_projection.snapshot(),
            owner_user_id,
        );
        if matches!(account_profile, None | Some("default"))
            && self
                .provider_account_profiles
                .list(&owner, Some(provider))?
                .is_empty()
        {
            // An unenrolled user can still create a session to add an account.
            return Ok(None);
        }
        self.provider_account_profiles
            .get(&owner, provider, account_profile.unwrap_or("default"))
            .map(|profile| Some(profile.profile_id))
    }

    pub(super) fn resolve_agent_account_request(
        &self,
        mut request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::CreateAgentRequest, DaemonError> {
        request.account_profile = self.resolve_agent_account_profile(
            &request.owner_user_id,
            &request.provider,
            request.account_profile.as_deref(),
        )?;
        Ok(request)
    }
}
