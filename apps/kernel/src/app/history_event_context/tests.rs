// MP-08 / MP-10 / MP-11: local history cannot inherit another agent's provenance.
use super::*;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

#[test]
fn history_context_requires_the_exact_local_session_agent_and_turn() {
    let mut app =
        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new("home", "home"))
        .unwrap();
    let (other_session, other_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new("other", "other"))
        .unwrap();
    let run = RuntimeProviderRun::new(
        "provider-run-1",
        &LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
            .with_agent_id(agent.id()),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "history-context-test".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    app.providers_mut().insert_run_for_test(run.clone());
    let resolver = HistoryEventContextResolver::new(
        app.providers.clone(),
        app.sessions.clone(),
        app.prompt_state_owner(),
        app.active_turn_store(),
        app.agents.clone(),
        app.provider_run_projection.clone(),
    );
    let entry = |session_id: &str, agent_id: &str| {
        SessionHistoryEntry::provider_output(
            session_id,
            "provider-run-1",
            Some(agent_id),
            crate::terminal::TerminalOutputKind::ProviderTool,
            None,
            "{}".into(),
        )
    };
    assert_eq!(
        resolver
            .resolve(&entry(session.id(), agent.id()))
            .provider
            .as_deref(),
        Some("opencode")
    );
    for entry in [
        entry(other_session.id(), agent.id()),
        entry(session.id(), other_agent.id()),
        entry(session.id(), "missing-agent"),
    ] {
        assert_eq!(resolver.resolve(&entry).provider, None);
    }
    let foreign_turn = ActiveTurnState::new(
        other_session.id().into(),
        other_agent.id().into(),
        "foreign-prompt".into(),
        "provider-run-1".into(),
    );
    let context = resolver.resolve_with_overrides(
        &entry(session.id(), agent.id()),
        Default::default(),
        Some(&foreign_turn),
    );
    assert_ne!(context.prompt_id.as_deref(), Some("foreign-prompt"));
    assert_ne!(context.turn_id.as_deref(), Some("foreign-prompt"));
    for active_run in [None, Some("stale-worker-run"), Some("provider-run-1")] {
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker".into(),
                    worker_machine_id: "worker-machine".into(),
                    execution_lease_id: "execution-lease".into(),
                    leased_agent_id: "leased-agent".into(),
                    active_worker_provider_run_id: active_run.map(str::to_string),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: None,
                },
            )
            .unwrap();
        assert_eq!(
            resolver.resolve(&entry(session.id(), agent.id())).provider,
            None,
            "unresolved worker provenance must not fall back to the colliding home provider"
        );
    }
    let projected =
        crate::provider::projected_leased_provider_run_id("leased-agent", "provider-run-1");
    app.update_remote_provider_run_projection(run.projected_for_home_agent_with_id(
        projected,
        session.id().into(),
        other_agent.id().into(),
    ));
    assert_eq!(
        resolver.resolve(&entry(session.id(), agent.id())).provider,
        None,
        "the worker projection must match the home agent, not just the run ID"
    );
}
