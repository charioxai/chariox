//! MP-08 / MP-10 / MP-11: selected profile identity and account authority.
use super::{provider_account_authority_owner_user_id, ProviderAccountProfileRegistry};
use crate::{config::DaemonConfig, error::DaemonError};

/// Worker replicas retain their exact runtime owner's namespace. Home account
/// aliases still resolve locally; an alias in the replica namespace cannot
/// shadow a home account. Never search another runtime owner's profiles.
pub(crate) fn provider_account_authority_owner_for_profile(
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    runtime_owner: &str,
    provider: &str,
    account_profile: &str,
) -> Result<String, DaemonError> {
    let authority = provider_account_authority_owner_user_id(config, runtime_owner);
    if authority != runtime_owner
        && profiles
            .find(runtime_owner, provider, account_profile)?
            .is_some_and(|profile| profile.profile_id == account_profile.trim())
    {
        return Ok(runtime_owner.to_string());
    }
    Ok(authority)
}
