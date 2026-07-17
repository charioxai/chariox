use super::{
    calculate_agent_layout, AgentInstance, AgentSubstituteProfile, GridPosition, RemoteAgentBinding,
};

#[test]
fn calculate_agent_layout_expands_past_six_agents() {
    assert_eq!(
        calculate_agent_layout(7),
        vec![
            GridPosition::new(0, 0, 1, 1),
            GridPosition::new(0, 1, 1, 1),
            GridPosition::new(0, 2, 1, 1),
            GridPosition::new(0, 3, 1, 1),
            GridPosition::new(1, 0, 1, 1),
            GridPosition::new(1, 1, 1, 1),
            GridPosition::new(1, 2, 1, 1),
        ]
    );
}

#[test]
fn substitute_deactivation_restores_primary_without_default_model() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "opencode",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.set_primary_profile("opencode", None, None);
    agent.add_substitute(AgentSubstituteProfile::new(
        "codex",
        "gpt-5.4",
        Some("medium".to_string()),
    ));

    let activated = agent.activate_substitute(0, "manual");
    assert_eq!(
        activated.as_ref().map(|profile| profile.provider.as_str()),
        Some("codex")
    );
    assert_eq!(agent.provider(), "codex");
    assert_eq!(agent.model(), Some("gpt-5.4"));
    assert_eq!(agent.effort(), Some("medium"));

    agent.deactivate_substitute();
    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), None);
    assert_eq!(agent.effort(), None);
}

#[test]
fn home_remote_agent_does_not_execute_worker_source_grants() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "codex",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.grant_extension(
        crate::extension::ExtensionGrant::script("worker-script", "worker-python")
            .from_source(crate::extension::ExtensionSource::Worker),
    );
    assert_eq!(agent.execution_script_grants().len(), 1);

    agent.set_remote_execution(Some(RemoteAgentBinding {
        worker_kernel_id: "worker-1".to_string(),
        worker_machine_id: "machine-1".to_string(),
        execution_lease_id: "lease-1".to_string(),
        leased_agent_id: "leased-1".to_string(),
        active_worker_provider_run_id: None,
        relay_url: None,
        relay_token: None,
    }));

    assert!(agent.execution_script_grants().is_empty());
    assert_eq!(agent.worker_script_grants().len(), 1);
}

#[test]
fn publication_materialization_drops_worker_authority_without_reinterpreting_it_locally() {
    let mut agent = AgentInstance::new(
        "agent-1",
        "agent-1",
        "session-1",
        None,
        "codex",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    );
    agent.grant_extension(crate::extension::ExtensionGrant::script(
        "home-script",
        "home-python",
    ));
    agent.grant_extension(
        crate::extension::ExtensionGrant::script("worker-script", "worker-python")
            .from_source(crate::extension::ExtensionSource::Worker),
    );
    agent.set_worker_extension_grant_sync(Some(
        crate::extension::RemoteExtensionManifestSyncStatus::synced("worker-hash".to_string()),
    ));
    agent.set_remote_execution(Some(RemoteAgentBinding {
        worker_kernel_id: "worker-1".to_string(),
        worker_machine_id: "machine-1".to_string(),
        execution_lease_id: "lease-1".to_string(),
        leased_agent_id: "leased-1".to_string(),
        active_worker_provider_run_id: None,
        relay_url: None,
        relay_token: None,
    }));

    let materialized =
        agent.materialized_for_publication_runtime("agent-2", "agent-2", "published-session");

    assert!(materialized.remote_execution().is_none());
    assert!(materialized.worker_extension_grant_sync().is_none());
    assert!(materialized.worker_script_grants().is_empty());
    assert_eq!(
        materialized
            .execution_script_grants()
            .iter()
            .map(|grant| grant.name.as_str())
            .collect::<Vec<_>>(),
        vec!["home-script"]
    );
}
