use super::*;
use crate::session::EnvironmentLifecycle;
use futures_util::FutureExt;

#[test]
fn room_start_admission_timeout_recovers_through_health_refresh() {
    run_test(start_admission_timeout_recovers);
}

#[test]
fn room_recovery_activation_preserves_focus_intent_across_relay_observations() {
    run_test(recovery_activation_preserves_focus);
}

async fn recovery_activation_preserves_focus() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let assertions = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        fixture
            .home
            .app
            .lock()
            .await
            .slices()
            .set_status("desktop", SliceStatus::Running, 1)
            .unwrap();
        let room = &fixture.rooms[0];
        dispatch_json(&fixture.home, bind(room, "desktop"))
            .await
            .unwrap();
        dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id": room, "viewport": {
                    "css_width": 1280, "css_height": 800, "device_scale_factor": 1,
                    "desktop_pixel_width": 1280, "desktop_pixel_height": 800
                }
            }}),
        )
        .await
        .unwrap();
        let runtime = &fixture.home.runtime_state;
        let mark = |target: &str| {
            std::collections::BTreeMap::from([(
                target.into(),
                ("app_1".into(), crate::session::AppPanelLayout::default()),
            )])
        };
        runtime
            .set_room_environment_app_tabs(room, mark("worker-tab"), true)
            .unwrap();
        let original = runtime
            .room_environment_snapshot(room)
            .unwrap()
            .focused_tab_id
            .unwrap();
        runtime.room_environment_prepare_app_recovery(room).unwrap();
        runtime
            .set_room_environment_app_tabs(room, Default::default(), true)
            .unwrap();
        runtime
            .reconcile_room_environment_controller_tabs(room, vec![], None)
            .unwrap();
        std::fs::write(
            fixture
                ._worker_state
                .root
                .join("recovery-blank-and-app-tabs"),
            "new physical pages",
        )
        .unwrap();
        let physical = runtime
            .reconcile_browser_controller_environment(room)
            .await
            .unwrap();
        let blank = physical.focused_tab_id.clone().unwrap();
        let observations = physical
            .tabs
            .iter()
            .map(|tab| {
                let binding = runtime
                    .room_environment_controller_tab_binding(room, &tab.tab_id)
                    .unwrap();
                crate::session::EnvironmentTabObservation {
                    runtime_target_id: binding.runtime_target_id,
                    document_id: binding.document_id,
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                }
            })
            .collect::<Vec<_>>();
        // Publish a verified App binding, then execute the production restore
        // activation through the real encrypted home/worker controller route.
        runtime
            .set_room_environment_app_tabs(room, mark("worker-popup"), true)
            .unwrap();
        assert_eq!(
            runtime
                .room_environment_snapshot(room)
                .unwrap()
                .focused_tab_id
                .as_deref(),
            Some(original.as_str())
        );
        let activated = runtime
            .restore_browser_environment_tab_focus(room, "restore-focus", &original)
            .await
            .unwrap();
        assert_eq!(activated.focused_tab_id.as_deref(), Some(original.as_str()));
        let chromium: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture._worker_state.root.join("chromium-state.json")).unwrap(),
        )
        .unwrap();
        assert!(chromium["activateCount"].as_u64().unwrap() > 0);
        assert_eq!(chromium["focusedTarget"], "worker-popup");
        runtime
            .reconcile_room_environment_controller_tabs(
                room,
                observations.clone(),
                Some("worker-tab"),
            )
            .unwrap();
        assert_eq!(
            runtime
                .room_environment_snapshot(room)
                .unwrap()
                .focused_tab_id
                .as_deref(),
            Some(original.as_str()),
            "late startup-blank receipt must not erase recovery activation"
        );
        // The regular Tab path is still an explicit choice and cancels intent.
        runtime
            .manage_browser_environment_tab(
                room,
                "choose-blank",
                &blank,
                crate::runtime::browser_controller_tab::BrowserTabAction::Activate,
            )
            .await
            .unwrap();
        runtime
            .reconcile_room_environment_controller_tabs(room, observations, Some("worker-tab"))
            .unwrap();
        assert_eq!(
            runtime
                .room_environment_snapshot(room)
                .unwrap()
                .focused_tab_id
                .as_deref(),
            Some(blank.as_str())
        );
    })
    .catch_unwind()
    .await;
    let cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    fixture.stop().await;
    cleanup.unwrap();
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn room_start_capture_failure_crosses_relay_and_recovers_through_health() {
    run_test(capture_failure_recovers);
}

async fn capture_failure_recovers() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let assertions = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        fixture
            .home
            .app
            .lock()
            .await
            .slices()
            .set_status("desktop", SliceStatus::Running, 1)
            .unwrap();
        let room = &fixture.rooms[0];
        dispatch_json(&fixture.home, bind(room, "desktop"))
            .await
            .unwrap();
        dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id": room, "viewport": {
                    "css_width": 1280, "css_height": 800, "device_scale_factor": 1,
                    "desktop_pixel_width": 1280, "desktop_pixel_height": 800
                }
            }}),
        )
        .await
        .unwrap();
        let runtime = &fixture.home.runtime_state;
        let before = runtime.room_environment_snapshot(room).unwrap();
        assert_eq!(before.lifecycle, EnvironmentLifecycle::Ready);
        runtime
            .transition_room_environment(room, EnvironmentLifecycle::Degraded)
            .unwrap();
        let fault = fixture
            ._worker_state
            .root
            .join("unavailable-canonical-capture");
        std::fs::write(&fault, "temporary capture loss").unwrap();
        let error = timeout(
            Duration::from_secs(35),
            runtime.finish_room_environment_controller_start(room, "test.start"),
        )
        .await
        .unwrap()
        .expect_err("worker capture is unavailable");
        assert!(
            matches!(error, DaemonError::RelayTransport {
            operation: "read relay peer response", ref code, ref message, ..
        } if code == "transport_error" && message.contains("failed with viewport_apply_failed:")),
            "{error}"
        );
        assert_eq!(
            runtime.room_environment_snapshot(room).unwrap().lifecycle,
            EnvironmentLifecycle::Degraded
        );
        std::fs::remove_file(fault).unwrap();
        runtime.schedule_room_environment_health_refresh(room);
        let recovered = timeout(Duration::from_secs(10), async {
            loop {
                let environment = runtime.room_environment_snapshot(room).unwrap();
                if environment.lifecycle == EnvironmentLifecycle::Ready {
                    break environment;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("normal health poll must recover the existing slice Room");
        assert_eq!(recovered.runtime_generation, before.runtime_generation);
        assert_eq!(recovered.tabs, before.tabs);
    })
    .catch_unwind()
    .await;
    let cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    fixture.stop().await;
    cleanup.unwrap();
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
}

async fn start_admission_timeout_recovers() {
    let mut fixture = LiveWorker::start_configured(false, true).await;
    let assertions = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        let slices = fixture.home.app.lock().await.slices().clone();
        slices
            .set_status("desktop", SliceStatus::Running, 1)
            .unwrap();
        let room = &fixture.rooms[0];
        dispatch_json(&fixture.home, bind(room, "desktop"))
            .await
            .unwrap();
        dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id": room, "viewport": {
                    "css_width": 1280, "css_height": 800, "device_scale_factor": 1,
                    "desktop_pixel_width": 1280, "desktop_pixel_height": 800
                }
            }}),
        )
        .await
        .unwrap();
        let runtime = &fixture.home.runtime_state;
        let before = runtime.room_environment_snapshot(room).unwrap();
        assert_eq!(before.lifecycle, EnvironmentLifecycle::Ready);
        assert!(!before.tabs.is_empty());
        // A restored Room retains its runtime and Tabs while startup completes.
        runtime
            .transition_room_environment(room, EnvironmentLifecycle::Degraded)
            .unwrap();
        let route = slices
            .guard_environment_use("desktop", Some(room), "browser_controller.route")
            .unwrap();
        // Exercise the actual queue deadline and startup caller, without
        // injecting a fabricated transport error or replacing the worker.
        let error = timeout(
            crate::slice::ENVIRONMENT_USE_ADMISSION_TIMEOUT + Duration::from_secs(10),
            runtime.finish_room_environment_controller_start(room, "test.start"),
        )
        .await
        .unwrap()
        .expect_err("occupied route must expire before dispatch");
        assert!(matches!(error, DaemonError::LocalTransport {
            operation: "browser_controller.route", ref message,
        } if message == crate::slice::ENVIRONMENT_USE_ADMISSION_EXPIRED));
        assert_eq!(
            runtime.room_environment_snapshot(room).unwrap().lifecycle,
            EnvironmentLifecycle::Degraded
        );
        drop(route);
        runtime.schedule_room_environment_health_refresh(room);
        let recovered = timeout(Duration::from_secs(10), async {
            loop {
                let environment = runtime.room_environment_snapshot(room).unwrap();
                if environment.lifecycle == EnvironmentLifecycle::Ready {
                    break environment;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("normal health refresh must finish the existing Room start");
        assert_eq!(recovered.runtime_generation, before.runtime_generation);
        assert_eq!(recovered.tabs.len(), before.tabs.len());
        for (restored, original) in recovered.tabs.iter().zip(&before.tabs) {
            assert_eq!(restored.tab_id, original.tab_id);
            assert_eq!(restored.document_revision, original.document_revision);
        }
    })
    .catch_unwind()
    .await;
    let cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    fixture.stop().await;
    cleanup.unwrap();
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
}
