use super::*;
use futures_util::FutureExt;

pub(super) fn process_store(
    root: &std::path::Path,
) -> crate::runtime::browser_controller_process::BrowserControllerProcessStore {
    let kernel = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    crate::runtime::browser_controller_process::BrowserControllerProcessStore::new(
        "node",
        vec![
            kernel.join("src/runtime/router/tests/room_environment_placement/live_worker/controller.fixture.mjs").display().to_string(),
            kernel.join("slice-linux-docker/docker").display().to_string(),
            root.join("controller.pid").display().to_string(),
        ],
        Duration::from_secs(3),
    )
}

#[test]
fn room_environment_controller_uses_its_slice_without_worker_agents() {
    run_test(uses_its_slice_without_worker_agents);
}

async fn uses_its_slice_without_worker_agents() {
    controller_scenario(false).await;
}

#[test]
fn room_environment_controller_uses_its_private_slice_relay() {
    run_test(uses_private_slice_relay);
}

async fn uses_private_slice_relay() {
    controller_scenario(true).await;
}

#[test]
fn room_environment_controller_rejects_unprovisioned_worker() {
    run_test(rejects_unprovisioned_worker);
}

async fn rejects_unprovisioned_worker() {
    let mut fixture = LiveWorker::start().await;
    let result = std::panic::AssertUnwindSafe(async {
        fixture.create_slice().await;
        let room = &fixture.rooms[0];
        dispatch_json(
            &fixture.home,
            json!({"BindRoomEnvironmentSlice": {
                "session_id":room,"slice_ref":"desktop"
            }}),
        )
        .await
        .unwrap();
        let error = dispatch_json(
            &fixture.home,
            json!({"StartRoomEnvironment": {
                "session_id":room,"viewport":{
                    "css_width":1280,"css_height":800,"device_scale_factor":1,
                    "desktop_pixel_width":1280,"desktop_pixel_height":800
                }
            }}),
        )
        .await
        .expect_err("unprovisioned worker must not yield a fake healthy Environment");
        assert!(
            error
                .to_string()
                .contains("browser_controller_scope_denied"),
            "{error}"
        );
    })
    .catch_unwind()
    .await;
    fixture.stop().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn room_environment_controller_boot_rejects_invalid_binding() {
    let state = TestState::new();
    for mismatch in ["key", "room", "kernel", "slice", "machine"] {
        let mut config = state.config.clone();
        config.host_machine_id = "slice:slice-1".into();
        let mut binding = crate::config::RoomEnvironmentWorkerBinding {
            home_kernel_id: "home".into(),
            home_public_key: config.relay_public_key.clone(),
            session_id: "room-1".into(),
            slice_id: "slice-1".into(),
        };
        match mismatch {
            "key" => binding.home_public_key = "not-a-relay-key".into(),
            "room" => binding.session_id.clear(),
            "kernel" => binding.home_kernel_id.clear(),
            "slice" => binding.slice_id.clear(),
            "machine" => config.host_machine_id = "another-machine".into(),
            _ => unreachable!(),
        }
        config.room_environment_worker_binding = Some(binding);
        assert!(
            matches!(
                DaemonApp::bootstrap(config),
                Err(DaemonError::InvalidConfig {
                    field: "room_environment_worker_binding",
                    ..
                })
            ),
            "{mismatch} must fail at worker boot"
        );
    }
}

pub(super) async fn controller_scenario(private_relay: bool) {
    let mut fixture = LiveWorker::start_configured(private_relay, true).await;
    let result = std::panic::AssertUnwindSafe(check_slice_controller(&fixture))
        .catch_unwind()
        .await;
    let pid = std::fs::read_to_string(fixture._worker_state.root.join("controller.pid"))
        .ok()
        .map(|pid| pid.trim().parse::<u32>().expect("controller PID"));
    let cleanup = fixture
        .worker
        .runtime_state
        .shutdown_browser_controller_process()
        .await;
    fixture.stop().await;
    cleanup.expect("stop fixture controller on success and failure");
    if let Some(pid) = pid {
        eprintln!("relay controller fixture PID: {pid}");
        assert!(
            !crate::runtime::process_health::process_running(pid),
            "controller must be reaped"
        );
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn check_slice_controller(fixture: &LiveWorker) {
    fixture.create_slice().await;
    let room = &fixture.rooms[0];
    let original = dispatch_json(
        &fixture.home,
        json!({"BindRoomEnvironmentSlice": {
            "session_id":room, "slice_ref":"desktop"
        }}),
    )
    .await
    .unwrap();
    let request = json!({"StartRoomEnvironment": {
        "session_id":room, "viewport": {
            "css_width":1280, "css_height":800, "device_scale_factor":1,
            "desktop_pixel_width":1280, "desktop_pixel_height":800
        }
    }});
    let first = dispatch_json(&fixture.home, request.clone())
        .await
        .expect("start bound Room browser without an execution lease");
    let environment = &first["RoomEnvironmentUpdated"]["environment"];
    assert_eq!(
        environment["tabs"][0]["url"], "https://worker.test/",
        "Room tabs must come from its bound worker controller, not an empty home store"
    );
    let token = {
        let mut app = fixture.home.app.lock().await;
        let agent = spawn_test_agent(&mut app, room, "browser-reader", "dev-stub");
        launch_test_provider(&mut app, room, agent.id(), "dev-stub", "dev-stub", "test")
            .runtime_mcp_auth_token()
            .unwrap()
            .to_string()
    };
    let status = fixture
        .home
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(&token, "slice_browser_status", json!({}))
        .await
        .expect("home Room agent reads its bound worker browser through runtime MCP");
    assert!(status.ok, "{:?}", status.payload);
    assert_eq!(status.payload["session_id"], *room);
    assert_eq!(status.payload["tab_id"], environment["tabs"][0]["tab_id"]);
    assert_eq!(
        status.payload["browser"]["buttons"][0]["label"],
        "Save on worker"
    );
    assert!(status.payload["browser"]["buttons"][0]["field_id"]
        .as_str()
        .is_some_and(|reference| reference.starts_with("element-")));
    assert!(fixture
        .home
        .runtime_state
        .runtime_tool_specs_for_auth_token(&token)
        .iter()
        .any(|spec| spec.name == "slice_browser_status"));
    super::controller_observations::check(fixture, &token, &status.payload).await;
    super::controller_mutations::check(fixture, &token, &status.payload).await;
    super::controller_cancellation::check(fixture, &token, &status.payload).await;
    super::controller_cancellation::check_running(fixture, &token, &status.payload).await;
    // A worker-local Room can even have the same textual session ID as the
    // home Room. It must not claim a provisioned browser via the local API.
    let local_room = {
        let mut app = fixture.worker.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .unwrap()
            .0
            .id()
            .to_string()
    };
    let local_error = dispatch_json(
        &fixture.worker,
        json!({"StartRoomEnvironment":{
            "session_id":local_room,"viewport":{
                "css_width":1280,"css_height":800,"device_scale_factor":1,
                "desktop_pixel_width":1280,"desktop_pixel_height":800
            }
        }}),
    )
    .await
    .expect_err("worker-local Room must not bypass home authorization");
    assert!(
        local_error
            .to_string()
            .contains("browser_controller_scope_denied"),
        "{local_error}"
    );
    let slice = fixture
        .home
        .app
        .lock()
        .await
        .slices()
        .environment_slice(room)
        .unwrap();
    let owner = fixture
        .home_state
        .config
        .slice_relay_override(&slice)
        .unwrap_or_else(|| fixture.home_state.config.clone());
    // Neither a claimed kernel ID nor possession of the relay token grants
    // control of the browser. Every denied release must leave the owner intact.
    for mismatch in ["room", "slice", "kernel", "key"] {
        let mut sender = owner.clone();
        if mismatch == "kernel" {
            sender.daemon_id = "different-home".into();
        }
        if mismatch == "key" {
            sender.relay_private_key =
                crate::transport::relay_crypto::generate_private_key_base64();
            sender.relay_public_key =
                crate::transport::relay_crypto::public_key_from_private_key_base64(
                    &sender.relay_private_key,
                )
                .unwrap();
        }
        for command in [
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::Release,
            crate::transport::room_browser_controller::RoomBrowserControllerCommand::CancelAction {
                execution_id: "00000000000000000000000000000000".into(),
            },
        ] {
            let denied = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &sender,
            chariox_relay::protocol::ClientTarget { daemon_id: Some("environment-worker".into()), daemon_alias: None },
            crate::transport::relay_peer::RelayPeerRequest::RoomBrowserController {
                session_id: if mismatch == "room" { fixture.rooms[1].clone() } else { room.clone() },
                slice_id: if mismatch == "slice" { "other-slice".into() } else { slice.id.clone() },
                command,
            },
            Duration::from_secs(3),
        ).await.expect_err("mismatched caller must not stop the owner's browser");
            assert!(
                denied
                    .to_string()
                    .contains("browser_controller_scope_denied"),
                "{mismatch}: {denied}"
            );
        }
    }
    let again = dispatch_json(&fixture.home, request).await.unwrap();
    assert_eq!(
        again["RoomEnvironmentUpdated"]["environment"]["environment_id"],
        environment["environment_id"]
    );
    assert_eq!(
        again["RoomEnvironmentUpdated"]["environment"]["tabs"],
        environment["tabs"]
    );
    dispatch_json(
        &fixture.home,
        json!({"StopRoomEnvironment":{"session_id":room}}),
    )
    .await
    .expect("stop controller through the home Room");
    assert_eq!(
        dispatch_json(&fixture.home, get(room)).await.unwrap(),
        original
    );
    dispatch_json(&fixture.home, json!({"DeleteSession":{"session_ref":room}}))
        .await
        .unwrap();
    assert!(
        dispatch_json(&fixture.home, bind(&fixture.rooms[1], "desktop"))
            .await
            .is_err(),
        "deleting a Room must not release its physical browser profile to another Room"
    );
}
