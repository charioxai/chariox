use super::*;

#[tokio::test]
async fn local_spawn_agents_inherit_session_account_profile_in_response_and_snapshot() {
    let worktree = crate::test_support::TestWorktree::new("spawn-account-defaults");
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream, selected, other) = {
        let mut app = app.lock().await;
        let profiles = app.provider_account_profile_registry();
        let selected = profiles
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                "session-account",
            )
            .unwrap();
        let other = profiles
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                "other-account",
            )
            .unwrap();
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(
                worktree.session_request().with_agent_defaults(
                    SessionAgentDefaults::new("codex")
                        .with_account_profile(&selected.profile_id)
                        .with_model("gpt-6-luna")
                        .with_effort("low"),
                ),
            )
            .unwrap();
        (
            session.id().to_string(),
            app.terminal_stream_store(),
            selected.profile_id,
            other.profile_id,
        )
    };
    let projection = SessionStateProjectionStore::default();
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        projection.clone(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );
    for batch in [false, true] {
        let mut request = serde_json::json!({"session_id": session_id});
        let item = serde_json::json!({});
        let kind = if batch {
            request["agents"] = serde_json::json!([item, {"account_profile": other}]);
            "SpawnAgents"
        } else {
            "SpawnAgent"
        };
        let request: LocalDaemonRequest =
            serde_json::from_value(serde_json::json!({(kind): request})).unwrap();
        let command =
            KernelCommand::from_local_request("spawn-account-defaults", None, None, &request);
        let response = runtime
            .dispatch_session_command(command, request)
            .await
            .unwrap();
        let agents = match response {
            LocalDaemonResponse::AgentSpawned { agent } => vec![agent],
            LocalDaemonResponse::AgentsSpawned { agents } => agents,
            _ => panic!("expected spawned agents"),
        };
        assert_eq!(agents[0].account_profile(), Some(selected.as_str()));
        assert_eq!(agents[0].provider(), "codex");
        assert_eq!(agents[0].model(), Some("gpt-6-luna"));
        assert_eq!(agents[0].effort(), Some("low"));
        if batch {
            assert_eq!(agents[1].account_profile(), Some(other.as_str()));
        }
        let snapshot = projection.get(&session_id).unwrap();
        let wire = serde_json::to_value(&snapshot).unwrap();
        for agent in agents {
            let projected = wire["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["id"] == agent.id())
                .unwrap();
            assert_eq!(
                projected["account_profile"],
                agent.account_profile().unwrap()
            );
            assert_eq!(projected["alias"], agent.alias().unwrap());
        }
    }
}
