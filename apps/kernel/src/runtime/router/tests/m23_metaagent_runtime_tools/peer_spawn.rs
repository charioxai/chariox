// MP-08 / MP-09 / MP-10 / MP-11 M2 P3: `agent spawn --room` makes an ordinary agent of another room
// of the same owner. Rooms from `world`: `room` (a -> {b -> {d}, s}, peer) and `other_room` (q).
use super::subtree::{error_text, room_command, world, World};
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
            format!("agent spawn px --room {room} --kernel somewhere"),
            "--room",
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
fn the_spawn_usage_documents_the_room_option() {
    assert!(
        crate::runtime::metaagent_command_registry::AGENT_SPAWN_USAGE
            .contains("[--room <room> [--no-follow]]")
    );
}
