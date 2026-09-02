use super::*;
use serde_json::{json, Value};

mod execution;
mod live_worker;

fn run_test<F: std::future::Future<Output = ()> + 'static>(test: fn() -> F) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
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
        Self { root, config }
    }

    fn router(&self) -> (CommandRouter, Vec<String>) {
        let mut app = DaemonApp::bootstrap(self.config.clone()).expect("boot kernel");
        let rooms = (0..2)
            .map(|_| {
                crate::app::KernelSessionService::new(&mut app)
                    .create_session(CreateSessionRequest::new("workspace", "worktree"))
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
            std::fs::remove_dir_all(&self.root).expect("remove drill-owned kernel state");
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
                    .create_session(CreateSessionRequest::new("workspace", "worktree"))
                    .expect("create Room")
                    .0
                    .id()
                    .to_string()
            })
            .collect();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
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
