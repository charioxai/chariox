use super::*;

async fn pin(
    router: &CommandRouter,
    session_id: &str,
    pinned: bool,
    user: &str,
) -> Result<serde_json::Value, DaemonError> {
    let request: LocalDaemonRequest = serde_json::from_value(
        serde_json::json!({"SetRoomPinned":{"session_id":session_id,"pinned":pinned}}),
    )
    .expect("pin request decodes");
    let mut command = KernelCommand::from_local_request("room-pin", None, None, &request);
    command.caller.user_id = Some(user.to_string());
    router
        .dispatch(command, request)
        .await
        .map(|response| serde_json::to_value(response).unwrap())
}

#[tokio::test]
async fn mp11_room_pin_updates_listing_and_survives_restore() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let id = session.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    pin(&router, &id, true, "stranger")
        .await
        .expect_err("only the owner pins Rooms");
    let response = pin(&router, &id, true, DEFAULT_LOCAL_USER_ID)
        .await
        .unwrap();
    assert_eq!(response["RoomPinned"]["session"]["pinned"], true);
    let session = router.runtime_state.session_snapshot(&id).await.unwrap();
    let value = serde_json::to_value(session).unwrap();
    assert_eq!(value["pinned"], true);
    let restored: crate::session::RuntimeSession = serde_json::from_value(value).unwrap();
    assert!(restored.pinned());
    let request: LocalDaemonRequest =
        serde_json::from_value(serde_json::json!({"GetWaitingRoomPublicSnapshot":null})).unwrap();
    let command = KernelCommand::from_local_request("room-pin-list", None, None, &request);
    let listing = serde_json::to_value(router.dispatch(command, request).await.unwrap()).unwrap();
    assert_eq!(
        listing["WaitingRoomPublicSnapshot"]["snapshot"]["sessions"][0]["pinned"],
        true
    );
    let response = pin(&router, &id, false, DEFAULT_LOCAL_USER_ID)
        .await
        .unwrap();
    let unpinned: crate::session::RuntimeSession =
        serde_json::from_value(response["RoomPinned"]["session"].clone()).unwrap();
    assert!(!unpinned.pinned());
}

#[tokio::test]
async fn mp11_room_pin_preserves_spawned_agents_in_response_and_projections() {
    let worktree = crate::test_support::TestWorktree::new("room-pin-spawned-agents");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let id = session.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "SpawnAgent": {"session_id": id, "alias": "second", "provider": "claude-code"}
    }))
    .unwrap();
    let command = KernelCommand::from_local_request("room-pin-spawn", None, None, &request);
    let spawned = match router.dispatch(command, request).await.unwrap() {
        LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
        _ => panic!("expected spawned agent"),
    };
    let expected = router.session_projection.get(&id).unwrap().agents().len();
    assert!(expected >= 2);
    for pinned in [true, false] {
        let response = pin(&router, &id, pinned, DEFAULT_LOCAL_USER_ID)
            .await
            .unwrap();
        let returned: crate::session::RuntimeSession =
            serde_json::from_value(response["RoomPinned"]["session"].clone()).unwrap();
        assert_eq!(
            returned.agents().len(),
            expected,
            "pin response retains every agent"
        );
        assert!(returned.agents().iter().any(|agent| agent.id() == spawned));
        let projected = router.session_projection.get(&id).unwrap();
        assert_eq!(
            projected.agents().len(),
            expected,
            "cached session retains every agent"
        );
        let request: LocalDaemonRequest =
            serde_json::from_value(serde_json::json!({"GetWaitingRoomPublicSnapshot": null}))
                .unwrap();
        let command =
            KernelCommand::from_local_request("room-pin-list-agents", None, None, &request);
        let listing =
            serde_json::to_value(router.dispatch(command, request).await.unwrap()).unwrap();
        let listed = &listing["WaitingRoomPublicSnapshot"]["snapshot"]["sessions"][0];
        assert_eq!(
            listed["agents"].as_array().unwrap().len(),
            expected,
            "waiting Room retains every agent"
        );
    }
}
