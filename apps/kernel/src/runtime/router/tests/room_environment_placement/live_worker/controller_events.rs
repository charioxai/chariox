use super::*;

pub(super) async fn check(fixture: &LiveWorker, token: &str, status: &Value) {
    let room = &fixture.rooms[0];
    let browser_generation = status["browser_generation"]
        .as_u64()
        .expect("browser status generation");
    let batch = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_events",
            json!({"browser_generation": browser_generation, "cursor": 0, "limit": 200}),
        )
        .await
        .expect("runtime MCP browser events cross the authenticated Room worker relay");
    assert!(batch.ok, "{:?}", batch.payload);
    assert_eq!(batch.payload["replay_gap"], false);
    assert_eq!(batch.payload["browser_generation"], browser_generation);
    let events = batch.payload["events"].as_array().expect("browser events");
    assert!(!events.is_empty());
    assert!(events
        .iter()
        .any(|event| event["kind"] == "download_progress"
            && event["data"]["guid"] == "worker-active-download"
            && event["data"]["state"] == "canceled"));

    let kinds = events
        .iter()
        .filter_map(|event| event["kind"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for expected in [
        "browser_connected",
        "console",
        "network_request",
        "network_response",
        "page_navigated",
        "dom_content_loaded",
        "page_loaded",
        "dialog_opened",
        "dialog_closed",
        "target_created",
        "download_started",
        "download_progress",
    ] {
        assert!(kinds.contains(expected), "missing {expected}: {kinds:?}");
    }
    assert!(events.iter().any(|event| {
        event["kind"] == "browser_connected"
            && event["tab_id"].is_null()
            && event["document_id"].is_null()
    }));
    assert!(events
        .iter()
        .filter(|event| !event["tab_id"].is_null())
        .all(|event| event["tab_id"]
            .as_str()
            .is_some_and(|tab_id| tab_id.starts_with("tab-"))));
    let popup_event = events
        .iter()
        .find(|event| event["kind"] == "target_created")
        .expect("popup lifecycle event");
    let popup_tab_id = popup_event["tab_id"]
        .as_str()
        .expect("new target receives a stable Room tab id before publication");
    let environment = fixture
        .home
        .runtime_state
        .room_environment_snapshot(room)
        .expect("Room environment after event reconciliation");
    assert!(environment.tabs.iter().any(|tab| {
        tab.tab_id == popup_tab_id
            && tab.url == "https://popup.worker.test/"
            && tab.title == "Worker popup"
    }));
    let serialized = batch.payload.to_string();
    assert!(!serialized.contains("must-not-cross-relay"));
    assert!(!serialized.contains("authorization"));

    let next_cursor = batch.payload["next_cursor"]
        .as_u64()
        .expect("next browser event cursor");
    let caught_up = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            "slice_browser_events",
            json!({
                "browser_generation": browser_generation,
                "cursor": next_cursor,
                "limit": 200
            }),
        )
        .await
        .expect("runtime MCP browser event cursor resumes through the bound worker");
    assert!(caught_up.ok, "{:?}", caught_up.payload);
    assert_eq!(caught_up.payload["replay_gap"], false);
    assert_eq!(caught_up.payload["events"], json!([]));
    assert_eq!(caught_up.payload["next_cursor"], next_cursor);
}
