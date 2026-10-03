use super::*;
use crate::session::{
    EnvironmentComponent as Component, EnvironmentComponentHealthState as Health,
    EnvironmentLifecycle as Lifecycle,
};

async fn fixture() -> (
    KernelRuntimeState,
    String,
    TestBrowserControllerTool,
    crate::slice::SliceStore,
) {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap(),
    ));
    let session_id = {
        let mut app = app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "browser-health-workspace",
                "browser-health-worktree",
            ))
            .unwrap()
            .0
            .id()
            .to_owned()
    };
    let tool = TestBrowserControllerTool::new();
    let script = std::fs::read_to_string(&tool.path).unwrap();
    let script = script.replace("printf 'reconcile\\n'", &format!("if [ -f '{}' ]; then printf '{{\"id\":%s,\"ok\":false,\"error\":{{\"code\":\"browser_debugger_unavailable\",\"message\":\"browser exited\"}}}}\\n' \"$id\"; continue; fi\nprintf 'reconcile\\n'", tool.root.join("browser-exited").display()));
    std::fs::write(&tool.path, script).unwrap();
    let script = std::fs::read_to_string(&tool.path).unwrap().replace(
        "printf 'reconcile\\n'",
        &format!(
            "if [ -f '{}' ]; then printf '{{\"id\":%s,\"ok\":false,\"error\":{{\"code\":\"controller_busy\",\"message\":\"foreground command pending\"}}}}\\n' \"$id\"; continue; fi\nif [ -f '{}' ]; then sleep 6; fi\nprintf 'reconcile\\n'",
            tool.root.join("browser-busy").display(),
            tool.root.join("browser-slow").display(),
        ),
    );
    std::fs::write(&tool.path, script).unwrap();
    let mut state = owned_runtime_state(&app).await;
    state.set_browser_controller_process_store_for_test(
        crate::runtime::browser_controller_process::BrowserControllerProcessStore::new(
            &tool.path,
            Vec::new(),
            Duration::from_secs(10),
        ),
    );
    state
        .start_room_environment(
            &session_id,
            crate::session::CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap(),
        )
        .unwrap();
    state
        .finish_room_environment_controller_start(&session_id, "test.start")
        .await
        .unwrap();
    for component in [
        Component::Browser,
        Component::BrowserController,
        Component::Desktop,
        Component::Streamer,
    ] {
        state
            .update_room_environment_component_health(&session_id, component, Health::Ready, None)
            .unwrap();
    }
    state
        .transition_room_environment(&session_id, Lifecycle::Ready)
        .unwrap();
    let slices = app.lock().await.slices();
    (state, session_id, tool, slices)
}

#[tokio::test]
async fn room_browser_health_detects_loss_and_recovers_without_controller_restart() {
    let (state, room, tool, _) = fixture().await;
    let before = state.room_environment_snapshot(&room).unwrap();
    std::fs::write(tool.root.join("browser-exited"), "").unwrap();
    state
        .refresh_room_browser_health(&room, before.runtime_generation)
        .await;
    let failed = state.room_environment_snapshot(&room).unwrap();
    assert_eq!(failed.lifecycle, Lifecycle::Degraded);
    let health = failed
        .health
        .iter()
        .find(|h| h.component == Component::Browser)
        .unwrap();
    assert_eq!(health.state, Health::Unavailable);
    assert_eq!(
        health.diagnostic_code.as_deref(),
        Some("browser_debugger_unavailable")
    );
    std::fs::remove_file(tool.root.join("browser-exited")).unwrap();
    state
        .refresh_room_browser_health(&room, before.runtime_generation)
        .await;
    let recovered = state.room_environment_snapshot(&room).unwrap();
    assert_eq!(recovered.lifecycle, Lifecycle::Ready);
    assert!(recovered.health.iter().all(|h| h.state == Health::Ready));
    assert_eq!(recovered.runtime_generation, before.runtime_generation);
    assert_eq!(
        std::fs::read_to_string(&tool.log)
            .unwrap()
            .lines()
            .filter(|line| *line == "start")
            .count(),
        1
    );
}

#[tokio::test]
async fn room_browser_health_ignores_stopped_and_new_generation_receipts() {
    let (state, room, _tool, _) = fixture().await;
    let before = state.room_environment_snapshot(&room).unwrap();
    state.stop_room_environment(&room).unwrap();
    state.observe_room_browser_health(&room, before.runtime_generation, None);
    assert_eq!(
        state.room_environment_snapshot(&room).unwrap().lifecycle,
        Lifecycle::Stopped
    );
    state
        .start_room_environment(&room, before.viewport)
        .unwrap();
    state
        .transition_room_environment(&room, Lifecycle::Ready)
        .unwrap();
    let restarted = state.room_environment_snapshot(&room).unwrap();
    assert_ne!(restarted.runtime_generation, before.runtime_generation);
    state.observe_room_browser_health(
        &room,
        before.runtime_generation,
        Some("browser_debugger_unavailable"),
    );
    assert_eq!(state.room_environment_snapshot(&room).unwrap(), restarted);
}

#[tokio::test]
async fn room_browser_health_timeout_and_route_errors_are_inconclusive() {
    let (state, room, tool, _) = fixture().await;
    let before = state.room_environment_snapshot(&room).unwrap();
    std::fs::write(tool.root.join("browser-busy"), "").unwrap();
    state
        .refresh_room_browser_health(&room, before.runtime_generation)
        .await;
    assert_eq!(state.room_environment_snapshot(&room).unwrap(), before);
    std::fs::remove_file(tool.root.join("browser-busy")).unwrap();
    let reconciles = || {
        std::fs::read_to_string(&tool.log)
            .unwrap()
            .lines()
            .filter(|line| *line == "reconcile")
            .count()
    };
    let count = reconciles();
    std::fs::write(tool.root.join("browser-slow"), "").unwrap();
    let started = std::time::Instant::now();
    state
        .refresh_room_browser_health(&room, before.runtime_generation)
        .await;
    assert!(started.elapsed() >= Duration::from_secs(4));
    assert_eq!(
        state.room_environment_snapshot(&room).unwrap(),
        before,
        "a healthy browser waiting on the serial controller queue must stay ready"
    );
    let next = std::time::Instant::now();
    state
        .refresh_room_browser_health(&room, before.runtime_generation)
        .await;
    assert!(
        next.elapsed() < Duration::from_secs(1),
        "skip a Room while its timed-out underlying query still runs"
    );
    std::fs::remove_file(tool.root.join("browser-slow")).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        state
            .refresh_room_browser_health(&room, before.runtime_generation)
            .await;
        if reconciles() == count + 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "probe ownership must end after its underlying receipt"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        reconciles(),
        count + 2,
        "only the original slow query and one subsequent query run"
    );
    assert_eq!(state.room_environment_snapshot(&room).unwrap(), before);
}

#[tokio::test]
async fn room_browser_health_probe_shares_controller_routes_and_yields_to_slice_lifecycle() {
    let (state, room, _tool, slices) = fixture().await;
    let slice = slices
        .create(
            "health-home",
            "health-machine",
            crate::slice::CreateSliceInput {
                name: "health-admission".into(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".into(),
                display_mode: crate::slice::SliceDisplayMode::Headed,
                display_backend: crate::slice::SliceDisplayBackend::Novnc,
                workspace_id: None,
                worktree_id: None,
                workspace_mount: None,
                development: None,
                worker_kernel_ref: Some("health-worker".into()),
                display_url: None,
                provider_auth: vec![],
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .unwrap();
    let slice = slices
        .bind_environment(&room, &slice.id, 2, |_| Ok(()))
        .unwrap();
    let slice = slices
        .set_status(&slice.id, crate::slice::SliceStatus::Running, 3)
        .unwrap();
    let command =
        crate::transport::room_browser_controller::RoomBrowserControllerCommand::Reconcile {
            viewport: state.room_environment_snapshot(&room).unwrap().viewport,
        };
    let probe = state
        .admit_room_browser_controller_route(&room, &slice.id, &command, true, None)
        .await
        .unwrap();
    assert!(
        probe.1.is_none(),
        "background health must never own the exclusive operation slot"
    );
    let foreground = state
        .admit_room_browser_controller_route(
            &room,
            &slice.id,
            &command,
            false,
            Some(tokio::time::Instant::now()),
        )
        .await
        .unwrap();
    assert!(
        foreground.1.is_some(),
        "a foreground command must remain admissible during a probe"
    );
    assert!(state
        .admit_room_browser_controller_route(&room, &slice.id, &command, true, None)
        .await
        .unwrap()
        .1
        .is_none());
    drop(foreground);
    let lifecycle = slices.try_begin_operation(&slice.id, "slice.stop").unwrap();
    assert!(
        state
            .admit_room_browser_controller_route(&room, &slice.id, &command, true, None)
            .await
            .is_err(),
        "health must yield to actual slice lifecycle authority"
    );
    drop(lifecycle);
}
