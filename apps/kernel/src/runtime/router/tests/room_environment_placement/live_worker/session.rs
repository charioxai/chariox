use super::*;

#[test]
fn room_environment_worker_session_alias_attaches_default_agent_to_slice() {
    run_test(session_alias_attaches_default_agent_to_slice);
}

async fn session_alias_attaches_default_agent_to_slice() {
    let mut fixture = LiveWorker::start().await;
    fixture.create_slice().await;
    let placement = fixture.placement();
    let created = dispatch_json(
        &fixture.home,
        json!({"CreateSession": {
            "workspace_id":"workspace", "worktree_id":"worktree",
            "agent_defaults":{"provider":"managed-dev-stub", "model":"default"},
            "kernel_ref":"desktop-worker", "worktree_placement":placement
        }}),
    )
    .await
    .expect("create a Room through its known slice worker alias");
    let session_id = created["SessionCreated"]["session"]["id"].as_str().unwrap();
    let agent = &created["SessionCreated"]["agent"];
    let agent_id = agent["id"].as_str().unwrap();
    let attached = dispatch_json(&fixture.home, json!({"GetSlice":{"slice_ref":"desktop"}}))
        .await
        .unwrap();
    let other_claim = dispatch_json(&fixture.home, bind(&fixture.rooms[1], "desktop")).await;
    dispatch_json(
        &fixture.home,
        json!({"DestroyAgent":{"session_id":session_id,"agent_id":agent_id}}),
    )
    .await
    .expect("delete the Room's leased default agent");
    fixture.stop().await;
    assert_eq!(
        agent["remote_execution"]["worker_kernel_id"],
        "environment-worker"
    );
    assert_eq!(
        attached["Slice"]["slice"]["agent_ids"],
        json!([agent_id]),
        "session creation must preserve the canonical slice attachment"
    );
    assert!(
        other_claim.is_err(),
        "another Room cannot claim the occupied slice"
    );
}
