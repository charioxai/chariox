// MP-08/MP-10: logical tab history must survive an earlier queued navigation.
use super::*;
use crate::runtime::browser_controller_history::BrowserHistoryAction;
use crate::session::EnvironmentActionState;
use futures_util::FutureExt;

#[test]
fn queued_tab_history_resolves_the_document_after_its_predecessor() {
    run_test(scenario);
}

async fn scenario() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let result = std::panic::AssertUnwindSafe(check(&mut fixture))
        .catch_unwind()
        .await;
    fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await
        .unwrap();
    fixture.stop().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn check(fixture: &mut LiveWorker) {
    fixture.create_slice().await;
    fixture
        .home
        .app
        .lock()
        .await
        .slices()
        .set_status(
            "desktop",
            crate::slice::SliceStatus::Running,
            crate::session::unix_epoch_ms(),
        )
        .unwrap();
    let room = &fixture.rooms[0];
    dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice":{
            "session_id":room,"slice_ref":"desktop"
        }}),
    )
    .await
    .unwrap();
    dispatch_json(
        &fixture.home,
        json!({"StartRoomEnvironment":{
            "session_id":room,"viewport":{"css_width":1280,"css_height":800,
            "device_scale_factor":1,"desktop_pixel_width":1280,"desktop_pixel_height":800}
        }}),
    )
    .await
    .unwrap();
    let agent = {
        let mut app = fixture.home.app.lock().await;
        spawn_test_agent(&mut app, room, "history-reader", "dev-stub")
            .id()
            .to_string()
    };
    let runtime = &fixture.home.runtime_state;
    let before = runtime.room_environment_snapshot(room).unwrap();
    let tab = before.tabs[0].tab_id.clone();
    let revision = before.tabs[0].document_revision;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let first_runtime = runtime.clone();
    let first_room = room.clone();
    let first_agent = agent.clone();
    let first_tab = tab.clone();
    let first = tokio::spawn(async move {
        first_runtime
            .execute_browser_mutation_as_agent(
                &first_room,
                &first_agent,
                &first_tab,
                revision,
                "history-predecessor",
                None,
                async {
                    let _ = started_tx.send(());
                    release_rx.await.unwrap();
                    first_runtime
                        .navigate_browser_environment_history(
                            &first_room,
                            "00000000000000000000000000000101",
                            &first_tab,
                            BrowserHistoryAction::Reload,
                        )
                        .await
                },
            )
            .await
    });
    started_rx.await.unwrap();
    let next_runtime = runtime.clone();
    let next_room = room.clone();
    let next_agent = agent.clone();
    let next_tab = tab.clone();
    let next = tokio::spawn(async move {
        next_runtime
            .navigate_browser_environment_history_as_agent(
                &next_room,
                &next_agent,
                &next_tab,
                BrowserHistoryAction::Reload,
            )
            .await
    });
    let queued = timeout(Duration::from_secs(2), async {
        loop {
            if runtime
                .room_environment_snapshot(room)
                .unwrap()
                .actions
                .iter()
                .any(|a| {
                    a.kind == "browser_history_reload" && a.state == EnvironmentActionState::Queued
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    let _ = release_tx.send(());
    let first = timeout(Duration::from_secs(5), first)
        .await
        .unwrap()
        .unwrap();
    let next = timeout(Duration::from_secs(5), next)
        .await
        .unwrap()
        .unwrap();
    queued.expect("tab history waits for the predecessor's reservation");
    first.expect("predecessor changes the document");
    next.expect("tab-only history must use the document current at execution");
    let after = runtime.room_environment_snapshot(room).unwrap();
    assert_eq!(after.focused_tab_id, before.focused_tab_id);
    assert!(after.tabs[0].document_revision >= revision + 2);
    assert_eq!(after.actions.len(), 2);
    assert!(after
        .actions
        .iter()
        .all(|a| a.state == EnvironmentActionState::Completed));
    assert!(after.actions[1].started_at_ms >= after.actions[0].finished_at_ms);
    let physical: Value = serde_json::from_slice(
        &std::fs::read(fixture._worker_state.root.join("chromium-state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        physical["reloadCount"], 2,
        "each queued action has one physical effect"
    );
}
