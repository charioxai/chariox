use super::*;

#[test]
fn room_environment_worker_cleanup_uses_the_slice_private_relay() {
    run_test(cleanup_uses_the_slice_private_relay);
}

async fn cleanup_uses_the_slice_private_relay() {
    let mut fixture = LiveWorker::start_with_private_relay(true).await;
    fixture.create_slice().await;
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent":{
            "session_id":fixture.rooms[0], "provider":"managed-dev-stub", "model":"default",
            "slice_ref":"desktop", "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn through the slice relay with its separate token");
    let agent = &spawned["AgentSpawned"]["agent"];
    let agent_id = agent["id"].as_str().unwrap();
    assert_ne!(
        agent["remote_execution"]["relay_url"],
        json!(fixture.home_state.config.relay_url)
    );
    let destroyed = dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{
            "session_id":fixture.rooms[0], "agent_id":agent_id
        }}),
    )
    .await;
    let listed = dispatch_json(
        &fixture.home,
        json!({"ListAgents":{"session_id":fixture.rooms[0]}}),
    )
    .await
    .unwrap();
    let slice = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    fixture.stop().await;
    let destroyed = destroyed.expect("cleanup must use the bound slice relay, not the home relay");
    assert_eq!(destroyed["AgentDestroyed"]["agent"]["id"], agent_id);
    assert!(listed["AgentsListed"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|agent| agent["id"] != agent_id));
    let slice: crate::slice::SliceRecord =
        serde_json::from_value(slice["Slice"]["slice"].clone()).unwrap();
    assert!(slice.agent_ids.is_empty());
}
