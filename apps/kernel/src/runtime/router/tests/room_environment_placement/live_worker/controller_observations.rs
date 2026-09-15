use super::*;

pub(super) async fn check(fixture: &LiveWorker, token: &str, status: &Value) {
    let runtime = &fixture.home.runtime_state;
    let specs = runtime.runtime_tool_specs_for_auth_token(token);
    for name in [
        "slice_browser_status",
        "slice_browser_find",
        "slice_browser_text",
        "slice_browser_wait_for_text",
    ] {
        assert!(specs.iter().any(|spec| spec.name == name), "missing {name}");
    }
    assert!(specs.iter().any(|spec| spec.name == "slice_screenshot"));
    let found = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_find",
            json!({"query":"Save on worker","kind":"button"}),
        )
        .await
        .unwrap();
    assert!(found.ok);
    assert_eq!(
        found.payload["browser"]["matches"][0]["field_id"],
        status["browser"]["buttons"][0]["field_id"],
        "element identity survives rediscovery"
    );
    let text = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_text", json!({}))
        .await
        .unwrap();
    assert!(text.ok);
    assert_eq!(text.payload["text"], "Save on worker\nWorker note");
    for (query, matched) in [("Save on worker", true), ("not in this page", false)] {
        let result = runtime
            .dispatch_authenticated_runtime_tool_call(
                token,
                "slice_browser_wait_for_text",
                json!({"text":query,"timeout_ms":100}),
            )
            .await
            .unwrap();
        assert_eq!(result.ok, matched, "{:?}", result.payload);
        assert_eq!(result.payload["browser"]["ok"], matched);
    }
    // Room selection comes from the authenticated provider run. Caller-supplied
    // identifiers cannot redirect a read, even with a valid token.
    let forged = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_status",
            json!({"session_id":fixture.rooms[1],"slice_id":"another-slice"}),
        )
        .await
        .unwrap();
    assert_eq!(forged.payload["session_id"], fixture.rooms[0]);
    assert_eq!(forged.payload["slice_id"], "slice-1");
    assert!(runtime
        .dispatch_authenticated_runtime_tool_call(
            "invalid-token",
            "slice_browser_status",
            json!({})
        )
        .await
        .is_err());
    let outsider = {
        let mut app = fixture.home.app.lock().await;
        let agent = spawn_test_agent(&mut app, &fixture.rooms[1], "outside-reader", "dev-stub");
        launch_test_provider(
            &mut app,
            &fixture.rooms[1],
            agent.id(),
            "dev-stub",
            "dev-stub",
            "test",
        )
        .runtime_mcp_auth_token()
        .unwrap()
        .to_string()
    };
    let outsider_specs = runtime.runtime_tool_specs_for_auth_token(&outsider);
    for name in [
        "slice_browser_status",
        "slice_screen_status",
        "slice_screenshot",
        "slice_ocr",
        "slice_find_text",
    ] {
        assert!(
            !outsider_specs.iter().any(|spec| spec.name == name),
            "Room outsider unexpectedly received {name}"
        );
    }
    assert!(runtime
        .dispatch_authenticated_runtime_tool_call(
            &outsider,
            "slice_browser_status",
            json!({"session_id":fixture.rooms[0],"slice_id":"slice-1"})
        )
        .await
        .is_err());
}
