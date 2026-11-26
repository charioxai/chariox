// Row 363: an owner's agent moves an agent in the tree as the owner can with SetAgentParent, with the
// same refusals. The new parent then controls the agent, the old parent no longer does, and the
// agent keeps its id and history.

use super::*;

async fn parent_of(world: &World, who: &str) -> Option<String> {
    world
        .app
        .lock()
        .await
        .agents()
        .get_agent(&world.ids[who])
        .unwrap()
        .parent_agent_id()
        .map(str::to_owned)
}

fn controls_it(result: &RuntimeToolResult) -> bool {
    !refused_by_policy(result)
}

/// What the owner's own move of `agent` under `parent` answers.
async fn owner_move(
    world: &World,
    agent: &str,
    parent: Option<&str>,
) -> Result<LocalDaemonResponse, DaemonError> {
    let request = LocalDaemonRequest::SetAgentParent(crate::local::SetAgentParentRequest {
        session_id: world.room.clone(),
        agent_id: world.ids[agent].clone(),
        parent_agent_id: parent.map(|parent| world.ids[parent].clone()),
    });
    let mut command = KernelCommand::from_local_request("owner-move", None, None, &request);
    command.caller = command
        .caller
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    world.router.dispatch(command, request).await
}

#[test]
fn an_agent_moves_a_sibling_and_a_grandchild_and_the_new_parent_controls_them() {
    run_large_stack_async_test("owner-parent-moves", moves);
}

async fn moves() {
    let world = world("owner-parent-moves").await;
    // d is b's child and a's grandchild: peer is above neither. Before this, nobody but the user moved it.
    assert!(!controls_it(
        &room_command(&world, "peer", "agent children d").await
    ));
    let grandchild = room_command(&world, "a", "agent parent d peer").await;
    assert!(grandchild.ok, "{grandchild:?}");
    // An agent's command answer omits response bodies; the new edge is read from the agent.
    assert_eq!(
        grandchild.payload["response"]["type"], "CommandAccepted",
        "{grandchild:?}"
    );
    assert_eq!(
        parent_of(&world, "d").await.as_deref(),
        Some(world.ids["peer"].as_str())
    );
    // A sibling: b moves s under itself.
    let sibling = room_command(&world, "b", "agent parent s b").await;
    assert!(sibling.ok, "{sibling:?}");
    assert_eq!(
        parent_of(&world, "s").await.as_deref(),
        Some(world.ids["b"].as_str())
    );
    // The new parent controls the lane, the old parent no longer does (cells b, c).
    for verb in ["agent children d", "agent deadline get d"] {
        assert!(
            controls_it(&room_command(&world, "peer", verb).await),
            "{verb}"
        );
    }
    for verb in ["agent children d", "agent deadline get d"] {
        let old = room_command(&world, "b", verb).await;
        assert!(refused_by_policy(&old), "{verb}: {old:?}");
        assert!(
            error_text(&old).contains("can control it"),
            "the plain reason: {old:?}"
        );
    }
    let old_prompt = room_command(&world, "a", "prompt d say hello").await;
    assert!(refused_by_policy(&old_prompt), "{old_prompt:?}");
    let new_prompt = room_command(&world, "peer", "prompt d say hello").await;
    assert!(controls_it(&new_prompt), "{new_prompt:?}");
    // The same agent, not a relaunch (cell d): its id and alias stay.
    let moved = world
        .app
        .lock()
        .await
        .agents()
        .get_agent(&world.ids["d"])
        .unwrap();
    assert_eq!(moved.alias(), Some("d"));
    assert_eq!(moved.id(), world.ids["d"]);
}

#[test]
fn an_agent_makes_an_agent_a_room_agent_with_none() {
    run_large_stack_async_test("owner-parent-none", to_none);
}

async fn to_none() {
    let world = world("owner-parent-none").await;
    let result = room_command(&world, "peer", "agent parent b none").await;
    assert!(result.ok, "{result:?}");
    assert_eq!(parent_of(&world, "b").await, None);
    assert!(refused_by_policy(
        &room_command(&world, "a", "agent children b").await
    ));
}

#[test]
fn a_cycle_is_refused_for_an_agent_with_the_owners_sentence() {
    run_large_stack_async_test("owner-parent-cycle", cycle);
}

async fn cycle() {
    let world = world("owner-parent-cycle").await;
    let owner = owner_move(&world, "a", Some("d")).await.unwrap_err();
    let a = world.ids["a"].clone();
    for command in [
        format!("agent parent {a} d"),
        format!("agent parent {a} b"),
        "agent parent peer peer".to_owned(),
    ] {
        let refused = room_command(&world, "peer", &command).await;
        assert!(!refused.ok, "{command}: {refused:?}");
        assert!(
            error_text(&refused)
                .contains("an agent cannot be placed under itself or under one of its descendants"),
            "{command}: {refused:?}"
        );
    }
    let agent = room_command(&world, "peer", &format!("agent parent {a} d")).await;
    assert!(
        error_text(&agent).contains(&owner.to_string()),
        "the same sentence for the agent and the owner: {agent:?} / {owner}"
    );
    assert_eq!(
        parent_of(&world, "b").await.as_deref(),
        Some(world.ids["a"].as_str())
    );
}

#[test]
fn another_room_is_refused_for_an_agent_as_for_the_owner() {
    run_large_stack_async_test("owner-parent-room", other_room);
}

async fn other_room() {
    let world = world("owner-parent-room").await;
    let q_id = world
        .router
        .runtime_state
        .list_agents()
        .into_iter()
        .find(|agent| agent.session_id() == world.other_room)
        .unwrap()
        .id()
        .to_owned();
    // The owner naming an agent that is not in the room is refused.
    let request = LocalDaemonRequest::SetAgentParent(crate::local::SetAgentParentRequest {
        session_id: world.other_room.clone(),
        agent_id: world.ids["b"].clone(),
        parent_agent_id: None,
    });
    let mut command = KernelCommand::from_local_request("owner-other-room", None, None, &request);
    command.caller = command
        .caller
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    assert!(world.router.dispatch(command, request).await.is_err());
    // An agent of the other Room, or naming one, is refused the same way.
    for (caller, command) in [
        ("q", "agent parent b none".to_owned()),
        ("peer", format!("agent parent b {q_id}")),
        ("peer", format!("agent parent {q_id} peer")),
    ] {
        let refused = room_command(&world, caller, &command).await;
        assert!(!refused.ok, "{caller}: {command}: {refused:?}");
    }
    assert_eq!(
        parent_of(&world, "b").await.as_deref(),
        Some(world.ids["a"].as_str())
    );
    // The raw request of an agent of another Room is refused by admission too.
    let raw = LocalDaemonRequest::SetAgentParent(crate::local::SetAgentParentRequest {
        session_id: world.room.clone(),
        agent_id: world.ids["b"].clone(),
        parent_agent_id: None,
    });
    assert!(world
        .router
        .runtime_state
        .authorize_room_agent_request(&q_id, &raw)
        .is_err());
}

#[test]
fn the_history_names_the_agent_that_moved_it() {
    run_large_stack_async_test("owner-parent-who", who_moved_it);
}

async fn who_moved_it() {
    let world = world("owner-parent-who").await;
    let moved = room_command(&world, "peer", "agent parent b peer").await;
    assert!(moved.ok, "{moved:?}");
    let by_owner = owner_move(&world, "s", Some("peer")).await.unwrap();
    assert!(matches!(
        by_owner,
        LocalDaemonResponse::AgentParentChanged { .. }
    ));
    let events = world
        .app
        .lock()
        .await
        .durable_state_store()
        .load_events_by_kind("agent.updated")
        .unwrap();
    let mover = |agent: &str| {
        events
            .iter()
            .rev()
            .find(|event| event.payload["agent"]["id"] == world.ids[agent].as_str())
            .map(|event| event.payload["moved_by"].clone())
    };
    assert_eq!(
        mover("b"),
        Some(serde_json::Value::String(world.ids["peer"].clone())),
        "the agent that moved it"
    );
    assert_eq!(
        mover("s"),
        Some(serde_json::Value::Null),
        "the owner's own move names no agent"
    );
}
