// A leased agent's event tools run at home and may wait there for the owner.
// The worker waits for the whole decision window, and a refused watched
// process ends with its cause. Regression, not acceptance.
use super::*;

struct Turn {
    fixture: LiveWorker,
    worker: AgentWorker,
    room: String,
    agent: String,
    token: String,
    task: crate::durable_state::agent_lifecycle::AgentTaskExecution,
}

/// A running leased turn whose worker gives up on a relay request after 3 s.
async fn running_turn() -> Turn {
    let mut fixture = LiveWorker::start().await;
    let mut state = agent_worker_state(&fixture);
    state.config.relay_request_timeout_ms = 3_000;
    let worker = start_agent_worker(&fixture, state).await;
    let room = fixture.rooms[0].clone();
    let attachment = owner_attachment(&fixture, &room).await;
    let agent =
        spawn_on_agent_worker_with_model(&mut fixture, &room, "controlled-cancel-idle").await;
    run_turn(&fixture, &room, &attachment, &agent, "Leased event tools").await;
    let task = store(&fixture)
        .await
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .pop()
        .unwrap();
    let run = worker.run_for(&fixture, &agent, &task.prompt_id).await;
    let token = run.runtime_mcp_auth_token().unwrap().to_string();
    Turn {
        fixture,
        worker,
        room,
        agent,
        token,
        task,
    }
}

impl Turn {
    fn call(
        &self,
        tool: &'static str,
        extra: serde_json::Value,
    ) -> tokio::task::JoinHandle<
        Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError>,
    > {
        let mut arguments =
            json!({"task_id":self.task.task_id,"origin_prompt_id":self.task.prompt_id});
        arguments
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let router = Arc::clone(&self.worker.router);
        let token = self.token.clone();
        tokio::spawn(async move {
            router
                .dispatch_authenticated_runtime_tool_call(&token, tool, arguments)
                .await
        })
    }

    async fn stop(mut self) {
        self.worker.stop().await;
        self.fixture.stop().await;
    }
}

#[test]
fn leased_event_tool_waits_for_home_beyond_the_relay_request_wait() {
    run_test(event_tool_waits_for_home_beyond_the_relay_request_wait);
}

async fn event_tool_waits_for_home_beyond_the_relay_request_wait() {
    let _tools = with_room_tools();
    let turn = running_turn().await;
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    turn.fixture
        .home
        .runtime_state
        .pause_forwarded_meta_request_for_test(entered_tx, release_rx);
    let call = turn.call("chariox.events.wakes", json!({}));
    timeout(Duration::from_secs(10), entered_rx)
        .await
        .expect("home receives the forwarded event tool")
        .expect("home request barrier remains installed");
    // The owner takes longer than the worker's ordinary relay request wait.
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    release_tx.send(()).expect("release the home request");
    let result = timeout(Duration::from_secs(20), call)
        .await
        .expect("the leased call returns")
        .unwrap();
    turn.stop().await;
    let result = result.expect("the event tool outlasts the relay request wait");
    assert!(result.ok, "{result:?}");
}

#[test]
fn leased_process_refusal_ends_with_its_cause() {
    run_test(process_refusal_ends_with_its_cause);
}

async fn process_refusal_ends_with_its_cause() {
    let _tools = with_room_tools();
    let turn = running_turn().await;
    let result = timeout(
        Duration::from_secs(20),
        turn.call(
            "chariox.events.process",
            json!({"argv":["/bin/sleep","1"],"cwd":"/nonexistent","label":"Refused worker source"}),
        ),
    )
    .await
    .expect("the leased call returns")
    .unwrap();
    let store = store(&turn.fixture).await;
    let ended = eventually("home settles the refused source", || {
        store
            .agent_wakes(Some(&turn.room), Some(&turn.agent))
            .unwrap()
            .pop()
            .filter(|w| !matches!(w.state.as_str(), "starting" | "running"))
    })
    .await;
    let notices = format!(
        "{:?}",
        store.agent_inbox(&turn.room, &turn.agent, 0).unwrap()
    );
    let worker_receipts = turn
        .worker
        .router
        .app
        .lock()
        .await
        .durable_state_store()
        .leased_process_receipts()
        .unwrap();
    turn.stop().await;
    let error = result
        .expect_err("a refused command is an error")
        .to_string();
    assert!(
        error.contains("cwd must be inside the leased workspace"),
        "{error}"
    );
    assert!(worker_receipts.is_empty(), "nothing ran on the worker");
    assert_eq!(ended.state, "lost", "{ended:?}");
    assert!(
        notices.contains("launch failed") && notices.contains("cwd must be inside"),
        "{notices}"
    );
    assert!(
        !notices.contains("worker has no durable receipt"),
        "{notices}"
    );
}
