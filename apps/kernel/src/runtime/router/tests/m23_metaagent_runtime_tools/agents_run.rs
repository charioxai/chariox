// `chariox.agents.run`: the workers' answers come back in the one call, and the caller is
// left with nothing to settle: no inbox event, no obligation, no wake turn.
use super::subtree::{world, World};
use super::*;

const AGENTS_RUN: &str = crate::transport::runtime_tools::AGENTS_RUN_TOOL;

async fn running_turn_of(world: &World, agent: &str) -> String {
    world
        .app
        .lock()
        .await
        .sessions()
        .get_session(&world.room)
        .unwrap()
        .active_prompt_for_agent(agent)
        .expect("the caller's turn is running")
        .id()
        .to_string()
}

/// The workers the call started, once every one of them has its brief as a running turn.
async fn started_workers(world: &World, names: &[&str]) -> Vec<(String, String)> {
    let mut waited = 0;
    loop {
        let session = world
            .router
            .runtime_state
            .session_snapshot(&world.room)
            .await
            .unwrap();
        let agents = world.router.runtime_state.session_agents(&world.room);
        let started = names
            .iter()
            .filter_map(|name| {
                let agent = agents.iter().find(|a| a.alias() == Some(*name))?;
                session.active_prompt_for_agent(agent.id())?;
                Some((name.to_string(), agent.id().to_string()))
            })
            .collect::<Vec<_>>();
        if started.len() == names.len() {
            return started;
        }
        waited += 1;
        assert!(
            waited < 500,
            "workers never started: {:?}",
            agents
                .iter()
                .map(|a| (
                    a.alias().map(str::to_owned),
                    session
                        .active_prompt_for_agent(a.id())
                        .map(|p| p.id().to_string()),
                    session.queued_prompts_for_agent(a.id()).map(|q| q.len()),
                ))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[test]
fn an_agents_run_call_answers_in_place_and_leaves_the_caller_nothing_to_settle() {
    run_large_stack_async_test("agents-run-answers", answers_in_place);
}

async fn answers_in_place() {
    let world = world("agents-run-answers").await;
    let caller = world.ids["a"].clone();
    let prompt = running_turn_of(&world, &caller).await;
    let call = world.router.dispatch_authenticated_runtime_tool_call(
        &world.tokens["a"],
        AGENTS_RUN,
        serde_json::json!({
            "workers": [
                {"name": "w1", "brief": "summarise module one"},
                {"name": "w2", "brief": "summarise module two"},
            ],
            "origin_prompt_id": prompt,
        }),
    );
    let drive = async {
        let workers = started_workers(&world, &["w1", "w2"]).await;
        for (name, agent) in &workers {
            world
                .router
                .runtime_state
                .end_agent_turn_for_test(&world.room, agent, &format!("answer of {name}"))
                .await
                .unwrap();
        }
        workers
            .into_iter()
            .map(|(_, agent)| agent)
            .collect::<Vec<_>>()
    };
    let (result, worker_agents) = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        tokio::join!(call, drive)
    })
    .await
    .expect("the call returns once both workers ended");
    let result = result.unwrap();
    assert!(result.ok, "{:?}", result.payload);
    let workers = result.payload["workers"].as_array().unwrap();
    assert_eq!(workers.len(), 2, "{:?}", result.payload);
    for (worker, name) in workers.iter().zip(["w1", "w2"]) {
        assert_eq!(worker["name"], name, "{worker}");
        assert_eq!(worker["status"], "done", "{worker}");
        assert_eq!(worker["answer"], format!("answer of {name}"), "{worker}");
    }
    assert!(result.payload.get("next_action").is_none());

    // The caller has nothing left to settle: no event for the answers the call returned, no open
    // obligation for the workers it answered and no new turn.
    let app = world.app.lock().await;
    let store = app.durable_state_store();
    let inbox = store.agent_inbox(&world.room, &caller, 0).unwrap();
    assert!(
        inbox.iter().all(|event| event.kind != "child_finished"),
        "{inbox:?}"
    );
    let task = store
        .agent_tasks(Some(&world.room), Some(&caller))
        .unwrap()
        .into_iter()
        .find(|task| task.prompt_id == prompt)
        .expect("the caller's turn has a task");
    let open = task
        .obligations
        .iter()
        .filter(|obligation| {
            obligation.requires_resolution()
                && obligation
                    .resource_id
                    .as_ref()
                    .is_some_and(|resource| worker_agents.contains(resource))
        })
        .collect::<Vec<_>>();
    assert!(open.is_empty(), "{open:?}");
    let session = app.sessions().get_session(&world.room).unwrap();
    assert_eq!(
        session
            .active_prompt_for_agent(&caller)
            .map(|prompt| prompt.id().to_string()),
        Some(prompt),
        "no turn replaced the caller's"
    );
    assert_eq!(
        session
            .queued_prompts_for_agent(&caller)
            .map(|queue| queue.len())
            .unwrap_or(0),
        0,
        "no wake prompt was queued for the caller"
    );
}

#[test]
fn a_worker_that_outlives_the_wait_is_handed_to_the_event_path() {
    run_large_stack_async_test("agents-run-timeout", outlives_the_wait);
}

async fn outlives_the_wait() {
    let world = world("agents-run-timeout").await;
    let caller = world.ids["a"].clone();
    let prompt = running_turn_of(&world, &caller).await;
    let call = world.router.dispatch_agents_run_within(
        &world.tokens["a"],
        serde_json::json!({
            "workers": [
                {"name": "fast", "brief": "answer now"},
                {"name": "slow", "brief": "take your time"},
            ],
            "origin_prompt_id": prompt,
        }),
        std::time::Duration::from_millis(1500),
    );
    let drive = async {
        let workers = started_workers(&world, &["fast", "slow"]).await;
        world
            .router
            .runtime_state
            .end_agent_turn_for_test(&world.room, &workers[0].1, "fast answer")
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(call, drive);
    let result = result.unwrap();
    let workers = result.payload["workers"].as_array().unwrap();
    assert_eq!(workers[0]["status"], "done", "{:?}", result.payload);
    assert_eq!(workers[0]["answer"], "fast answer");
    assert_eq!(workers[1]["status"], "running", "{:?}", result.payload);
    let registration = workers[1]["registration_id"].as_str().expect("a follow");
    assert!(
        result.payload["next_action"]
            .as_str()
            .is_some_and(|text| text.contains(registration)),
        "{:?}",
        result.payload
    );

    // The slow worker ends after the call stopped waiting: its answer reaches the caller once,
    // as an event; the answered worker produced none.
    let slow = workers[1]["agent"].as_str().unwrap().to_string();
    world
        .router
        .runtime_state
        .end_agent_turn_for_test(&world.room, &slow, "slow answer")
        .await
        .unwrap();
    let app = world.app.lock().await;
    let inbox = app
        .durable_state_store()
        .agent_inbox(&world.room, &caller, 0)
        .unwrap();
    let finished = inbox
        .iter()
        .filter(|event| event.kind == "child_finished")
        .collect::<Vec<_>>();
    assert_eq!(finished.len(), 1, "{inbox:?}");
    assert!(
        format!("{:?}", finished[0]).contains("slow answer"),
        "{finished:?}"
    );
}

#[test]
fn a_worker_that_cannot_start_fails_alone() {
    run_large_stack_async_test("agents-run-refusal", refused_worker_fails_alone);
}

async fn refused_worker_fails_alone() {
    let world = world("agents-run-refusal").await;
    let caller = world.ids["a"].clone();
    let prompt = running_turn_of(&world, &caller).await;
    // "peer" is an alias this room already uses, so spawning it is refused.
    let call = world.router.dispatch_authenticated_runtime_tool_call(
        &world.tokens["a"],
        AGENTS_RUN,
        serde_json::json!({
            "workers": [
                {"name": "peer", "brief": "taken name"},
                {"name": "w1", "brief": "fine"},
            ],
            "origin_prompt_id": prompt,
        }),
    );
    let drive = async {
        let workers = started_workers(&world, &["w1"]).await;
        world
            .router
            .runtime_state
            .end_agent_turn_for_test(&world.room, &workers[0].1, "w1 answer")
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(call, drive);
    let result = result.unwrap();
    let workers = result.payload["workers"].as_array().unwrap();
    assert_eq!(workers[0]["status"], "failed", "{:?}", result.payload);
    assert!(workers[0]["reason"].as_str().is_some_and(|r| !r.is_empty()));
    assert_eq!(workers[1]["status"], "done", "{:?}", result.payload);
    assert_eq!(workers[1]["answer"], "w1 answer");
}

#[test]
fn a_call_dropped_before_its_answer_gives_the_finished_workers_back_to_the_event_path() {
    run_large_stack_async_test("agents-run-cancelled", cancelled_call_restores_ends);
}

async fn cancelled_call_restores_ends() {
    let world = world("agents-run-cancelled").await;
    let caller = world.ids["a"].clone();
    let prompt = running_turn_of(&world, &caller).await;
    let call = world.router.dispatch_agents_run(
        &world.tokens["a"],
        serde_json::json!({
            "workers": [
                {"name": "fast", "brief": "answer now"},
                {"name": "slow", "brief": "take your time"},
            ],
            "origin_prompt_id": prompt,
        }),
    );
    let mut call = Box::pin(call);
    let drive = async {
        let workers = started_workers(&world, &["fast", "slow"]).await;
        world
            .router
            .runtime_state
            .end_agent_turn_for_test(&world.room, &workers[0].1, "fast answer")
            .await
            .unwrap();
        // The call has taken the fast end and still waits for the slow worker.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        workers
    };
    let workers = tokio::select! {
        outcome = &mut call => panic!("the call returned while a worker ran: {outcome:?}"),
        workers = drive => workers,
    };
    // The caller's turn is cancelled: the call is dropped before it returns its answer.
    drop(call);

    let store = world.app.lock().await.durable_state_store();
    let mut finished = 0;
    let mut inbox = Vec::new();
    for _ in 0..100 {
        inbox = store.agent_inbox(&world.room, &caller, 0).unwrap();
        finished = inbox
            .iter()
            .filter(|event| {
                event.kind == "child_finished" && format!("{event:?}").contains("fast answer")
            })
            .count();
        if finished > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        finished, 1,
        "the finished worker is announced once: {inbox:?}"
    );

    // The slow worker's end reaches the caller the same way: the dropped call no longer waits.
    world
        .router
        .runtime_state
        .end_agent_turn_for_test(&world.room, &workers[1].1, "slow answer")
        .await
        .unwrap();
    let inbox = store.agent_inbox(&world.room, &caller, 0).unwrap();
    assert_eq!(
        inbox
            .iter()
            .filter(|event| event.kind == "child_finished")
            .count(),
        2,
        "{inbox:?}"
    );
}

#[tokio::test]
async fn meta_controllers_and_regular_agents_both_discover_agents_run() {
    let env = TestMetaRuntimeEnv::new("agents-run-discovery");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, regular) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let controller = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("controller should spawn");
    let controller = activate_test_agent_meta_mode(&mut app, controller);
    let mut token_of = |agent: &str| {
        launch_test_provider(
            &mut app,
            session.id(),
            agent,
            "dev-stub",
            "dev-stub",
            "model",
        )
        .runtime_mcp_auth_token()
        .expect("run exposes a runtime MCP token")
        .to_string()
    };
    let regular_token = token_of(regular.id());
    let controller_token = token_of(controller.id());
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    for (who, token) in [
        ("regular agent", regular_token),
        ("controller", controller_token),
    ] {
        let specs = router.runtime_tool_specs_for_auth_token(&token);
        assert!(
            specs.iter().any(|spec| spec.name == AGENTS_RUN),
            "the {who} must discover {AGENTS_RUN}"
        );
    }
}

#[test]
fn a_call_dropped_while_a_worker_end_is_in_flight_still_hears_of_it() {
    run_large_stack_async_test("agents-run-in-flight", dropped_during_claim);
}

async fn dropped_during_claim() {
    let world = world("agents-run-in-flight").await;
    let caller = world.ids["a"].clone();
    let prompt = running_turn_of(&world, &caller).await;
    let mut call = Box::pin(world.router.dispatch_agents_run(
        &world.tokens["a"],
        serde_json::json!({
            "workers": [{"name": "only", "brief": "answer now"}],
            "origin_prompt_id": prompt,
        }),
    ));
    let workers = tokio::select! {
        outcome = &mut call => panic!("the call returned before its worker ended: {outcome:?}"),
        workers = started_workers(&world, &["only"]) => workers,
    };
    // The worker's end has left the waiter map and is not sent yet when the call is dropped; the
    // drop lets it send right after the call drained its channel.
    let reached = world.router.runtime_state.pause_next_worker_claim();
    let state = world.router.runtime_state.clone();
    let (room, worker) = (world.room.clone(), workers[0].1.clone());
    let ending = tokio::spawn(async move {
        state
            .end_agent_turn_for_test(&room, &worker, "only answer")
            .await
    });
    tokio::select! {
        outcome = &mut call => panic!("the call returned before its end was sent: {outcome:?}"),
        reached = reached => reached.unwrap(),
    }
    drop(call);
    ending.await.unwrap().unwrap();

    // Neither the call's answer nor the event path may lose the end.
    let store = world.app.lock().await.durable_state_store();
    let mut inbox = Vec::new();
    for _ in 0..100 {
        inbox = store.agent_inbox(&world.room, &caller, 0).unwrap();
        if inbox.iter().any(|event| event.kind == "child_finished") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let finished = inbox
        .iter()
        .filter(|event| {
            event.kind == "child_finished" && format!("{event:?}").contains("only answer")
        })
        .count();
    assert_eq!(
        finished, 1,
        "the in-flight end is announced once: {inbox:?}"
    );
}
