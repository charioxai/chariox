// MP-08 / MP-09 / MP-10 / MP-11 A2: a parent controls everything below it, with no /sudo,
// and nobody else. Tree: a -> {b -> {d}, s}; peer p has no parent; q lives in another room.
use super::*;
use crate::transport::runtime_tools::RuntimeToolResult;

pub(super) struct World {
    pub(super) router: CommandRouter,
    pub(super) app: Arc<Mutex<DaemonApp>>,
    pub(super) room: String,
    pub(super) other_room: String,
    pub(super) tokens: std::collections::BTreeMap<&'static str, String>,
    pub(super) ids: std::collections::BTreeMap<&'static str, String>,
    pub(super) _env: TestMetaRuntimeEnv,
}

pub(super) async fn room_command(world: &World, who: &str, command: &str) -> RuntimeToolResult {
    world
        .router
        .dispatch_authenticated_runtime_tool_call(
            &world.tokens[who],
            "chariox.room.run_command",
            serde_json::json!({"command": command}),
        )
        .await
        .unwrap()
}

pub(super) fn error_text(result: &RuntimeToolResult) -> String {
    result.payload["error"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// A refusal by the tree policy, not a failure of the command itself.
fn refused_by_policy(result: &RuntimeToolResult) -> bool {
    let error = error_text(result);
    !result.ok
        && (error.contains("can control it")
            || error.contains("unavailable in this room")
            || error.contains("outside the caller room"))
}

pub(super) async fn launch(app: &Arc<Mutex<DaemonApp>>, room: &str, agent: &str) -> String {
    let mut app = app.lock().await;
    launch_test_provider(&mut app, room, agent, "dev-stub", "dev-stub", "room-model")
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned()
}

pub(super) async fn world(label: &str) -> World {
    let env = TestMetaRuntimeEnv::new(label);
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (room, a) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let (other, q) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let peer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(room.id(), "dev-stub").with_alias("peer"))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let mut world = World {
        router,
        app: Arc::clone(&app),
        room: room.id().to_owned(),
        other_room: other.id().to_owned(),
        tokens: Default::default(),
        ids: Default::default(),
        _env: env,
    };
    world
        .tokens
        .insert("a", launch(&app, room.id(), a.id()).await);
    world
        .tokens
        .insert("peer", launch(&app, room.id(), peer.id()).await);
    world
        .tokens
        .insert("q", launch(&app, other.id(), q.id()).await);
    world.ids.insert("a", a.id().to_owned());
    world.ids.insert("peer", peer.id().to_owned());
    for child in ["b", "s"] {
        let spawned = room_command(
            &world,
            "a",
            &format!("agent spawn {child} --provider dev-stub"),
        )
        .await;
        assert!(spawned.ok, "{spawned:?}");
        world.ids.insert(
            child,
            spawned.payload["response"]["agent"]["id"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    let b = world.ids["b"].clone();
    world
        .tokens
        .insert("b", launch(&app, &world.room, &b).await);
    let spawned = room_command(&world, "b", "agent spawn d --provider dev-stub").await;
    assert!(spawned.ok, "{spawned:?}");
    world.ids.insert(
        "d",
        spawned.payload["response"]["agent"]["id"]
            .as_str()
            .unwrap()
            .to_owned(),
    );
    world
}

#[test]
fn a_parent_controls_its_subtree_and_nobody_else() {
    run_large_stack_async_test("subtree-command-table", command_table);
}

async fn command_table() {
    let world = world("subtree-command-table").await;
    assert_eq!(
        room_command(&world, "a", "agent children").await.payload["response"]["children"]
            .as_array()
            .unwrap()
            .len(),
        3,
        "a sees b, s and the grandchild d"
    );
    let verbs = [
        "agent cancel {}",
        "agent deadline set {} 30m",
        "agent deadline get {}",
        "agent children {}",
    ];
    // (caller, target): a child and a grandchild are controlled by their ancestors.
    let allowed = [("a", "b"), ("a", "d"), ("b", "d")];
    // sibling, peer, upward, peer to a child, another room.
    let refused = [
        ("b", "s"),
        ("a", "peer"),
        ("b", "a"),
        ("peer", "b"),
        ("q", "b"),
    ];
    for verb in verbs {
        for (caller, target) in allowed {
            let result = room_command(&world, caller, &verb.replace("{}", target)).await;
            assert!(
                !refused_by_policy(&result),
                "{caller} -> {target}: {verb}: {result:?}"
            );
        }
        for (caller, target) in refused {
            let result = room_command(&world, caller, &verb.replace("{}", target)).await;
            assert!(
                refused_by_policy(&result),
                "{caller} -> {target}: {verb}: {result:?}"
            );
        }
    }
    let deadline = room_command(&world, "a", "agent deadline get d").await;
    assert!(deadline.ok, "{deadline:?}");
    assert!(
        deadline.payload["response"]["deadline"]["deadline_ms"]
            .as_u64()
            .is_some(),
        "the grandchild's deadline was armed by its grandparent"
    );
    // A caller that is not a Room agent has no tools at all.
    assert!(world
        .router
        .dispatch_authenticated_runtime_tool_call(
            "not-an-agent-token",
            "chariox.room.run_command",
            serde_json::json!({"command": "agent children"}),
        )
        .await
        .is_err());
    // Room-wide reads are open to every room agent.
    for who in ["a", "peer", "b"] {
        for command in ["agent usage", "agent profiles"] {
            let result = room_command(&world, who, command).await;
            assert!(result.ok, "{who}: {command}: {result:?}");
        }
    }
    let _ = (&world.app, &world.other_room);
}

#[test]
fn only_an_agent_above_may_prompt_an_agent() {
    run_large_stack_async_test("subtree-prompt", prompt_table);
}

// A prompt starts the target's turn, which ends the target's own tool calls for this test:
// the refusals run first, then the allowed prompts from the deepest caller up.
async fn prompt_table() {
    let world = world("subtree-prompt").await;
    // sibling, peer, upward, peer to a child, another room.
    for (caller, target) in [
        ("b", "s"),
        ("a", "peer"),
        ("b", "a"),
        ("peer", "b"),
        ("q", "b"),
    ] {
        let result = room_command(&world, caller, &format!("prompt {target} say hello")).await;
        assert!(
            refused_by_policy(&result),
            "{caller} -> {target}: prompt: {result:?}"
        );
    }
    // a child and a grandchild are prompted by their ancestors.
    for (caller, target) in [("b", "d"), ("a", "s"), ("a", "b")] {
        let result = room_command(&world, caller, &format!("prompt {target} say hello")).await;
        assert!(
            !refused_by_policy(&result),
            "{caller} -> {target}: prompt: {result:?}"
        );
    }
}

#[test]
fn only_the_user_moves_an_agent_in_the_tree() {
    run_large_stack_async_test("subtree-set-parent", set_parent);
}

async fn set_parent() {
    let world = world("subtree-set-parent").await;
    let request = LocalDaemonRequest::SetAgentParent(crate::local::SetAgentParentRequest {
        session_id: world.room.clone(),
        agent_id: world.ids["d"].clone(),
        parent_agent_id: Some(world.ids["peer"].clone()),
    });
    for who in ["a", "b", "peer", "q"] {
        assert!(
            world
                .router
                .runtime_state
                .authorize_room_agent_request(
                    &world
                        .ids
                        .get(who)
                        .cloned()
                        .unwrap_or_else(|| world.ids["a"].clone()),
                    &request
                )
                .is_err(),
            "{who} must not reparent anyone"
        );
    }
    let mut command = KernelCommand::from_local_request("user-set-parent", None, None, &request);
    command.caller = command
        .caller
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    let response = world.router.dispatch(command, request).await.unwrap();
    assert!(matches!(
        response,
        LocalDaemonResponse::AgentParentChanged { .. }
    ));
    assert!(
        refused_by_policy(&room_command(&world, "a", "agent children d").await),
        "d left a's subtree"
    );
    assert!(!refused_by_policy(
        &room_command(&world, "peer", "agent children d").await
    ));
    // Nobody can be placed under itself or under its own descendant.
    let cycle = LocalDaemonRequest::SetAgentParent(crate::local::SetAgentParentRequest {
        session_id: world.room.clone(),
        agent_id: world.ids["a"].clone(),
        parent_agent_id: Some(world.ids["b"].clone()),
    });
    let mut command = KernelCommand::from_local_request("user-cycle", None, None, &cycle);
    command.caller = command
        .caller
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    assert!(world.router.dispatch(command, cycle).await.is_err());
}

#[test]
fn a_spawn_never_has_more_authority_than_its_parent() {
    run_large_stack_async_test("subtree-spawn-level", spawn_level);
}

async fn spawn_level() {
    let world = world("subtree-spawn-level").await;
    // `careful` asks before it acts.
    let careful = {
        let mut app = world.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&world.room, "dev-stub")
                    .with_alias("careful")
                    .with_permission_level_override(
                        crate::provider::AgentPermissionLevel::Required,
                    ),
            )
            .unwrap()
    };
    let mut world = world;
    let token = launch(&world.app, &world.room, careful.id()).await;
    world.tokens.insert("careful", token);
    let above = room_command(
        &world,
        "careful",
        "agent spawn bold --provider dev-stub --permission yolo",
    )
    .await;
    assert!(
        !above.ok && error_text(&above).contains("more authority than its parent"),
        "{above:?}"
    );
    let same = room_command(
        &world,
        "careful",
        "agent spawn calm --provider dev-stub --permission required",
    )
    .await;
    assert!(same.ok, "{same:?}");
    let inherited = room_command(&world, "careful", "agent spawn heir --provider dev-stub").await;
    assert!(inherited.ok, "{inherited:?}");
    let agents = world
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(&world.room);
    for alias in ["calm", "heir"] {
        let child = agents
            .iter()
            .find(|agent| agent.alias() == Some(alias))
            .unwrap();
        assert_eq!(
            child.permission_level_override(),
            Some(crate::provider::AgentPermissionLevel::Required),
            "{alias} starts at its parent's level"
        );
        assert_eq!(child.parent_agent_id(), Some(careful.id()));
    }
    // A parent that runs freely may start a child that asks.
    let lower = room_command(
        &world,
        "a",
        "agent spawn lower --provider dev-stub --permission required",
    )
    .await;
    assert!(lower.ok, "{lower:?}");
}

#[test]
fn a_retried_spawn_with_the_same_command_id_is_one_agent() {
    run_large_stack_async_test("subtree-spawn-command-id", command_id);
}

async fn command_id() {
    let world = world("subtree-spawn-command-id").await;
    let first = room_command(
        &world,
        "a",
        "agent spawn once --provider dev-stub --deadline 30m --command-id retry-1",
    )
    .await;
    let second = room_command(
        &world,
        "a",
        "agent spawn once --provider dev-stub --deadline 30m --command-id retry-1",
    )
    .await;
    assert!(first.ok && second.ok, "{first:?} {second:?}");
    assert_eq!(
        first.payload["response"]["agent"]["id"],
        second.payload["response"]["agent"]["id"]
    );
    let agents = world
        .app
        .lock()
        .await
        .agents()
        .get_session_agents(&world.room);
    assert_eq!(
        agents
            .iter()
            .filter(|agent| agent.alias() == Some("once"))
            .count(),
        1
    );
    let other = room_command(
        &world,
        "a",
        "agent spawn twice --provider dev-stub --command-id retry-2",
    )
    .await;
    assert_ne!(
        first.payload["response"]["agent"]["id"],
        other.payload["response"]["agent"]["id"]
    );
}

#[test]
fn a_spawn_deadline_is_armed_and_never_later_than_the_parents() {
    run_large_stack_async_test("subtree-spawn-deadline", spawn_deadline);
}

async fn spawn_deadline() {
    let world = world("subtree-spawn-deadline").await;
    let timed = room_command(
        &world,
        "a",
        "agent spawn timed --provider dev-stub --deadline 30m",
    )
    .await;
    assert!(timed.ok, "{timed:?}");
    let deadline = room_command(&world, "a", "agent deadline get timed")
        .await
        .payload["response"]["deadline"]["deadline_ms"]
        .as_u64()
        .unwrap();
    let in_half_an_hour = crate::session::unix_epoch_ms() + 30 * 60_000;
    assert!(deadline.abs_diff(in_half_an_hour) < 60_000, "{deadline}");
    // The parent itself ends in ten minutes: a two-hour child ends with it.
    world
        .router
        .runtime_state
        .set_agent_deadline(
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::SetAgentDeadlineRequest {
                session_id: world.room.clone(),
                agent_id: world.ids["a"].clone(),
                deadline_ms: crate::session::unix_epoch_ms() + 10 * 60_000,
            },
        )
        .unwrap();
    let bounded = room_command(
        &world,
        "a",
        "agent spawn bounded --provider dev-stub --deadline 2h",
    )
    .await;
    assert!(bounded.ok, "{bounded:?}");
    let child = room_command(&world, "a", "agent deadline get bounded")
        .await
        .payload["response"]["deadline"]["deadline_ms"]
        .as_u64()
        .unwrap();
    assert!(
        child <= crate::session::unix_epoch_ms() + 10 * 60_000 + 1_000,
        "{child}"
    );
}

#[test]
fn deleting_an_agent_ends_its_sub_agents_with_it() {
    run_large_stack_async_test("subtree-delete-with-children", delete_with_children);
}

async fn delete_with_children() {
    let world = world("subtree-delete-with-children").await;
    let deleted = room_command(&world, "a", "agent delete b").await;
    assert!(deleted.ok, "{deleted:?}");
    let app = world.app.lock().await;
    for gone in ["b", "d"] {
        assert!(
            app.agents().get_agent(&world.ids[gone]).is_err(),
            "{gone} outlived its parent"
        );
    }
    assert!(app.agents().get_agent(&world.ids["s"]).is_ok());
}

#[test]
fn a_relay_refused_remote_spawn_settles_its_obligation() {
    run_large_stack_async_test("subtree-remote-refusal", remote_refusal);
}

async fn remote_refusal() {
    let world = world("subtree-remote-refusal").await;
    let app = world.app.lock().await;
    let store = app.durable_state_store();
    let actor = app.agents().get_agent(&world.ids["a"]).unwrap();
    let session = app.sessions().get_session(&world.room).unwrap();
    let register = || {
        crate::runtime::room_dispatch_registration::register(
            &store,
            &actor,
            app.providers()
                .get_run_for_agent(&world.room, actor.id())
                .as_ref()
                .map(|run| run.id()),
            session
                .active_prompt_for_agent(actor.id())
                .map(|prompt| prompt.id()),
            "delegate",
            None,
        )
        .unwrap()
    };
    let unresolved = |id: &str| {
        store
            .agent_tasks(Some(&world.room), Some(actor.id()))
            .unwrap()
            .iter()
            .flat_map(|task| task.obligations.iter())
            .any(|obligation| obligation.id == id && obligation.requires_resolution())
    };
    // The relay turns the request away: nothing exists over there, the obligation ends rejected.
    let refused = register();
    assert!(unresolved(&refused));
    let error = crate::app::settle_remote_spawn_failure(
        &store,
        Some(&refused),
        "worker",
        DaemonError::LocalTransport {
            operation: "remote spawn",
            message: "relay token does not allow requested action".into(),
        },
    );
    assert!(error
        .to_string()
        .contains("does not allow requested action"));
    assert!(
        !unresolved(&refused),
        "ORCH-11: a definite refusal settles the obligation"
    );
    // No kernel was chosen: nothing was created, so the obligation ends rejected as well.
    let unplaced = register();
    crate::app::settle_remote_spawn_failure(
        &store,
        Some(&unplaced),
        "worker",
        DaemonError::NoRemoteKernelAvailable {
            machine_ref: "worker".into(),
            provider: "codex".into(),
            message: "kernel `worker` is not visible".into(),
        },
    );
    assert!(
        !unresolved(&unplaced),
        "a spawn that found no kernel leaves no open delegate obligation"
    );
    // The worker answered that the working directory is missing on it: it created nothing,
    // so the obligation ends rejected and the parent can go on.
    let missing_dir = register();
    let error = crate::app::settle_remote_spawn_failure(
        &store,
        Some(&missing_dir),
        "worker",
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "transport_error".into(),
            message: "local transport `resolve leased agent working directory` failed: remote working directory `/missing/w1` does not exist".into(),
            retryable: true,
        },
    );
    assert!(error.to_string().contains("does not exist"), "{error}");
    assert!(
        !unresolved(&missing_dir),
        "a worker's answered refusal settles the obligation"
    );
    // The worker dropped off after the relay forwarded the request: it may have created the agent.
    let disconnected = register();
    let error = crate::app::settle_remote_spawn_failure(
        &store,
        Some(&disconnected),
        "worker",
        DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "target_disconnected".into(),
            message: "target daemon disconnected from relay".into(),
            retryable: true,
        },
    );
    assert!(error.to_string().contains("uncertain dispatch"), "{error}");
    assert!(unresolved(&disconnected));
    // Any other failure may have created the agent: the obligation stays for inspection.
    let uncertain = register();
    let error = crate::app::settle_remote_spawn_failure(
        &store,
        Some(&uncertain),
        "worker",
        DaemonError::LocalTransport {
            operation: "remote spawn",
            message: "worker timed out".into(),
        },
    );
    assert!(error.to_string().contains("uncertain dispatch"), "{error}");
    assert!(unresolved(&uncertain));
}
