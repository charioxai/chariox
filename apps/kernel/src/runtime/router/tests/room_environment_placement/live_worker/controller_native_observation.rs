//! MP-08 / MP-10 / MP-11: native navigation refreshes observation, never old action authority.
use super::controller_worker_mcp::prepare_cross_worker_room_display;
use super::*;
use futures_util::FutureExt;

#[test]
fn mp08_mp10_mp11_room_read_observes_native_navigation_and_fences_old_refs() {
    crate::test_support::isolated_env_test!();
    run_test(check);
}

async fn check() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let result = std::panic::AssertUnwindSafe(async {
        let (room, _, _) = prepare_cross_worker_room_display(&fixture).await;
        let started = dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id":room, "viewport":{"css_width":1280,"css_height":800,
                "device_scale_factor":1,"desktop_pixel_width":1280,"desktop_pixel_height":800}
            }}),
        )
        .await
        .unwrap();
        let tab = started["RoomEnvironmentUpdated"]["environment"]["focused_tab_id"]
            .as_str()
            .unwrap()
            .to_string();
        let runtime = &fixture.home.runtime_state;
        let before = runtime
            .capture_browser_environment_snapshot(&room, &tab)
            .await
            .unwrap();
        let old_ref = before
            .dom_nodes
            .iter()
            .find(|n| n.node_name == "BUTTON")
            .unwrap()
            .element_ref
            .clone();
        std::fs::write(
            fixture
                ._worker_state
                .root
                .join("external-browser-navigation"),
            "https://native.worker.test/",
        )
        .unwrap();
        let after = runtime
            .capture_browser_environment_snapshot(&room, &tab)
            .await
            .expect(
                "MP-08 current Room read observes native document instead of refusing old binding",
            );
        let state = runtime.room_environment_snapshot(&room).unwrap();
        assert_eq!(
            state.tabs.iter().find(|t| t.tab_id == tab).unwrap().url,
            "https://native.worker.test/"
        );
        assert_eq!(after.tab_id, before.tab_id);
        assert_eq!(after.runtime_generation, before.runtime_generation);
        assert!(after.document_revision > before.document_revision);
        assert_eq!(
            runtime
                .resolve_room_environment_element_reference(&room, &old_ref)
                .unwrap_err()
                .code(),
            "environment_stale_element_reference"
        );
        dispatch_json(
            &fixture.home,
            json!({"StopRoomEnvironment":{"session_id":room}}),
        )
        .await
        .unwrap();
        let pids = std::fs::read(fixture._worker_state.root.join("controller.pids")).unwrap();
        assert!(runtime
            .capture_browser_environment_snapshot(&room, &tab)
            .await
            .is_err());
        assert_eq!(
            std::fs::read(fixture._worker_state.root.join("controller.pids")).unwrap(),
            pids,
            "MP-11 stopped read cannot restart controller"
        );
    })
    .catch_unwind()
    .await;
    fixture.stop().await;
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
