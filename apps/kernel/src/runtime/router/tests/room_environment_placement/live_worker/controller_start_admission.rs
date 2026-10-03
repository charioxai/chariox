use super::*;
use crate::session::EnvironmentLifecycle;
use futures_util::FutureExt;

#[test]
fn room_start_admission_timeout_recovers_through_health_refresh() {
    run_test(start_admission_timeout_recovers);
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
