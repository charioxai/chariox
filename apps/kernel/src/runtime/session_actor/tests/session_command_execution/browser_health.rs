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
            "if [ -f '{}' ]; then printf '{{\"id\":%s,\"ok\":false,\"error\":{{\"code\":\"controller_busy\",\"message\":\"foreground command pending\"}}}}\\n' \"$id\"; continue; fi\nif [ -f '{}' ]; then sleep 6; fi\nif [ -f '{}' ]; then sleep 2; fi\nprintf 'reconcile\\n'",
            tool.root.join("browser-busy").display(),
            tool.root.join("browser-slow").display(),
            tool.root.join("browser-delayed").display(),
        ),
    );
    std::fs::write(&tool.path, script).unwrap();
    let script = std::fs::read_to_string(&tool.path).unwrap().replace(
        "    *'\"method\":\"shutdown\"'*)",
        r#"    *'"method":"browser.cookies.recover"'*)
      printf '{"id":%s,"ok":true,"result":{"status":"verified"}}\n' "$id"
      ;;
    *'"method":"shutdown"'*)"#,
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
                source_slice_ref: None,
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
            browser_bar_visible: state
                .room_environment_snapshot(&room)
                .unwrap()
                .browser_bar_visible,
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
            Some(tokio::time::Instant::now() + std::time::Duration::from_secs(5)),
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

#[tokio::test]
async fn ready_state_read_uses_health_only_and_preserves_foreground_tabs() {
    let (state, room, tool, _) = fixture().await;
    // The fixture's reconciliation always reports target-a. A foreground tab
    // projection after dispatch must survive the asynchronous read receipt.
    let before = std::fs::read_to_string(&tool.log).unwrap();
    std::fs::write(tool.root.join("browser-delayed"), "").unwrap();
    state.schedule_room_environment_health_refresh(&room);
    tokio::time::sleep(Duration::from_millis(200)).await;
    state
        .reconcile_room_environment_controller_tabs(&room, Vec::new(), None)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(9), async {
        while std::fs::read_to_string(&tool.log).unwrap() == before {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    })
    .await
    .unwrap();
    assert!(state
        .room_environment_snapshot(&room)
        .unwrap()
        .tabs
        .is_empty());
}

#[tokio::test]
async fn ready_state_read_ignores_busy_but_degrades_positive_browser_loss() {
    let (state, room, tool, _) = fixture().await;
    std::fs::write(tool.root.join("browser-busy"), "").unwrap();
    state.schedule_room_environment_health_refresh(&room);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        state.room_environment_snapshot(&room).unwrap().lifecycle,
        Lifecycle::Ready
    );
    std::fs::remove_file(tool.root.join("browser-busy")).unwrap();
    std::fs::write(tool.root.join("browser-exited"), "").unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            state.schedule_room_environment_health_refresh(&room);
            if state.room_environment_snapshot(&room).unwrap().lifecycle == Lifecycle::Degraded {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ready_state_read_recovers_pending_import() {
    let (state, room, _tool, _) = fixture().await;
    let guard = state
        .begin_exclusive_browser_import(&room, "11111111111111111111111111111111", "local")
        .await
        .unwrap();
    drop(guard);
    assert!(state
        .ensure_browser_import_execution_allowed(&room)
        .is_err());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            state.schedule_room_environment_health_refresh(&room);
            if state.ensure_browser_import_execution_allowed(&room).is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        state.room_environment_snapshot(&room).unwrap().lifecycle,
        Lifecycle::Ready
    );
}

#[tokio::test]
async fn browser_health_receipts_classify_routes_and_keep_transient_failures_inconclusive() {
    use crate::error::DaemonError;
    let relay = |code: &str, message: &str| DaemonError::RelayTransport {
        operation: "read relay peer response",
        code: code.into(),
        message: message.into(),
        retryable: true,
    };
    let local = |message: &str| DaemonError::LocalTransport {
        operation: "browser_controller.route",
        message: message.into(),
    };
    for (error, lost) in [
        (relay("target_not_connected", "worker offline"), true),
        (relay("target_disconnected", "worker disconnected"), true),
        (relay("target_not_allowed", "target denied"), true),
        (
            relay(
                "transport_error",
                "browser_controller_scope_denied: wrong Room",
            ),
            true,
        ),
        (local("browser_controller_scope_denied: wrong Room"), true),
        (relay("transport_error", "temporary transport error"), false),
        (local("controller_busy: foreground command pending"), false),
        (local("request timed out"), false),
    ] {
        let (state, room, _tool, _) = fixture().await;
        let before = state.room_environment_snapshot(&room).unwrap();
        // This is the same receipt consumer called by refresh after real relay
        // delivery. Typed errors cannot be produced by the local stdio tool.
        state.observe_room_browser_health_receipt(
            &room,
            state
                .admit_room_browser_health_probe(&room, before.runtime_generation)
                .unwrap(),
            Err(error),
        );
        let after = state.room_environment_snapshot(&room).unwrap();
        if lost {
            assert_eq!(after.lifecycle, Lifecycle::Degraded);
            let controller = after
                .health
                .iter()
                .find(|h| h.component == Component::BrowserController)
                .unwrap();
            assert_eq!(controller.state, Health::Unavailable);
            assert_eq!(
                controller.diagnostic_code.as_deref(),
                Some("browser_controller_unreachable")
            );
        } else {
            assert_eq!(after, before, "transient receipts must remain inconclusive");
        }
    }
}

// MP-08/MP-10: a worker can restart its controller during a background probe.
// The receipt must attribute that loss without projecting stale tabs/viewport.
#[tokio::test]
async fn browser_health_receipt_attributes_controller_generation_without_stale_projection() {
    use crate::runtime::browser_controller_process::BrowserControllerReconciliation;
    use crate::transport::room_browser_controller::RoomBrowserControllerResult;
    let (state, room, _tool, _) = fixture().await;
    let before = state.room_environment_snapshot(&room).unwrap();
    let mut process = state
        .ensure_browser_controller_process_started(&room)
        .await
        .unwrap()
        .unwrap();
    process.runtime_generation += 1;
    let reconciliation = BrowserControllerReconciliation {
        process,
        // Deliberately stale/empty browser data: background health cannot own it.
        browser: serde_json::from_value(serde_json::json!({
            "browser_generation": 1, "event_cursor": 1, "tabs": [],
            "focused_target_id": null,
            "resource_inventory": {"browser_ids": [], "profile_ids": []},
            "viewport": {"css_width": 10, "css_height": 10, "device_scale_factor": 1,
                "desktop_pixel_width": 10, "desktop_pixel_height": 10}
        }))
        .unwrap(),
    };
    let admission = state
        .admit_room_browser_health_probe(&room, before.runtime_generation)
        .unwrap();
    let receipt = || {
        Ok(RoomBrowserControllerResult::Reconciled {
            reconciliation: Some(reconciliation.clone()),
        })
    };
    state.observe_room_browser_health_receipt(&room, admission, receipt());
    let attributed = state.room_environment_snapshot(&room).unwrap();
    let controller = attributed
        .health
        .iter()
        .find(|health| health.component == Component::BrowserController)
        .unwrap();
    assert_eq!(controller.state, Health::Starting);
    assert_eq!(
        controller.diagnostic_code.as_deref(),
        Some("controller_restarted")
    );
    assert_eq!(attributed.lifecycle, Lifecycle::Degraded);
    assert_eq!(attributed.tabs, before.tabs);
    assert_eq!(attributed.viewport, before.viewport);
    assert_eq!(attributed.runtime_generation, before.runtime_generation);
    state.observe_room_browser_health_receipt(&room, admission, receipt());
    assert_eq!(
        state.room_environment_snapshot(&room).unwrap(),
        attributed,
        "same controller generation must not reopen recovery or clear its fence"
    );
    state.stop_room_environment(&room).unwrap();
    let stopped = state.room_environment_snapshot(&room).unwrap();
    state.observe_room_browser_health_receipt(&room, admission, receipt());
    assert_eq!(
        state.room_environment_snapshot(&room).unwrap(),
        stopped,
        "late controller receipts cannot resurrect a stopped Room"
    );
    state
        .start_room_environment(&room, before.viewport)
        .unwrap();
    state
        .transition_room_environment(&room, Lifecycle::Ready)
        .unwrap();
    let newer = state.room_environment_snapshot(&room).unwrap();
    state.observe_room_browser_health_receipt(&room, admission, receipt());
    assert_eq!(
        state.room_environment_snapshot(&room).unwrap(),
        newer,
        "late controller receipts cannot mutate a newer Room generation"
    );
}

// MP-08 / MP-10: A foreground recovery supersedes an in-flight health receipt.
#[tokio::test]
async fn browser_health_receipt_after_foreground_recovery_cannot_reopen_recovery() {
    use crate::runtime::browser_controller_process::BrowserControllerReconciliation;
    use crate::transport::room_browser_controller::RoomBrowserControllerResult;
    let (state, room, _tool, _) = fixture().await;
    let before = state.room_environment_snapshot(&room).unwrap();
    let process = state
        .ensure_browser_controller_process_started(&room)
        .await
        .unwrap()
        .unwrap();
    let old_receipt = BrowserControllerReconciliation {
        process: process.clone(),
        browser: serde_json::from_value(serde_json::json!({
            "browser_generation": 1, "event_cursor": 1, "tabs": [],
            "focused_target_id": null,
            "resource_inventory": {"browser_ids": [], "profile_ids": []},
            "viewport": {"css_width": 1280, "css_height": 800, "device_scale_factor": 1,
                "desktop_pixel_width": 1280, "desktop_pixel_height": 800}
        }))
        .unwrap(),
    };
    let admission = state
        .admit_room_browser_health_probe(&room, before.runtime_generation)
        .unwrap();
    let mut newer = old_receipt.clone();
    newer.process.runtime_generation += 1;
    newer.browser.browser_generation += 1;
    state
        .observe_browser_controller_reconciliation(&room, newer)
        .unwrap();
    let recovered = state.room_environment_snapshot(&room).unwrap();
    assert_eq!(recovered.runtime_generation, before.runtime_generation);
    assert_eq!(recovered.lifecycle, Lifecycle::Ready);
    assert!(recovered
        .health
        .iter()
        .all(|health| health.state == Health::Ready));
    state.observe_room_browser_health_receipt(
        &room,
        admission,
        Ok(RoomBrowserControllerResult::Reconciled {
            reconciliation: Some(old_receipt),
        }),
    );
    assert_eq!(state.room_environment_snapshot(&room).unwrap(), recovered,
        "late controller A receipt must not degrade recovered controller B or invalidate references");
    // The foreground B observation must still be authoritative afterward.
    let mut fresh = process;
    fresh.runtime_generation += 1;
    let fresh = BrowserControllerReconciliation {
        process: fresh,
        browser: serde_json::from_value(serde_json::json!({
            "browser_generation": 2, "event_cursor": 1, "tabs": [],
            "focused_target_id": null,
            "resource_inventory": {"browser_ids": [], "profile_ids": []},
            "viewport": {"css_width": 1280, "css_height": 800, "device_scale_factor": 1,
                "desktop_pixel_width": 1280, "desktop_pixel_height": 800}
        }))
        .unwrap(),
    };
    assert_eq!(
        state
            .observe_browser_controller_reconciliation(&room, fresh)
            .unwrap(),
        recovered
    );
}
