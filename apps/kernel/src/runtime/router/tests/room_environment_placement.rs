use super::*;
use crate::slice::{SliceHostRuntimeState, SliceOperationStatus, SliceStatus};
use crate::test_support::TestWorktree;
use fs2::FileExt;
use serde_json::{json, Value};

mod execution;
mod live_worker;

fn run_test<F: std::future::Future<Output = ()> + 'static>(test: fn() -> F) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let _env_guard = crate::env_lock::lock();
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .unwrap()
                .block_on(test())
        })
        .unwrap()
        .join()
        .expect("placement test thread");
}

struct TestState {
    root: std::path::PathBuf,
    config: DaemonConfig,
    worktree: TestWorktree,
}

impl TestState {
    fn new() -> Self {
        let mut config = DaemonConfig::for_tests();
        let root = std::path::PathBuf::from(config.user_config.state.path.as_ref().unwrap())
            .parent()
            .unwrap()
            .to_path_buf();
        config.local_socket_path = root.join("kernel.sock");
        config.user_config_path = root.join("config.toml");
        config = config.with_session_history_root(root.join("history"));
        config.user_config.history.operational.path =
            Some(root.join("history.db").display().to_string());
        config.user_config.artifacts.operational.root =
            Some(root.join("artifacts").display().to_string());
        config.user_config.artifacts.operational.index_path =
            Some(root.join("artifacts.db").display().to_string());
        Self {
            root,
            config,
            worktree: TestWorktree::new("room-environment-placement"),
        }
    }

    fn session_request(&self) -> CreateSessionRequest {
        self.worktree.session_request()
    }

    fn router(&self) -> (CommandRouter, Vec<String>) {
        let mut app = DaemonApp::bootstrap(self.config.clone()).expect("boot kernel");
        let rooms = (0..2)
            .map(|_| {
                crate::app::KernelSessionService::new(&mut app)
                    .create_session(self.session_request())
                    .expect("create Room")
                    .0
                    .id()
                    .to_string()
            })
            .collect();
        (
            CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2),
            rooms,
        )
    }
}

impl Drop for TestState {
    fn drop(&mut self) {
        if self.root.exists() {
            if let Err(error) = std::fs::remove_dir_all(&self.root) {
                if std::thread::panicking() {
                    // Aborted background tasks can still hold the state briefly.
                    // Preserve the original test failure instead of aborting the
                    // entire suite with a second panic during unwinding.
                    eprintln!("drill cleanup failed at {}: {error}", self.root.display());
                } else {
                    panic!("remove drill-owned kernel state: {error}");
                }
            }
        }
    }
}

async fn dispatch_json(router: &CommandRouter, request: Value) -> Result<Value, DaemonError> {
    let request: LocalDaemonRequest = serde_json::from_value(request).expect("public request");
    let command = KernelCommand::from_local_request("placement", None, None, &request);
    router
        .dispatch(command, request)
        .await
        .map(|response| serde_json::to_value(response).expect("public response"))
}

async fn wait_for_durable_owner_release(path: &std::path::Path) {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".owner.lock");
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("durable owner lock should exist after first kernel bootstrap");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match lock_file.try_lock_exclusive() {
            Ok(()) => {
                fs2::FileExt::unlock(&lock_file)
                    .expect("durable owner probe should release its lock");
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "first kernel durable owner must release before restart"
                );
                tokio::task::yield_now().await;
            }
            Err(error) => panic!("durable owner probe failed: {error}"),
        }
    }
}

#[test]
fn room_environment_placement_survives_restart_for_two_separate_rooms() {
    run_test(survives_restart_for_two_separate_rooms);
}

async fn survives_restart_for_two_separate_rooms() {
    let state = TestState::new();
    let config = state.config.clone();
    let (rooms, bindings) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first boot");
        let rooms: Vec<String> = (0..2)
            .map(|_| {
                crate::app::KernelSessionService::new(&mut app)
                    .create_session(state.session_request())
                    .expect("create Room")
                    .0
                    .id()
                    .to_string()
            })
            .collect();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let mut bindings = Vec::new();
        for (index, room) in rooms.iter().enumerate() {
            let name = format!("room-desktop-{index}");
            let created = dispatch_json(
                &router,
                json!({"CreateSlice": {
                    "name": name, "base": "clean", "display_mode": "headed"
                }}),
            )
            .await
            .expect("record-only slice creation");
            let slice_id = created["SliceCreated"]["slice"]["id"].as_str().unwrap();
            let legacy_wire = created["SliceCreated"]["slice"].clone();
            assert!(legacy_wire.get("environment_session_id").is_none());
            let legacy: crate::slice::SliceRecord =
                serde_json::from_value(legacy_wire.clone()).unwrap();
            assert_eq!(legacy.environment_session_id, None);
            assert_eq!(serde_json::to_value(legacy).unwrap(), legacy_wire);
            let binding = dispatch_json(
                &router,
                json!({"BindRoomEnvironmentSlice": {
                    "session_id": room, "slice_ref": name
                }}),
            )
            .await
            .expect("bind Room to its desktop");
            assert_eq!(
                binding["RoomEnvironmentSlice"]["binding"]["slice_id"],
                slice_id
            );
            assert_eq!(
                binding["RoomEnvironmentSlice"]["binding"]["session_id"],
                *room
            );
            assert_eq!(
                dispatch_json(
                    &router,
                    json!({"BindRoomEnvironmentSlice": {
                        "session_id": room, "slice_ref": slice_id
                    }})
                )
                .await
                .unwrap(),
                binding,
                "binding by canonical ID is idempotent"
            );
            bindings.push(binding);
        }
        router
            .shutdown_cleanup()
            .await
            .expect("first kernel shutdown cleanup");
        drop(router);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while Arc::strong_count(&app) > 1 && tokio::time::Instant::now() < deadline {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            Arc::strong_count(&app),
            1,
            "all first-kernel runtime lanes must release their app owner before restart"
        );
        drop(app);
        wait_for_durable_owner_release(&config.durable_state_path()).await;
        (rooms, bindings)
    };
    let app = DaemonApp::bootstrap(config).expect("restart kernel from durable state");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    for (room, binding) in rooms.iter().zip(bindings) {
        assert_eq!(
            dispatch_json(
                &router,
                json!({"GetRoomEnvironmentSlice": {
                    "session_id": room
                }})
            )
            .await
            .expect("read restored binding"),
            binding
        );
    }
}

async fn create_desktop(router: &CommandRouter, name: &str) {
    dispatch_json(
        router,
        json!({"CreateSlice": {
            "name": name, "base": "clean", "display_mode": "headed"
        }}),
    )
    .await
    .expect("create desktop record without a container");
}

fn bind(room: &str, slice: &str) -> Value {
    json!({"BindRoomEnvironmentSlice": {"session_id": room, "slice_ref": slice}})
}

fn get(room: &str) -> Value {
    json!({"GetRoomEnvironmentSlice": {"session_id": room}})
}

#[test]
fn room_environment_placement_rejects_ambiguous_slice_names() {
    run_test(rejects_ambiguous_slice_names);
}

async fn rejects_ambiguous_slice_names() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "slice-2").await;
    create_desktop(&router, "second").await;
    assert!(
        dispatch_json(&router, bind(&rooms[0], "slice-2"))
            .await
            .is_err(),
        "a name that also identifies another slice must not select either profile"
    );
}

#[test]
fn room_environment_placement_rejects_shared_worker_references() {
    run_test(rejects_shared_worker_references);
}

#[test]
fn room_environment_placement_allows_colocated_containers_with_distinct_worker_identities() {
    run_test(allows_colocated_containers_with_distinct_worker_identities);
}

async fn allows_colocated_containers_with_distinct_worker_identities() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    let slices = router.app.lock().await.slices().clone();
    for (index, name) in ["first", "second"].into_iter().enumerate() {
        create_desktop(&router, name).await;
        let slice = slices.resolve(name).unwrap();
        slices
            .set_worker_presence(
                name,
                Some(format!("container-kernel-{index}")),
                Some(format!("slice:{}", slice.id)),
                vec!["codex".to_string()],
                10,
            )
            .unwrap();
    }
    let first = dispatch_json(&router, json!({"GetSlice":{"slice_ref":"first"}}))
        .await
        .unwrap();
    let second = dispatch_json(&router, json!({"GetSlice":{"slice_ref":"second"}}))
        .await
        .unwrap();
    assert_eq!(
        first["Slice"]["slice"]["owner_machine_id"],
        second["Slice"]["slice"]["owner_machine_id"]
    );
    for (index, name) in ["first", "second"].into_iter().enumerate() {
        dispatch_json(&router, bind(&rooms[index], name))
            .await
            .expect("separate containers on one Docker host can belong to different Rooms");
    }
}

async fn rejects_shared_worker_references() {
    for identity in ["alias", "kernel", "machine"] {
        let state = TestState::new();
        let (router, rooms) = state.router();
        let slices = router.app.lock().await.slices().clone();
        for name in ["first", "second"] {
            dispatch_json(
                &router,
                json!({"CreateSlice":{
                    "name":name,"base":"clean","display_mode":"headed",
                    "worker_kernel_ref":if identity == "alias" { "same-worker" } else { name }
                }}),
            )
            .await
            .unwrap();
            slices
                .set_worker_presence(
                    name,
                    Some(if identity == "kernel" {
                        "same-kernel".to_string()
                    } else {
                        format!("kernel-{name}")
                    }),
                    Some(if identity == "machine" {
                        "same-worker-machine".to_string()
                    } else {
                        format!("slice:{name}")
                    }),
                    Vec::new(),
                    10,
                )
                .unwrap();
        }
        assert!(
            dispatch_json(&router, bind(&rooms[0], "first"))
                .await
                .is_err(),
            "duplicate {identity} targets cannot establish separate physical profiles"
        );
    }
}

#[test]
fn room_environment_placement_survives_stop_and_retains_deleted_room_reservation() {
    run_test(survives_stop_and_retains_deleted_room_reservation);
}

async fn survives_stop_and_retains_deleted_room_reservation() {
    live_worker::controller_placement_lifecycle().await;
}

#[test]
fn room_environment_placement_rejects_active_operations_and_other_room_agents() {
    run_test(rejects_active_operations_and_other_room_agents);
}

async fn rejects_active_operations_and_other_room_agents() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "busy").await;
    let slices = router.app.lock().await.slices().clone();
    let operation = slices.try_begin_operation("busy", "state.save").unwrap();
    assert!(dispatch_json(&router, bind(&rooms[0], "busy"))
        .await
        .is_err());
    drop(operation);
    // Configure an existing attachment; observe placement only through the public command.
    slices
        .attach_agent("busy", &rooms[1], "existing-agent", 1)
        .unwrap();
    let denied = dispatch_json(&router, bind(&rooms[0], "busy"))
        .await
        .unwrap_err();
    assert!(denied
        .to_string()
        .contains("environment_slice_binding_rejected"));
    dispatch_json(&router, bind(&rooms[1], "busy"))
        .await
        .unwrap();
}

#[test]
fn room_environment_placement_rejects_competing_claims_and_reassignment() {
    run_test(rejects_competing_claims_and_reassignment);
}

async fn rejects_competing_claims_and_reassignment() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "shared").await;
    create_desktop(&router, "other").await;
    let (first, second) = tokio::join!(
        dispatch_json(&router, bind(&rooms[0], "shared")),
        dispatch_json(&router, bind(&rooms[1], "shared")),
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "exactly one Room may claim a physical profile"
    );
    let (owner, loser) = if first.is_ok() {
        (&rooms[0], &rooms[1])
    } else {
        (&rooms[1], &rooms[0])
    };
    let original = dispatch_json(&router, get(owner)).await.unwrap();
    assert_eq!(
        dispatch_json(&router, get(loser)).await.unwrap(),
        json!({"RoomEnvironmentSlice":{"binding":null}})
    );
    assert!(dispatch_json(&router, bind(owner, "other")).await.is_err());
    assert_eq!(dispatch_json(&router, get(owner)).await.unwrap(), original);
    dispatch_json(&router, bind(loser, "other"))
        .await
        .expect("different Room can use its own physical profile");
}

#[test]
fn room_environment_placement_rejects_headless_and_missing_targets() {
    run_test(rejects_headless_and_missing_targets);
}

async fn rejects_headless_and_missing_targets() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    dispatch_json(
        &router,
        json!({"CreateSlice":{"name":"headless","base":"clean"}}),
    )
    .await
    .unwrap();
    for target in ["headless", "missing", ""] {
        assert!(dispatch_json(&router, bind(&rooms[0], target))
            .await
            .is_err());
    }
    assert_eq!(
        dispatch_json(&router, get(&rooms[0])).await.unwrap(),
        json!({"RoomEnvironmentSlice":{"binding":null}})
    );
    assert!(dispatch_json(&router, get("missing-room")).await.is_err());
}

async fn dispatch_remote(
    router: &CommandRouter,
    value: Value,
    user: Option<&str>,
) -> Result<Value, DaemonError> {
    let request: LocalDaemonRequest = serde_json::from_value(value).unwrap();
    router
        .dispatch(remote_command_for_request(&request, user), request)
        .await
        .map(|response| serde_json::to_value(response).unwrap())
}

#[test]
fn room_environment_placement_requires_room_ownership_but_members_can_read() {
    run_test(requires_room_ownership_but_members_can_read);
}

async fn requires_room_ownership_but_members_can_read() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "private").await;
    for user in [None, Some("stranger")] {
        assert!(dispatch_remote(&router, bind(&rooms[0], "private"), user)
            .await
            .is_err());
        assert!(dispatch_remote(&router, get(&rooms[0]), user)
            .await
            .is_err());
    }
    let invitation = dispatch_json(
        &router,
        json!({"CreateSessionInvite":{"session_id":rooms[0]}}),
    )
    .await
    .unwrap();
    let token = invitation["SessionInviteCreated"]["invite"]["invite_token"]
        .as_str()
        .expect("invitation token");
    dispatch_remote(
        &router,
        json!({"JoinSessionInvite":{"invite_token":token,"user_id":"collaborator"}}),
        Some("collaborator"),
    )
    .await
    .unwrap();
    let denial = dispatch_remote(&router, bind(&rooms[0], "private"), Some("collaborator"))
        .await
        .unwrap_err();
    assert!(matches!(denial, DaemonError::OwnershipAccessDenied { .. }));
    let bound = dispatch_json(&router, bind(&rooms[0], "private"))
        .await
        .unwrap();
    assert_eq!(
        dispatch_remote(&router, get(&rooms[0]), Some("collaborator"))
            .await
            .unwrap(),
        bound
    );
}

#[test]
fn room_environment_placement_does_not_publish_a_failed_durable_write() {
    run_test(does_not_publish_a_failed_durable_write);
}

async fn does_not_publish_a_failed_durable_write() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "durable").await;
    let database =
        rusqlite::Connection::open(state.config.user_config.state.path.as_ref().unwrap()).unwrap();
    database.execute_batch("CREATE TRIGGER reject_placement BEFORE INSERT ON durable_state_events
        WHEN NEW.kind = 'slice.updated' BEGIN SELECT RAISE(ABORT, 'injected storage failure'); END;").unwrap();
    assert!(dispatch_json(&router, bind(&rooms[0], "durable"))
        .await
        .is_err());
    assert_eq!(
        dispatch_json(&router, get(&rooms[0])).await.unwrap(),
        json!({"RoomEnvironmentSlice":{"binding":null}})
    );
    database
        .execute_batch("DROP TRIGGER reject_placement;")
        .unwrap();
    dispatch_json(&router, bind(&rooms[1], "durable"))
        .await
        .expect("failed commit did not reserve the profile");
}

#[test]
fn authenticated_worker_repair_append_failure_remains_unhealthy_and_retryable() {
    run_test(append_failure_remains_unhealthy_and_retryable);
}

async fn append_failure_remains_unhealthy_and_retryable() {
    let state = TestState::new();
    let (router, _) = state.router();
    create_desktop(&router, "transient-health").await;
    let slices = router.app.lock().await.slices().clone();
    let slice = slices
        .resolve("transient-health")
        .expect("transient slice should resolve");
    let worker_machine_id = format!("slice:{}", slice.id);
    slices
        .set_worker_presence(
            &slice.id,
            Some("worker-1".to_string()),
            Some(worker_machine_id.clone()),
            vec!["codex".to_string()],
            43,
        )
        .expect("worker presence should update");
    slices
        .set_status(&slice.id, SliceStatus::Running, 44)
        .expect("slice should be running before restart reconciliation");
    slices.reconcile_after_kernel_restart_with_host_state(45, |_| SliceHostRuntimeState::Unknown);
    let worker = chariox_relay::protocol::RelayKernelPresence {
        kernel_id: "worker-1".to_string(),
        machine_id: worker_machine_id,
        machine_alias: None,
        relay_alias: None,
        kernel_alias: None,
        available_providers: vec!["codex".to_string(), "opencode".to_string()],
        provider_accounts: Vec::new(),
        capabilities: Vec::new(),
        accepting_remote_leases: true,
        leased_agent_count: 0,
        local_session_count: 0,
        public_key: "authenticated-worker-key".to_string(),
    };
    let database =
        rusqlite::Connection::open(state.config.user_config.state.path.as_ref().unwrap()).unwrap();
    database.execute_batch("CREATE TRIGGER reject_worker_repair BEFORE INSERT ON durable_state_events
        WHEN NEW.kind = 'slice.updated' BEGIN SELECT RAISE(ABORT, 'injected storage failure'); END;").unwrap();

    router
        .runtime_state
        .reconcile_authenticated_slice_worker_presence(&worker)
        .expect_err("append failure must reject the repair");
    let retained = slices
        .resolve(&slice.id)
        .expect("slice should remain available after append failure");
    assert_eq!(retained.status, SliceStatus::Unhealthy);
    assert_eq!(
        retained.last_operation.as_deref(),
        Some("restart_reconcile")
    );
    assert_eq!(
        retained.last_operation_status,
        Some(SliceOperationStatus::Reconciled)
    );
    assert_eq!(retained.providers, vec!["codex"]);

    database
        .execute_batch("DROP TRIGGER reject_worker_repair;")
        .unwrap();
    assert!(router
        .runtime_state
        .reconcile_authenticated_slice_worker_presence(&worker)
        .expect("retry should durably repair the slice"));
    let repaired = slices
        .resolve(&slice.id)
        .expect("repaired slice should remain available");
    assert_eq!(repaired.status, SliceStatus::Running);
    assert_eq!(repaired.providers, vec!["codex", "opencode"]);
}

#[test]
fn a_room_whose_slice_is_down_says_so_and_still_stops() {
    run_test(slice_down_is_reported_and_stop_completes);
}

async fn slice_down_is_reported_and_stop_completes() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "paused").await;
    router
        .app
        .lock()
        .await
        .slices()
        .set_status("paused", SliceStatus::Running, 1)
        .unwrap();
    // The slice's relay runs inside its container; a stopped container
    // refuses the connection.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    router
        .app
        .lock()
        .await
        .slices()
        .set_relay_endpoint(
            "paused",
            Some(crate::slice::SliceRelayEndpoint {
                url: format!("ws://127.0.0.1:{port}"),
                private: true,
            }),
            1,
        )
        .unwrap();
    dispatch_json(&router, bind(&rooms[0], "paused"))
        .await
        .unwrap();
    let start = json!({"StartRoomEnvironment": {
        "session_id": &rooms[0], "viewport": {
            "css_width":1280,"css_height":800,"device_scale_factor":1,
            "desktop_pixel_width":1280,"desktop_pixel_height":800
        }
    }});
    for request in [
        start,
        json!({"RetryRoomEnvironment": {"session_id": &rooms[0]}}),
    ] {
        let error = dispatch_json(&router, request)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("room_slice_unreachable"), "{error}");
        assert!(error.contains("`paused`"), "names the slice: {error}");
    }
    let stopped = dispatch_json(
        &router,
        json!({"StopRoomEnvironment": {"session_id": &rooms[0]}}),
    )
    .await
    .expect("a Room whose slice is gone stops without its relay");
    assert_eq!(
        stopped["RoomEnvironmentUpdated"]["environment"]["lifecycle"], "stopped",
        "{stopped}"
    );
}

#[test]
fn shared_relay_failure_and_private_relay_timeout_keep_stop_failed() {
    run_test(relay_failure_keeps_stop_failed);
}

async fn relay_failure_keeps_stop_failed() {
    for private in [false, true] {
        let mut state = TestState::new();
        state.config.relay_request_timeout_ms = 30;
        state.config.relay_token = Some("fixture-relay-token".into());
        let (router, rooms) = state.router();
        create_desktop(&router, "live").await;
        router
            .app
            .lock()
            .await
            .slices()
            .set_status("live", SliceStatus::Running, 1)
            .unwrap();
        let relay = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = relay.local_addr().unwrap().port();
        // Public/shared relay: connection refused. Private relay: accepts TCP
        // but never completes the WebSocket handshake, so the route times out.
        let relay = if private {
            Some(relay)
        } else {
            drop(relay);
            None
        };
        router
            .app
            .lock()
            .await
            .slices()
            .set_relay_endpoint(
                "live",
                Some(crate::slice::SliceRelayEndpoint {
                    url: format!("ws://127.0.0.1:{port}"),
                    private,
                }),
                1,
            )
            .unwrap();
        dispatch_json(&router, bind(&rooms[0], "live"))
            .await
            .unwrap();
        let start_error = dispatch_json(
            &router,
            json!({"StartRoomEnvironment": {
                "session_id": &rooms[0], "viewport": {
                    "css_width":1280,"css_height":800,"device_scale_factor":1,
                    "desktop_pixel_width":1280,"desktop_pixel_height":800
                }
            }}),
        )
        .await
        .expect_err("relay failed before controller readiness")
        .to_string();
        assert!(
            !start_error.contains("room_slice_unreachable"),
            "{start_error}"
        );

        let error = dispatch_json(
            &router,
            json!({"StopRoomEnvironment": {"session_id": &rooms[0]}}),
        )
        .await
        .expect_err("relay failure does not prove the controller was released")
        .to_string();
        assert!(!error.contains("room_slice_unreachable"), "{error}");
        assert!(!error.contains("start the slice"), "{error}");
        assert!(
            error.contains(if private {
                "timed out"
            } else {
                "Connection refused"
            }),
            "{error}"
        );
        let environment = dispatch_json(
            &router,
            json!({"GetRoomEnvironmentState": {"session_id": &rooms[0]}}),
        )
        .await
        .unwrap();
        assert!(
            environment.to_string().contains("controller_stop_failed"),
            "{environment}"
        );
        assert_eq!(
            environment["RoomEnvironmentState"]["environment"]["lifecycle"], "failed",
            "{environment}"
        );
        drop(relay);
    }
}

#[test]
fn room_environment_controller_does_not_block_start_on_stopped_or_starting_slice() {
    run_test(controller_does_not_block_start_on_stopped_or_starting_slice);
}

async fn controller_does_not_block_start_on_stopped_or_starting_slice() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "desktop").await;
    dispatch_json(&router, bind(&rooms[0], "desktop"))
        .await
        .unwrap();
    router.shutdown_cleanup().await.unwrap();
    drop(router);
    wait_for_durable_owner_release(&state.config.durable_state_path()).await;
    let app = DaemonApp::bootstrap(state.config.clone()).expect("restart throwaway kernel");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);
    let slices = router.app.lock().await.slices().clone();
    for status in [SliceStatus::Stopped, SliceStatus::Starting] {
        slices.set_status("desktop", status, 42).unwrap();
        for _ in 0..3 {
            let route = router
                .runtime_state
                .ensure_browser_controller_process_started(&rooms[0]);
            tokio::pin!(route);
            let result = tokio::select! {
                result = &mut route => Some(result),
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => None,
            };
            let _start = slices
                .try_begin_operation("desktop", "slice.start")
                .expect("recurring routes must never reserve a stopped/starting slice");
            let error = result
                .expect("offline route must fail promptly")
                .unwrap_err();
            assert!(
                error.to_string().contains("slice is not running"),
                "{error}"
            );
        }
    }
    router.shutdown_cleanup().await.unwrap();
}
