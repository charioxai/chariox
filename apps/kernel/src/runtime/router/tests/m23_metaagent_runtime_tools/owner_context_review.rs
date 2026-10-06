//! MP-08/MP-11: Meta delegation cannot approve the source owner's copy review.
use super::*;

#[test]
fn mp08_mp11_kernel_only_owner_review_rejects_meta_and_agent_answers() {
    crate::test_support::isolated_env_test!();
    run_large_stack_async_test("kernel-only-human-review", kernel_only_human_review);
}

async fn kernel_only_human_review() {
    let env = TestMetaRuntimeEnv::new("kernel-only-human-review");
    let worktree = crate::test_support::TestWorktree::new("kernel-only-human-review");
    let mut config = DaemonConfig::for_tests();
    config.user_config.state.path = Some(env.root.join("state.db").display().to_string());
    config.user_config_path = env.root.join("config.toml");
    config = config.with_session_history_root(env.root.join("sessions"));
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, worker) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let meta = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .unwrap();
    let meta = activate_test_agent_meta_mode(&mut app, meta);
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), meta.id());
    let run = launch_test_provider(
        &mut app,
        session.id(),
        meta.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let auth = run.runtime_mcp_auth_token().unwrap().to_owned();
    let projection = app.config_projection_store();
    let mut config = projection.snapshot();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: session.owner_user_id().into(),
        ..Default::default()
    });
    projection.update(config);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let runtime = router.runtime_state();

    for choice in ["cancel", "continue"] {
        let context = format!("human-only-{choice}");
        let id = format!("owner-context:{context}");
        let reviewer = runtime.clone();
        let task = tokio::spawn(async move {
            reviewer
                .review_credential_free_owner_context(
                    &crate::managed_context::package::ManagedContextDevelopmentSelection::Empty,
                    &context,
                    "Owner target machine",
                )
                .await
        });
        let interaction = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let snapshot = runtime.session_snapshot(session.id()).await.unwrap();
                if let Some(review) = snapshot.active_interactions().iter().find(|i| i.id() == id) {
                    break review.clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let attempted = router
            .dispatch_authenticated_runtime_tool_call(
                &auth,
                crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
                serde_json::json!({"interaction_id": id, "choice_id": "continue"}),
            )
            .await
            .unwrap();
        assert!(
            !attempted.ok,
            "MP-11 Meta approval bypassed owner review: {:?}",
            attempted.payload
        );
        assert!(interaction.agent_id().is_none());
        assert!(interaction.kernel_operation_id().is_some());
        assert!(runtime
            .resolve_runtime_interaction(session.id(), &id, "continue", None)
            .await
            .is_err());
        assert!(runtime
            .resolve_terminal_runtime_interaction(
                session.id(),
                &id,
                "continue",
                None,
                Some("another-owner"),
            )
            .await
            .is_err());
        for class in [
            None,
            Some(crate::local::KernelConnectionClass::Host),
            Some(crate::local::KernelConnectionClass::KernelAgent),
        ] {
            assert!(
                runtime
                    .answer_terminal_runtime_interaction(
                        session.id(),
                        &id,
                        "continue",
                        None,
                        Some(session.owner_user_id()),
                        None,
                        None,
                        class,
                    )
                    .await
                    .is_err(),
                "MP-11 nonterminal owner identity must not approve review"
            );
        }
        assert!(
            !task.is_finished(),
            "MP-11 rejected replies must leave review pending"
        );
        let events = router
            .dispatch_authenticated_runtime_tool_call(
                &auth,
                crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
                serde_json::json!({"kind": "runtime.interaction"}),
            )
            .await
            .unwrap();
        assert!(events.ok);
        assert!(
            events.payload["events"].as_array().unwrap().is_empty(),
            "MP-11 owner review must not enter Meta prompt/event context"
        );
        let request =
            LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
                session_id: session.id().into(),
                interaction_id: id,
                choice_id: choice.into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            });
        let mut command =
            KernelCommand::from_local_request("owner-review-answer", None, None, &request);
        command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
        assert!(matches!(
            router.dispatch(command, request).await.unwrap(),
            LocalDaemonResponse::InteractionResponded { .. }
        ));
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        if choice == "continue" {
            result.unwrap();
        } else {
            assert!(matches!(
                result,
                Err(DaemonError::ManagedContext {
                    code: "managed_context_review_cancelled",
                    retryable: false,
                    ..
                })
            ));
        }
    }
}
