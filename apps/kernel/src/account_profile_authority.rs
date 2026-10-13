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

/// MP-08 / MP-10 / MP-11: kernel-local resolution provenance, never a client grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedProviderAccount {
    runtime_owner: String,
    owner: String,
    provider: String,
    profile_id: String,
    selection: String,
}

impl ResolvedProviderAccount {
    pub(crate) fn selection(&self) -> &str {
        &self.selection
    }
}

pub(crate) fn provider_account_selection_for_run(
    run: &crate::provider::RuntimeProviderRun,
) -> &str {
    run.resolved_provider_account
        .as_ref()
        .map(|account| account.selection())
        .unwrap_or_else(|| run.account_profile())
}

pub(crate) fn bind_provider_account_authority(
    request: &mut crate::provider::LaunchProviderRequest,
    owner: String,
    selection: String,
) {
    request.resolved_provider_account = Some(ResolvedProviderAccount {
        runtime_owner: request.owner_user_id.clone(),
        owner,
        provider: request.provider.clone(),
        profile_id: request.account_profile.clone(),
        selection,
    });
}

pub(crate) fn provider_account_authority_for_launch(
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    request: &crate::provider::LaunchProviderRequest,
) -> Result<String, DaemonError> {
    resolved_authority(
        config,
        profiles,
        &request.owner_user_id,
        &request.provider,
        &request.account_profile,
        request.resolved_provider_account.as_ref(),
        &request.provider_account_env,
    )
}

pub(crate) fn provider_account_authority_for_run(
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    run: &crate::provider::RuntimeProviderRun,
) -> Result<String, DaemonError> {
    resolved_authority(
        config,
        profiles,
        run.owner_user_id(),
        run.provider(),
        run.account_profile(),
        run.resolved_provider_account.as_ref(),
        &crate::provider::provider_account_environment_on_kernel(run),
    )
}

fn resolved_authority(
    config: &DaemonConfig,
    profiles: &ProviderAccountProfileRegistry,
    runtime_owner: &str,
    provider: &str,
    profile_id: &str,
    resolved: Option<&ResolvedProviderAccount>,
    environment: &std::collections::BTreeMap<String, String>,
) -> Result<String, DaemonError> {
    let authority = provider_account_authority_owner_user_id(config, runtime_owner);
    let invalid = || DaemonError::InvalidConfig {
        field: "provider account authority",
        message: "resolved account no longer matches its runtime owner, provider or profile",
    };
    if let Some(resolved) = resolved {
        if resolved.runtime_owner != runtime_owner
            || resolved.provider != provider
            || resolved.profile_id != profile_id
            || (resolved.owner != authority && resolved.owner != runtime_owner)
            || profiles
                .find(&resolved.owner, provider, profile_id)?
                .is_none_or(|profile| profile.profile_id != profile_id)
        {
            return Err(invalid());
        }
        return Ok(resolved.owner.clone());
    }
    // Restored runs retain account-root paths in their existing PTY environment.
    // Recover the selected namespace from those roots rather than guessing from
    // an exact id which may exist in both the home and replica namespaces.
    let keys = provider_account_root_keys(provider);
    if keys.iter().any(|key| environment.contains_key(*key)) {
        let mut matches = Vec::new();
        for owner in [&authority, runtime_owner] {
            if matches.iter().any(|matched| matched == owner) {
                continue;
            }
            if profiles
                .find(owner, provider, profile_id)?
                .is_some_and(|profile| profile.profile_id == profile_id)
            {
                let roots = profiles.resolve_environment(owner, provider, profile_id)?;
                if !roots.is_empty()
                    && roots.iter().all(|(key, value)| {
                        environment.get(key).is_some_and(|selected| {
                            std::path::Path::new(selected) == std::path::Path::new(value)
                        })
                    })
                {
                    matches.push(owner.to_string());
                }
            }
        }
        return if matches.len() == 1 {
            Ok(matches.remove(0))
        } else {
            Err(invalid())
        };
    }
    provider_account_authority_owner_for_profile(
        config,
        profiles,
        runtime_owner,
        provider,
        profile_id,
    )
}

pub(crate) fn copy_provider_account_selection(
    run: &crate::provider::RuntimeProviderRun,
    request: &mut crate::provider::LaunchProviderRequest,
) {
    request.resolved_provider_account = run.resolved_provider_account.clone();
    request.provider_account_env = crate::provider::provider_account_environment_on_kernel(run);
}

fn provider_account_root_keys(provider: &str) -> &'static [&'static str] {
    match crate::provider::canonical_provider_family(provider) {
        Some("codex") => &["CODEX_HOME"],
        Some("claude") => &["CLAUDE_CONFIG_DIR"],
        Some("opencode") => &[
            "OPENCODE_CONFIG_DIR",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
        ],
        _ => &[],
    }
}
