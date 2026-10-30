//! MP-08 / MP-09 / MP-10 / MP-11 A12-S04: definite refusal versus uncertain dispatch.
use super::*;

#[test]
fn room_prompt_dispatch_unauthenticated_peer_rejects_obligation() {
    run_large_stack_async_test("room-prompt-unauthenticated", unauthenticated_peer);
}

async fn unauthenticated_peer() {
    let env = TestMetaRuntimeEnv::new("room-prompt-unauthenticated");
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            env.root.to_string_lossy(),
            env.root.to_string_lossy(),
        ))
        .unwrap();
    let profile = app
        .provider_account_profile_registry()
        .create_managed(actor.owner_user_id(), "codex", "Unauthenticated peer")
        .unwrap();
    app.provider_account_profile_registry()
        .mark_logged_out(actor.owner_user_id(), "codex", &profile.profile_id)
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "codex")
                .with_alias("cold-peer")
                .with_account_profile(profile.profile_id),
        )
        .unwrap();
    let run = launch_test_provider(
        &mut app,
        session.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let auth = run.runtime_mcp_auth_token().unwrap().to_owned();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let result = router
        .dispatch_authenticated_runtime_tool_call(
            &auth,
            "chariox.room.run_command",
            serde_json::json!({"command":"prompt cold-peer read the README"}),
        )
        .await;
    let result = result.expect("Room command should return its refusal");
    assert!(!result.ok, "unauthenticated peer must refuse the prompt");
    let error = result.payload["error"].as_str().unwrap();
    assert!(error.contains("not authenticated"), "{error}");
    let guard = app.lock().await;
    let tasks = guard
        .durable_state_store()
        .agent_tasks(Some(session.id()), Some(actor.id()))
        .unwrap();
    let obligations = &tasks.last().unwrap().obligations;
    assert_eq!(obligations.len(), 1);
    assert_eq!(
        (
            obligations[0].dispatch_state.as_str(),
            obligations[0].status.as_str()
        ),
        ("rejected", "failed"),
        "a definite pre-reservation refusal must not leave an open intent: {obligations:?}"
    );
    assert!(!error.contains("uncertain"), "{error}");
    assert!(guard
        .providers()
        .get_run_for_agent(session.id(), peer.id())
        .is_none());
    assert!(guard
        .durable_state_store()
        .agent_tasks(Some(session.id()), Some(peer.id()))
        .unwrap()
        .is_empty());
    let session = guard.sessions().get_session(session.id()).unwrap();
    assert!(session.active_prompt_for_agent(peer.id()).is_none());
    assert!(session
        .queued_prompts_for_agent(peer.id())
        .is_none_or(|queue| queue.is_empty()));
}

#[test]
fn room_prompt_dispatch_post_reservation_failure_stays_uncertain() {
    run_large_stack_async_test("room-prompt-uncertain", post_reservation_failure);
}

async fn post_reservation_failure() {
    let env = TestMetaRuntimeEnv::new("room-prompt-uncertain");
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            env.root.to_string_lossy(),
            env.root.to_string_lossy(),
        ))
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("ready-peer"))
        .unwrap();
    let peer_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "room-model",
            )
            .with_agent_id(peer.id()),
        )
        .unwrap();
    app.update_provider_run_projection(peer_run);
    let run = launch_test_provider(
        &mut app,
        session.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let auth = run.runtime_mcp_auth_token().unwrap().to_owned();
    let db = rusqlite::Connection::open(app.durable_state_store().path()).unwrap();
    // The prompt owner and session mirror have already admitted this prompt when
    // persisting its projection fails. Rejection would invite a duplicate retry.
    db.execute_batch(&format!(
        "CREATE TRIGGER fail_prompt_projection BEFORE INSERT ON durable_state_events
         WHEN NEW.kind='session.prompt_state.updated'
          AND json_extract(NEW.payload_json,'$.agent_id')='{}'
         BEGIN SELECT RAISE(FAIL, 'injected post-reservation failure'); END;",
        peer.id()
    ))
    .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let result = router
        .dispatch_authenticated_runtime_tool_call(
            &auth,
            "chariox.room.run_command",
            serde_json::json!({"command":"prompt ready-peer read the README"}),
        )
        .await
        .expect("Room command should return the projection failure");
    assert!(!result.ok, "projection failure must reach the caller");
    let error = result.payload["error"].as_str().unwrap();
    assert!(error.contains("uncertain dispatch"), "{error}");
    assert!(
        error.contains("injected post-reservation failure"),
        "{error}"
    );
    let guard = app.lock().await;
    let tasks = guard
        .durable_state_store()
        .agent_tasks(Some(session.id()), Some(actor.id()))
        .unwrap();
    let obligations = &tasks.last().unwrap().obligations;
    assert_eq!(obligations.len(), 1);
    assert_eq!(
        (
            obligations[0].dispatch_state.as_str(),
            obligations[0].status.as_str()
        ),
        ("intent", "open"),
        "post-reservation failure must retain its unresolved dispatch: {obligations:?}"
    );
    assert!(
        guard
            .sessions()
            .get_session(session.id())
            .unwrap()
            .active_prompt_for_agent(peer.id())
            .is_some(),
        "the failure must follow actual prompt admission"
    );
}
