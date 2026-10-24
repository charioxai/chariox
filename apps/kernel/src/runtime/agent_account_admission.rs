//! MP-08 / MP-10 / MP-11: pin owner-scoped account references before agent creation.
use crate::account_profile::ProviderAccountProfileRegistry;
use crate::agent::CreateAgentRequest;
use crate::{DaemonConfig, DaemonError};

pub(crate) fn resolve_spawn_account(
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    request: &mut CreateAgentRequest,
) -> Result<(), DaemonError> {
    let Some(family) = crate::provider::canonical_provider_family(&request.provider) else {
        return Ok(());
    };
    let Some(reference) = request.account_profile.as_deref().map(str::trim) else {
        return Ok(());
    };
    if reference.is_empty() {
        return Ok(());
    }
    let owner = crate::account_profile::provider_account_authority_owner_for_profile(
        config,
        profiles,
        &request.owner_user_id,
        family,
        reference,
    )?;
    if let Some(selected) = profiles.find(&owner, family, reference)? {
        // Defaults are home aliases. A stable ID may instead name a worker
        // replica, so retain the home reference when pinning would change its
        // authority. Launch records the resolved ID and namespace together.
        let pinned_owner = crate::account_profile::provider_account_authority_owner_for_profile(
            config,
            profiles,
            &request.owner_user_id,
            family,
            &selected.profile_id,
        )?;
        request.account_profile = Some(if reference == "default" || pinned_owner != owner {
            reference.to_string()
        } else {
            selected.profile_id
        });
        return Ok(());
    }
    // A not-yet-configured default remains a normal login/selection path.
    if reference == "default" {
        return Ok(());
    }
    Err(DaemonError::ProviderProfileRefused {
        reason: crate::error::ProviderProfileRefusal::Unavailable,
    })
}
