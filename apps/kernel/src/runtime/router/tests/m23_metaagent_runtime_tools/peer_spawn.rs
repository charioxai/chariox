// MP-08 / MP-09 / MP-10 / MP-11 M2 P3: `agent spawn --room` makes an ordinary agent of another room
// of the same owner. Rooms from `world`: `room` (a -> {b -> {d}, s}, peer) and `other_room` (q).
use super::subtree::{error_text, launch, room_command, world, world_with_config, World};
use super::*;
use crate::provider::AgentPermissionLevel;
use crate::transport::runtime_tools::RuntimeToolResult;

async fn agents_of(world: &World, room: &str) -> Vec<crate::agent::AgentInstance> {
    world.app.lock().await.agents().get_session_agents(room)
}

async fn spawn_into(world: &World, who: &str, room: &str, rest: &str) -> RuntimeToolResult {
    room_command(
        world,
        who,
        &format!("agent spawn px --provider dev-stub --room {room} {rest}"),
    )
    .await
}

#[test]
fn a_room_agent_spawns_an_ordinary_peer_in_another_room_of_the_same_owner() {
    run_large_stack_async_test("peer-spawn-happy", happy_path);
}

async fn happy_path() {
    let world = world("peer-spawn-happy").await;
    let before = agents_of(&world, &world.room).await.len();
    let spawned = spawn_into(&world, "a", &world.other_room.clone(), "--no-follow").await;
    assert!(spawned.ok, "{spawned:?}");
    let id = spawned.payload["response"]["agent"]["id"].as_str().unwrap();
    let in_target = agents_of(&world, &world.other_room).await;
    let peer = in_target
        .iter()
        .find(|agent| agent.id() == id)
        .expect("peer in the target room");
    assert_eq!(peer.session_id(), world.other_room);
    assert_eq!(peer.alias(), Some("px"));
    assert!(
        peer.parent_agent_id().is_none(),
        "a peer has no parent edge"
    );
    assert!(
        !peer.is_sub_agent(),
        "it counts in the room's max_agents and is listed"
    );
    assert_eq!(
        agents_of(&world, &world.room).await.len(),
        before,
        "nothing was added to the caller room"
    );
    let LocalDaemonResponse::AgentsListed { agents } = world
        .router
        .dispatch(
            KernelCommand::from_local_request(
                "peer-spawn",
                None,
                None,
                &LocalDaemonRequest::ListAgents(crate::local::ListAgentsRequest {
                    session_id: world.other_room.clone(),
                }),
            ),
            LocalDaemonRequest::ListAgents(crate::local::ListAgentsRequest {
                session_id: world.other_room.clone(),
            }),
        )
        .await
        .unwrap()
    else {
        panic!("agent list")
    };
    assert!(
        agents.iter().any(|agent| agent.id() == id),
        "listed in the target room"
    );
}

#[test]
fn a_peer_is_never_above_the_authority_of_its_spawner() {
    run_large_stack_async_test("peer-spawn-clamp", clamp);
}

async fn clamp() {
    let world = world("peer-spawn-clamp").await;
    world
        .app
        .lock()
        .await
        .agents()
        .update_agent_config(
            &world.ids["a"],
            None,
            Some(Some(AgentPermissionLevel::Required)),
            None,
            None,
        )
        .unwrap();
    let room = world.other_room.clone();
    let raised = spawn_into(&world, "a", &room, "--permission yolo").await;
    assert!(
        !raised.ok && error_text(&raised).contains("more authority than its parent"),
        "{raised:?}"
    );
    assert!(agents_of(&world, &room)
        .await
        .iter()
        .all(|agent| agent.alias() != Some("px")));
    let inherited = spawn_into(&world, "a", &room, "").await;
    assert!(inherited.ok, "{inherited:?}");
    let id = inherited.payload["response"]["agent"]["id"]
        .as_str()
        .unwrap();
    let peer = agents_of(&world, &room)
        .await
        .into_iter()
        .find(|agent| agent.id() == id)
        .unwrap();
    assert_eq!(
        peer.permission_level_override(),
        Some(AgentPermissionLevel::Required),
        "the peer starts at the spawner's level"
    );
}

#[test]
fn a_sub_agent_cannot_spawn_a_peer_in_another_room() {
    run_large_stack_async_test("peer-spawn-sub-agent", sub_agent_refused);
}

async fn sub_agent_refused() {
    let world = world("peer-spawn-sub-agent").await;
    let room = world.other_room.clone();
    let refused = spawn_into(&world, "b", &room, "").await;
    assert!(
        !refused.ok && error_text(&refused).contains("only a room agent"),
        "{refused:?}"
    );
    assert!(agents_of(&world, &room)
        .await
        .iter()
        .all(|agent| agent.alias() != Some("px")));
}

#[test]
fn a_peer_is_refused_in_a_room_of_another_owner_or_a_room_that_does_not_exist() {
    run_large_stack_async_test("peer-spawn-owner", other_owner);
}

async fn other_owner() {
    let world = world("peer-spawn-owner").await;
    let foreign = {
        let mut app = world.app.lock().await;
        let workspace = world._env.root.join("workspace");
        let mut request =
            CreateSessionRequest::new(workspace.to_string_lossy(), workspace.to_string_lossy());
        request.owner_user_id = "someone-else".to_owned();
        let (room, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(request)
            .unwrap();
        room.id().to_owned()
    };
    // The room refuses a non-member before the spawn is admitted.
    let refused = spawn_into(&world, "a", &foreign, "").await;
    assert!(!refused.ok, "{refused:?}");
    assert!(agents_of(&world, &foreign)
        .await
        .iter()
        .all(|agent| agent.alias() != Some("px")));
    // The admission itself refuses a room of another owner for a member (a guest of that room).
    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        no_subscribe: false,
        session_id: foreign.clone(),
        alias: Some("px".to_owned()),
        provider: Some("dev-stub".to_owned()),
        account_profile: None,
        model: None,
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: None,
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let denied = world
        .router
        .runtime_state
        .authorize_room_agent_request(&world.ids["a"], &request)
        .unwrap_err()
        .to_string();
    assert!(denied.contains("same owner"), "{denied}");
    let missing = spawn_into(&world, "a", "no-such-room", "").await;
    assert!(!missing.ok, "{missing:?}");
}

#[test]
fn room_spawn_options_are_checked_where_they_are_parsed() {
    run_large_stack_async_test("peer-spawn-parse", parse_refusals);
}

async fn parse_refusals() {
    let world = world("peer-spawn-parse").await;
    let room = world.other_room.clone();
    for (command, wanted) in [
        (
            "agent spawn px --room".to_owned(),
            "usage: agent spawn --room",
        ),
        (
            format!("agent spawn px --room {room} --no-follow --command-id c1"),
            "--command-id",
        ),
        (
            format!("agent spawn px --room {room} --kernel somewhere --no-follow"),
            "kernel_unreachable",
        ),
        (
            format!("agent spawn px --room {room} --slice new"),
            "--room",
        ),
        (
            "agent spawn px --no-follow".to_owned(),
            "--no-follow needs --room",
        ),
    ] {
        let refused = room_command(&world, "a", &command).await;
        assert!(
            !refused.ok && error_text(&refused).contains(wanted),
            "{command}: {refused:?}"
        );
    }
    assert!(agents_of(&world, &room)
        .await
        .iter()
        .all(|agent| agent.alias() != Some("px")));
}

#[test]
fn a_sub_agent_spawn_on_an_unknown_machine_is_refused_and_creates_nothing() {
    run_large_stack_async_test("peer-spawn-unknown-machine", unknown_machine_refused);
}

async fn unknown_machine_refused() {
    let world = world("peer-spawn-unknown-machine").await;
    for flag in ["--machine", "--kernel"] {
        let refused = room_command(
            &world,
            "a",
            &format!("agent spawn sx --provider dev-stub {flag} unknown-xyz"),
        )
        .await;
        assert!(!refused.ok, "{flag}: {refused:?}");
    }
    assert!(agents_of(&world, &world.room)
        .await
        .iter()
        .all(|agent| agent.alias() != Some("sx")));
}

#[test]
fn a_sub_agent_spawn_naming_the_own_machine_runs_here() {
    run_large_stack_async_test("peer-spawn-own-machine", own_machine_runs_here);
}

async fn own_machine_runs_here() {
    use crate::runtime::state::peer_agent_outbound::peer_agent_link_for_tests;
    let world = world("peer-spawn-own-machine").await;
    let own = world.app.lock().await.config().daemon_id.clone();
    // The relay names this kernel under its machine's name, as it would for any kernel.
    peer_agent_link_for_tests::register(
        &own,
        "own-machine",
        std::sync::Arc::new(|_, _, _| panic!("the own kernel is not a peer")),
    );
    for (alias, name) in [("sm", "own-machine"), ("si", own.as_str())] {
        for flag in ["--machine", "--kernel"] {
            let alias = format!("{alias}{}", &flag[2..3]);
            let spawned = room_command(
                &world,
                "a",
                &format!("agent spawn {alias} --provider dev-stub {flag} {name}"),
            )
            .await;
            assert!(spawned.ok, "{flag} {name}: {spawned:?}");
            let child = agents_of(&world, &world.room)
                .await
                .into_iter()
                .find(|agent| agent.alias() == Some(alias.as_str()))
                .unwrap_or_else(|| panic!("{flag} {name}: no child in this room"));
            assert!(child.remote_execution().is_none(), "{flag} {name}");
        }
    }
    peer_agent_link_for_tests::unregister(&own);
}

#[test]
fn a_spawn_naming_this_kernel_keeps_the_parent_folder() {
    run_large_stack_async_test(
        "peer-spawn-own-kernel-folder",
        own_kernel_keeps_the_parent_folder,
    );
}

/// `--kernel` naming this kernel by id or alias creates a local child, which works in its
/// parent's checkout, not in the Room's folder.
async fn own_kernel_keeps_the_parent_folder() {
    let mut world = world_with_config("peer-spawn-own-kernel-folder", |config| {
        config.daemon_alias = Some("home-kernel".to_string());
    })
    .await;
    let room_folder = world._env.root.join("workspace").display().to_string();
    let checkout = world._env.root.join("feature-checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    let checkout = checkout.display().to_string();
    assert_ne!(room_folder, checkout);
    let parent = crate::app::KernelSessionService::new(&mut *world.app.lock().await)
        .spawn_agent(
            CreateAgentRequest::new(&world.room, "dev-stub")
                .with_alias("feature")
                .with_worktree(&checkout),
        )
        .unwrap();
    let token = launch(&world.app, &world.room, parent.id()).await;
    world.tokens.insert("feature", token);
    let own = world.app.lock().await.config().daemon_id.clone();
    for (alias, kernel) in [("ki", own.as_str()), ("ka", "home-kernel")] {
        let spawned = room_command(
            &world,
            "feature",
            &format!("agent spawn {alias} --provider dev-stub --kernel {kernel}"),
        )
        .await;
        assert!(spawned.ok, "--kernel {kernel}: {spawned:?}");
        let child = agents_of(&world, &world.room)
            .await
            .into_iter()
            .find(|agent| agent.alias() == Some(alias))
            .unwrap_or_else(|| panic!("--kernel {kernel}: no child in this room"));
        assert!(child.remote_execution().is_none(), "--kernel {kernel}");
        assert_eq!(
            child.worktree_id(),
            Some(checkout.as_str()),
            "--kernel {kernel}: the child works in its parent's checkout"
        );
    }
}

#[test]
fn the_spawn_usage_documents_the_room_option() {
    assert!(
        crate::runtime::metaagent_command_registry::AGENT_SPAWN_USAGE
            .contains("[--room <room> [--kernel <kernel-ref>] [--no-follow]]")
    );
}

#[test]
fn a_peer_spawn_follows_the_peer_unless_told_not_to() {
    run_large_stack_async_test("peer-spawn-follow", follows);
}

async fn follows() {
    let world = world("peer-spawn-follow").await;
    let other = world.other_room.clone();
    let followed = spawn_into(&world, "a", &other, "").await;
    assert!(followed.ok, "{followed:?}");
    let peer = followed.payload["response"]["agent"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let registration = followed.payload["follow_registration_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the spawner follows its peer: {followed:?}"))
        .to_owned();
    let runtime = world.router.runtime_state();
    let store = runtime.durable_state_store();
    let registrations = store
        .agent_tasks(Some(&world.room), Some(&world.ids["a"]))
        .unwrap()
        .into_iter()
        .flat_map(|task| store.agent_registrations(&task.task_id).unwrap())
        .collect::<Vec<_>>();
    assert!(
        registrations
            .iter()
            .any(|r| r.id == registration && r.source_id == peer && r.peer && r.live),
        "{registrations:?}"
    );

    let unfollowed = room_command(
        &world,
        "a",
        &format!("agent spawn py --provider dev-stub --room {other} --no-follow"),
    )
    .await;
    assert!(unfollowed.ok, "{unfollowed:?}");
    assert!(
        unfollowed.payload["follow_registration_id"].is_null(),
        "{unfollowed:?}"
    );
    let quiet = unfollowed.payload["response"]["agent"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!store
        .agent_tasks(Some(&world.room), Some(&world.ids["a"]))
        .unwrap()
        .into_iter()
        .flat_map(|task| store.agent_registrations(&task.task_id).unwrap())
        .any(|r| r.source_id == quiet));

    // Following is also open later, by the peer's address in its room.
    let task = store
        .agent_tasks(Some(&world.room), Some(&world.ids["a"]))
        .unwrap()
        .pop()
        .unwrap();
    let subscribe = |source: &str| {
        world.router.dispatch_authenticated_runtime_tool_call(
            &world.tokens["a"],
            "chariox.events.subscribe",
            serde_json::json!({"task_id": task.task_id, "origin_prompt_id": task.prompt_id,
                "source_id": source, "room": other}),
        )
    };
    let subscribed = subscribe("py").await.unwrap();
    assert!(subscribed.ok, "{subscribed:?}");
    assert_eq!(subscribed.payload["source_id"], quiet.as_str());
    let id = subscribed.payload["registration_id"].as_str().unwrap();
    assert!(store
        .agent_registrations(&task.task_id)
        .unwrap()
        .iter()
        .any(|r| r.id == id && r.source_id == quiet && r.peer));
    let refused = format!("{:?}", subscribe("nobody").await);
    assert!(
        refused.contains("agent is unavailable in that room"),
        "{refused}"
    );
}
