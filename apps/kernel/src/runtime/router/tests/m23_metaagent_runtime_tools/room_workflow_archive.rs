// Room commands `workflow archive|unarchive|delete`: creator-only, hidden from
// lists, dispatch refusal that names unarchive.
use super::*;

async fn room_command(
    router: &CommandRouter,
    auth: &str,
    command: &str,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    router
        .dispatch_authenticated_runtime_tool_call(
            auth,
            "chariox.room.run_command",
            serde_json::json!({"command":command}),
        )
        .await
        .unwrap()
}

fn listed_aliases(result: &crate::transport::runtime_tools::RuntimeToolResult) -> Vec<String> {
    result.payload["response"]["workflows"]
        .as_array()
        .expect("workflows array")
        .iter()
        .filter_map(|workflow| workflow["alias"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn room_workflow_archive_commands_follow_creator_authority_and_hide_from_lists() {
    run_large_stack_async_test("room-workflow-archive", archive_commands);
}

async fn archive_commands() {
    let env = TestMetaRuntimeEnv::new("room-workflow-archive");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, a) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("peer"))
        .unwrap();
    let a_run = launch_test_provider(
        &mut app,
        session.id(),
        a.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let peer_run = launch_test_provider(
        &mut app,
        session.id(),
        peer.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    let auth_a = a_run.runtime_mcp_auth_token().unwrap().to_string();
    let auth_peer = peer_run.runtime_mcp_auth_token().unwrap().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    assert!(
        room_command(&router, &auth_a, "agent spawn b --provider dev-stub")
            .await
            .ok
    );
    assert!(room_command(&router, &auth_a, "workflow new dup").await.ok);
    assert!(
        room_command(&router, &auth_a, "workflow node add dup b")
            .await
            .ok
    );
    let resolved = room_command(&router, &auth_a, "workflow resolve dup").await;
    let node_id = resolved.payload["response"]["workflow"]["nodes"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        room_command(
            &router,
            &auth_a,
            &format!("workflow endpoint new dup {node_id} start")
        )
        .await
        .ok
    );

    // A4: a peer that did not create the workflow can neither hide nor delete it.
    for command in ["workflow archive dup", "workflow delete dup"] {
        let refused = room_command(&router, &auth_peer, command).await;
        assert!(!refused.ok, "{command}: {refused:?}");
    }
    assert_eq!(
        listed_aliases(&room_command(&router, &auth_a, "workflow list").await),
        ["dup"]
    );

    // A1: the creator archives; it leaves the list, stays readable, refuses dispatch.
    let archived = room_command(&router, &auth_a, "workflow archive dup").await;
    assert!(archived.ok, "{archived:?}");
    assert!(listed_aliases(&room_command(&router, &auth_a, "workflow list").await).is_empty());
    let all = room_command(&router, &auth_a, "workflow list --archived").await;
    assert_eq!(listed_aliases(&all), ["dup"]);
    assert!(all.payload["response"]["workflows"][0]["archived_at_ms"].is_u64());
    assert!(
        room_command(&router, &auth_a, "workflow resolve dup")
            .await
            .ok
    );
    let run = room_command(&router, &auth_a, "workflow run dup start hello").await;
    assert!(!run.ok, "{run:?}");
    assert!(
        run.payload.to_string().contains("workflow unarchive dup"),
        "refusal names unarchive: {run:?}"
    );

    // A2: unarchive lists it again.
    assert!(
        room_command(&router, &auth_a, "workflow unarchive dup")
            .await
            .ok
    );
    assert_eq!(
        listed_aliases(&room_command(&router, &auth_a, "workflow list").await),
        ["dup"]
    );

    // A3: the creator deletes it for good.
    let deleted = room_command(&router, &auth_a, "workflow delete dup").await;
    assert!(deleted.ok, "{deleted:?}");
    assert!(room_command(&router, &auth_a, "workflow list --archived")
        .await
        .payload["response"]["workflows"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(
        !room_command(&router, &auth_a, "workflow resolve dup")
            .await
            .ok
    );
}
