//! MP-08 / MP-10 / MP-11: room overview is a public MCP projection.

#[tokio::test]
async fn room_overview_excludes_transport_and_provider_private_state_in_both_mcp_representations() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "overview-workspace",
            "overview-worktree",
        ))
        .unwrap();
    app.agents
        .bind_remote_execution(
            actor.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "overview-worker".into(),
                worker_machine_id: "overview-machine".into(),
                execution_lease_id: "overview-lease".into(),
                leased_agent_id: "overview-leased-agent".into(),
                active_worker_provider_run_id: Some("overview-worker-run".into()),
                relay_url: Some("wss://relay.example.test".into()),
                relay_token: Some("synthetic-overview-relay-canary".into()),
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("private-peer")
                .with_owner_user_id("other-owner"),
        )
        .unwrap();
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let state =
        crate::runtime::router::CommandRouter::with_interactive_capacity(app, 1).runtime_state();
    let result = state
        .meta_session_overview(&session, &actor, Default::default())
        .unwrap();
    let (content, structured) =
        crate::transport::mcp_server::runtime_tool_content_for_test(result.payload);
    for representation in [
        serde_json::to_string(&content).unwrap(),
        structured.to_string(),
    ] {
        assert!(
            !representation.contains("synthetic-overview-relay-canary"),
            "MP-11 transport credential exposed"
        );
        assert!(
            !representation.contains("relay_token"),
            "MP-11 relay credential field exposed"
        );
        assert!(
            !representation.contains("provider_resume_state"),
            "MP-11 provider-private state exposed"
        );
        assert!(
            !representation.contains(peer.id()),
            "MP-11 private collaborator agent exposed"
        );
    }
    let public = &structured["agents"][0];
    assert_eq!(public["id"], actor.id());
    assert_eq!(
        public["remote_execution"]["worker_kernel_id"],
        "overview-worker"
    );
    assert_eq!(
        public["remote_execution"]["relay_url"],
        "wss://relay.example.test"
    );
    assert_eq!(
        public["remote_execution"]["active_worker_provider_run_id"],
        "overview-worker-run"
    );
}
