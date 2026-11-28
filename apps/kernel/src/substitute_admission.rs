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
    config: &crate::config::DaemonConfig,
    runtime_owner: &str,
    candidate: &AgentSubstituteProfile,
) -> Result<Option<String>, DaemonError> {
    if candidate
        .kernel_id
        .as_deref()
        .is_some_and(|kernel_id| kernel_id != config.daemon_id)
    {
        return Ok(Some("it runs on another kernel".to_string()));
    }
    let Some(provider) = crate::provider::canonical_provider_family(&candidate.provider) else {
        return Ok(None);
    };
    let profile_id = candidate.account_profile.as_deref().unwrap_or("default");
    let owner = crate::account_profile::provider_account_authority_owner_for_profile(
        config,
        registry,
        runtime_owner,
        provider,
        profile_id,
    )?;
    if registry.find(&owner, provider, profile_id)?.is_none() {
        return Ok(Some(format!(
            "its saved {} account is no longer available",
            candidate.provider
        )));
    }
    match registry.require_account_for_model(
        &owner,
        provider,
        profile_id,
        Some(&candidate.model),
        "substitute admission",
    ) {
        Ok(()) => Ok(None),
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
    config: &crate::config::DaemonConfig,
    agent: &AgentInstance,
    tried: &[AgentSubstituteProfile],
) -> Result<SubstituteChoice, DaemonError> {
    let mut skipped = Vec::new();
    for (index, candidate) in agent.substitutes().iter().enumerate() {
        if tried.contains(candidate) {
            continue;
        }
        match unusable_reason(registry, config, agent.owner_user_id(), candidate)? {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_admission_keeps_each_profiles_owner_and_home_alias() {
        crate::test_support::isolated_env_test!();
        let root = crate::test_support::TestWorktree::new("substitute-profile-authority");
        let registry =
            ProviderAccountProfileRegistry::open(root.path().join("profiles.json")).unwrap();
        let mut config = crate::config::DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            user_id: "cloud-owner".into(),
            ..Default::default()
        });
        let home = registry.create_managed("local", "codex", "Home").unwrap();
        registry
            .set_default("local", "codex", &home.profile_id)
            .unwrap();
        let replica = registry
            .create_managed("cloud-owner", "codex", "Replica")
            .unwrap();
        registry
            .set_default("cloud-owner", "codex", &replica.profile_id)
            .unwrap();
        crate::test_support::authenticate_provider_account(
            &registry,
            "cloud-owner",
            "codex",
            &replica.profile_id,
        )
        .unwrap();
        let explicit = AgentSubstituteProfile::new("codex", "model", None)
            .with_account_profile(Some(replica.profile_id.clone()));
        assert_eq!(
            unusable_reason(&registry, &config, "cloud-owner", &explicit).unwrap(),
            None,
            "an exact replica profile retains its own namespace"
        );
        assert!(
            unusable_reason(&registry, &config, "another-owner", &explicit)
                .unwrap()
                .is_some(),
            "another owner's exact profile is never a substitute"
        );
        let alias = AgentSubstituteProfile::new("codex", "model", None);
        assert!(
            unusable_reason(&registry, &config, "cloud-owner", &alias)
                .unwrap()
                .is_some(),
            "the authenticated replica's default alias cannot shadow the unavailable home default"
        );
        crate::test_support::authenticate_provider_account(
            &registry,
            "local",
            "codex",
            &home.profile_id,
        )
        .unwrap();
        assert_eq!(
            unusable_reason(&registry, &config, "cloud-owner", &alias).unwrap(),
            None
        );
        let remote = AgentSubstituteProfile::new("dev-stub", "model", None)
            .with_kernel_id(Some("another-kernel".into()));
        assert!(unusable_reason(&registry, &config, "cloud-owner", &remote)
            .unwrap()
            .is_some());
    }
}

// Workflow waiting retains the reset behavior, with the same per-profile authority as admission.
pub(crate) fn earliest_reset_ms(
    registry: &ProviderAccountProfileRegistry,
    config: &crate::config::DaemonConfig,
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
            let owner = crate::account_profile::provider_account_authority_owner_for_profile(
                config,
                registry,
                owner_user_id,
                provider,
                &profile_id,
            )
            .ok()?;
            registry
                .find(&owner, provider, &profile_id)
                .ok()
                .flatten()?
                .blocked_until(&model, now_ms)
                .flatten()
        })
        .min()
}

pub(crate) fn reset_label(at_ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at_ms as i64)
        .map(|time| time.format("%b %-d, %H:%M UTC").to_string())
        .unwrap_or_default()
}
