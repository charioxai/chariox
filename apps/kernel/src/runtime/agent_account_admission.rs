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
    let owner = crate::account_profile::provider_account_authority_owner_user_id(
        config,
        &request.owner_user_id,
    );
    let available = profiles.list(&owner, Some(family))?;
    let selected = profiles
        .find(&owner, family, reference)?
        .or_else(|| available.iter().find(|p| p.label == reference).cloned());
    if let Some(selected) = selected {
        request.account_profile = Some(selected.profile_id);
        return Ok(());
    }
    // A not-yet-configured default remains a normal login/selection path.
    if reference == "default" {
        return Ok(());
    }
    let choices = available
        .iter()
        .map(|p| format!("{} ({})", p.label, p.profile_id))
        .collect::<Vec<_>>()
        .join(", ");
    Err(DaemonError::LocalTransport {
        operation: "agent.spawn",
        message: format!(
            "account profile `{reference}` is not registered for {family}; available profiles: {}",
            if choices.is_empty() { "none" } else { &choices }
        ),
    })
}
