use super::*;

pub(super) async fn check(fixture: &LiveWorker, token: &str) {
    let status = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .expect("read a fresh browser reference before response-loss drill");
    let worker_relay_state = fixture.worker.app.lock().await.relay_client_state();
    let chromium_state_path = fixture._worker_state.root.join("chromium-state.json");
    let before: Value =
        serde_json::from_slice(&std::fs::read(&chromium_state_path).unwrap()).unwrap();
    let click_count_before = before["clickCount"].as_u64().unwrap();
    worker_relay_state
        .write()
        .await
        .test_lose_next_peer_response_payload();

    let result = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":status.payload["browser"]["buttons"][0]["field_id"]}),
        )
        .await
        .expect("lost terminal response must recover the completed worker action");
    assert!(result.ok, "{:?}", result.payload);

    let chromium: Value =
        serde_json::from_slice(&std::fs::read(chromium_state_path).unwrap()).unwrap();
    assert_eq!(
        chromium["clickCount"].as_u64().unwrap(),
        click_count_before + 1,
        "recovering a lost response must not repeat physical input"
    );
    let environment = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let action_id = result.payload["action_id"].as_str().unwrap();
    assert_eq!(
        environment["RoomEnvironmentState"]["environment"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["action_id"] == action_id)
            .unwrap()["state"],
        "completed",
        "the home ledger must use recovered worker completion evidence"
    );

    check_missing_receipt_fails_closed(fixture, token).await;
}

async fn check_missing_receipt_fails_closed(fixture: &LiveWorker, token: &str) {
    let status = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    let chromium_state_path = fixture._worker_state.root.join("chromium-state.json");
    let before: Value =
        serde_json::from_slice(&std::fs::read(&chromium_state_path).unwrap()).unwrap();
    let click_count_before = before["clickCount"].as_u64().unwrap();
    let worker_relay_state = fixture.worker.app.lock().await.relay_client_state();
    worker_relay_state
        .write()
        .await
        .test_lose_next_peer_response_payload_and_forget_action_receipts();

    let error = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":status.payload["browser"]["buttons"][0]["field_id"]}),
        )
        .await
        .expect_err("missing worker completion proof must fail closed");
    assert!(
        error.to_string().contains("receipt") || error.to_string().contains("proof"),
        "{error}"
    );
    let chromium: Value =
        serde_json::from_slice(&std::fs::read(&chromium_state_path).unwrap()).unwrap();
    assert_eq!(
        chromium["clickCount"].as_u64().unwrap(),
        click_count_before + 1,
        "missing completion proof must not resend physical input"
    );
    let environment = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let latest = environment["RoomEnvironmentState"]["environment"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .max_by_key(|action| action["sequence"].as_u64().unwrap())
        .unwrap();
    assert_eq!(latest["state"], "failed");

    let fresh = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":fresh.payload["browser"]["buttons"][0]["field_id"]}),
        )
        .await
        .expect("a new execution remains available after fail-closed recovery");
    let after: Value =
        serde_json::from_slice(&std::fs::read(chromium_state_path).unwrap()).unwrap();
    assert_eq!(
        after["clickCount"].as_u64().unwrap(),
        click_count_before + 2
    );
}
