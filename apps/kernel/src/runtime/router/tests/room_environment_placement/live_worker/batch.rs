use super::*;

#[test]
fn room_environment_worker_partial_batch_failure_rolls_back_created_agents() {
    run_test(partial_batch_failure_rolls_back_created_agents);
}

async fn partial_batch_failure_rolls_back_created_agents() {
    let mut fixture = LiveWorker::start().await;
    fixture.create_slice().await;
    let query = json!({"ListAgents":{"session_id":fixture.rooms[0]}});
    let before = dispatch_json(&fixture.home, query.clone()).await.unwrap();
    let good_placement = fixture.placement();
    let mut bad_placement = fixture.placement();
    bad_placement["from_ref"] = json!("refs/heads/chariox-test-nonexistent-batch-ref");
    let error = dispatch_json(&fixture.home, json!({"SpawnAgents":{
        "session_id":fixture.rooms[0], "agents":[
            {"provider":"managed-dev-stub", "model":"default", "kernel_ref":"desktop-worker", "worktree_placement":good_placement},
            {"provider":"managed-dev-stub", "model":"default", "slice_ref":"desktop", "worktree_placement":bad_placement}
        ]
    }})).await.expect_err("the second worker cannot create a worktree from a missing ref");
    let after = dispatch_json(&fixture.home, query).await.unwrap();
    let slice = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    // Clean up even against the broken implementation, before asserting rollback.
    let original_ids = before["AgentsListed"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|agent| agent["id"].clone())
        .collect::<Vec<_>>();
    let retained = after["AgentsListed"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|agent| !original_ids.contains(&agent["id"]))
        .collect::<Vec<_>>();
    for agent in &retained {
        dispatch_json(
            &fixture.home,
            json!({"DestroyAgent":{
                "session_id":fixture.rooms[0], "agent_id":agent["id"]
            }}),
        )
        .await
        .expect("remove any retained batch agent before finishing the test");
    }
    fixture.stop().await;
    assert!(
        error
            .to_string()
            .contains("chariox-test-nonexistent-batch-ref"),
        "{error}"
    );
    assert!(
        retained.is_empty(),
        "a failed batch left agents behind: {retained:?}"
    );
    let slice: crate::slice::SliceRecord =
        serde_json::from_value(slice["Slice"]["slice"].clone()).unwrap();
    assert!(
        slice.agent_ids.is_empty(),
        "rolled-back agents must not remain attached"
    );
}
