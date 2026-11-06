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
    assert!(
        listed(&world, "ListArchivedAgents").await.is_empty(),
        "the archived list never carries them either"
    );
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
