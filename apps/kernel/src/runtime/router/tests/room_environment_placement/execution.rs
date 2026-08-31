use super::*;

#[test]
fn room_environment_execution_rejects_every_cross_room_admission_path() {
    run_test(rejects_every_cross_room_admission_path);
}

async fn rejects_every_cross_room_admission_path() {
    let mut failures = Vec::new();
    for path in [
        "kernel-alias",
        "kernel-id",
        "machine-id",
        "batch",
        "create-slice",
        "create-kernel",
        "move",
    ] {
        let state = TestState::new();
        let (router, rooms) = state.router();
        create_desktop(&router, "reserved").await;
        router
            .app
            .lock()
            .await
            .slices()
            .set_worker_presence(
                "reserved",
                Some("reserved-kernel".to_string()),
                Some("reserved-machine".to_string()),
                Vec::new(),
                1,
            )
            .unwrap();
        dispatch_json(&router, bind(&rooms[0], "reserved"))
            .await
            .unwrap();
        let slice = dispatch_json(&router, json!({"GetSlice":{"slice_ref":"reserved"}}))
            .await
            .unwrap();
        let worker = slice["Slice"]["slice"]["worker_kernel_ref"]
            .as_str()
            .unwrap();
        let agents_query = json!({"ListAgents":{"session_id":rooms[1]}});
        let agents = dispatch_json(&router, agents_query.clone()).await.unwrap();
        let sessions_query = json!({"ListSessions":null});
        let sessions = dispatch_json(&router, sessions_query.clone())
            .await
            .unwrap();
        let agent_id = agents["AgentsListed"]["agents"][0]["id"].as_str().unwrap();
        let request = match path {
            "kernel-alias" => json!({"SpawnAgent":{"session_id":rooms[1],"kernel_ref":worker}}),
            "kernel-id" => {
                json!({"SpawnAgent":{"session_id":rooms[1],"kernel_ref":"reserved-kernel"}})
            }
            "machine-id" => {
                json!({"SpawnAgent":{"session_id":rooms[1],"kernel_ref":"reserved-machine"}})
            }
            "batch" => json!({"SpawnAgents":{"session_id":rooms[1],"agents":[
                {"provider":"codex"}, {"provider":"codex","slice_ref":"reserved"}
            ]}}),
            "create-slice" => json!({"CreateSession":{"workspace_id":"other-workspace",
                "worktree_id":"other-worktree","slice_ref":"reserved"}}),
            "create-kernel" => json!({"CreateSession":{"workspace_id":"other-workspace",
                "worktree_id":"other-worktree","kernel_ref":worker}}),
            "move" => json!({"MoveAgentToRemote":{"session_id":rooms[1],
                "agent_ref":agent_id,"machine_ref":worker}}),
            _ => unreachable!(),
        };
        let error = dispatch_json(&router, request)
            .await
            .unwrap_err()
            .to_string();
        if !error.contains("environment_slice_access_denied") {
            failures.push(format!("{path}: {error}"));
        }
        if dispatch_json(&router, agents_query).await.unwrap() != agents {
            failures.push(format!("{path}: changed existing Room agents"));
        }
        if dispatch_json(&router, sessions_query).await.unwrap() != sessions {
            failures.push(format!("{path}: changed Room list or focus"));
        }
        if dispatch_json(&router, json!({"GetSlice":{"slice_ref":"reserved"}}))
            .await
            .unwrap()
            != slice
        {
            failures.push(format!("{path}: mutated reserved slice"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn room_environment_execution_preserves_owner_admission_and_releases_failed_guards() {
    run_test(preserves_owner_admission_and_releases_failed_guards);
}

async fn preserves_owner_admission_and_releases_failed_guards() {
    for reserved in [false, true] {
        let state = TestState::new();
        let (router, rooms) = state.router();
        create_desktop(&router, "target").await;
        if reserved {
            dispatch_json(&router, bind(&rooms[0], "target"))
                .await
                .unwrap();
        }
        let slice = dispatch_json(&router, json!({"GetSlice":{"slice_ref":"target"}}))
            .await
            .unwrap();
        let slice_id = slice["Slice"]["slice"]["id"].as_str().unwrap();
        let error = dispatch_json(
            &router,
            json!({"SpawnAgents":{
                "session_id":rooms[0], "agents":[
                    {"provider":"codex","slice_ref":"target"},
                    {"provider":"codex","slice_ref":slice_id}
                ]
            }}),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("relay"),
            "owner/legacy admission must reach the offline worker: {error}"
        );
        assert!(
            !error.contains("active") && !error.contains("environment_slice_access_denied"),
            "two aliases for one slice must share one operation guard: {error}"
        );
        if reserved {
            let denied = dispatch_json(
                &router,
                json!({"SpawnAgent":{
                    "session_id":rooms[1],"slice_ref":"target"
                }}),
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(
                denied.contains("environment_slice_access_denied"),
                "failed owner spawn must release its operation marker: {denied}"
            );
        } else {
            dispatch_json(&router, bind(&rooms[1], "target"))
                .await
                .expect("failed legacy admission leaves the unassigned slice available");
        }
    }

    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "available").await;
    create_desktop(&router, "reserved").await;
    dispatch_json(&router, bind(&rooms[0], "reserved"))
        .await
        .unwrap();
    let denied = dispatch_json(
        &router,
        json!({"SpawnAgents":{
            "session_id":rooms[1],"agents":[
                {"slice_ref":"available"}, {"slice_ref":"reserved"}
            ]
        }}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(denied.contains("environment_slice_access_denied"));
    dispatch_json(&router, bind(&rooms[1], "available"))
        .await
        .expect("later preflight failure must release earlier slice guards");
}

#[test]
fn room_environment_execution_rejects_cross_room_spawn_before_worktree_mutation() {
    run_test(rejects_cross_room_spawn_before_worktree_mutation);
}

async fn rejects_cross_room_spawn_before_worktree_mutation() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "reserved").await;
    dispatch_json(&router, bind(&rooms[0], "reserved"))
        .await
        .unwrap();
    let before = dispatch_json(&router, json!({"GetSlice":{"slice_ref":"reserved"}}))
        .await
        .unwrap();
    let result = dispatch_json(
        &router,
        json!({"SpawnAgent":{
            "session_id": rooms[1], "slice_ref": "reserved", "provider": "codex"
        }}),
    )
    .await;
    assert_eq!(
        dispatch_json(&router, json!({"GetSlice":{"slice_ref":"reserved"}}))
            .await
            .unwrap(),
        before,
        "another Room must not mutate the reserved slice before remote spawn fails"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("environment_slice_access_denied"),
        "reject by Room ownership, not by an incidental offline-worker failure"
    );
}
