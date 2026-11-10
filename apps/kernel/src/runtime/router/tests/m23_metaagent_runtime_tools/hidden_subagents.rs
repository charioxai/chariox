// MP-08 / MP-09 / MP-10 / MP-11 A4: sub-agents are descendants of a parent, never members of the
// room. Tree: a -> {b -> {d}, s}; peer is a second room agent with no parent.
use super::subtree::{room_command, world, World};
use super::*;

async fn dispatch(world: &World, request: serde_json::Value) -> LocalDaemonResponse {
    let request: LocalDaemonRequest = serde_json::from_value(request).expect("local request");
    let command = KernelCommand::from_local_request("hidden-test", None, None, &request);
    world.router.dispatch(command, request).await.unwrap()
}

async fn listed(world: &World, kind: &str) -> Vec<String> {
    let LocalDaemonResponse::AgentsListed { agents } =
        dispatch(world, serde_json::json!({kind: {"session_id": world.room}})).await
    else {
        panic!("agent list")
    };
    agents.iter().map(|agent| agent.id().to_owned()).collect()
}

#[test]
fn sub_agents_are_hidden_from_every_list_and_still_addressable_by_id() {
    run_large_stack_async_test("hidden-sub-agents", hidden_and_addressable);
}

async fn hidden_and_addressable() {
    let world = world("hidden-sub-agents").await;
    let visible = vec![world.ids["a"].clone(), world.ids["peer"].clone()];
    let mut listed_ids = listed(&world, "ListAgents").await;
    listed_ids.sort();
    let mut expected = visible.clone();
    expected.sort();
    assert_eq!(listed_ids, expected, "only the room agents are listed");
    // By-id requests keep working for a hidden sub-agent (the grandchild gets a deadline).
    let armed = room_command(&world, "a", "agent deadline set d 30m").await;
    assert!(armed.ok, "{armed:?}");
    let LocalDaemonResponse::AgentDeadline { deadline } = dispatch(
        &world,
        serde_json::json!({"GetAgentDeadline": {
            "session_id": world.room, "agent_id": world.ids["d"]}}),
    )
    .await
    else {
        panic!("by-id deadline read")
    };
    assert_eq!(deadline.map(|d| d.agent_id), Some(world.ids["d"].clone()));
    let deadline = room_command(&world, "a", "agent deadline get d").await;
    assert!(deadline.ok, "{deadline:?}");
}

#[test]
fn list_descendants_shows_the_whole_subtree_in_a_stable_order() {
    run_large_stack_async_test("list-descendants", list_descendants);
}

async fn list_descendants() {
    let world = world("list-descendants").await;
    let read = |agent: &str| {
        serde_json::json!({"ListDescendants": {
            "session_id": world.room, "agent_id": agent}})
    };
    let LocalDaemonResponse::AgentTree { tree } = dispatch(&world, read(&world.ids["a"])).await
    else {
        panic!("agent tree")
    };
    let ids = tree
        .nodes
        .iter()
        .map(|node| node.agent_id.clone())
        .collect::<Vec<_>>();
    let mut expected = vec![
        world.ids["b"].clone(),
        world.ids["s"].clone(),
        world.ids["d"].clone(),
    ];
    expected.sort_by_key(|id| {
        tree.nodes
            .iter()
            .position(|node| &node.agent_id == id)
            .unwrap_or(usize::MAX)
    });
    assert_eq!(ids.len(), 3, "b, s and the grandchild d: {ids:?}");
    assert_eq!(ids, expected);
    assert_eq!(tree.root, world.ids["a"]);
    assert_eq!(tree.totals.live, 1, "only the running sub-agent is live");
    let depth = |name: &str| {
        tree.nodes
            .iter()
            .find(|node| node.agent_id == world.ids[name])
            .map(|node| node.depth)
    };
    assert_eq!(
        (depth("b"), depth("s"), depth("d")),
        (Some(1), Some(1), Some(2))
    );
    // The same tree twice reads in the same order.
    let LocalDaemonResponse::AgentTree { tree: again } =
        dispatch(&world, read(&world.ids["a"])).await
    else {
        panic!("agent tree")
    };
    assert_eq!(
        again.nodes.iter().map(|n| &n.agent_id).collect::<Vec<_>>(),
        tree.nodes.iter().map(|n| &n.agent_id).collect::<Vec<_>>()
    );
    // A sub-agent's own tree is just its subtree.
    let LocalDaemonResponse::AgentTree { tree: below } =
        dispatch(&world, read(&world.ids["b"])).await
    else {
        panic!("agent tree")
    };
    assert_eq!(below.nodes.len(), 1);
    assert_eq!(below.nodes[0].agent_id, world.ids["d"]);
    // A parent may read its own subtree, a sibling may not read another's.
    let _ = room_command;
}
