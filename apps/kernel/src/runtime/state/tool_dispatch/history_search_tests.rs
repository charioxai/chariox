//! MP-08 / MP-10 / MP-11: discovery must agree with room history admission.
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use crate::transport::runtime_tools::META_HISTORY_SEARCH_TOOL;

fn assert_meta_history_discovery_and_dispatch(room_tools: bool) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let worktree = crate::test_support::TestWorktree::new("meta-history-discovery");
                    let mut config = crate::config::DaemonConfig::for_tests()
                        .with_session_history_root(worktree.path().join("history"));
                    config.room_agent_tools = room_tools;
                    let mut app = crate::DaemonApp::bootstrap(config).unwrap();
                    let (room, actor) = crate::app::KernelSessionService::new(&mut app)
                        .create_session(worktree.session_request())
                        .unwrap();
                    app.agents_mut()
                        .activate_agent_meta_mode(actor.id(), None)
                        .unwrap();
                    let request = LaunchProviderRequest::new(
                        room.id(),
                        "dev-stub",
                        "dev-stub",
                        "default",
                        "default",
                    )
                    .with_agent_id(actor.id());
                    let mut run = RuntimeProviderRun::new(
                        "meta-history-discovery-run",
                        &request,
                        ProviderLaunchResult {
                            endpoint_mode: AgentEndpointMode::External,
                            process_label: "metadata-only".into(),
                            pty_target: None,
                            pty_program: None,
                            pty_args: vec![],
                            pty_env: Default::default(),
                            pty_env_remove: vec![],
                            working_directory: Some(worktree.path().to_owned()),
                            structured_endpoint: None,
                        },
                    );
                    run.mark_running();
                    run.set_runtime_mcp_auth_token(Some("meta-history-discovery-bearer".into()));
                    app.providers_mut().insert_run_for_test(run.clone());
                    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
                        std::sync::Arc::new(tokio::sync::Mutex::new(app)),
                        1,
                    );
                    let state = router.runtime_state();
                    state.owned.provider_run_projection.update(run);
                    for specs in [
                        router.runtime_tool_specs_for_auth_token("meta-history-discovery-bearer"),
                        state
                            .runtime_tool_specs_for_auth_token_async(
                                "meta-history-discovery-bearer".into(),
                            )
                            .await
                            .unwrap(),
                    ] {
                        assert!(specs
                            .iter()
                            .any(|spec| spec.name == "chariox.meta.session_overview"));
                        assert_eq!(
                            specs
                                .iter()
                                .any(|spec| spec.name == META_HISTORY_SEARCH_TOOL),
                            room_tools,
                            "Meta history search discovery must match room-tools admission",
                        );
                    }
                    state
                        .owned
                        .operational_history_store
                        .append_operational_event(
                            crate::history::HistoryEventKind::UserPrompt,
                            Some(crate::history::HistoryEventRole::User),
                            Some("meta_history_public_marker".into()),
                            Default::default(),
                            crate::history::HistoryEventTurnContext {
                                session_id: Some(room.id().into()),
                                agent_id: Some(actor.id().into()),
                                public_history_owner_user_id: Some(room.owner_user_id().into()),
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    let overview = router
                        .dispatch_authenticated_runtime_tool_call(
                            "meta-history-discovery-bearer",
                            "chariox.meta.session_overview",
                            serde_json::json!({}),
                        )
                        .await
                        .unwrap();
                    assert!(overview.ok, "existing legacy Meta tools remain usable");
                    for name in [META_HISTORY_SEARCH_TOOL, "chariox.history.search"] {
                        let result = router
                            .dispatch_authenticated_runtime_tool_call(
                                "meta-history-discovery-bearer",
                                name,
                                serde_json::json!({"query":"meta_history_public_marker"}),
                            )
                            .await;
                        if room_tools {
                            let result = result.unwrap();
                            assert!(result.ok);
                            assert_eq!(result.payload["hits"].as_array().unwrap().len(), 1);
                        } else {
                            assert!(
                                result.is_err(),
                                "unadvertised room search must still deny legacy callers"
                            );
                        }
                    }
                    assert!(crate::transport::runtime_tools::room_runtime_tool_specs()
                        .iter()
                        .any(|spec| spec.name == "chariox.history.search"));
                });
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn public_history_meta_discovery_and_dispatch_with_room_tools_disabled() {
    assert_meta_history_discovery_and_dispatch(false);
}

#[test]
fn public_history_meta_discovery_and_dispatch_with_room_tools_enabled() {
    assert_meta_history_discovery_and_dispatch(true);
}
