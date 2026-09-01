use super::*;

pub(super) async fn check(fixture: &LiveWorker) {
    let room = &fixture.rooms[0];
    let batch = fixture
        .home
        .runtime_state
        .poll_browser_environment_events(room, 1, 0, 200)
        .await
        .expect("browser events cross the authenticated Room worker relay");
    assert!(!batch.replay_gap);
    assert_eq!(batch.browser_generation, 1);
    assert!(!batch.events.is_empty());

    let kinds = batch
        .events
        .iter()
        .map(|event| event.kind.as_str())
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
    assert!(batch.events.iter().any(|event| {
        event.kind == "browser_connected" && event.tab_id.is_none() && event.document_id.is_none()
    }));
    assert!(batch
        .events
        .iter()
        .filter(|event| event.tab_id.is_some())
        .all(|event| event
            .tab_id
            .as_deref()
            .is_some_and(|tab_id| tab_id.starts_with("tab-"))));
    let popup_event = batch
        .events
        .iter()
        .find(|event| event.kind == "target_created")
        .expect("popup lifecycle event");
    let popup_tab_id = popup_event
        .tab_id
        .as_deref()
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
    let serialized = format!("{batch:?}");
    assert!(!serialized.contains("must-not-cross-relay"));
    assert!(!serialized.contains("authorization"));

    let caught_up = fixture
        .home
        .runtime_state
        .poll_browser_environment_events(room, 1, batch.next_cursor, 200)
        .await
        .expect("browser event cursor resumes through the bound worker");
    assert!(!caught_up.replay_gap);
    assert!(caught_up.events.is_empty());
    assert_eq!(caught_up.next_cursor, batch.next_cursor);
}
