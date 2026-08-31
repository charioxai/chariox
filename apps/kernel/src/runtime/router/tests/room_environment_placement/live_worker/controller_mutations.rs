use super::*;

pub(super) async fn check(fixture: &LiveWorker, token: &str, status: &Value) {
    let runtime = &fixture.home.runtime_state;
    let reference = status["browser"]["buttons"][0]["field_id"]
        .as_str()
        .unwrap();
    let clicked = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":reference}),
        )
        .await
        .expect("home agent clicks its worker browser");
    assert!(clicked.ok, "{:?}", clicked.payload);
    assert_eq!(clicked.payload["browser"]["action_kind"], "click");
    assert_eq!(
        clicked.payload["actor_id"],
        format!("agent:{}", status["agent_id"].as_str().unwrap())
    );
    assert!(clicked.payload["action_id"]
        .as_str()
        .is_some_and(|id| id.starts_with("action-")));
    let after = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        after.payload["browser"]["buttons"][0]["label"], "Saved on worker",
        "the page must actually change, not merely acknowledge an action"
    );
    let room = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let action = room["RoomEnvironmentState"]["environment"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["action_id"] == clicked.payload["action_id"])
        .unwrap();
    assert_eq!(action["state"], "completed");
    assert_eq!(action["actor_id"], clicked.payload["actor_id"]);
    let field = status["browser"]["fields"][0]["field_id"].as_str().unwrap();
    let filled = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_fill",
            json!({"field_id":field,"text":"A note from home"}),
        )
        .await
        .expect("home agent fills its worker form");
    assert!(filled.ok, "{:?}", filled.payload);
    assert_eq!(filled.payload["browser"]["action_kind"], "fill");
    let submitted = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_submit",
            json!({"field_id":field}),
        )
        .await
        .expect("home agent submits its worker form");
    assert!(submitted.ok, "{:?}", submitted.payload);
    assert_eq!(submitted.payload["browser"]["action_kind"], "submit");
    let after = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        after.payload["browser"]["buttons"][0]["label"],
        "Submitted: A note from home"
    );
    let room = dispatch_json(
        &fixture.home,
        json!({"GetRoomEnvironmentState":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    for result in [&filled, &submitted] {
        let action = room["RoomEnvironmentState"]["environment"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["action_id"] == result.payload["action_id"])
            .unwrap();
        assert_eq!(action["state"], "completed");
        assert_eq!(action["actor_id"], clicked.payload["actor_id"]);
    }
    let specs = runtime.runtime_tool_specs_for_auth_token(token);
    for name in [
        "slice_browser_click",
        "slice_browser_fill",
        "slice_browser_submit",
    ] {
        assert!(specs.iter().any(|spec| spec.name == name), "missing {name}");
    }
    let denied_desktop = dispatch_json(
        &fixture.home,
        json!({"RequestRoomEnvironmentInputTakeover":{
            "session_id":fixture.rooms[0],"target":{"kind":"desktop"}
        }}),
    )
    .await
    .expect_err("a ready browser must not make the starting desktop available");
    assert!(denied_desktop.to_string().contains("environment_not_ready"));
    let target = json!({"kind":"browser_tab","id":status["tab_id"]});
    let takeover = dispatch_json(
        &fixture.home,
        json!({"RequestRoomEnvironmentInputTakeover":{
            "session_id":fixture.rooms[0],"target":target
        }}),
    )
    .await
    .unwrap();
    assert!(
        takeover["RoomEnvironmentTakeoverUpdated"]["environment"]["input_ownership"]
            .as_array()
            .unwrap()
            .iter()
            .any(|owner| owner["target"] == target
                && owner["actor_id"].as_str().unwrap().starts_with("user:"))
    );
    let denied = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":reference}),
        )
        .await
        .expect_err("human input ownership must block worker mutations");
    assert!(denied.to_string().contains("belongs to"), "{denied}");
    let untouched = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        untouched.payload["browser"]["buttons"][0]["label"],
        "Submitted: A note from home"
    );
    dispatch_json(
        &fixture.home,
        json!({"ReleaseRoomEnvironmentInput":{
            "session_id":fixture.rooms[0],"target":target
        }}),
    )
    .await
    .unwrap();
    let resumed = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":reference}),
        )
        .await
        .expect("explicit human release allows agent input again");
    assert!(resumed.ok, "{:?}", resumed.payload);
    let after_release = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
        .await
        .unwrap();
    assert_eq!(
        after_release.payload["browser"]["buttons"][0]["label"],
        "Saved on worker"
    );
    assert!(runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_click",
            json!({"field_id":"element-missing"})
        )
        .await
        .is_err());
    let outsider = {
        let mut app = fixture.home.app.lock().await;
        let agent = spawn_test_agent(&mut app, &fixture.rooms[1], "outside-clicker", "dev-stub");
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
    assert!(runtime
        .dispatch_authenticated_runtime_tool_call(
            &outsider,
            "slice_browser_click",
            json!({"field_id":reference,"session_id":fixture.rooms[0],"slice_id":"slice-1"})
        )
        .await
        .is_err());
}
