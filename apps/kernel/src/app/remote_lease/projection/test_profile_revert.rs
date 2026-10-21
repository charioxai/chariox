// MP-08 / MP-10 / MP-11: a lagging projection of the previous provider's run
// must not revert a committed provider switch.
use crate::app::DaemonApp;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult,
    RuntimeProviderRun,
};

const WORKER: &str = "profile-revert-worker";

#[test]
fn a_lagging_previous_provider_run_keeps_the_switched_profile() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "revert-workspace",
            "revert-worktree",
        ))
        .unwrap();
    app.agents
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: WORKER.into(),
                worker_machine_id: "revert-machine".into(),
                execution_lease_id: "revert-lease".into(),
                leased_agent_id: "revert-leased-agent".into(),
                // A late dispatch receipt re-bound the previous Claude run.
                active_worker_provider_run_id: Some("previous-claude-run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    // The switch to Codex is committed on the home.
    app.agents
        .set_agent_runtime_profile_with_account_profile(
            agent.id(),
            "codex",
            Some("gpt-6-luna".into()),
            Some("low".into()),
            Some("codex-account".into()),
            Default::default(),
        )
        .unwrap();
    // The worker's last event for the previous Claude run arrives afterwards.
    let request = LaunchProviderRequest::new(
        "worker-session",
        "claude",
        "claude-p",
        "claude-account",
        "claude-haiku-5-5",
    )
    .with_agent_id("revert-leased-agent")
    .with_client_interface(ProviderClientInterface::Chariox);
    let mut run = RuntimeProviderRun::new(
        "previous-claude-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "revert-fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    let event = crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id: session.id().into(),
        home_agent_id: agent.id().into(),
        provider_run_id: run.id().into(),
        provider_run: Some(run),
        prompts: Vec::new(),
        output_chunks: Vec::new(),
        notices: Vec::new(),
        completions: Vec::new(),
    };
    assert!(
        crate::app::RemoteLeaseRuntime::new(&mut app)
            .project_remote_runtime_projection(
                crate::runtime::relay_peer_authority::test_projection_authority(WORKER),
                event,
            )
            .unwrap()
            .accepted
    );
    let current = app.agents.get_agent(agent.id()).unwrap();
    assert_eq!(current.provider(), "codex");
    assert_eq!(current.model(), Some("gpt-6-luna"));
    assert_eq!(current.provider_account_profile(), "codex-account");
}
