// Cross-Room children use the authenticated Room commands and local client reads.
use super::subtree::{error_text, launch, room_command, world, World};
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
fn cross_room_spawn_sets_its_spawner_as_parent() {
    run_large_stack_async_test("peer-spawn-happy", happy_path);
}

async fn happy_path() {
    let world = world("peer-spawn-happy").await;
    let before = agents_of(&world, &world.room).await.len();
    let spawned = spawn_into(&world, "a", &world.other_room, "").await;
    assert!(spawned.ok, "{spawned:?}");
    let id = spawned.payload["response"]["agent"]["id"].as_str().unwrap();
    let in_target = agents_of(&world, &world.other_room).await;
    let peer = in_target
        .iter()
        .find(|agent| agent.id() == id)
        .expect("peer in the target room");
    assert_eq!(peer.session_id(), world.other_room);
    assert_eq!(peer.alias(), Some("px"));
    assert_eq!(peer.parent_agent_id(), Some(world.ids["a"].as_str()));
    assert!(peer.is_sub_agent());
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
    let request = LocalDaemonRequest::ListDescendants(crate::local::ListDescendantsRequest {
        session_id: world.room.clone(),
        agent_id: world.ids["a"].clone(),
    });
    let LocalDaemonResponse::AgentTree { tree } = world
        .router
        .dispatch(
            KernelCommand::from_local_request("cross-room-tree", None, None, &request),
            request,
        )
        .await
        .unwrap()
    else {
        panic!("missing parent tree")
    };
    let child = tree
        .nodes
        .iter()
        .find(|child| child.agent_id == id)
        .unwrap();
    assert_eq!(child.room_id.as_deref(), Some(world.other_room.as_str()));
    assert!(child.started_by.is_some());
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
fn cross_room_spawn_stays_in_the_spawners_folder_scope() {
    run_large_stack_async_test("cross-room-folder", || async {
        let world = world("cross-room-folder").await;
        let folder = world._env.root.join("outside");
        std::fs::create_dir_all(&folder).unwrap();
        let room = {
            let mut app = world.app.lock().await;
            crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new(
                    folder.to_string_lossy(),
                    folder.to_string_lossy(),
                ))
                .unwrap()
                .0
        };
        let result = spawn_into(&world, "a", room.id(), "").await;
        assert!(
            !result.ok && error_text(&result).contains("outside that scope"),
            "{result:?}"
        );
    });
}

#[test]
fn cross_room_spawn_uses_the_current_authority_of_its_parent() {
    run_large_stack_async_test("cross-room-lowered-parent", || async {
        let mut world = world("cross-room-lowered-parent").await;
        let other = world.other_room.clone();
        let spawned = spawn_into(&world, "a", &other, "--permission yolo").await;
        assert!(spawned.ok, "{spawned:?}");
        let child = spawned.payload["response"]["agent"]["id"].as_str().unwrap();
        let request = serde_json::from_value(serde_json::json!({"UpdateAgentConfig": {
            "session_id": world.room,
            "agent_id": world.ids["a"],
            "execution_mode": "plan",
            "permission_level": "required",
        }}))
        .unwrap();
        world
            .router
            .dispatch(
                KernelCommand::from_local_request("cross-room-parent-drop", None, None, &request),
                request,
            )
            .await
            .unwrap();
        world
            .tokens
            .insert("px", launch(&world.app, &other, child).await);
        let refused = room_command(
            &world,
            "px",
            "agent spawn grandchild --provider dev-stub --permission yolo",
        )
        .await;
        assert!(
            !refused.ok && error_text(&refused).contains("more authority than its parent"),
            "{refused:?}"
        );
    });
}

#[test]
fn cross_room_spawn_from_a_child_keeps_the_same_tree() {
    run_large_stack_async_test("cross-room-grandchild", || async {
        let world = world("cross-room-grandchild").await;
        let spawned = spawn_into(&world, "b", &world.other_room, "").await;
        assert!(spawned.ok, "{spawned:?}");
        let created = agents_of(&world, &world.other_room).await;
        assert_eq!(
            created
                .iter()
                .find(|agent| agent.alias() == Some("px"))
                .unwrap()
                .parent_agent_id(),
            Some(world.ids["b"].as_str()),
        );
        let listed = room_command(&world, "a", "agent children").await;
        assert!(listed.payload["response"]["children"]
            .as_array()
            .unwrap()
            .iter()
            .any(|child| child["alias"] == "px" && child["depth"] == 2));
    });
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
        initial_task: None,
        peer_spawn: None,
        max_context_tokens: None,
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
            "reports to you automatically",
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

/// `--kernel` naming this kernel creates a local child, which works in its parent's checkout,
/// not in the Room's folder.
async fn own_kernel_keeps_the_parent_folder() {
    let mut world = world("peer-spawn-own-kernel-folder").await;
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
    for (alias, kernel) in [("ki", own.as_str())] {
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
fn cross_room_spawn_receives_parent_instructions() {
    run_large_stack_async_test("cross-room-instruction", || async {
        let world = world("cross-room-instruction").await;
        let room = &world.other_room;
        let spawned = spawn_into(&world, "a", room, "").await;
        assert!(spawned.ok, "{spawned:?}");
        let id = spawned.payload["response"]["agent"]["id"].as_str().unwrap();
        let token = launch(&world.app, room, id).await;
        let sent = room_command(
            &world,
            "a",
            &format!("agent message px 'Write a report' --room {room}"),
        )
        .await;
        assert!(sent.ok, "{sent:?}");
        let task = world
            .router
            .runtime_state()
            .durable_state_store()
            .agent_tasks(Some(room), Some(id))
            .unwrap()
            .pop()
            .unwrap();
        let inbox = world
            .router
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "chariox.events.inbox",
                serde_json::json!({"task_id": task.task_id, "origin_prompt_id": task.prompt_id}),
            )
            .await
            .unwrap();
        let events = inbox.payload["events"].as_array().unwrap();
        assert!(
            events.iter().any(|event| {
                event["source"]["relation"] == "ancestor"
                    && event["guidance"]
                        .as_str()
                        .is_some_and(|guidance| guidance.contains("From your parent agent"))
                    && event["payload"]["message"] == "Write a report"
            }),
            "{inbox:?}"
        );
    });
}

#[test]
fn room_naming_the_callers_own_room_is_refused_instead_of_making_a_child() {
    run_large_stack_async_test("peer-spawn-own-room", own_room_refused);
}

async fn own_room_refused() {
    let world = world("peer-spawn-own-room").await;
    let own = world.room.clone();
    let before = agents_of(&world, &own).await.len();
    for who in ["a", "b"] {
        let refused = spawn_into(&world, who, &own, "").await;
        assert!(
            !refused.ok && error_text(&refused).contains("omit --room"),
            "{who}: {refused:?}"
        );
    }
    assert_eq!(
        agents_of(&world, &own).await.len(),
        before,
        "nothing was created"
    );
    // Without --room the same call is the sub-agent path.
    let child = room_command(&world, "a", "agent spawn cx --provider dev-stub").await;
    assert!(child.ok, "{child:?}");
    let id = child.payload["response"]["agent"]["id"].as_str().unwrap();
    let child = agents_of(&world, &own)
        .await
        .into_iter()
        .find(|agent| agent.id() == id)
        .unwrap();
    assert_eq!(child.parent_agent_id(), Some(world.ids["a"].as_str()));
}

#[test]
fn cross_room_spawn_is_controlled_by_its_parent() {
    run_large_stack_async_test("cross-room-parent-control", || async {
        let world = world("cross-room-parent-control").await;
        let room = &world.other_room;
        let spawned = spawn_into(&world, "a", room, "--deadline 30m").await;
        assert!(spawned.ok, "{spawned:?}");
        let id = spawned.payload["response"]["agent"]["id"].as_str().unwrap();
        let _token = launch(&world.app, room, id).await;
        for command in [
            format!("agent deadline px 10m --room {room}"),
            format!("agent cancel px --room {room}"),
            format!("agent delete px --room {room}"),
        ] {
            let result = room_command(&world, "a", &command).await;
            assert!(result.ok, "{command}: {result:?}");
        }
        assert!(agents_of(&world, room)
            .await
            .iter()
            .all(|agent| agent.id() != id));
    });
}

#[test]
fn a_room_spawn_with_slice_options_is_refused_before_any_slice_exists() {
    run_large_stack_async_test("peer-spawn-own-room-slice", own_room_slice);
}

async fn own_room_slice() {
    let world = world("peer-spawn-own-room-slice").await;
    let own = world.room.clone();
    let slices_before = world.router.runtime_state().list_slices().len();
    for option in ["--slice new", "--slice new:headed"] {
        let refused = spawn_into(&world, "a", &own, option).await;
        assert!(
            !refused.ok && error_text(&refused).contains("does not accept --slice"),
            "{option}: {refused:?}"
        );
    }
    assert_eq!(
        world.router.runtime_state().list_slices().len(),
        slices_before,
        "the parser refuses the options before a slice is made"
    );
}

#[test]
fn a_cross_room_peer_cannot_be_deleted_by_an_unrelated_agent() {
    run_large_stack_async_test("delete-cross-room-peer", || async {
        let world = world("delete-cross-room-peer").await;
        let other = &world.other_room;
        let agents = agents_of(&world, other).await;
        let [peer] = agents.as_slice() else {
            panic!("the destination Room starts with one ordinary peer");
        };
        let refused = room_command(
            &world,
            "a",
            &format!("agent delete {} --room {other}", peer.id()),
        )
        .await;
        assert!(
            !refused.ok && error_text(&refused).contains("agents above"),
            "{refused:?}"
        );
        assert!(agents_of(&world, other)
            .await
            .iter()
            .any(|agent| agent.id() == peer.id()));
    });
}
