use super::*;

#[test]
fn room_environment_execution_rejects_worker_identity_collisions_after_binding() {
    crate::test_support::isolated_env_test!();
    run_test(rejects_worker_identity_collisions_after_binding);
}

async fn rejects_worker_identity_collisions_after_binding() {
    let mut failures = Vec::new();
    for reference in ["kernel_ref", "slice_ref"] {
        let state = TestState::new();
        let (router, rooms) = state.router();
        for (index, name) in ["first", "second"].into_iter().enumerate() {
            create_desktop(&router, name).await;
            dispatch_json(&router, bind(&rooms[index], name))
                .await
                .unwrap();
        }
        // Worker discovery changes after reservations were established.
        let slices = router.app.lock().await.slices().clone();
        for name in ["first", "second"] {
            slices
                .set_worker_presence(
                    name,
                    Some("shared-kernel".to_string()),
                    Some("shared-machine".to_string()),
                    Vec::new(),
                    1,
                )
                .unwrap();
        }
        let mut spawn = json!({"session_id":rooms[0],"provider":"codex"});
        spawn[reference] = json!(if reference == "kernel_ref" {
            "shared-machine"
        } else {
            "first"
        });
        let error = dispatch_json(&router, json!({"SpawnAgent":spawn}))
            .await
            .unwrap_err()
            .to_string();
        if !error.contains("worker reference is shared by another slice") {
            failures.push(format!("{reference}: {error}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn room_environment_execution_rejects_every_cross_room_admission_path() {
    crate::test_support::isolated_env_test!();
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
    crate::test_support::isolated_env_test!();
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
    crate::test_support::isolated_env_test!();
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

#[test]
fn room_environment_friendly_alias_rejects_foreign_room_before_execution() {
    run_test(friendly_alias_rejects_foreign_room_before_execution);
}

#[test]
fn room_environment_friendly_alias_rejects_active_operation_before_execution() {
    run_test(friendly_alias_rejects_active_operation_before_execution);
}

async fn friendly_alias_rejects_foreign_room_before_execution() {
    friendly_alias_admission(false).await;
}

async fn friendly_alias_rejects_active_operation_before_execution() {
    friendly_alias_admission(true).await;
}

async fn friendly_alias_admission(busy: bool) {
    let mut failures = Vec::new();
    for path in ["spawn", "batch", "create", "move"] {
        let state = TestState::new();
        let (router, rooms) = state.router();
        create_desktop(&router, "desktop").await;
        let slices = router.app.lock().await.slices().clone();
        let record = slices.resolve("desktop").unwrap();
        assert_eq!(
            record.worker_kernel_ref.len(),
            135,
            "use the new default identity"
        );
        slices
            .set_worker_presence(
                "desktop",
                Some(record.worker_kernel_ref.clone()),
                Some(record.owner_machine_id.clone()),
                vec!["codex".to_string()],
                1,
            )
            .unwrap();
        dispatch_json(&router, bind(&rooms[0], "desktop"))
            .await
            .unwrap();
        let active = busy.then(|| slices.try_begin_operation("desktop", "state.save").unwrap());
        let room = if busy { &rooms[0] } else { &rooms[1] };
        let agents_query = json!({"ListAgents":{"session_id":room}});
        let before_agents = dispatch_json(&router, agents_query.clone()).await.unwrap();
        let sessions_query = json!({"ListSessions":null});
        let before_sessions = dispatch_json(&router, sessions_query.clone())
            .await
            .unwrap();
        let before_slice = dispatch_json(&router, json!({"GetSlice":{"slice_ref":"desktop"}}))
            .await
            .unwrap();
        let agent_id = before_agents["AgentsListed"]["agents"][0]["id"]
            .as_str()
            .unwrap();
        let request = match path {
            "spawn" => json!({"SpawnAgent":{"session_id":room,"kernel_ref":"slice:desktop"}}),
            "batch" => json!({"SpawnAgents":{"session_id":room,"agents":[
                {"provider":"codex"}, {"provider":"codex","kernel_ref":"slice:desktop"}
            ]}}),
            "create" => json!({"CreateSession":{"workspace_id":"other-workspace",
                "worktree_id":"other-worktree","kernel_ref":"slice:desktop"}}),
            "move" => json!({"MoveAgentToRemote":{"session_id":room,
                "agent_ref":agent_id,"machine_ref":"slice:desktop"}}),
            _ => unreachable!(),
        };
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            dispatch_json(&router, request),
        )
        .await
        .expect("admission must settle before worker contact")
        .unwrap_err()
        .to_string();
        let expected = if busy {
            "active `state.save` operation"
        } else {
            "environment_slice_access_denied"
        };
        if !error.contains(expected) {
            failures.push(format!(
                "{path}: expected preflight {expected}, got {error}"
            ));
        }
        assert_eq!(
            dispatch_json(&router, agents_query).await.unwrap(),
            before_agents
        );
        assert_eq!(
            dispatch_json(&router, sessions_query).await.unwrap(),
            before_sessions
        );
        assert_eq!(
            dispatch_json(&router, json!({"GetSlice":{"slice_ref":"desktop"}}))
                .await
                .unwrap(),
            before_slice
        );
        drop(active);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn room_environment_friendly_alias_shares_canonical_admission_without_authorizing_alias() {
    run_test(friendly_alias_shares_canonical_admission_without_authorizing_alias);
}

async fn friendly_alias_shares_canonical_admission_without_authorizing_alias() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "desktop").await;
    let slices = router.app.lock().await.slices().clone();
    let record = slices.resolve("desktop").unwrap();
    dispatch_json(&router, bind(&rooms[0], "desktop"))
        .await
        .unwrap();
    let admission = router
        .runtime_state
        .guard_slice_execution(
            Some(&rooms[0]),
            [
                (None, Some("slice:desktop")),
                (None, Some(record.worker_kernel_ref.as_str())),
            ],
            "agent.spawn",
        )
        .await
        .unwrap();
    assert_eq!(
        admission.slice_ids,
        vec![Some(record.id.clone()), Some(record.id.clone())]
    );
    assert!(slices.try_begin_operation("desktop", "state.save").is_err());
    assert!(
        slices
            .resolve_by_worker_kernel_ref("slice:desktop")
            .is_none(),
        "friendly input resolution must not widen identity authorization"
    );
    drop(admission);
    assert!(slices.try_begin_operation("desktop", "state.save").is_ok());
    assert_eq!(
        router
            .runtime_state
            .guard_slice_execution(
                Some(&rooms[0]),
                [(None, Some("ordinary-remote-worker"))],
                "agent.spawn",
            )
            .await
            .unwrap()
            .slice_ids,
        vec![None]
    );
}

#[test]
fn room_environment_friendly_alias_rejects_canonical_collision() {
    run_test(friendly_alias_rejects_canonical_collision);
}

async fn friendly_alias_rejects_canonical_collision() {
    let state = TestState::new();
    let (router, rooms) = state.router();
    create_desktop(&router, "desktop").await;
    dispatch_json(&router, bind(&rooms[0], "desktop"))
        .await
        .unwrap();
    dispatch_json(
        &router,
        json!({"CreateSlice":{
            "name":"other", "base":"clean", "display_mode":"headed",
            "worker_kernel_ref":"slice:desktop"
        }}),
    )
    .await
    .unwrap();
    let error = dispatch_json(
        &router,
        json!({"SpawnAgent":{
            "session_id":rooms[1], "kernel_ref":"slice:desktop"
        }}),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("ambiguous slice execution reference"),
        "{error}"
    );
}
