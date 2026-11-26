//! "Agents may acquire machines without asking" through the agent's `chariox.machines.acquire`
//! and `chariox.machines.release` tools against a stand-in connected service.
use super::*;
use crate::durable_state::acquired_machines::{AcquiredMachine, AcquiredState};
use crate::local::{JobMachineRate, MachineOption};
use crate::provider::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A connected service offering a small and a big class. It answers acquires as `script` says,
/// replays a reused request_id with its first allocation, and lists each allocation with its state.
struct Service {
    api_url: String,
    acquires: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    machines: Arc<std::sync::Mutex<Vec<Listed>>>,
    script: Arc<std::sync::Mutex<Script>>,
}

struct Listed {
    allocation_ref: String,
    state: String,
    request_id: String,
}

/// How the stand-in service answers.
struct Script {
    acquire: AcquireReply,
    /// The state a released machine is left in.
    release_leaves: &'static str,
    /// The next list takes its snapshot, signals `.0`, and answers only after `.1` is notified.
    held_list: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}

#[derive(Clone)]
enum AcquireReply {
    Machine,
    /// The machine is created, but its answer cannot be read.
    Lost,
    /// Refuses at once: the caller is no longer authorized.
    Refused,
    /// Answers only after the notify, then refuses or creates the machine.
    Held {
        gate: Arc<tokio::sync::Notify>,
        refuse: bool,
    },
}

fn option(class_id: &str, vcpu: u32, amount_micros: u64) -> MachineOption {
    MachineOption {
        class_id: class_id.into(),
        region: "north".into(),
        vcpu,
        ram_gb: f64::from(vcpu) * 2.0,
        disk_gb: 40,
        rate_per_minute: JobMachineRate {
            amount_micros,
            currency: "EUR".into(),
        },
        available: true,
        note: None,
    }
}

fn machine_json(allocation_ref: &str, state: &str) -> serde_json::Value {
    serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
        "allocation_ref":allocation_ref,"state":state,"kernel_target":null,
        "access_state":"pending","access_reason":null})
}

async fn service() -> Service {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let service = Service {
        api_url,
        acquires: Arc::default(),
        machines: Arc::default(),
        script: Arc::new(std::sync::Mutex::new(Script {
            acquire: AcquireReply::Machine,
            release_leaves: "deleted",
            held_list: None,
        })),
    };
    let (seen, listed, script) = (
        service.acquires.clone(),
        service.machines.clone(),
        service.script.clone(),
    );
    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let (seen, listed, script) = (seen.clone(), listed.clone(), script.clone());
            tokio::spawn(answer(socket, seen, listed, script));
        }
    });
    service
}

async fn answer(
    mut socket: tokio::net::TcpStream,
    seen: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    listed: Arc<std::sync::Mutex<Vec<Listed>>>,
    script: Arc<std::sync::Mutex<Script>>,
) {
    let mut bytes = Vec::new();
    let body = loop {
        let mut chunk = [0; 4096];
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
        let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let length: usize = std::str::from_utf8(&bytes[..end])
            .unwrap()
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        if bytes.len() >= end + 4 + length {
            break serde_json::from_slice::<serde_json::Value>(&bytes[end + 4..end + 4 + length])
                .unwrap();
        }
    };
    let request = &body["request"];
    let create = |request_id: &str| {
        let mut machines = listed.lock().unwrap();
        if let Some(replayed) = machines.iter().find(|m| m.request_id == request_id) {
            return machine_json(&replayed.allocation_ref, &replayed.state);
        }
        let allocation_ref = format!("alloc-{}", machines.len() + 1);
        machines.push(Listed {
            allocation_ref: allocation_ref.clone(),
            state: "creating".into(),
            request_id: request_id.into(),
        });
        machine_json(&allocation_ref, "creating")
    };
    let (status, response) = match request["action"].as_str().unwrap() {
        "capabilities" => (
            200,
            serde_json::json!({"kind":"capabilities","machine_allocation":{
            "revision":"v1","actions":["acquire"],"offer":null,
            "offers":[option("small", 2, 1_000), option("big", 8, 4_000)]}})
            .to_string(),
        ),
        "acquire" => {
            seen.lock().unwrap().push(request.clone());
            let request_id = request["request_id"].as_str().unwrap();
            let reply = script.lock().unwrap().acquire.clone();
            match reply {
                AcquireReply::Machine => (200, create(request_id).to_string()),
                AcquireReply::Lost => {
                    create(request_id);
                    (200, "{\"kind\":".into())
                }
                // A refusal main's `acquire_failure` classifies as one: nothing is held under the id.
                AcquireReply::Refused => (
                    400,
                    serde_json::json!({"error":{"code":"invalid_request","message":"not allowed now"}}).to_string(),
                ),
                AcquireReply::Held { gate, refuse } => {
                    gate.notified().await;
                    if refuse {
                        (409, serde_json::json!({"error":{"code":"capacity_exceeded","message":"none left"}}).to_string())
                    } else {
                        (200, create(request_id).to_string())
                    }
                }
            }
        }
        "list" => {
            let snapshot = serde_json::json!({"kind":"machines","saved_states":[],
                "machines": listed.lock().unwrap().iter().map(|m| serde_json::json!({
                    "allocation_ref":m.allocation_ref,"name":m.allocation_ref,"state":m.state,"revision":1,
                    "since":"2026-11-19T00:00:00Z","kernel_id":null,"machine_id":null,"available_actions":[]}))
                    .collect::<Vec<_>>()}).to_string();
            let held = script.lock().unwrap().held_list.take();
            if let Some((reached, gate)) = held {
                reached.notify_one();
                gate.notified().await;
            }
            (200, snapshot)
        }
        "cancel_job" => (
            200,
            serde_json::json!({"kind":"job_cancelled","job_ref":request["job_ref"]}).to_string(),
        ),
        "get_job" => {
            let job_ref = request["job_ref"].as_str().unwrap();
            match listed
                .lock()
                .unwrap()
                .iter()
                .find(|m| m.request_id == job_ref)
            {
                Some(m) => (200, machine_json(&m.allocation_ref, &m.state).to_string()),
                None => (
                    404,
                    serde_json::json!({"error":{"code":"not_found","message":"no job"}})
                        .to_string(),
                ),
            }
        }
        "get" => {
            let allocation_ref = request["allocation_ref"].as_str().unwrap();
            let machines = listed.lock().unwrap();
            let state = &machines
                .iter()
                .find(|m| m.allocation_ref == allocation_ref)
                .unwrap()
                .state;
            (200, machine_json(allocation_ref, state).to_string())
        }
        "release" => {
            let allocation_ref = request["allocation_ref"].as_str().unwrap().to_owned();
            let leaves = script.lock().unwrap().release_leaves;
            for machine in listed.lock().unwrap().iter_mut() {
                if machine.allocation_ref == allocation_ref {
                    machine.state = leaves.into();
                }
            }
            (200, machine_json(&allocation_ref, "deleting").to_string())
        }
        other => panic!("unexpected control-plane action {other}"),
    };
    // The socket may be gone when the caller was cancelled.
    let _ = socket
        .write_all(
            format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .as_bytes(),
        )
        .await;
}

/// A kernel with the owner's offload settings, one running agent connected to `api_url`, and the
/// agent's tool token.
pub(super) async fn agent_kernel(
    api_url: String,
    settings: crate::config::UserAgentsConfig,
) -> (
    KernelRuntimeState,
    String,
    String,
    crate::test_support::TestWorktree,
) {
    let worktree = crate::test_support::TestWorktree::new("machines-without-asking");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.agents = settings;
    let mut app = crate::DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let mut request =
        LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "test-model")
            .with_agent_id(agent.id())
            .with_execution_mode(AgentExecutionMode::Build)
            .with_permission_level(AgentPermissionLevel::Yolo)
            .with_client_interface(ProviderClientInterface::Chariox);
    request.write_access_mode = ProviderWriteAccessMode::Unrestricted;
    let mut run = RuntimeProviderRun::new(
        format!("machine-tool-run-{}", rand::random::<u64>()),
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "machine-tool-test".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: BTreeMap::new(),
            pty_env_remove: vec![],
            working_directory: Some(worktree.path().to_owned()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    let token = format!("machine-tool-token-{}", rand::random::<u64>());
    run.set_runtime_mcp_auth_token(Some(token.clone()));
    app.providers_mut().insert_run_for_test(run);
    let owner = session.owner_user_id().to_owned();
    let room = session.id().to_owned();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    let state = router.runtime_state().with_job_router(router);
    let mut config = state.owned.config_projection.snapshot();
    config.relay_url = Some("wss://relay.test".into());
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-test-credential".into()),
        kernel_public_key_thumbprint: None,
        api_url,
        email: "user@example.test".into(),
        account_id: "account".into(),
        user_id: owner,
        account_slug: "account".into(),
        realm_id: "realm".into(),
        relay_url: "wss://relay.test".into(),
        issuer_id: "issuer".into(),
        client_id: None,
        client_alias: None,
        machine_id: None,
        machine_credential: None,
        cloud_session_token: None,
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: None,
        kernel_unlinked: false,
        kernel_removal_pending: false,
    });
    state.owned.config_projection.update(config);
    (state, token, room, worktree)
}

fn allowed(
    classes: Option<&[&str]>,
    max: Option<u32>,
    cap: Option<u64>,
) -> crate::config::UserAgentsConfig {
    crate::config::UserAgentsConfig {
        offload_without_asking: Some(true),
        offload_classes: classes.map(|classes| classes.iter().map(|c| (*c).to_owned()).collect()),
        offload_max_machines: max,
        offload_daily_cap_micros: cap,
        ..Default::default()
    }
}

/// The key a request has in the ledger: main's derived service request_id for this agent.
fn ledger_key(state: &KernelRuntimeState, agent: &str, request_id: &str) -> String {
    super::choice::service_request_id(
        &state.owned.config_projection.snapshot().daemon_id,
        agent,
        request_id,
    )
}

/// The only running agent's id.
pub(super) fn only_agent(state: &KernelRuntimeState) -> String {
    state.owned.agent_store.list_agents()[0].id().to_owned()
}

pub(super) async fn acquire(
    state: &KernelRuntimeState,
    token: &str,
    request_id: &str,
    class_id: &str,
) -> Result<serde_json::Value, DaemonError> {
    state
        .machine_tool(
            token,
            "chariox.machines.acquire",
            serde_json::json!({"request_id":request_id,"class_id":class_id,"region":"north"}),
        )
        .await
        .map(|result| result.payload)
}

/// Starts an acquire that must wait for the owner, and returns its card's message once it is shown.
pub(super) async fn acquire_waits_for_a_card(
    state: &KernelRuntimeState,
    token: &str,
    room: &str,
    request_id: &str,
    class_id: &str,
) -> String {
    let call = spawn_acquire(state, token, request_id, class_id);
    waits_for_a_card(state, room, call).await
}

fn spawn_acquire(
    state: &KernelRuntimeState,
    token: &str,
    request_id: &str,
    class_id: &str,
) -> tokio::task::JoinHandle<Result<serde_json::Value, DaemonError>> {
    let (state, token, request_id, class_id) = (
        state.clone(),
        token.to_owned(),
        request_id.to_owned(),
        class_id.to_owned(),
    );
    tokio::spawn(async move { acquire(&state, &token, &request_id, &class_id).await })
}

/// Waits until `call` shows the owner's card, and returns the card's message.
async fn waits_for_a_card(
    state: &KernelRuntimeState,
    room: &str,
    call: tokio::task::JoinHandle<Result<serde_json::Value, DaemonError>>,
) -> String {
    let message = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(card) = state
                .owned
                .session_store
                .get_session(room)
                .unwrap()
                .active_interactions()
                .first()
            {
                break card.message().to_owned();
            }
            assert!(!call.is_finished(), "acquire ended with no card");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the owner's card is shown");
    assert!(
        !call.is_finished(),
        "nothing is acquired before the owner answers"
    );
    call.abort();
    message
}

/// Waits until the ledger satisfies `done`.
async fn ledger_reaches(state: &KernelRuntimeState, done: impl Fn(&[AcquiredMachine]) -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let acquired = state
                .owned
                .durable_state_store
                .acquired_machines(0)
                .unwrap();
            if done(&acquired) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "ledger never reached the state: {:?}",
            state.owned.durable_state_store.acquired_machines(0)
        )
    });
}

/// Makes the next attempt at `request_id` stop where it found no row for its request. The receiver
/// hears when it stopped; a send on the sender lets it go on.
fn stop_unrecorded_attempt(
    request_id: &str,
) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
    let (stopped, stopped_heard) = std::sync::mpsc::channel();
    let (go_on, go_on_heard) = std::sync::mpsc::channel::<()>();
    *without_asking::UNRECORDED_ATTEMPT.lock().unwrap() = Some((
        request_id.into(),
        Box::new(move || {
            stopped.send(()).unwrap();
            go_on_heard
                .recv_timeout(Duration::from_secs(10))
                .expect("the test lets the attempt go on");
        }),
    ));
    (stopped_heard, go_on)
}

async fn has_stopped(stopped: std::sync::mpsc::Receiver<()>) {
    tokio::task::spawn_blocking(move || stopped.recv_timeout(Duration::from_secs(5)))
        .await
        .unwrap()
        .expect("the attempt stops where it found no row");
}

/// Waits until the service has received `count` acquires.
async fn acquires_reach(service: &Service, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.acquires.lock().unwrap().len() < count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the attempt reaches the service");
}

/// The service holds the next acquire until the returned gate is notified, then refuses or creates.
fn hold_next_acquire(service: &Service, refuse: bool) -> Arc<tokio::sync::Notify> {
    let gate = Arc::new(tokio::sync::Notify::new());
    service.script.lock().unwrap().acquire = AcquireReply::Held {
        gate: gate.clone(),
        refuse,
    };
    gate
}

fn running(machine: &AcquiredMachine, allocation_ref: &str) -> bool {
    machine.state
        == AcquiredState::Running {
            allocation_ref: allocation_ref.into(),
        }
}

fn notices(state: &KernelRuntimeState) -> Vec<String> {
    state
        .owned
        .terminal_stream
        .notice_records()
        .into_iter()
        .map(|notice| notice.message)
        .collect()
}

#[tokio::test]
async fn within_the_limits_an_agent_acquires_with_no_card_and_the_room_records_class_rate_and_cost()
{
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, None, None)).await;
    let acquired = tokio::time::timeout(
        Duration::from_secs(5),
        acquire(&state, &token, "r1", "small"),
    )
    .await
    .expect("no owner decision is waited for")
    .unwrap();
    assert_eq!(acquired["allocation"]["allocation_ref"], "alloc-1");
    assert_eq!(service.acquires.lock().unwrap().len(), 1);
    assert!(state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .active_interactions()
        .is_empty());
    let started = notices(&state);
    assert!(
        started
            .iter()
            .any(|notice| notice.contains("acquired small without asking at 0.001 EUR per minute")),
        "{started:?}"
    );
    let acquired = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap();
    assert_eq!(acquired.len(), 1);
    assert_eq!(acquired[0].class_id, "small");
    assert_eq!(acquired[0].rate_micros, 1_000);
    assert_eq!(
        acquired[0].state,
        AcquiredState::Running {
            allocation_ref: "alloc-1".into()
        }
    );

    state
        .machine_tool(
            &token,
            "chariox.machines.release",
            serde_json::json!({"allocation_ref":"alloc-1"}),
        )
        .await
        .unwrap();
    // The status read that confirms deletion ends it.
    state
        .machine_tool(
            &token,
            "chariox.machines.status",
            serde_json::json!({"allocation_ref":"alloc-1"}),
        )
        .await
        .unwrap();
    let acquired = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap();
    assert!(matches!(acquired[0].state, AcquiredState::Ended { .. }));
    let ended = notices(&state);
    assert!(
        ended
            .iter()
            .any(|notice| notice.contains("The small machine that")
                && notice.contains("ended after 1 min; it cost 0.001 EUR")),
        "{ended:?}"
    );
}

#[tokio::test]
async fn a_class_the_owner_did_not_allow_asks_the_owner_and_the_card_says_why() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, None, None)).await;
    let message = acquire_waits_for_a_card(&state, &token, &room, "r1", "big").await;
    assert!(
        message
            .contains("Asked because big is not a class agents may acquire without asking (small)"),
        "{message}"
    );
    assert!(service.acquires.lock().unwrap().is_empty());
}

#[tokio::test]
async fn more_machines_than_the_owner_allows_at_the_same_time_ask_the_owner() {
    let service = service().await;
    let (state, token, room, _workspace) = agent_kernel(
        service.api_url.clone(),
        allowed(Some(&["small", "big"]), Some(1), None),
    )
    .await;
    acquire(&state, &token, "r1", "big").await.unwrap();
    let message = acquire_waits_for_a_card(&state, &token, &room, "r2", "small").await;
    assert!(
        message.contains("Asked because the most machines acquired without asking at the same time (1) are already running"),
        "{message}"
    );
    assert_eq!(service.acquires.lock().unwrap().len(), 1);
    // A machine that ended elsewhere frees its place: the service lists it deleted.
    service.machines.lock().unwrap()[0].state = "deleted".into();
    tokio::time::timeout(
        Duration::from_secs(5),
        acquire(&state, &token, "r3", "small"),
    )
    .await
    .expect("no owner decision once a place is free")
    .unwrap();
    assert_eq!(service.acquires.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn spending_at_the_daily_cap_asks_the_owner() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, None, Some(0))).await;
    let message = acquire_waits_for_a_card(&state, &token, &room, "r1", "small").await;
    assert!(
        message.contains("Asked because machines acquired without asking cost 0 EUR today, at the daily cap of 0 EUR"),
        "{message}"
    );
    assert!(service.acquires.lock().unwrap().is_empty());
}

#[tokio::test]
async fn with_the_setting_off_every_acquisition_asks_the_owner() {
    let service = service().await;
    let mut settings = allowed(None, None, None);
    settings.offload_without_asking = Some(false);
    let (state, token, room, _workspace) = agent_kernel(service.api_url.clone(), settings).await;
    let message = acquire_waits_for_a_card(&state, &token, &room, "r1", "small").await;
    assert!(message.starts_with("Acquire small in north"), "{message}");
    assert!(!message.contains("Asked because"), "{message}");
    assert!(service.acquires.lock().unwrap().is_empty());
}

#[test]
fn only_the_owner_changes_the_setting_and_its_limits() {
    for path in [
        "agents.offload_without_asking",
        "agents.offload_classes",
        "agents.offload_max_machines",
        "agents.offload_daily_cap_micros",
    ] {
        let set = LocalDaemonRequest::SetUserConfigValue(crate::local::SetUserConfigValueRequest {
            path: path.into(),
            value: "1".into(),
        });
        let unset =
            LocalDaemonRequest::UnsetUserConfigValue(crate::local::UnsetUserConfigValueRequest {
                path: path.into(),
            });
        for request in [set, unset] {
            assert_eq!(
                crate::runtime::state::reach::reach_need(&request),
                crate::runtime::state::reach::ReachNeed::HumanOnly,
                "{path}"
            );
            // Not inside a /sudo window either: the limits spend the owner's money too.
            assert!(
                crate::runtime::state::sudo::sudo_request_forbidden(&request),
                "{path} under /sudo"
            );
        }
    }
}

/// A card that closes without the owner's answer never reads as the owner's refusal: the agent is
/// told what closed it and that it may retry.
#[tokio::test]
async fn a_card_closed_without_an_answer_names_its_cause_and_never_claims_an_owner_decision() {
    let service = service().await;
    let mut settings = allowed(None, None, None);
    settings.offload_without_asking = Some(false);
    let (state, token, room, _workspace) = agent_kernel(service.api_url.clone(), settings).await;
    for (request_id, close) in [("timed-out", true), ("withdrawn", false)] {
        let (call_state, call_token) = (state.clone(), token.clone());
        let call =
            tokio::spawn(
                async move { acquire(&call_state, &call_token, request_id, "small").await },
            );
        let id = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(card) = state
                    .owned
                    .session_store
                    .get_session(&room)
                    .unwrap()
                    .active_interactions()
                    .first()
                {
                    break card.id().to_owned();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if close {
            // The kernel's own deadline sweep closes it as timed out.
            state.timeout_runtime_interaction(&room, &id).await.unwrap();
        } else {
            // Its responder is gone with no answer at all.
            state.owned.pending_interactions.write().remove(&id);
        }
        let error = tokio::time::timeout(Duration::from_secs(5), call)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(!error.contains("did not approve"), "{error}");
        assert!(!error.contains("denied"), "{error}");
        assert!(error.contains("closed without a decision"), "{error}");
        assert!(
            error.contains(if close {
                "nobody answered it"
            } else {
                "the kernel withdrew it"
            }),
            "{error}"
        );
        assert!(error.contains("retry with the same request_id"), "{error}");
        assert!(service.acquires.lock().unwrap().is_empty());
        // Nothing is left for the owner to answer.
        tokio::time::timeout(Duration::from_secs(5), async {
            while !state
                .owned
                .session_store
                .get_session(&room)
                .unwrap()
                .active_interactions()
                .is_empty()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

/// A machine acquired without asking that a status read finds deleted (its release was queued
/// while it was still starting, or it ended elsewhere) is closed with its cost at once.
#[tokio::test]
async fn a_status_read_that_finds_the_machine_deleted_closes_its_record() {
    let service = service().await;
    let (state, token, _room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, None, None)).await;
    acquire(&state, &token, "r1", "small").await.unwrap();
    service.machines.lock().unwrap()[0].state = "deleted".into();
    state
        .machine_tool(
            &token,
            "chariox.machines.status",
            serde_json::json!({"allocation_ref":"alloc-1"}),
        )
        .await
        .unwrap();
    let acquired = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap();
    assert!(
        matches!(acquired[0].state, AcquiredState::Ended { .. }),
        "{acquired:?}"
    );
    assert!(notices(&state)
        .iter()
        .any(|notice| notice.contains("The small machine that") && notice.contains("it cost")));
}

/// Finding 1: a list the service snapshotted before another acquisition was accepted never ends
/// that machine, so the maximum holds.
#[tokio::test]
async fn a_list_snapshotted_before_an_acquisition_was_accepted_never_frees_its_place() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(2), None)).await;
    acquire(&state, &token, "r0", "small").await.unwrap();
    let (reached, gate) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    service.script.lock().unwrap().held_list = Some((reached.clone(), gate.clone()));
    // B's list is taken now, with only alloc-1, and delivered after A runs.
    let b = spawn_acquire(&state, &token, "b", "small");
    tokio::time::timeout(Duration::from_secs(5), reached.notified())
        .await
        .expect("B asked for the list");
    let a = tokio::time::timeout(
        Duration::from_secs(5),
        acquire(&state, &token, "a", "small"),
    )
    .await
    .expect("A has the second place")
    .unwrap();
    assert_eq!(a["allocation"]["allocation_ref"], "alloc-2");
    gate.notify_one();
    let message = waits_for_a_card(&state, &room, b).await;
    assert!(message.contains("(2) are already running"), "{message}");
    assert_eq!(service.acquires.lock().unwrap().len(), 2);
    let acquired = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap();
    assert!(
        acquired.iter().any(|m| running(m, "alloc-2")),
        "{acquired:?}"
    );
}

/// Finding 2: an acquisition whose answer was lost keeps its place and its cost, and a retry
/// with the same request_id settles it with no card.
#[tokio::test]
async fn an_acquisition_whose_answer_was_lost_keeps_counting_until_a_retry_settles_it() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(1), None)).await;
    service.script.lock().unwrap().acquire = AcquireReply::Lost;
    let error = acquire(&state, &token, "r1", "small")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("retry with request_id \"r1\""), "{error}");
    service.script.lock().unwrap().acquire = AcquireReply::Machine;
    let message = acquire_waits_for_a_card(&state, &token, &room, "r2", "small").await;
    assert!(message.contains("(1) are already running"), "{message}");
    let replayed = tokio::time::timeout(
        Duration::from_secs(5),
        acquire(&state, &token, "r1", "small"),
    )
    .await
    .expect("a retry of an admitted request is not asked again")
    .unwrap();
    assert_eq!(replayed["allocation"]["allocation_ref"], "alloc-1");
    ledger_reaches(&state, |acquired| {
        acquired.len() == 1 && running(&acquired[0], "alloc-1")
    })
    .await;
}

/// Review 5968697039 finding 1: a retry refused after a lost answer proves nothing about the
/// first machine, so the request keeps its place and its cost.
#[tokio::test]
async fn a_refused_retry_of_a_lost_answer_keeps_the_machine_counted() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(1), None)).await;
    service.script.lock().unwrap().acquire = AcquireReply::Lost;
    acquire(&state, &token, "r1", "small").await.unwrap_err();
    service.script.lock().unwrap().acquire = AcquireReply::Refused;
    let refused = acquire(&state, &token, "r1", "small")
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("not allowed now"), "{refused}");
    let acquired = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap();
    assert!(
        acquired.len() == 1
            && acquired[0].request_id == ledger_key(&state, &only_agent(&state), "r1")
            && acquired[0].state.is_live(),
        "the machine that may exist still counts and costs: {acquired:?}"
    );
    service.script.lock().unwrap().acquire = AcquireReply::Machine;
    let message = acquire_waits_for_a_card(&state, &token, &room, "r2", "small").await;
    assert!(message.contains("(1) are already running"), "{message}");
}

/// Review 5969099840: two overlapping attempts at the same request, both refused, each release
/// their own reservation once: the ledger ends empty, and the place and the day's budget are free.
#[tokio::test]
async fn overlapping_attempts_that_are_all_refused_free_the_place_and_the_budget() {
    let service = service().await;
    let (state, token, _room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(1), Some(1_000))).await;
    let gate = Arc::new(tokio::sync::Notify::new());
    service.script.lock().unwrap().acquire = AcquireReply::Held {
        gate: gate.clone(),
        refuse: true,
    };
    let reached = |count: usize| {
        let acquires = service.acquires.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(5), async {
                while acquires.lock().unwrap().len() < count {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the attempt reaches the service");
        }
    };
    let first = spawn_acquire(&state, &token, "r1", "small");
    reached(1).await;
    let second = spawn_acquire(&state, &token, "r1", "small");
    reached(2).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !(first.is_finished() && second.is_finished()) {
            gate.notify_one();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both attempts are answered");
    for call in [first, second] {
        assert!(call.await.unwrap().is_err(), "the service refused it");
    }
    let acquired = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap();
    assert!(acquired.is_empty(), "nothing was acquired: {acquired:?}");
    service.script.lock().unwrap().acquire = AcquireReply::Machine;
    tokio::time::timeout(
        Duration::from_secs(5),
        acquire(&state, &token, "after", "small"),
    )
    .await
    .expect("no owner decision while the place and the budget are free")
    .unwrap();
}

/// Review 5969421887: attempt B at a request finds no row; before B goes on, attempt A creates
/// the row and is sent. B must count in that row, so A's refusal cannot free the place while B is
/// in flight, and B's machine is recorded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attempt_registered_while_a_sibling_admits_keeps_the_place_through_its_refusal() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(1), None)).await;
    let (stopped, go_on) =
        stop_unrecorded_attempt(&ledger_key(&state, &only_agent(&state), "gap-held"));
    let b = spawn_acquire(&state, &token, "gap-held", "small");
    has_stopped(stopped).await;
    let refuse_a = hold_next_acquire(&service, true);
    let a = spawn_acquire(&state, &token, "gap-held", "small");
    acquires_reach(&service, 1).await;
    let create_b = hold_next_acquire(&service, false);
    go_on.send(()).unwrap();
    acquires_reach(&service, 2).await;
    refuse_a.notify_one();
    assert!(a.await.unwrap().is_err(), "the service refused A");
    let acquired = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap();
    assert_eq!(
        acquired.len(),
        1,
        "B is still in flight, so its request keeps its place: {acquired:?}"
    );
    create_b.notify_one();
    b.await.unwrap().expect("the service created B's machine");
    ledger_reaches(&state, |acquired| {
        acquired.len() == 1 && running(&acquired[0], "alloc-1")
    })
    .await;
    acquire_waits_for_a_card(&state, &token, &room, "after", "small").await;
}

/// Attempt B finds no row and, before it goes on, attempt A is admitted, refused and released: B
/// is admitted anew, within the limits, and its machine is recorded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attempt_that_finds_its_siblings_row_released_is_admitted_anew() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(1), None)).await;
    let (stopped, go_on) =
        stop_unrecorded_attempt(&ledger_key(&state, &only_agent(&state), "gap-freed"));
    let b = spawn_acquire(&state, &token, "gap-freed", "small");
    has_stopped(stopped).await;
    service.script.lock().unwrap().acquire = AcquireReply::Refused;
    acquire(&state, &token, "gap-freed", "small")
        .await
        .expect_err("the service refused A");
    ledger_reaches(&state, <[AcquiredMachine]>::is_empty).await;
    service.script.lock().unwrap().acquire = AcquireReply::Machine;
    go_on.send(()).unwrap();
    b.await.unwrap().expect("B goes ahead with no card");
    ledger_reaches(&state, |acquired| {
        acquired.len() == 1 && running(&acquired[0], "alloc-1")
    })
    .await;
    acquire_waits_for_a_card(&state, &token, &room, "after", "small").await;
}

/// Attempts A and B at one request are both in flight; A is refused, then B's machine arrives: the
/// refusal leaves B's count, so the machine is recorded and keeps the place.
#[tokio::test]
async fn a_machine_that_arrives_after_a_siblings_refusal_is_recorded() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(1), None)).await;
    let refuse_a = hold_next_acquire(&service, true);
    let a = spawn_acquire(&state, &token, "both", "small");
    acquires_reach(&service, 1).await;
    let create_b = hold_next_acquire(&service, false);
    let b = spawn_acquire(&state, &token, "both", "small");
    acquires_reach(&service, 2).await;
    refuse_a.notify_one();
    assert!(a.await.unwrap().is_err(), "the service refused A");
    create_b.notify_one();
    b.await.unwrap().expect("the service created B's machine");
    ledger_reaches(&state, |acquired| {
        acquired.len() == 1 && running(&acquired[0], "alloc-1")
    })
    .await;
    acquire_waits_for_a_card(&state, &token, &room, "after", "small").await;
}

/// Review 5968697039 finding 1: a retry that fails its reauthorization was never sent, and it
/// leaves the lost request counted.
#[tokio::test]
async fn a_retry_that_fails_reauthorization_keeps_a_lost_answer_counted() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(1), None)).await;
    service.script.lock().unwrap().acquire = AcquireReply::Lost;
    acquire(&state, &token, "r1", "small").await.unwrap_err();
    let agent = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap()[0]
        .agent_id
        .clone();
    let retry = ControlPlaneRequest::Acquire {
        capability_revision: "v1".into(),
        request_id: ledger_key(&state, &agent, "r1"),
        class_id: Some("small".into()),
        region: Some("north".into()),
        timeout_s: None,
    };
    let without_asking::Admission::Within(hold) = state
        .admit_without_asking(&room, &agent, &retry, None, &limits::AccountBound::None)
        .unwrap()
    else {
        panic!("a retry of an admitted request goes ahead without asking");
    };
    state.withdraw_acquisition(hold);
    let acquired = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap();
    assert!(
        acquired.len() == 1 && acquired[0].state.is_live(),
        "the machine that may exist still counts and costs: {acquired:?}"
    );
    service.script.lock().unwrap().acquire = AcquireReply::Machine;
    let message = acquire_waits_for_a_card(&state, &token, &room, "r2", "small").await;
    assert!(message.contains("(1) are already running"), "{message}");
}

/// Review 5968697039 finding 2: an acquisition without a request_id whose answer was lost names
/// the request_id the kernel chose, and a retry with it settles the machine.
#[tokio::test]
async fn a_lost_answer_names_the_generated_request_id_and_a_retry_with_it_settles() {
    let service = service().await;
    let (state, token, _room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(1), None)).await;
    service.script.lock().unwrap().acquire = AcquireReply::Lost;
    let error = state
        .machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"class_id":"small","region":"north"}),
        )
        .await
        .unwrap_err();
    // The service sees the id the kernel derives; the agent is told, and retries with, its own.
    let derived = service.acquires.lock().unwrap()[0]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let DaemonError::MachineRequest { code, message } = &error else {
        panic!("{error}");
    };
    assert_eq!(code, "acquisition_uncertain");
    let generated = message
        .split("request_id \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("the answer names the request_id to retry with: {message}"))
        .to_owned();
    assert_ne!(generated, derived, "{message}");
    assert!(!message.contains(&derived), "{message}");
    service.script.lock().unwrap().acquire = AcquireReply::Machine;
    let replayed = acquire(&state, &token, &generated, "small").await.unwrap();
    assert_eq!(replayed["allocation"]["allocation_ref"], "alloc-1");
    assert_eq!(
        service.acquires.lock().unwrap().last().unwrap()["request_id"],
        derived,
        "the retry reaches the service under the id the lost try used"
    );
    ledger_reaches(&state, |acquired| {
        acquired.len() == 1 && running(&acquired[0], "alloc-1")
    })
    .await;
}

/// Finding 3: a cancelled acquisition never leaves a hold behind: the service's answer still
/// settles it, as a running machine or as a freed place.
#[tokio::test]
async fn a_cancelled_acquisition_is_settled_by_the_services_answer() {
    let service = service().await;
    let (state, token, _room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(2), None)).await;
    for (request_id, refuse) in [("accepted", false), ("refused", true)] {
        let gate = Arc::new(tokio::sync::Notify::new());
        service.script.lock().unwrap().acquire = AcquireReply::Held {
            gate: gate.clone(),
            refuse,
        };
        let sent = service.acquires.lock().unwrap().len();
        let call = spawn_acquire(&state, &token, request_id, "small");
        tokio::time::timeout(Duration::from_secs(5), async {
            while service.acquires.lock().unwrap().len() == sent {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        gate.notify_one();
        ledger_reaches(&state, |acquired| {
            acquired.len() == 1 && running(&acquired[0], "alloc-1")
        })
        .await;
    }
    // The refused one freed its place: a second machine goes ahead with no card.
    service.script.lock().unwrap().acquire = AcquireReply::Machine;
    tokio::time::timeout(
        Duration::from_secs(5),
        acquire(&state, &token, "after", "small"),
    )
    .await
    .expect("no owner decision while a place is free")
    .unwrap();
}

/// Finding 4: a retry of an accepted request replays its machine with no card and no new place.
#[tokio::test]
async fn a_retry_of_an_accepted_request_replays_it_without_asking_again() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(4), None)).await;
    acquire(&state, &token, "r1", "small").await.unwrap();
    let replayed = tokio::time::timeout(
        Duration::from_secs(5),
        acquire(&state, &token, "r1", "small"),
    )
    .await
    .expect("the accepted request is not asked again")
    .unwrap();
    assert_eq!(replayed["allocation"]["allocation_ref"], "alloc-1");
    assert!(state
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .active_interactions()
        .is_empty());
    let acquired = state
        .owned
        .durable_state_store
        .acquired_machines(0)
        .unwrap();
    assert_eq!(acquired.len(), 1, "{acquired:?}");
    assert!(running(&acquired[0], "alloc-1"));
}

/// Finding 5: a machine being deleted keeps its place and its cost until deletion is confirmed.
#[tokio::test]
async fn a_machine_being_deleted_counts_until_its_deletion_is_confirmed() {
    let service = service().await;
    let (state, token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(1), None)).await;
    service.script.lock().unwrap().release_leaves = "deleting";
    acquire(&state, &token, "r1", "small").await.unwrap();
    state
        .machine_tool(
            &token,
            "chariox.machines.release",
            serde_json::json!({"allocation_ref":"alloc-1"}),
        )
        .await
        .unwrap();
    assert!(!notices(&state).iter().any(|n| n.contains("ended after")));
    let message = acquire_waits_for_a_card(&state, &token, &room, "r2", "small").await;
    assert!(message.contains("(1) are already running"), "{message}");
    service.machines.lock().unwrap()[0].state = "deleted".into();
    state
        .machine_tool(
            &token,
            "chariox.machines.status",
            serde_json::json!({"allocation_ref":"alloc-1"}),
        )
        .await
        .unwrap();
    assert!(notices(&state).iter().any(|n| n.contains("ended after")));
    tokio::time::timeout(
        Duration::from_secs(5),
        acquire(&state, &token, "r3", "small"),
    )
    .await
    .expect("no owner decision once deletion is confirmed")
    .unwrap();
}

/// Finding 3, job path: a job cancelled or found after a restart while its acquisition was
/// pending is settled by the job's cleanup: no machine frees the place; a found machine runs until
/// its confirmed deletion ends it.
#[tokio::test]
async fn a_jobs_cleanup_settles_its_pending_acquisition() {
    use crate::durable_state::acquired_machines::{Admitted, Registered};
    let service = service().await;
    let (state, _token, room, _workspace) =
        agent_kernel(service.api_url.clone(), allowed(None, Some(2), None)).await;
    let store = &state.owned.durable_state_store;
    for job in ["job-gone", "job-found"] {
        assert_eq!(
            store
                .admit_attempt(
                    AcquiredMachine {
                        agent_id: "agent".into(),
                        request_id: format!("job:{job}"),
                        job_ref: Some(job.into()),
                        room_id: room.clone(),
                        class_id: "small".into(),
                        rate_micros: 1_000,
                        currency: "EUR".into(),
                        started_ms: crate::session::unix_epoch_ms(),
                        state: AcquiredState::Pending,
                    },
                    2,
                    None,
                    0,
                )
                .unwrap(),
            Admitted::Registered(Registered::Counted)
        );
    }
    service.machines.lock().unwrap().push(Listed {
        allocation_ref: "alloc-job".into(),
        state: "deleted".into(),
        request_id: "job-found".into(),
    });
    state.job_compute_release_job_ref("job-gone").await.unwrap();
    state
        .job_compute_release_job_ref("job-found")
        .await
        .unwrap();
    let row = |request_id: &str| {
        store
            .acquired_machines(0)
            .unwrap()
            .into_iter()
            .find(|machine| machine.agent_id == "agent" && machine.request_id == request_id)
            .map(|machine| machine.state)
    };
    assert_eq!(row("job:job-gone"), None);
    assert!(matches!(
        row("job:job-found"),
        Some(AcquiredState::Ended { allocation_ref, .. }) if allocation_ref == "alloc-job"
    ));
}
