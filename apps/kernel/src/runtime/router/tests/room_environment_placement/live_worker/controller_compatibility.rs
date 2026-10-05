use super::*;

pub(super) async fn check(fixture: &LiveWorker, token: &str) {
    let runtime = &fixture.home.runtime_state;
    let room = &fixture.rooms[0];
    let specs = runtime.runtime_tool_specs_for_auth_token(token);
    for name in [
        "slice_open_url",
        "slice_browser_wait_for_selector",
        "slice_browser_wait_for_idle",
    ] {
        assert!(
            specs.iter().any(|spec| spec.name == name),
            "bound Room runtime MCP omitted compatibility tool {name}"
        );
    }

    let selector = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_wait_for_selector",
            json!({"selector":"#worker-save","timeout_ms":500}),
        )
        .await
        .expect("selector wait reaches the bound worker controller");
    assert!(selector.ok, "{:?}", selector.payload);
    assert_eq!(selector.payload["source"], "browser_controller");
    assert_eq!(selector.payload["browser"]["action_kind"], "selector");

    let idle = runtime
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_wait_for_idle",
            json!({"timeout_ms":500}),
        )
        .await
        .expect("idle wait reaches the bound worker controller");
    assert!(idle.ok, "{:?}", idle.payload);
    assert_eq!(idle.payload["source"], "browser_controller");
    assert_eq!(idle.payload["browser"]["action_kind"], "idle");

    let url = "https://navigated.worker.test/path?view=compatibility";
    let navigated = runtime
        .dispatch_authenticated_runtime_tool_call(token, "slice_open_url", json!({"url":url}))
        .await
        .expect("legacy open-url tool reaches the bound worker controller");
    assert!(navigated.ok, "{:?}", navigated.payload);
    assert_eq!(navigated.payload["source"], "browser_controller");
    assert_eq!(navigated.payload["browser"]["action_kind"], "navigate");
    assert_eq!(navigated.payload["browser"]["url"], url);

    let environment = runtime
        .room_environment_snapshot(room)
        .expect("Room environment after compatibility navigation");
    let focused = environment
        .focused_tab_id
        .as_deref()
        .and_then(|focused| environment.tabs.iter().find(|tab| tab.tab_id == focused))
        .expect("focused tab after compatibility navigation");
    assert_eq!(focused.url, url);
    let action = environment
        .actions
        .iter()
        .find(|action| action.action_id == navigated.payload["action_id"])
        .expect("compatibility navigation action ledger entry");
    assert_eq!(action.kind, "navigate");
    assert_eq!(
        action.actor_id,
        navigated.payload["actor_id"]
            .as_str()
            .expect("navigation actor id")
    );
    assert_eq!(
        action.state,
        crate::session::EnvironmentActionState::Completed
    );

    let physical = std::fs::read_to_string(fixture._worker_state.root.join("chromium-state.json"))
        .expect("worker browser state after compatibility navigation");
    let physical: Value = serde_json::from_str(&physical).expect("worker browser state JSON");
    assert_eq!(physical["url"], url);
}

// MP-08/MP-10/MP-11: phase reporting preserves the read error and never replays input.
pub(super) async fn check_navigation_read_failures(fixture: &LiveWorker, token: &str) {
    let root = &fixture._worker_state.root;
    let state_file = root.join("chromium-state.json");
    let fault_file = root.join("navigation-read-fault.json");
    for (phase, expected_delta) in [("preflight_reconcile", 0), ("post_navigation_reconcile", 1)] {
        let state: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
        let before = state["navigateCount"].as_u64().unwrap_or(0);
        std::fs::write(
            &fault_file,
            serde_json::to_vec(&json!({"phase":phase,"beforeCount":before})).unwrap(),
        )
        .unwrap();
        let result = fixture
            .home
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                token,
                "slice_open_url",
                json!({"url":"https://worker.test/read-failure"}),
            )
            .await;
        std::fs::remove_file(&fault_file).unwrap();
        let error = result.expect_err("external read failure must remain an error");
        let message = error.to_string();
        assert!(
            message.contains("browser_cdp_timeout"),
            "original error code must survive: {message}"
        );
        assert!(
            message.contains(&format!("browser_navigation_phase={phase}")),
            "missing read phase: {message}"
        );
        let after: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
        assert_eq!(after["navigateCount"].as_u64().unwrap_or(0), before + expected_delta,
            "read diagnosis must neither navigate before preflight nor replay an acknowledged navigation");
        fixture
            .home
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(token, "slice_browser_status", json!({}))
            .await
            .expect("read recovery after fixture fault removal");
    }
}
