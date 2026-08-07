use super::*;

#[tokio::test]
async fn get_session_state_reconciles_a_stale_projection_without_app_lock_access() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let provider_run = launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    router.active_turns.start(crate::app::ActiveTurnState::new(
        session_id.clone(),
        agent_id.clone(),
        "prompt-1".to_string(),
        provider_run.id().to_string(),
    ));
    let interaction = RuntimeInteraction::new(
        "interaction-1",
        &agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Info,
        Some("Approve file changes?".to_string()),
        "Approve file changes?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _resolution = router
        .runtime_state
        .create_runtime_interaction(&session_id, interaction)
        .await
        .expect("interaction should register");
    router.session_projection.update(session.clone());
    assert!(
        router
            .session_projection
            .get(&session_id)
            .expect("stale session projection should exist")
            .active_interactions()
            .is_empty(),
        "regression setup must replace the current projection with a stale snapshot",
    );

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-state-interaction", None, None, &state_request);
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "GetSessionState should reconcile through owned stores without app lock access"
    );

    drop(app_guard);
    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState {
            session,
            agent_activity,
            ..
        } => {
            assert_eq!(session.focused_agent_id(), Some(agent_id.as_str()));
            assert_eq!(session.agents().len(), 1);
            assert_eq!(session.active_interactions().len(), 1);
            let activity = agent_activity
                .get(&agent_id)
                .expect("agent activity should include focused agent");
            assert!(
                activity.busy,
                "active turn must keep focused agent working during permission popup"
            );
            assert!(
                activity.active_turn.is_some(),
                "active turn projection must survive interaction projection refresh"
            );
        }
        _ => panic!("unexpected state response"),
    }
}

#[tokio::test]
async fn runtime_interaction_registration_and_resolution_wake_terminal_subscribers() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let interaction = RuntimeInteraction::new(
        "interaction-terminal-wake",
        &agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Info,
        Some("Approve file changes?".to_string()),
        "Approve file changes?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );

    let before_projection_sequence = router.session_projection_change_sequence();
    let before_terminal_session_sequence = router.terminal_session_change_sequence(&session_id);
    let receiver = router
        .runtime_state
        .create_runtime_interaction(&session_id, interaction)
        .await
        .expect("interaction should register");

    assert!(
        router.session_projection_change_sequence() > before_projection_sequence,
        "runtime interaction registration should publish a projection change"
    );
    assert!(
        router.terminal_session_change_sequence(&session_id) > before_terminal_session_sequence,
        "runtime interaction registration should wake terminal subscription waiters"
    );

    let before_resolve_terminal_session_sequence =
        router.terminal_session_change_sequence(&session_id);
    router
        .runtime_state
        .resolve_runtime_interaction(&session_id, "interaction-terminal-wake", "allow_once", None)
        .await
        .expect("interaction should resolve");
    assert!(
        router.terminal_session_change_sequence(&session_id)
            > before_resolve_terminal_session_sequence,
        "runtime interaction resolution should wake terminal subscription waiters"
    );
    let resolution = receiver.await.expect("resolution should be delivered");
    assert_eq!(resolution.choice_id.as_deref(), Some("allow_once"));
    assert_eq!(resolution.reply.as_deref(), Some("allow"));
}

#[tokio::test]
async fn runtime_interaction_rejects_agent_outside_session() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (first_session, _first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("first session should be created");
    let (second_session, second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
        .expect("second session should be created");
    let first_session_id = first_session.id().to_string();
    let second_agent_id = second_agent.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let interaction = RuntimeInteraction::new(
        "interaction-cross-session",
        &second_agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Info,
        Some("Approve file changes?".to_string()),
        "Approve file changes?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );

    let error = router
        .runtime_state
        .create_runtime_interaction(&first_session_id, interaction)
        .await
        .expect_err("cross-session runtime interactions should be rejected");

    assert!(matches!(
        error,
        DaemonError::AgentNotInSession {
            session_id,
            agent_id,
        } if session_id == first_session_id && agent_id == second_agent_id
    ));
    let first_snapshot = router
        .runtime_state
        .session_snapshot_projection(first_session.id(), 0)
        .expect("first session projection should resolve");
    let second_snapshot = router
        .runtime_state
        .session_snapshot_projection(second_session.id(), 0)
        .expect("second session projection should resolve");
    assert!(first_snapshot.session.active_interactions().is_empty());
    assert!(second_snapshot.session.active_interactions().is_empty());
}

#[tokio::test]
async fn subscription_snapshot_includes_runtime_interaction_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "interaction-subscription",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);

    let initial = router
        .relay_watch_subscription_state(&session_id, &attachment_id, true, None, 0)
        .await;
    let initial_snapshot = match initial {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => snapshot
            .as_ref()
            .clone()
            .expect("initial subscription should include a snapshot"),
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    };
    let initial_activity_revision = initial_snapshot.metadata.last_event_id;
    let interaction = RuntimeInteraction::new(
        "interaction-subscription-1",
        &agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Info,
        Some("Approve file changes?".to_string()),
        "Approve file changes?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _resolution = router
        .runtime_state
        .create_runtime_interaction(&session_id, interaction)
        .await
        .expect("interaction should register");
    router
        .session_projection
        .update(initial_snapshot.session.clone());
    assert!(
        router
            .session_projection
            .get(&session_id)
            .expect("stale session projection should exist")
            .active_interactions()
            .is_empty(),
        "regression setup must replace the current projection with a stale snapshot",
    );

    let update = router
        .relay_watch_subscription_state(
            &session_id,
            &attachment_id,
            true,
            Some(initial_snapshot),
            0,
        )
        .await;
    match update {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => {
            let snapshot = snapshot
                .as_ref()
                .as_ref()
                .expect("runtime interaction should change subscription snapshot");
            assert_eq!(
                snapshot.metadata.last_event_id,
                router.session_projection_change_sequence(),
                "subscription snapshots must carry the current monotonic activity revision",
            );
            assert!(
                snapshot.metadata.last_event_id > initial_activity_revision,
                "updated subscription snapshots must advance beyond the previous activity revision",
            );
            assert_eq!(snapshot.session.active_interactions().len(), 1);
            assert_eq!(
                snapshot.session.active_interactions()[0].id(),
                "interaction-subscription-1"
            );
            assert_eq!(
                router
                    .session_projection
                    .get(&session_id)
                    .expect("subscription reconciliation should republish the session")
                    .active_interactions()[0]
                    .id(),
                "interaction-subscription-1",
                "subscription reconciliation must heal a stale shared projection",
            );
        }
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    }
}

fn native_interaction_subscription_app(
    client_id: &str,
) -> (Arc<Mutex<DaemonApp>>, String, String, String) {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            client_id,
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    (
        Arc::new(Mutex::new(app)),
        session.id().to_string(),
        agent.id().to_string(),
        attachment.id().to_string(),
    )
}

fn native_interaction_subscription_routers(
    client_id: &str,
) -> (
    Box<CommandRouter>,
    Box<CommandRouter>,
    String,
    String,
    String,
) {
    let (app, session_id, agent_id, attachment_id) = native_interaction_subscription_app(client_id);
    (
        Box::new(CommandRouter::with_interactive_capacity(
            Arc::clone(&app),
            1,
        )),
        Box::new(CommandRouter::with_interactive_capacity(app, 1)),
        session_id,
        agent_id,
        attachment_id,
    )
}

#[tokio::test]
async fn dispatched_native_provider_interaction_updates_subscription_projection() {
    let (app, session_id, agent_id, attachment_id) =
        native_interaction_subscription_app("native-interaction-subscription");
    let router = Box::new(CommandRouter::with_interactive_capacity(app, 1));

    let initial = router
        .relay_watch_subscription_state(&session_id, &attachment_id, true, None, 0)
        .await;
    let initial_snapshot = match initial {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => snapshot
            .as_ref()
            .clone()
            .expect("initial subscription should include a snapshot"),
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    };

    let request = LocalDaemonRequest::RequestNativeProviderInteraction(
        RequestNativeProviderInteractionRequest::allow_deny(
            &session_id,
            &agent_id,
            "native-interaction-dispatch",
            Some("Approve file changes?".to_string()),
            "Approve file changes?".to_string(),
            Some(30),
        ),
    );
    let command = KernelCommand::from_local_request(
        "cmd-native-interaction-subscription",
        None,
        None,
        &request,
    );
    let dispatch_router = router.clone();
    let dispatch_task =
        tokio::spawn(async move { Box::pin(dispatch_router.dispatch(command, request)).await });

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let state = router
                .runtime_state
                .session_snapshot_projection(&session_id, 0)
                .expect("session projection should resolve")
                .session;
            if state
                .active_interactions()
                .iter()
                .any(|interaction| interaction.id() == "native-interaction-dispatch")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("native interaction should become active");

    let update = router
        .relay_watch_subscription_state(
            &session_id,
            &attachment_id,
            true,
            Some(initial_snapshot),
            0,
        )
        .await;
    match update {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => {
            let snapshot = snapshot
                .as_ref()
                .as_ref()
                .expect("runtime interaction should change subscription snapshot");
            assert_eq!(snapshot.session.active_interactions().len(), 1);
            assert_eq!(
                snapshot.session.active_interactions()[0].id(),
                "native-interaction-dispatch"
            );
        }
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    }

    router
        .runtime_state
        .resolve_runtime_interaction(
            &session_id,
            "native-interaction-dispatch",
            "allow_once",
            None,
        )
        .await
        .expect("interaction should resolve");
    dispatch_task
        .await
        .expect("dispatch task should join")
        .expect("request should resolve");
}

#[tokio::test]
async fn native_provider_interaction_wakes_subscription_projection_across_routers() {
    tokio::spawn(native_provider_interaction_wakes_subscription_projection_across_routers_impl())
        .await
        .expect("cross-router native interaction test should join");
}

async fn native_provider_interaction_wakes_subscription_projection_across_routers_impl() {
    let (relay_router, local_router, session_id, agent_id, attachment_id) =
        native_interaction_subscription_routers("cross-router-native-interaction-subscription");

    let initial = relay_router
        .relay_watch_subscription_state(&session_id, &attachment_id, true, None, 0)
        .await;
    let initial_snapshot = match initial {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => snapshot
            .as_ref()
            .clone()
            .expect("initial subscription should include a snapshot"),
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    };

    let before_relay_sequence = relay_router.session_projection_change_sequence();
    let request = LocalDaemonRequest::RequestNativeProviderInteraction(
        RequestNativeProviderInteractionRequest::allow_deny(
            &session_id,
            &agent_id,
            "cross-router-native-interaction",
            Some("Approve file changes?".to_string()),
            "Approve file changes?".to_string(),
            Some(30),
        ),
    );
    let command = KernelCommand::from_local_request(
        "cmd-cross-router-native-interaction",
        None,
        None,
        &request,
    );
    let dispatch_task =
        tokio::spawn(async move { Box::pin(local_router.dispatch(command, request)).await });

    timeout(
        Duration::from_secs(1),
        relay_router.wait_for_session_projection_change_after(before_relay_sequence),
    )
    .await
    .expect("subscription router should wake for cross-router projection changes");
    assert!(
        relay_router.session_projection_change_sequence() > before_relay_sequence,
        "subscription router should observe projection changes published by the request router"
    );

    let update = relay_router
        .relay_watch_subscription_state(
            &session_id,
            &attachment_id,
            true,
            Some(initial_snapshot),
            0,
        )
        .await;
    match update {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => {
            let snapshot = snapshot
                .as_ref()
                .as_ref()
                .expect("runtime interaction should change subscription snapshot");
            assert_eq!(snapshot.session.active_interactions().len(), 1);
            assert_eq!(
                snapshot.session.active_interactions()[0].id(),
                "cross-router-native-interaction"
            );
        }
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    }

    relay_router
        .runtime_state
        .resolve_runtime_interaction(
            &session_id,
            "cross-router-native-interaction",
            "allow_once",
            None,
        )
        .await
        .expect("interaction should resolve");
    dispatch_task
        .await
        .expect("dispatch task should join")
        .expect("request should resolve");
}
