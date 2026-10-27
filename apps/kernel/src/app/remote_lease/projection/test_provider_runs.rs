// MP-08 / MP-10 / MP-11: home projections follow admitted worker run replacement.
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, ProviderRunState,
    RuntimeProviderRun,
};
use crate::runtime::projection::ProviderRunProjectionStore;

fn running(id: &str, agent_id: &str) -> RuntimeProviderRun {
    let request = LaunchProviderRequest::new(
        "session-1",
        "managed-dev-stub",
        "managed-dev-stub",
        "default",
        "default",
    )
    .with_agent_id(agent_id);
    let mut run = RuntimeProviderRun::new(
        id,
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "projection-test".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    run
}

#[test]
fn admitted_worker_replacement_retires_only_same_lease_agent_projection() {
    let store = ProviderRunProjectionStore::default();
    for (id, agent) in [
        ("leased:lease-1:old", "agent-1"),
        ("leased:lease-2:other-lease", "agent-1"),
        ("leased:lease-10:prefix-neighbor", "agent-1"),
        ("leased:lease-1:other-agent", "agent-2"),
        ("local-run", "agent-1"),
    ] {
        store.update_remote_snapshot(running(id, agent));
    }
    store.update_remote_snapshot(
        running("leased:lease-1:other-session", "agent-1").projected_for_home_agent_with_id(
            "leased:lease-1:other-session".to_string(),
            "session-2".to_string(),
            "agent-1".to_string(),
        ),
    );
    store.update_remote_snapshot(running("leased:lease-1:new", "agent-1"));

    assert_eq!(
        store.get("leased:lease-1:old").unwrap().state(),
        ProviderRunState::Ended
    );
    for id in [
        "leased:lease-1:new",
        "leased:lease-2:other-lease",
        "leased:lease-10:prefix-neighbor",
        "leased:lease-1:other-agent",
        "leased:lease-1:other-session",
        "local-run",
    ] {
        assert_eq!(store.get(id).unwrap().state(), ProviderRunState::Running);
    }
}

#[test]
fn ended_remote_projection_cannot_be_revived_by_delayed_running_snapshot() {
    let store = ProviderRunProjectionStore::default();
    let old = running("leased:lease-1:old", "agent-1");
    let mut ended = old.clone();
    ended.mark_ended();
    store.update_remote_snapshot(ended.clone());
    store.update_remote_snapshot(running("leased:lease-1:new", "agent-1"));
    assert_eq!(
        store.update_remote_snapshot(old).state(),
        ProviderRunState::Ended
    );
    store.update_remote_snapshot(ended);
    assert_eq!(
        store.get("leased:lease-1:new").unwrap().state(),
        ProviderRunState::Running
    );
}
