// MP-08/MP-09/MP-10/MP-11 A10: home-ordered wakes for leased agents through
// the real home kernel, real relay and real worker kernel (dev-stub provider).
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation, Outcome, Registration};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::ClientTarget;

/// Real room tools on both kernels; set before the fixture builds configs.
fn with_room_tools() -> HomePersistenceEnvironment {
    HomePersistenceEnvironment::set("CHARIOX_ROOM_AGENT_TOOLS", "1".as_ref())
}

async fn spawn_leased(fixture: &mut LiveWorker, room: &str) -> (String, String) {
    fixture.create_slice().await;
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent":{
            "session_id":room, "provider":"managed-dev-stub", "model":"terminal-echo-a",
            "slice_ref":"desktop", "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn a leased agent on the real worker");
    let agent = spawned["AgentSpawned"]["agent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let attached = dispatch_json(
        &fixture.home,
        json!({"AttachToSession":{
            "session_id":room,"client_id":"a10-owner","capability_level":"FullTerminal"
        }}),
    )
    .await
    .unwrap();
    let attachment = attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    (agent, attachment)
}

async fn store(fixture: &LiveWorker) -> crate::durable_state::DurableKernelStateStore {
    fixture.home.app.lock().await.durable_state_store()
}

/// A local delegator task that waits on `source` (the leased delegate).
fn delegator_waiting_on(
    store: &crate::durable_state::DurableKernelStateStore,
    room: &str,
    delegator: &str,
    source: &str,
) -> String {
    let Outcome::Task(task) = store
        .agent_lifecycle(Operation::Begin {
            owner: "user-local".into(),
            room: room.into(),
            agent: delegator.into(),
            prompt: "a10-delegator-turn".into(),
            run: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("delegator task admitted")
    };
    store
        .agent_lifecycle(Operation::Subscribe {
            task: task.task_id.clone(),
            prompt: "a10-delegator-turn".into(),
            registration: Registration {
                id: "completion-a10-delegate".into(),
                task_id: task.task_id.clone(),
                source_id: source.into(),
                obligation_id: None,
                source_cursor: 0,
                live: true,
            },
        })
        .unwrap();
    task.task_id
}

async fn leased_run(fixture: &LiveWorker, room: &str, agent: &str) -> String {
    timeout(Duration::from_secs(10), async {
        loop {
            if let Some(run) = fixture
                .home
                .provider_run_projection
                .list_for_session(room)
                .into_iter()
                .find(|run| {
                    run.agent_instance_id() == Some(agent)
                        && run.state() == crate::provider::ProviderRunState::Running
                })
            {
                break run.id().to_string();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("leased turn projects a running worker run")
}

async fn eventually<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    timeout(Duration::from_secs(15), async {
        loop {
            if let Some(value) = probe() {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

async fn local_agent(fixture: &LiveWorker, room: &str, leased: &str) -> String {
    let app = fixture.home.app.lock().await;
    app.agents()
        .list_agents()
        .into_iter()
        .find(|agent| {
            agent.session_id() == room && agent.id() != leased && agent.remote_execution().is_none()
        })
        .expect("the room's local default agent")
        .id()
        .to_string()
}

async fn binding(fixture: &LiveWorker, agent: &str) -> crate::agent::RemoteAgentBinding {
    let app = fixture.home.app.lock().await;
    app.agents()
        .get_agent(agent)
        .unwrap()
        .remote_execution()
        .cloned()
        .expect("leased binding")
}

/// The worker's exact receipt for one home prompt, read over the real relay.
async fn worker_receipt(
    fixture: &LiveWorker,
    agent: &str,
    home_prompt: &str,
) -> Option<crate::transport::relay_peer::LeasedPromptReceipt> {
    let remote = binding(fixture, agent).await;
    match send_peer_request_via_temporary_connection(
        &fixture.home_state.config,
        ClientTarget {
            daemon_id: Some(remote.worker_kernel_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::GetLeasedPromptReceipt {
            leased_agent_id: remote.leased_agent_id,
            home_prompt_id: home_prompt.into(),
        },
    )
    .await
    .expect("read-only worker receipt query")
    {
        RelayPeerResponse::LeasedPromptReceiptQueried { receipt } => receipt,
        other => panic!("unexpected receipt response: {other:?}"),
    }
}

#[test]
fn mp10_a10_leased_delegate_completion_wakes_the_home_delegator() {
    run_test(home_completed_leased_turn_wakes_the_home_delegator);
}

#[test]
fn mp10_a10_worker_completed_leased_turn_wakes_the_home_delegator() {
    run_test(worker_completed_leased_turn_wakes_the_home_delegator);
}

async fn home_completed_leased_turn_wakes_the_home_delegator() {
    leased_delegate_completion_wakes_the_home_delegator(false).await
}

/// The path official providers take: the worker finishes the turn and the
/// home learns it from the leased runtime projection.
async fn worker_completed_leased_turn_wakes_the_home_delegator() {
    leased_delegate_completion_wakes_the_home_delegator(true).await
}

/// A leased child finishing its turn is a home-committed task outcome: the
/// local delegator that waits on it receives exactly one completion event.
/// On the base the leased completion never settled the home task, so the
/// delegator kept waiting with nothing in its inbox.
async fn leased_delegate_completion_wakes_the_home_delegator(worker_completes: bool) {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start().await;
    let room = fixture.rooms[0].clone();
    let (child, attachment) = spawn_leased(&mut fixture, &room).await;
    let parent = local_agent(&fixture, &room, &child).await;
    let store = store(&fixture).await;
    delegator_waiting_on(&store, &room, &parent, &child);
    dispatch_json(
        &fixture.home,
        json!({"FocusAgent":{"session_id":room,"agent_id":child}}),
    )
    .await
    .unwrap();
    dispatch_json(
        &fixture.home,
        json!({"SubmitPrompt":{
            "session_id":room,"attachment_id":attachment,"target_agent_id":child,
            "prompt":"MP-10 A10 delegated leased work"
        }}),
    )
    .await
    .expect("submit the delegated turn through the real relay");
    leased_run(&fixture, &room, &child).await;
    fixture
        .home
        .runtime_state
        .record_leased_answer_for_test(&room, &child, "MP-10 A10 delegated answer")
        .unwrap();
    if worker_completes {
        // As in the real drill: the owner watches the delegator meanwhile.
        dispatch_json(
            &fixture.home,
            json!({"FocusAgent":{"session_id":room,"agent_id":parent}}),
        )
        .await
        .unwrap();
        let leased = binding(&fixture, &child).await.leased_agent_id;
        let (backing_session, _) =
            crate::app::RemoteLeaseRuntime::new(&mut *fixture.worker.app.lock().await)
                .leased_agent_backing(&leased)
                .expect("worker backing agent");
        dispatch_json(
            &fixture.worker,
            json!({"CompletePrompt":{"session_id":backing_session}}),
        )
        .await
        .expect("the worker finishes the leased turn");
    } else {
        dispatch_json(&fixture.home, json!({"CompletePrompt":{"session_id":room}}))
            .await
            .expect("the leased turn completes on the worker");
    }
    let events = eventually("the delegator's completion event", || {
        let events: Vec<_> = store
            .agent_inbox(&room, &parent, 0)
            .unwrap()
            .into_iter()
            .filter(|event| event.source_id == child)
            .collect();
        (!events.is_empty()).then_some(events)
    })
    .await;
    let child_tasks = store.agent_tasks(Some(&room), Some(&child)).unwrap();
    fixture
        .worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true)
        .unwrap();
    fixture.stop().await;
    assert_eq!(events.len(), 1, "one completion event: {events:?}");
    assert_eq!(events[0].kind, "source_completed", "{events:?}");
    assert!(
        child_tasks.iter().all(|task| task
            .provider_run_id
            .as_deref()
            .is_some_and(|run| run.starts_with("leased:"))),
        "home task binds the exact projected worker run: {child_tasks:?}"
    );
}

#[test]
fn mp10_a10_wake_to_leased_agent_is_home_ordered_with_one_worker_run() {
    run_test(wake_to_leased_agent_is_home_ordered_with_one_worker_run);
}

/// A durable event for a leased agent is delivered by the home kernel as one
/// leased prompt whose worker acceptance is the event's exact receipt. A lost
/// worker reply (partition after acceptance) is reconciled from the worker's
/// receipt; it never admits a second provider turn. Base: refused outright.
async fn wake_to_leased_agent_is_home_ordered_with_one_worker_run() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start().await;
    let room = fixture.rooms[0].clone();
    let (leased, _) = spawn_leased(&mut fixture, &room).await;
    let store = store(&fixture).await;
    let Outcome::Event(event) = store
        .agent_lifecycle(Operation::Occur(ledger::occurrence(
            &room,
            &leased,
            "a10-source",
            "a10-occurrence-1",
            "source_completed",
            json!({"summary":"MP-10 A10 source finished"}),
        )))
        .unwrap()
    else {
        panic!("durable occurrence")
    };
    // Partition after acceptance: the worker admits the turn, its reply is lost.
    fixture
        .worker
        .app
        .lock()
        .await
        .relay_client_state()
        .write()
        .await
        .test_lose_next_leased_prompt_response();
    let delivered = fixture
        .home
        .runtime_state
        .deliver_agent_inbox_for_test(&room, &leased)
        .await;
    assert!(
        delivered.is_ok(),
        "leased delivery is home-ordered, not refused: {delivered:?}"
    );
    let prompt = format!("agent-event-{leased}-{}", event.sequence);
    let settled = eventually("exact receipt after the lost reply", || {
        store
            .agent_event_for_prompt(&room, &leased, &prompt)
            .unwrap()
            .filter(|event| event.state == "accepted")
    })
    .await;
    // Redelivery attempts are idempotent while the turn runs.
    fixture
        .home
        .runtime_state
        .deliver_agent_inbox_for_test(&room, &leased)
        .await
        .unwrap();
    let receipt = worker_receipt(&fixture, &leased, &prompt).await;
    let run = leased_run(&fixture, &room, &leased).await;
    let after = store
        .agent_event_for_prompt(&room, &leased, &prompt)
        .unwrap();
    fixture
        .worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true)
        .unwrap();
    fixture.stop().await;
    let receipt = receipt.expect("worker holds the exact receipt for the home prompt");
    // MP-08/MP-10/MP-11: the admitted provider can finish before this sample.
    // Its completed receipt still binds the same home prompt and worker run;
    // the identity checks below must hold in either phase.
    assert!(matches!(
        receipt.phase,
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Active
            | crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed
    ));
    // One attempt: the home id carries no retry suffix and still names the
    // single worker run the receipt reports.
    assert_eq!(settled.provider_run_id.as_deref(), Some(run.as_str()));
    assert_eq!(
        run,
        crate::provider::projected_leased_provider_run_id(
            &binding_leased_agent_id(&run),
            &receipt.worker_provider_run_id
        )
    );
    assert_eq!(after.map(|event| event.state), Some("accepted".to_string()));
}

fn binding_leased_agent_id(projected: &str) -> String {
    projected
        .strip_prefix("leased:")
        .and_then(|rest| rest.split_once(':'))
        .map(|(leased, _)| leased.to_string())
        .unwrap()
}

/// An independent worker kernel on the fixture relay (placement R) that the
/// test can restart from its own durable state.
struct AgentWorker {
    state: TestState,
    router: Arc<CommandRouter>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

const AGENT_WORKER: &str = "a10-agent-worker";

fn agent_worker_state(fixture: &LiveWorker) -> TestState {
    let mut state = TestState::new();
    state.config.relay_url = fixture.home_state.config.relay_url.clone();
    state.config.relay_token = fixture.home_state.config.relay_token.clone();
    state.config.relay_heartbeat_ms = 50;
    state.config.daemon_id = AGENT_WORKER.into();
    state.config.daemon_alias = Some(AGENT_WORKER.into());
    state.config.host_machine_id = "a10-agent-worker-machine".into();
    state
}

async fn start_agent_worker(fixture: &LiveWorker, state: TestState) -> AgentWorker {
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(
            DaemonApp::bootstrap(state.config.clone()).expect("agent worker bootstrap"),
        )),
        2,
    ));
    let (stop, receiver) = watch::channel(false);
    let relay_state = router.app.lock().await.relay_client_state();
    let task = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router(
            Arc::clone(&router),
            relay_state,
            receiver,
        ),
    );
    eventually_async("the agent worker joins the relay", || async {
        send_peer_request_via_temporary_connection(
            &fixture.home_state.config,
            ClientTarget {
                daemon_id: Some(AGENT_WORKER.into()),
                daemon_alias: None,
            },
            RelayPeerRequest::Ping {
                value: "a10-ready".into(),
            },
        )
        .await
        .ok()
    })
    .await;
    AgentWorker {
        state,
        router,
        stop,
        task,
    }
}

impl AgentWorker {
    /// Stops this worker kernel and starts it again from the same state.
    async fn restart(self, fixture: &LiveWorker) -> AgentWorker {
        let AgentWorker {
            state,
            router,
            stop,
            task,
        } = self;
        let _ = router
            .app
            .lock()
            .await
            .teardown_provider_processes(Some("managed-dev-stub"), true);
        let _ = stop.send(true);
        let _ = timeout(Duration::from_secs(5), task).await;
        drop(router);
        wait_for_durable_owner_release(&state.config.durable_state_path()).await;
        start_agent_worker(fixture, state).await
    }

    async fn stop(self) {
        let _ = self
            .router
            .app
            .lock()
            .await
            .teardown_provider_processes(Some("managed-dev-stub"), true);
        let _ = self.stop.send(true);
        let _ = timeout(Duration::from_secs(5), self.task).await;
    }

    /// Wait for the exact home prompt's active receipt on the current lease.
    /// An older Running projection on the same lease is not this turn.
    async fn run_for(
        &self,
        fixture: &LiveWorker,
        agent: &str,
        home_prompt: &str,
    ) -> crate::provider::RuntimeProviderRun {
        eventually_async(
            "the current lease projects the exact worker prompt",
            || async {
                let remote = binding(fixture, agent).await;
                let receipt = worker_receipt(fixture, agent, home_prompt).await?;
                if receipt.phase != crate::transport::relay_peer::LeasedPromptReceiptPhase::Active
                    || receipt.execution_lease_id.as_deref()
                        != Some(remote.execution_lease_id.as_str())
                {
                    return None;
                }
                let projected = crate::provider::projected_leased_provider_run_id(
                    &remote.leased_agent_id,
                    &receipt.worker_provider_run_id,
                );
                fixture
                    .home
                    .provider_run_projection
                    .list_for_session(&fixture.rooms[0])
                    .into_iter()
                    .find(|run| {
                        run.id() == projected
                            && run.agent_instance_id() == Some(agent)
                            && run.state() == crate::provider::ProviderRunState::Running
                    })?;
                self.router
                    .app
                    .lock()
                    .await
                    .providers()
                    .get_run(&receipt.worker_provider_run_id)
                    .ok()
            },
        )
        .await
    }
}

async fn eventually_async<T, F: std::future::Future<Output = Option<T>>>(
    what: &str,
    mut probe: impl FnMut() -> F,
) -> T {
    timeout(Duration::from_secs(15), async {
        loop {
            if let Some(value) = probe().await {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

async fn spawn_on_agent_worker(fixture: &mut LiveWorker, room: &str) -> String {
    let placement = fixture.placement();
    let spawned = dispatch_json(
        &fixture.home,
        json!({"SpawnAgent":{
            "session_id":room, "provider":"managed-dev-stub", "model":"terminal-echo-a",
            "kernel_ref":AGENT_WORKER, "worktree_placement":placement
        }}),
    )
    .await
    .expect("spawn a leased agent on the independent worker");
    spawned["AgentSpawned"]["agent"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn owner_attachment(fixture: &LiveWorker, room: &str) -> String {
    let attached = dispatch_json(
        &fixture.home,
        json!({"AttachToSession":{
            "session_id":room,"client_id":"a10-terminal","capability_level":"FullTerminal"
        }}),
    )
    .await
    .unwrap();
    attached["SessionAttached"]["attachment"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn run_turn(fixture: &LiveWorker, room: &str, attachment: &str, agent: &str, text: &str) {
    dispatch_json(
        &fixture.home,
        json!({"FocusAgent":{"session_id":room,"agent_id":agent}}),
    )
    .await
    .unwrap();
    dispatch_json(
        &fixture.home,
        json!({"SubmitPrompt":{
            "session_id":room,"attachment_id":attachment,"target_agent_id":agent,"prompt":text
        }}),
    )
    .await
    .expect("submit a leased turn through the real relay");
    leased_run(fixture, room, agent).await;
}

#[test]
fn mp10_a10_wake_reaches_a_leased_agent_after_its_worker_restarts() {
    run_test(wake_reaches_a_leased_agent_after_its_worker_restarts);
}

/// The worker kernel restarts between two turns. A durable wake for its
/// leased agent is then delivered by the home over the current lease and
/// settles on the restarted worker's exact receipt. Base: refused outright.
async fn wake_reaches_a_leased_agent_after_its_worker_restarts() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start().await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let attachment = owner_attachment(&fixture, &room).await;
    let leased = spawn_on_agent_worker(&mut fixture, &room).await;
    run_turn(
        &fixture,
        &room,
        &attachment,
        &leased,
        "MP-10 A10 turn before restart",
    )
    .await;
    fixture
        .home
        .runtime_state
        .record_leased_answer_for_test(&room, &leased, "MP-10 A10 answer before restart")
        .unwrap();
    dispatch_json(&fixture.home, json!({"CompletePrompt":{"session_id":room}}))
        .await
        .expect("the first leased turn completes");
    let worker = worker.restart(&fixture).await;
    let store = store(&fixture).await;
    let Outcome::Event(event) = store
        .agent_lifecycle(Operation::Occur(ledger::occurrence(
            &room,
            &leased,
            "a10-source",
            "a10-after-restart",
            "source_completed",
            json!({"summary":"MP-10 A10 wake after worker restart"}),
        )))
        .unwrap()
    else {
        panic!("durable occurrence")
    };
    let prompt = format!("agent-event-{leased}-{}", event.sequence);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let delivered = loop {
        let attempt = fixture
            .home
            .runtime_state
            .deliver_agent_inbox_for_test(&room, &leased)
            .await;
        let current = store.agent_inbox(&room, &leased, 0).unwrap();
        if let Some(event) = current
            .iter()
            .find(|event| event.state == "accepted" && event.prompt_id.as_deref() == Some(&prompt))
        {
            break event.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "wake not accepted after the worker restart: last delivery {attempt:?}, inbox {current:?}, binding {:?}",
            binding(&fixture, &leased).await
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let receipt = worker_receipt(&fixture, &leased, &prompt).await;
    let current = binding(&fixture, &leased).await;
    worker.stop().await;
    fixture.stop().await;
    let receipt = receipt.expect("the restarted worker holds the wake's exact receipt");
    assert_eq!(
        receipt.execution_lease_id.as_deref(),
        Some(current.execution_lease_id.as_str()),
        "the wake ran on the current lease"
    );
    assert_eq!(
        delivered.provider_run_id,
        Some(crate::provider::projected_leased_provider_run_id(
            &current.leased_agent_id,
            &receipt.worker_provider_run_id
        ))
    );
}

/// The owner opens a sudo window for `agent` through the real entry and
/// answers the passkey popup; returns the owner and the verification time.
async fn open_leased_sudo(
    fixture: &LiveWorker,
    room: &str,
    attachment: &str,
    agent: &str,
    window_length: Duration,
) -> (String, std::time::Instant) {
    use crate::local::{ApprovalPasskey, KernelConnectionClass, PasskeyPromptKind};
    let room = room.to_string();
    let attachment = attachment.to_string();
    let agent = agent.to_string();
    crate::runtime::state::SUDO_WINDOW_LENGTH_FOR_TEST
        .lock()
        .unwrap()
        .insert(room.clone(), window_length);
    let owner = fixture
        .home
        .app
        .lock()
        .await
        .sessions()
        .get_session(&room)
        .unwrap()
        .owner_user_id()
        .to_string();
    let state = fixture.home.runtime_state.clone();
    let request = crate::local::SubmitPromptRequest {
        session_id: room.clone(),
        attachment_id: attachment.clone(),
        target_agent_id: Some(agent.clone()),
        prompt: "/sudo MP-10 A10 list the host's sessions".into(),
        attachments: Vec::new(),
    };
    let submitter = owner.clone();
    let submitted = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, &submitter, "a10-terminal")
            .await
    });
    let popup = eventually("the owner's sudo passkey popup", || {
        fixture
            .home
            .runtime_state
            .passkey_prompts_for(&owner)
            .into_iter()
            .find(|prompt| prompt.kind == PasskeyPromptKind::Sudo)
    })
    .await;
    fixture
        .home
        .runtime_state
        .answer_terminal_runtime_interaction(
            &popup.session_id,
            &popup.interaction_id,
            "approve",
            None,
            Some(&owner),
            Some(&ApprovalPasskey::new(OWNER_PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .expect("the owner approves with a fresh passkey");
    let verified = std::time::Instant::now();
    timeout(Duration::from_secs(15), submitted)
        .await
        .expect("sudo entry returns")
        .unwrap()
        .expect("the home admits leased elevation");
    (owner, verified)
}

const OWNER_PASSKEY: &str = "a10 fixture owner passkey";

#[test]
fn mp10_a10_leased_sudo_window_is_enforced_on_home_and_worker() {
    run_test(leased_sudo_window_is_enforced_on_home_and_worker);
}

/// The owner opens a sudo window for a leased agent with a fresh passkey.
/// The worker lists the privileged tool only for that agent's elevated turn
/// and forwards each call home, where the same window is checked again. A
/// sibling leased agent on the same worker inherits nothing. When the window
/// expires both kernels refuse. Base: the home refuses leased sudo outright.
async fn leased_sudo_window_is_enforced_on_home_and_worker() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start_with_home_passkey(OWNER_PASSKEY).await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let attachment = owner_attachment(&fixture, &room).await;
    let agent = spawn_on_agent_worker(&mut fixture, &room).await;
    let sibling = spawn_on_agent_worker(&mut fixture, &room).await;
    run_turn(
        &fixture,
        &room,
        &attachment,
        &sibling,
        "MP-10 A10 regular sibling turn",
    )
    .await;
    let window_length = Duration::from_secs(8);
    let (owner, verified) =
        open_leased_sudo(&fixture, &room, &attachment, &agent, window_length).await;
    let home_prompt = fixture
        .home
        .runtime_state
        .list_sudo_turns(&owner)
        .pop()
        .unwrap()
        .prompt_id
        .unwrap();
    let run = worker.run_for(&fixture, &agent, &home_prompt).await;
    let token = run.runtime_mcp_auth_token().unwrap().to_string();
    let worker_session = run.session_id().to_string();
    let attached = dispatch_json(
        &worker.router,
        json!({"AttachToSession":{
            "session_id":worker_session,"client_id":"a10-worker-terminal",
            "capability_level":"FullTerminal"
        }}),
    )
    .await
    .unwrap();
    let worker_owner = worker
        .router
        .app
        .lock()
        .await
        .sessions()
        .get_session(&worker_session)
        .unwrap()
        .owner_user_id()
        .to_string();
    let worker_local_sudo = timeout(
        Duration::from_secs(2),
        worker.router.runtime_state.submit_sudo_prompt(
            crate::local::SubmitPromptRequest {
                session_id: worker_session,
                attachment_id: attached["SessionAttached"]["attachment"]["id"]
                    .as_str()
                    .unwrap()
                    .into(),
                target_agent_id: run.agent_instance_id().map(str::to_owned),
                prompt: "/sudo MP-10 A10 worker-local entry".into(),
                attachments: Vec::new(),
            },
            &worker_owner,
            "a10-worker-terminal",
        ),
    )
    .await;
    let lists = |token: &str| {
        worker
            .router
            .runtime_tool_specs_for_auth_token(token)
            .iter()
            .any(|spec| spec.name == "chariox_kernel_request")
    };
    eventually("the worker lists the tool for the elevated turn", || {
        lists(&token).then_some(())
    })
    .await;
    let request = json!({"request":{"ListSessions":null}});
    let allowed = worker
        .router
        .dispatch_authenticated_runtime_tool_call(&token, "chariox_kernel_request", request.clone())
        .await;
    let sibling_prompt = store(&fixture)
        .await
        .agent_tasks(Some(&room), Some(&sibling))
        .unwrap()
        .pop()
        .unwrap()
        .prompt_id;
    let sibling_run = worker.run_for(&fixture, &sibling, &sibling_prompt).await;
    let sibling_token = sibling_run.runtime_mcp_auth_token().unwrap().to_string();
    let sibling_listed = lists(&sibling_token);
    let sibling_call = worker
        .router
        .dispatch_authenticated_runtime_tool_call(
            &sibling_token,
            "chariox_kernel_request",
            request.clone(),
        )
        .await;
    // The home alone decides: a call naming the sibling's own lease and turn
    // is refused there too, even if a worker were to forward it.
    let sibling_context = worker
        .router
        .runtime_state
        .leased_forward_context(&sibling_run)
        .await
        .unwrap()
        .unwrap();
    // Check the sibling's own current lease and turn over the real relay,
    // then the elevated agent's binding as a positive control.
    let forward = |context| {
        send_peer_request_via_temporary_connection(
            &worker.state.config,
            ClientTarget {
                daemon_id: Some(fixture.home_state.config.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::ForwardMetaRuntimeTool {
                context,
                tool_name: "chariox_kernel_request".into(),
                arguments: request.clone(),
            },
        )
    };
    let sibling_at_home = forward(sibling_context).await;
    let elevated_context = worker
        .router
        .runtime_state
        .leased_forward_context(&run)
        .await
        .unwrap()
        .unwrap();
    let elevated_at_home = forward(elevated_context.clone()).await;
    // Each current causal binding is required independently. A mismatched
    // prompt, provider run or execution lease must deny at home even while
    // this agent's sudo window is live; a valid request still works afterward.
    let mut stale_prompt = elevated_context.clone();
    stale_prompt.home_prompt_id = Some("a10-previous-home-prompt".into());
    let mut stale_run = elevated_context.clone();
    stale_run.worker_provider_run_id = "a10-previous-worker-run".into();
    let mut stale_lease = elevated_context.clone();
    stale_lease.leased_agent_id = "a10-previous-leased-agent".into();
    let stale_prompt_result = forward(stale_prompt).await;
    let stale_run_result = forward(stale_run).await;
    let stale_lease_result = forward(stale_lease).await;
    let valid_after_denials = forward(elevated_context.clone()).await;
    // Expiry: both ends refuse once the home deadline passes.
    tokio::time::sleep(
        window_length.saturating_sub(verified.elapsed()) + Duration::from_millis(500),
    )
    .await;
    fixture.home.runtime_state.sweep_sudo();
    let home_live_after = fixture.home.runtime_state.list_sudo_turns(&owner);
    let listed_after = lists(&token);
    let expired_call = worker
        .router
        .dispatch_authenticated_runtime_tool_call(&token, "chariox_kernel_request", request.clone())
        .await;
    let expired_at_home = forward(elevated_context).await;
    worker.stop().await;
    fixture.stop().await;
    assert!(
        worker_local_sudo
            .as_ref()
            .is_ok_and(|result| result.as_ref().is_err_and(|error| error
                .to_string()
                .contains("authorized only by its home kernel"))),
        "the worker cannot independently elevate a leased backing agent: {worker_local_sudo:?}"
    );
    let allowed = allowed.expect("the elevated leased turn acts as the host");
    assert!(allowed.ok, "{allowed:?}");
    assert!(
        allowed.payload.to_string().contains("SessionsListed"),
        "{allowed:?}"
    );
    assert!(
        !sibling_listed,
        "the sibling leased agent never lists the tool"
    );
    assert!(
        sibling_call.as_ref().is_err_and(|error| error
            .to_string()
            .contains("no live sudo window on this worker")),
        "the worker refuses an unelevated leased turn: {sibling_call:?}"
    );
    assert!(
        sibling_at_home
            .as_ref()
            .is_err_and(|error| error.to_string().contains("no sudo authority")),
        "the home refuses the sibling's turn: {sibling_at_home:?}"
    );
    assert!(
        matches!(&elevated_at_home, Ok(RelayPeerResponse::MetaRuntimeToolHandled { result }) if result.ok),
        "control: the same path admits the elevated turn: {elevated_at_home:?}"
    );
    assert!(stale_prompt_result.is_err(), "stale prompt must deny");
    assert!(stale_run_result.is_err(), "stale provider run must deny");
    assert!(
        stale_lease_result.is_err(),
        "stale leased binding must deny"
    );
    assert!(
        matches!(&valid_after_denials, Ok(RelayPeerResponse::MetaRuntimeToolHandled { result }) if result.ok),
        "valid binding remains usable after the denials: {valid_after_denials:?}"
    );
    // After expiry the home refuses at its forwarded-binding or window fence.
    assert!(
        expired_at_home.is_err(),
        "the home refuses after expiry too: {expired_at_home:?}"
    );
    assert!(
        home_live_after.is_empty(),
        "home window ended: {home_live_after:?}"
    );
    assert!(!listed_after, "the worker stops listing the tool at expiry");
    assert!(
        expired_call.as_ref().is_err_and(|error| error
            .to_string()
            .contains("no live sudo window on this worker")),
        "the worker refuses after expiry: {expired_call:?}"
    );
}

#[test]
fn mp10_a10_leased_room_tools_act_at_home_with_the_lease_origin() {
    run_test(leased_room_tools_act_at_home_with_the_lease_origin);
}

/// Placement parity: a leased agent's event and message tools run against
/// the home ledger with its current lease as provider origin, and the worker
/// keeps no task ledger of its own for the backing turn. Base: event tools
/// hit the worker ledger and durable messages lack a provider origin.
async fn leased_room_tools_act_at_home_with_the_lease_origin() {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start().await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let attachment = owner_attachment(&fixture, &room).await;
    let leased = spawn_on_agent_worker(&mut fixture, &room).await;
    let peer = local_agent(&fixture, &room, &leased).await;
    run_turn(
        &fixture,
        &room,
        &attachment,
        &leased,
        "MP-10 A10 leased room tools",
    )
    .await;
    let store = store(&fixture).await;
    let task = store
        .agent_tasks(Some(&room), Some(&leased))
        .unwrap()
        .pop()
        .expect("the home admitted the leased turn's task");
    let run = worker.run_for(&fixture, &leased, &task.prompt_id).await;
    let token = run.runtime_mcp_auth_token().unwrap().to_string();
    let inbox = worker
        .router
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "chariox.events.inbox",
            json!({"task_id":task.task_id,"origin_prompt_id":task.prompt_id,"after":0}),
        )
        .await;
    let sent = worker
        .router
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "chariox.send_agent_message",
            json!({"agent":peer,"message":"MP-10 A10 leased hello","origin_prompt_id":task.prompt_id}),
        )
        .await;
    let peer_inbox = store.agent_inbox(&room, &peer, 0).unwrap();
    let worker_tasks = worker
        .router
        .app
        .lock()
        .await
        .durable_state_store()
        .agent_tasks(None, None)
        .unwrap();
    worker.stop().await;
    fixture.stop().await;
    assert!(
        inbox.as_ref().is_ok_and(|r| r.ok),
        "the leased event tool reads the home ledger: {inbox:?}"
    );
    assert!(
        sent.as_ref().is_ok_and(|r| r.ok),
        "the leased agent sends a durable message: {sent:?}"
    );
    assert!(
        peer_inbox
            .iter()
            .any(|event| event.source_id == leased && event.kind == "message"),
        "the home records the message from the leased agent: {peer_inbox:?}"
    );
    assert!(
        worker_tasks.is_empty(),
        "the worker supervises no task of its own: {worker_tasks:?}"
    );
}

#[test]
fn mp10_a10_elevated_leased_work_continues_after_a_wake() {
    run_test(elevated_leased_work_continues_after_a_wake);
}

/// The elevated leased turn sets a timer and yields through its worker (the
/// home ledger records both), the worker finishes the turn, and a wake
/// correlated with the same owner work starts a continuation that passes the
/// worker fence and the home window again. Base: the leased continuation was
/// never admitted under the window's work hold.
async fn elevated_leased_work_continues_after_a_wake() {
    leased_work_continues_after_a_wake(false).await;
}

#[test]
fn mp10_a10_worker_restart_resumes_leased_work_without_elevation() {
    run_test(worker_restart_resumes_leased_work_without_elevation);
}

async fn worker_restart_resumes_leased_work_without_elevation() {
    leased_work_continues_after_a_wake(true).await;
}

async fn leased_work_continues_after_a_wake(restart_worker: bool) {
    let _tools = with_room_tools();
    let mut fixture = LiveWorker::start_with_home_passkey(OWNER_PASSKEY).await;
    let worker = start_agent_worker(&fixture, agent_worker_state(&fixture)).await;
    let room = fixture.rooms[0].clone();
    let attachment = owner_attachment(&fixture, &room).await;
    let agent = spawn_on_agent_worker(&mut fixture, &room).await;
    let (owner, _) = open_leased_sudo(
        &fixture,
        &room,
        &attachment,
        &agent,
        Duration::from_secs(1800),
    )
    .await;
    let window = fixture
        .home
        .runtime_state
        .list_sudo_turns(&owner)
        .pop()
        .expect("live window");
    let task = window.task_id.clone().expect("window bound to owner work");
    let prompt = window.prompt_id.clone().unwrap();
    let first = worker.run_for(&fixture, &agent, &prompt).await;
    let token = first.runtime_mcp_auth_token().unwrap().to_string();
    let call = |name: &'static str, args: Value| {
        let worker = &worker;
        let token = token.clone();
        async move {
            worker
                .router
                .dispatch_authenticated_runtime_tool_call(&token, name, args)
                .await
        }
    };
    let timer = call(
        "chariox.events.timer",
        json!({"task_id":task,"origin_prompt_id":prompt,"delay_ms":600000,"label":"a10"}),
    )
    .await
    .expect("the leased timer registers at home");
    let registration = timer.payload["registration_id"]
        .as_str()
        .unwrap()
        .to_string();
    call(
        "chariox.events.yield",
        json!({"task_id":task,"origin_prompt_id":prompt,"registration_ids":[registration],
               "inbox_cursor":0,"reason":"MP-10 A10 waits on its timer","deadline_ms":crate::session::unix_epoch_ms() + 600_000}),
    )
    .await
    .expect("the leased yield commits at home");
    fixture
        .home
        .runtime_state
        .record_leased_answer_for_test(&room, &agent, "MP-10 A10 waiting on the timer")
        .unwrap();
    let leased = binding(&fixture, &agent).await.leased_agent_id;
    let (backing_session, _) =
        crate::app::RemoteLeaseRuntime::new(&mut *worker.router.app.lock().await)
            .leased_agent_backing(&leased)
            .expect("worker backing agent");
    dispatch_json(
        &worker.router,
        json!({"CompletePrompt":{"session_id":backing_session}}),
    )
    .await
    .expect("the worker finishes the elevated turn");
    let store = store(&fixture).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let tasks = store.agent_tasks(Some(&room), Some(&agent)).unwrap();
        if tasks.iter().any(|t| {
            t.task_id == task
                && t.state == crate::durable_state::agent_lifecycle::ExecutionState::Waiting
        }) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let windows = fixture.home.runtime_state.list_sudo_turns(&owner);
            let inbox = store.agent_inbox(&room, &agent, 0).unwrap();
            panic!("elevated task never waited: tasks {tasks:?}, windows {windows:?}, inbox {inbox:?}, binding {:?}", binding(&fixture, &agent).await);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Restart from the same durable state while the home-owned timer wait
    // remains open. Its next correlated turn must run as regular work.
    let worker = if restart_worker {
        worker.restart(&fixture).await
    } else {
        worker
    };
    let Outcome::Event(event) = store
        .agent_lifecycle(Operation::Occur(ledger::occurrence(
            &room,
            &agent,
            &registration,
            "a10-correlated",
            "source_completed",
            json!({"task_id":task,"summary":"MP-10 A10 correlated result"}),
        )))
        .unwrap()
    else {
        panic!("durable occurrence")
    };
    let delivered = fixture
        .home
        .runtime_state
        .deliver_agent_inbox_for_test(&room, &agent)
        .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let accepted = loop {
        let inbox = store.agent_inbox(&room, &agent, 0).unwrap();
        if let Some(e) = inbox
            .iter()
            .find(|e| e.sequence == event.sequence && e.state == "accepted")
        {
            break e.clone();
        }
        if tokio::time::Instant::now() >= deadline {
            let tasks = store.agent_tasks(Some(&room), Some(&agent)).unwrap();
            let windows = fixture.home.runtime_state.list_sudo_turns(&owner);
            let retry = fixture
                .home
                .runtime_state
                .deliver_agent_inbox_for_test(&room, &agent)
                .await;
            panic!("continuation not accepted: first delivery {delivered:?}, retry {retry:?}, inbox {inbox:?}, tasks {tasks:?}, windows {windows:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let next = worker
        .run_for(&fixture, &agent, accepted.prompt_id.as_deref().unwrap())
        .await;
    let continuation_context = if restart_worker {
        let app = worker.router.app.lock().await;
        let active = app
            .prompt_owner_active_prompt_for_agent_snapshot(
                next.session_id(),
                next.agent_instance_id().unwrap(),
            )
            .unwrap()
            .expect("the restarted worker holds the continuation");
        Some((
            active.prompt().contains("MP-10 A10 correlated result"),
            active.hidden_system_context().to_string(),
        ))
    } else {
        None
    };
    let next_token = next.runtime_mcp_auth_token().unwrap().to_string();
    let listed = worker
        .router
        .runtime_tool_specs_for_auth_token(&next_token)
        .iter()
        .any(|spec| spec.name == "chariox_kernel_request");
    let elevated = worker
        .router
        .dispatch_authenticated_runtime_tool_call(
            &next_token,
            "chariox_kernel_request",
            json!({"request":{"ListSessions":null}}),
        )
        .await;
    fixture.home.runtime_state.sweep_sudo();
    let remaining_windows = fixture.home.runtime_state.list_sudo_turns(&owner);
    worker.stop().await;
    fixture.stop().await;
    if let Some((current_wake, context)) = continuation_context {
        assert!(current_wake, "context must belong to this admitted wake");
        assert!(
            context.contains("MP-10 A10 list the host's sessions"),
            "a fresh continuation must retain the original user request"
        );
        assert!(
            context.contains("MP-10 A10 waits on its timer"),
            "a fresh continuation must retain its previous yield state"
        );
    }
    assert!(delivered.is_ok(), "{delivered:?}");
    assert!(accepted.prompt_id.is_some());
    if restart_worker {
        assert!(!listed, "the restarted worker lists no privileged tool");
        assert!(elevated.is_err(), "the restarted worker denies elevation");
        assert!(
            remaining_windows.is_empty(),
            "the home ends the lost window"
        );
    } else {
        assert!(
            listed,
            "the continuation lists the tool under the live fence"
        );
        assert!(
            elevated.as_ref().is_ok_and(|r| r.ok),
            "the continuation acts as the host on both ends: {elevated:?}"
        );
    }
}
