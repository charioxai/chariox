// MP-08 / MP-09 / MP-10 / MP-11: a room agent can read what `--room` and `--slice` accept for it.
// Rooms from `world`: `room` (a -> {b -> {d}, s}, peer) and `other_room` (q), one owner.
use super::subtree::{error_text, room_command, world, World};
use super::*;
use crate::local::CreateSliceRequest;
use crate::slice::{SliceBackendKind, SliceDisplayMode};
use crate::transport::runtime_tools::RuntimeToolResult;

fn ids_of(result: &RuntimeToolResult, key: &str) -> Vec<String> {
    result.payload["response"][key]
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` list in {result:?}"))
        .iter()
        .map(|entry| entry["id"].as_str().unwrap().to_owned())
        .collect()
}

async fn foreign_room(world: &World) -> String {
    let mut app = world.app.lock().await;
    let workspace = world._env.root.join("workspace");
    let mut request =
        CreateSessionRequest::new(workspace.to_string_lossy(), workspace.to_string_lossy());
    request.owner_user_id = "someone-else".to_owned();
    let (room, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(request)
        .unwrap();
    room.id().to_owned()
}

#[test]
fn room_list_names_the_rooms_of_the_owner_that_room_accepts() {
    run_large_stack_async_test("reach-room-list", room_list);
}

async fn room_list() {
    let world = world("reach-room-list").await;
    let foreign = foreign_room(&world).await;
    for who in ["a", "b", "q"] {
        let listed = room_command(&world, who, "room list").await;
        assert!(listed.ok, "{who}: {listed:?}");
        let ids = ids_of(&listed, "rooms");
        assert!(
            ids.contains(&world.room) && ids.contains(&world.other_room),
            "{ids:?}"
        );
        assert!(
            !ids.contains(&foreign),
            "another owner's room is not listed: {ids:?}"
        );
        let current = listed.payload["response"]["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|room| room["current"] == true)
            .map(|room| room["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let own = if who == "q" {
            &world.other_room
        } else {
            &world.room
        };
        assert_eq!(current, std::slice::from_ref(own), "{who}");
    }
    // Every listed room other than its own is a room `agent spawn --room` accepts.
    let accepted = room_command(
        &world,
        "a",
        &format!(
            "agent spawn px --provider dev-stub --room {} --no-follow",
            world.other_room
        ),
    )
    .await;
    assert!(accepted.ok, "{accepted:?}");
    let refused = room_command(&world, "a", &format!("room list {}", world.room)).await;
    assert!(!refused.ok, "room list takes no argument: {refused:?}");
}

#[test]
fn slice_list_names_only_the_slices_the_room_may_use() {
    run_large_stack_async_test("reach-slice-list", slice_list);
}

async fn make_slice(world: &World, name: &str, display_mode: SliceDisplayMode) {
    world
        .router
        .runtime_state()
        .create_slice(CreateSliceRequest {
            source_slice_ref: None,
            name: name.to_owned(),
            backend: SliceBackendKind::LocalDocker,
            os: "linux".to_owned(),
            display_mode,
            display_backend: Default::default(),
            workspace_id: None,
            worktree_id: None,
            workspace_mount: Some("/workspace".to_owned()),
            development: None,
            worker_kernel_ref: None,
            display_url: None,
            provider_auth: Vec::new(),
            from_saved_state: None,
            base: Some(crate::local::SliceCreateBase::Clean),
        })
        .await
        .unwrap();
}

async fn slice_list() {
    let world = world("reach-slice-list").await;
    make_slice(&world, "shared", SliceDisplayMode::Headless).await;
    make_slice(&world, "theirs", SliceDisplayMode::Headed).await;
    world
        .app
        .lock()
        .await
        .slices()
        .bind_environment(&world.other_room, "theirs", 1, |_| Ok(()))
        .unwrap();
    let names = |result: &RuntimeToolResult| -> Vec<String> {
        result.payload["response"]["slices"]
            .as_array()
            .unwrap_or_else(|| panic!("slices in {result:?}"))
            .iter()
            .map(|slice| slice["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let mine = room_command(&world, "a", "slice list").await;
    assert!(mine.ok, "{mine:?}");
    assert_eq!(
        names(&mine),
        ["shared"],
        "a slice bound to another room is not offered"
    );
    let theirs = room_command(&world, "q", "slice list").await;
    assert!(theirs.ok, "{theirs:?}");
    let mut seen = names(&theirs);
    seen.sort();
    assert_eq!(seen, ["shared", "theirs"]);
    // Only what a person needs to choose: no credentials, ports or provider state.
    let entry = &mine.payload["response"]["slices"][0];
    let mut keys = entry
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(
        keys,
        ["backend", "display_mode", "id", "name", "os", "status"]
    );
    // A listed slice resolves the way `agent spawn --slice` resolves it.
    let id = entry["id"].as_str().unwrap();
    world
        .router
        .runtime_state()
        .resolve_spawn_slice(id, &world.ids["a"], "local")
        .unwrap();
    // The lifecycle commands stay outside the room surface.
    for command in [
        "slice stop shared",
        "slice start shared",
        "slice show shared",
    ] {
        let refused = room_command(&world, "a", command).await;
        assert!(
            !refused.ok && error_text(&refused).contains("outside the regular room surface"),
            "{command}: {refused:?}"
        );
    }
}

#[test]
fn slice_list_and_spawn_hide_a_slice_of_another_owner() {
    run_large_stack_async_test("reach-slice-owner", slice_of_another_owner);
}

async fn slice_of_another_owner() {
    let world = world("reach-slice-owner").await;
    let foreign = foreign_room(&world).await;
    make_slice(&world, "mine", SliceDisplayMode::Headless).await;
    make_slice(&world, "theirs", SliceDisplayMode::Headless).await;
    {
        let app = world.app.lock().await;
        // Unreserved, but its only user is a room of another owner.
        app.slices().attach_session("theirs", &foreign, 1).unwrap();
        app.slices()
            .attach_session("mine", &world.other_room, 1)
            .unwrap();
    }
    let listed = room_command(&world, "a", "slice list").await;
    assert!(listed.ok, "{listed:?}");
    let mut names = listed.payload["response"]["slices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slice| slice["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["mine"], "another owner's slice is not listed");
    let refused = room_command(
        &world,
        "a",
        "agent spawn px --provider dev-stub --slice theirs",
    )
    .await;
    assert!(
        !refused.ok && error_text(&refused).contains("unknown slice"),
        "another owner's slice reads as unknown: {refused:?}"
    );
}

#[test]
fn the_room_command_docs_describe_the_two_lists_as_available() {
    run_large_stack_async_test("reach-docs", docs);
}

async fn docs() {
    let world = world("reach-docs").await;
    let token = &world.tokens["a"];
    for command in ["room list", "slice list"] {
        let doc = world
            .router
            .dispatch_authenticated_runtime_tool_call(
                token,
                "chariox.commands.docs",
                serde_json::json!({"command": command}),
            )
            .await
            .unwrap();
        assert!(doc.ok, "{command}: {doc:?}");
        assert_eq!(doc.payload["name"], command, "{doc:?}");
        assert!(doc.payload["mutates"] == false, "{command}: {doc:?}");
        assert!(
            doc.payload["metaagent_policy"] != "deny",
            "{command}: {doc:?}"
        );
        assert!(
            !doc.payload["authority"]
                .as_str()
                .unwrap()
                .contains("unavailable"),
            "{command}: {doc:?}"
        );
    }
    let room_list = world
        .router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.commands.docs",
            serde_json::json!({"command": "room list"}),
        )
        .await
        .unwrap();
    assert!(
        room_list.payload["description"]
            .as_str()
            .unwrap_or_default()
            .contains("`agent spawn --room` refuses your own room"),
        "the list says its own room is not valid for spawn: {room_list:?}"
    );
}
