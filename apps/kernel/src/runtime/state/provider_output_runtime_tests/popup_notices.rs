// MP-08/MP-10: real authenticated tool dispatch and shared interaction resolution.
use super::*;

#[tokio::test]
async fn popup_notice_accepts_one_acknowledgement_and_rejects_empty_choices() {
    let workspace = crate::test_support::TestWorktree::new("popup-notice");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "dev-stub",
        "dev-stub",
        "default",
        "default",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "popup-notice-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "fixture-popup".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.set_runtime_mcp_auth_token(Some("fixture-popup-token".into()));
    run.mark_running();
    app.providers_mut().insert_run_for_test(run);
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    for choice_count in [1, 2] {
        let mut choices =
            vec![serde_json::json!({"id":"ack", "label":"OK", "reply":"acknowledged"})];
        if choice_count == 2 {
            choices.push(serde_json::json!({"id":"cancel", "label":"Cancel", "reply":"cancelled"}));
        }
        let call = runtime.dispatch_authenticated_runtime_tool_call(
            "fixture-popup-token",
            crate::transport::runtime_tools::REQUEST_POPUP_TOOL_ALIAS,
            serde_json::json!({"message":"Fixture notice", "choices": choices}),
        );
        tokio::pin!(call);
        let interaction = tokio::select! {
            result = &mut call => panic!("notice must wait for human acknowledgement: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let session = runtime.owned.session_store.get_session(session.id()).unwrap();
                    if let Some(interaction) = session.active_interactions().first() {
                        break interaction.clone();
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }) => result.expect("notice must enter the shared interaction store"),
        };
        assert_eq!(interaction.choices().len(), choice_count);
        runtime
            .resolve_runtime_interaction(session.id(), interaction.id(), "ack", None)
            .await
            .unwrap();
        let result = call.await.unwrap();
        assert!(result.ok);
        assert_eq!(result.payload["status"], "answered");
        assert_eq!(result.payload["choice_id"], "ack");
        assert_eq!(result.payload["reply"], "acknowledged");
        assert!(runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap()
            .active_interactions()
            .is_empty());
    }
    let empty = runtime
        .dispatch_authenticated_runtime_tool_call(
            "fixture-popup-token",
            crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
            serde_json::json!({"message": "Fixture notice", "choices": []}),
        )
        .await
        .unwrap_err();
    assert!(empty.to_string().contains("at least one choice"));
}
