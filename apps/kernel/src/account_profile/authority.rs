//! MP-01/MP-08/MP-11: host aliases and admitted lease namespaces share one policy.
/// Provider accounts belong to the person operating the home kernel. Local
/// clients identify that person as `local`, while the owner's Cloud clients
/// use the configured Cloud user id. Only that configured id aliases the host
/// owner; collaborators retain independent account namespaces.
pub(crate) fn provider_account_authority_owner_user_id(
    config: &crate::config::DaemonConfig,
    runtime_owner_user_id: &str,
) -> String {
    if config
        .cloud_relay
        .as_ref()
        .is_some_and(|profile| profile.user_id == runtime_owner_user_id)
    {
        crate::session::DEFAULT_LOCAL_USER_ID.to_string()
    } else {
        runtime_owner_user_id.to_string()
    }
}

/// Only authenticated lease admission or its stored backing-agent/run binding
/// can preserve an execution owner's namespace. This is not client input.
pub(crate) fn provider_account_execution_owner_user_id(
    config: &crate::config::DaemonConfig,
    runtime_owner_user_id: &str,
    lease_owner_authorized: bool,
) -> String {
    if lease_owner_authorized {
        runtime_owner_user_id.to_string()
    } else {
        provider_account_authority_owner_user_id(config, runtime_owner_user_id)
    }
}

/// A receiving worker uses the admitted run's delegated credential, not a local login.
/// The caller supplies a run only after confirming its lease provenance.
pub(crate) fn require_agent_account_authenticated(
    registry: &super::ProviderAccountProfileRegistry,
    owner: &str,
    agent: &crate::agent::AgentInstance,
    delegated_run: Option<&crate::provider::RuntimeProviderRun>,
    operation: &'static str,
) -> Result<(), crate::error::DaemonError> {
    let Some(provider) = crate::provider::canonical_provider_family(agent.provider()) else {
        return Ok(());
    };
    if delegated_run.is_some_and(|run| run.authenticates_leased_claude_agent(agent)) {
        registry.get(owner, provider, agent.provider_account_profile())?;
    } else {
        registry.require_authenticated(
            owner,
            provider,
            agent.provider_account_profile(),
            agent.model(),
            operation,
        )?;
    }
    Ok(())
}
