//! Which profile an agent's turn is admitted on. The configured profile runs the
//! turn while its account can; when the account is refused for a reason a
//! substitute can answer (its usage is used up, or it is not signed in), the
//! first configured substitute whose own account can run takes the turn. The
//! refusal is the answer only when none can.

use crate::account_profile::ProviderAccountProfileRegistry;
use crate::agent::{AgentInstance, AgentSubstituteProfile};
use crate::error::{DaemonError, ProviderProfileRefusal};

/// A refusal a substitute can answer. A missing account is a choice the person
/// made, so it stays a refusal.
pub(crate) fn substitute_can_answer(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::ProviderProfileRefused {
            reason: ProviderProfileRefusal::UsageExhausted
                | ProviderProfileRefusal::Unverified
                | ProviderProfileRefusal::Unauthenticated,
            ..
        }
    )
}

/// Why `candidate` cannot run a turn now, or `None` when it can.
pub(crate) fn unusable_reason(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    daemon_id: &str,
    candidate: &AgentSubstituteProfile,
) -> Result<Option<String>, DaemonError> {
    if candidate
        .kernel_id
        .as_deref()
        .is_some_and(|kernel_id| kernel_id != daemon_id)
    {
        return Ok(Some("it runs on another kernel".to_string()));
    }
    let Some(provider) = crate::provider::canonical_provider_family(&candidate.provider) else {
        return Ok(None);
    };
    let profile_id = candidate.account_profile.as_deref().unwrap_or("default");
    if registry
        .find(owner_user_id, provider, profile_id)?
        .is_none()
    {
        return Ok(Some(format!(
            "its saved {} account is no longer available",
            candidate.provider
        )));
    }
    match registry.require_authenticated(
        owner_user_id,
        provider,
        profile_id,
        Some(&candidate.model),
        "substitute admission",
    ) {
        Ok(_) => Ok(None),
        Err(DaemonError::ProviderProfileRefused {
            reason,
            account_name,
            ..
        }) => Ok(Some(format!(
            "account `{account_name}` {}",
            reason.local_message()
        ))),
        Err(error) => Err(error),
    }
}

/// Which substitute runs a turn, and why each earlier untried one was skipped.
pub(crate) struct SubstituteChoice {
    pub(crate) index: Option<usize>,
    /// Each skipped substitute's position and the reason it cannot run.
    pub(crate) skipped: Vec<(usize, String)>,
}

/// The first substitute of `agent` not in `tried` that can run, in the order the
/// person configured them, with the reason each skipped one cannot.
pub(crate) fn first_usable_substitute(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    daemon_id: &str,
    agent: &AgentInstance,
    tried: &[AgentSubstituteProfile],
) -> Result<SubstituteChoice, DaemonError> {
    let mut skipped = Vec::new();
    for (index, candidate) in agent.substitutes().iter().enumerate() {
        if tried.contains(candidate) {
            continue;
        }
        match unusable_reason(registry, owner_user_id, daemon_id, candidate)? {
            None => {
                return Ok(SubstituteChoice {
                    index: Some(index),
                    skipped,
                })
            }
            Some(reason) => skipped.push((index, reason)),
        }
    }
    Ok(SubstituteChoice {
        index: None,
        skipped,
    })
}

/// When a turn of `agent` can next run: the earliest instant one of its
/// profiles' usage is back. `None` when no profile names a reset.
pub(crate) fn earliest_reset_ms(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    agent: &AgentInstance,
    now_ms: u64,
) -> Option<u64> {
    let primary = (
        agent.provider().to_string(),
        agent.provider_account_profile().to_string(),
        agent.model().unwrap_or("default").to_string(),
    );
    let substitutes = agent.substitutes().iter().map(|candidate| {
        (
            candidate.provider.clone(),
            candidate
                .account_profile
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            candidate.model.clone(),
        )
    });
    std::iter::once(primary)
        .chain(substitutes)
        .filter_map(|(provider, profile_id, model)| {
            let provider = crate::provider::canonical_provider_family(&provider)?;
            registry
                .find(owner_user_id, provider, &profile_id)
                .ok()
                .flatten()?
                .blocked_until(&model, now_ms)
                .flatten()
        })
        .min()
}

/// "Nov 22, 16:00 UTC": when an account's usage is back, as the Room reads it.
pub(crate) fn reset_label(at_ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at_ms as i64)
        .map(|time| time.format("%b %-d, %H:%M UTC").to_string())
        .unwrap_or_default()
}
