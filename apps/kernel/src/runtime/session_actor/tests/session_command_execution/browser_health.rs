use super::*;
use crate::session::{
    EnvironmentComponent as Component, EnvironmentComponentHealthState as Health,
    EnvironmentLifecycle as Lifecycle,
};

async fn fixture() -> (KernelRuntimeState, String, TestBrowserControllerTool) {
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
    let mut state = owned_runtime_state(&app).await;
    state.set_browser_controller_process_store_for_test(
        crate::runtime::browser_controller_process::BrowserControllerProcessStore::new(
            &tool.path,
            Vec::new(),
            Duration::from_secs(5),
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
    (state, session_id, tool)
}

#[tokio::test]
async fn room_browser_health_detects_loss_and_recovers_without_controller_restart() {
    let (state, room, tool) = fixture().await;
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
    let (state, room, _tool) = fixture().await;
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
