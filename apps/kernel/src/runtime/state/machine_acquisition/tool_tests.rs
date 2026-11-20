//! The agent's `chariox.machines.acquire` tool against a stand-in connected service.
use super::*;
use crate::durable_state::acquire_choices::AcquireChoiceState;
use crate::local::{ControlPlaneMachineTarget, MachineAllocationCapability, MachineOption};
use crate::provider::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// How the stand-in service answers the next acquire.
enum Answer {
    /// Commits the allocation, then fails with this status and Cloud code (none for a bare 5xx).
    FailAfterCommit(&'static str, Option<&'static str>),
    /// Commits the allocation, then closes the connection: the response is lost.
    LoseResponse,
    /// Refuses with this status and code, commits nothing and leaves the request open.
    Refuse(&'static str, &'static str),
    /// Refuses like `Refuse` at once, but delivers the refusal only when `gate` is notified.
    RefuseLater(&'static str, &'static str, Arc<tokio::sync::Notify>),
    /// Commits the allocation, notifies `arrived` and delivers the success only when `release` is
    /// notified.
    AcceptLater {
        arrived: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    },
}

struct Service {
    api_url: String,
    acquires: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    /// The options the service lists; a test changes them to change the stock.
    offers: Arc<std::sync::Mutex<Vec<MachineOption>>>,
    /// Answers for the next acquires, before the service's own judgement.
    answers: Arc<std::sync::Mutex<std::collections::VecDeque<Answer>>>,
    capability_failures: Arc<std::sync::Mutex<CapabilityFailures>>,
    ready_target: Arc<std::sync::Mutex<Option<ControlPlaneMachineTarget>>>,
}

/// How the stand-in service answers the next capability calls, in order; it answers at once when
/// none is queued.
enum CapabilityAnswer {
    Answer,
    Fail(&'static str, &'static str),
    /// Notifies `arrived` when the call comes in, and answers only when `release` is notified.
    Hold {
        arrived: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    },
}

type CapabilityFailures = std::collections::VecDeque<CapabilityAnswer>;

/// A connected service that lists `offers` and, like Cloud, binds each accepted request id to its
/// class and region and replays it, whatever the stock says now. A new request for a class named
/// in `sold_out` is refused and, like Cloud, closed: every later try of it is refused as closed.
async fn service(offers: Vec<MachineOption>, sold_out: &'static [&'static str]) -> Service {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let acquires = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = acquires.clone();
    let offers = Arc::new(std::sync::Mutex::new(offers));
    let listed = offers.clone();
    let answers = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let next_answer = answers.clone();
    let capability_failures = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let next_capability_failure = capability_failures.clone();
    let ready_target = Arc::new(std::sync::Mutex::new(None::<ControlPlaneMachineTarget>));
    let target = ready_target.clone();
    let mut accepted = BTreeMap::<String, (serde_json::Value, serde_json::Value, String)>::new();
    let mut closed = std::collections::BTreeSet::<String>::new();
    let mut released = std::collections::BTreeSet::<String>::new();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0 && bytes.len() < 131072);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let length: usize = std::str::from_utf8(&bytes[..header_end])
                .unwrap()
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < header_end + length {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            let request = &body["request"];
            let error = |code: &str| serde_json::json!({"error":{"code":code,"message":code}});
            let sold_out_closed = serde_json::json!({"error":{"code":"capacity_exceeded","message":"sold out","details":{"request_id":"closed"}}});
            let (status, response) = match request["action"].as_str().unwrap() {
                "capabilities" => {
                    let answer = next_capability_failure.lock().unwrap().pop_front();
                    let capability = MachineAllocationCapability {
                        revision: "v1".into(),
                        actions: vec!["acquire".into()],
                        offer: None,
                        offers: listed.lock().unwrap().clone(),
                        job_offers: vec![],
                        job_default_class_id: None,
                    };
                    let response =
                        serde_json::json!({"kind":"capabilities","machine_allocation":capability});
                    match answer {
                        Some(CapabilityAnswer::Fail(status, code)) => (status, error(code)),
                        Some(CapabilityAnswer::Hold { arrived, release }) => {
                            arrived.notify_one();
                            tokio::spawn(async move {
                                release.notified().await;
                                respond(socket, "200 OK", response).await;
                            });
                            continue;
                        }
                        Some(CapabilityAnswer::Answer) | None => ("200 OK", response),
                    }
                }
                "acquire" => {
                    seen.lock().unwrap().push(request.clone());
                    let id = request["request_id"].as_str().unwrap().to_owned();
                    let pair = (request["class_id"].clone(), request["region"].clone());
                    if closed.contains(&id) {
                        respond(socket, "429 Too Many Requests", sold_out_closed).await;
                        continue;
                    }
                    let answer = next_answer.lock().unwrap().pop_front();
                    let commit = |accepted: &mut BTreeMap<_, _>| {
                        let count = accepted.len() + 1;
                        accepted
                            .entry(id.clone())
                            .or_insert_with(|| {
                                (
                                    pair.0.clone(),
                                    pair.1.clone(),
                                    format!("allocation-{count}"),
                                )
                            })
                            .clone()
                    };
                    match (answer, accepted.get(&id).cloned()) {
                        (Some(Answer::Refuse(status, code)), _) => (status, error(code)),
                        (Some(Answer::RefuseLater(status, code, gate)), _) => {
                            let response = error(code);
                            tokio::spawn(async move {
                                gate.notified().await;
                                respond(socket, status, response).await;
                            });
                            continue;
                        }
                        (Some(Answer::AcceptLater { arrived, release }), _) => {
                            let (_, _, allocation_ref) = commit(&mut accepted);
                            tokio::spawn(async move {
                                arrived.notify_one();
                                release.notified().await;
                                respond(
                                    socket,
                                    "200 OK",
                                    serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
                                        "allocation_ref":allocation_ref,"state":"creating","kernel_target":null,
                                        "access_state":"pending","access_reason":null}),
                                )
                                .await;
                            });
                            continue;
                        }
                        (_, Some((ref class, ref region, _)))
                            if (class, region) != (&pair.0, &pair.1) =>
                        {
                            ("409 Conflict", error("identity_conflict"))
                        }
                        (None, None)
                            if pair
                                .0
                                .as_str()
                                .is_some_and(|class| sold_out.contains(&class)) =>
                        {
                            closed.insert(id);
                            ("429 Too Many Requests", sold_out_closed)
                        }
                        (Some(Answer::LoseResponse), _) => {
                            commit(&mut accepted);
                            continue;
                        }
                        (Some(Answer::FailAfterCommit(status, code)), _) => {
                            commit(&mut accepted);
                            (status, code.map(error).unwrap_or(serde_json::json!({})))
                        }
                        (None, _) => {
                            let (_, _, allocation_ref) = commit(&mut accepted);
                            (
                                "200 OK",
                                serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
                                    "allocation_ref":allocation_ref,"state":"creating","kernel_target":null,
                                    "access_state":"pending","access_reason":null}),
                            )
                        }
                    }
                }
                "get" | "release" => {
                    let allocation_ref = request["allocation_ref"].as_str().unwrap();
                    assert!(accepted.values().any(|(_, _, id)| id == allocation_ref));
                    if request["action"] == "release" {
                        assert_eq!(request["expected_revision"], 1);
                        released.insert(allocation_ref.to_owned());
                    }
                    let (state, revision, actions, target) = if released.contains(allocation_ref) {
                        ("deleted", 2, vec![], None)
                    } else {
                        (
                            "ready",
                            1,
                            vec!["stop", "release"],
                            target.lock().unwrap().clone(),
                        )
                    };
                    (
                        "200 OK",
                        serde_json::json!({"kind":"machine","revision":revision,"available_actions":actions,
                            "allocation_ref":allocation_ref,"state":state,"kernel_target":target,
                            "access_state":"pending","access_reason":null}),
                    )
                }
                other => panic!("unexpected control-plane action {other}"),
            };
            respond(socket, status, response).await;
        }
    });
    Service {
        api_url,
        acquires,
        offers,
        answers,
        capability_failures,
        ready_target,
    }
}

async fn respond(mut socket: tokio::net::TcpStream, status: &str, response: serde_json::Value) {
    let response = response.to_string();
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

/// A kernel whose owner allows automatic allocation, one running agent, and its tool token.
async fn agent_kernel(
    api_url: String,
) -> (
    KernelRuntimeState,
    String,
    crate::test_support::TestWorktree,
) {
    let worktree = crate::test_support::TestWorktree::new("machine-tool");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.agents.offload_without_asking = Some(true);
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
        kernel_unlinked: false,
        kernel_removal_pending: false,
        token_expires_at_ms: None,
    });
    state.owned.config_projection.update(config);
    (state, token, worktree)
}

fn option(class_id: &str, region: &str, available: bool) -> MachineOption {
    super::tests::option(class_id, region, available)
}

#[tokio::test]
async fn an_unverified_ready_machine_reports_reconnecting_and_can_be_released() {
    let service = service(vec![option("small", "north", true)], &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let acquired = state
        .machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"request_id":"rental"}),
        )
        .await
        .unwrap();
    let args =
        serde_json::json!({"allocation_ref":acquired.payload["allocation"]["allocation_ref"]});
    let status = state
        .machine_tool(&token, "chariox.machines.status", args.clone())
        .await
        .unwrap();
    assert_eq!(status.payload["state"], "reconnecting");
    assert_eq!(status.payload["access_state"], "pending");
    assert_eq!(status.payload["access_reason"], "kernel_target_unverified");
    assert_eq!(
        status.payload["available_actions"],
        serde_json::json!(["stop", "release"])
    );

    let presence = chariox_relay::protocol::RelayKernelPresence {
        kernel_id: "rental-kernel".into(),
        machine_id: "rental-machine".into(),
        public_key: "rental-key".into(),
        machine_alias: None,
        relay_alias: None,
        kernel_alias: None,
        capabilities: vec![],
        available_providers: vec![],
        provider_accounts: vec![],
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
        local_protocol: Some(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION),
    };
    *service.ready_target.lock().unwrap() = Some(ControlPlaneMachineTarget {
        kernel_id: presence.kernel_id.clone(),
        machine_id: presence.machine_id.clone(),
        relay_realm_id: "realm".into(),
        key_thumbprint: "a".repeat(64),
        presence: None,
    });
    let inventory = &state.owned.notification_inventory_projection;
    inventory.update(Default::default(), vec![presence]);
    let gateway = state.job_router.as_ref().unwrap().kernel_gateway();
    assert_eq!(gateway.kernels().await.unwrap().len(), 1);
    inventory.update(Default::default(), vec![]);
    let status = state
        .machine_tool(&token, "chariox.machines.status", args.clone())
        .await
        .unwrap();
    assert_eq!(status.payload["state"], "reconnecting");
    assert_eq!(status.payload["access_state"], "pending");
    assert_eq!(status.payload["access_reason"], "kernel_offline");

    let release = state
        .machine_tool(&token, "chariox.machines.release", args.clone())
        .await
        .unwrap();
    assert_eq!(release.payload["state"], "deleted");
    let status = state
        .machine_tool(&token, "chariox.machines.status", args)
        .await
        .unwrap();
    assert_eq!(status.payload["state"], "deleted");
}

#[tokio::test]
async fn acquire_without_a_class_takes_the_first_option_in_stock() {
    let service = service(
        vec![
            option("big", "north", false),
            option("small", "north", true),
            option("medium", "south", true),
        ],
        &[],
    )
    .await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let result = state
        .machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"reason":"Run the build","request_id":"r1"}),
        )
        .await
        .unwrap();
    assert_eq!(result.payload["option"]["class_id"], "small");
    let acquires = service.acquires.lock().unwrap();
    assert_eq!(acquires.len(), 1);
    assert_eq!(acquires[0]["class_id"], "small");
    assert_eq!(acquires[0]["region"], "north");
}

#[tokio::test]
async fn acquire_with_nothing_in_stock_is_refused_before_the_service_is_asked() {
    let service = service(vec![option("big", "north", false)], &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let error = state
        .machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"reason":"Run the build"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("No machine option is in stock"), "{error}");
    assert!(error.contains("chariox.machines.options"), "{error}");
    assert!(service.acquires.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_named_option_that_sold_out_is_refused_with_the_options_in_stock() {
    let service = service(
        vec![option("big", "north", true), option("small", "south", true)],
        &["big"],
    )
    .await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let error = state
        .machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"reason":"Run the build","class_id":"big","region":"north"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("big in north is out of stock"), "{error}");
    assert!(error.contains("In stock now: small in south."), "{error}");
    // The service closed the request, so only now may the refusal say it holds nothing.
    assert!(
        error.contains("nothing was allocated or charged under it"),
        "{error}"
    );
    assert!(error.contains("Use a new request_id"), "{error}");
}

#[tokio::test]
async fn a_refused_class_leaves_the_same_class_in_other_regions_in_stock() {
    let service = service(
        vec![
            option("small", "north", true),
            option("small", "south", true),
        ],
        &["small"],
    )
    .await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let error = state
        .machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"reason":"Run the build","class_id":"small","region":"north"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("small in north is out of stock"), "{error}");
    assert!(error.contains("In stock now: small in south"), "{error}");
}

#[tokio::test]
async fn a_retry_keeps_the_option_its_request_took_after_the_stock_changed() {
    let service = service(
        vec![option("small", "north", true), option("big", "south", true)],
        &[],
    )
    .await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let acquire = || {
        state.machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"reason":"Run the build","request_id":"r1"}),
        )
    };
    acquire().await.unwrap();
    // The response was lost and the option sold out meanwhile: nothing is in stock any more.
    for offer in service.offers.lock().unwrap().iter_mut() {
        offer.available = false;
    }
    let retried = acquire().await.unwrap();
    assert_eq!(
        retried.payload["allocation"]["allocation_ref"],
        "allocation-1"
    );
    let acquires = service.acquires.lock().unwrap();
    assert_eq!(acquires.len(), 2);
    assert_eq!(acquires[1]["class_id"], "small");
    assert_eq!(acquires[1]["region"], "north");
    assert_eq!(acquires[1]["request_id"], acquires[0]["request_id"]);
}

#[tokio::test]
async fn a_refused_request_resends_its_option_and_a_new_request_id_chooses_again() {
    let service = service(
        vec![option("big", "north", true), option("small", "south", true)],
        &["big"],
    )
    .await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let acquire = |request_id: &str| {
        state.machine_tool(&token, "chariox.machines.acquire", acquire_args(request_id))
    };
    // The listing still shows "big" in stock, but the service has sold it.
    assert!(acquire("r1").await.is_err());
    service.offers.lock().unwrap()[0].available = false;
    let error = acquire("r1").await.unwrap_err().to_string();
    assert!(error.contains("big in north is out of stock"), "{error}");
    assert_closed(&error, "r1");
    acquire("r2").await.unwrap();
    let acquires = service.acquires.lock().unwrap();
    let pairs: Vec<_> = acquires
        .iter()
        .map(|acquire| (acquire["class_id"].clone(), acquire["region"].clone()))
        .collect();
    assert_eq!(
        pairs,
        [("big", "north"), ("big", "north"), ("small", "south")]
            .map(|(class, region)| (class.into(), region.into()))
    );
}

/// A refusal that does not close its request may leave a try of it in flight, so it sends the
/// caller back to that request_id only: it never says nothing was charged, never offers a new
/// request_id and never treats a list of machines as proof.
fn assert_retry_the_same_request(error: &str, request_id: &str) {
    assert!(!error.contains("nothing was allocated"), "{error}");
    assert!(!error.contains("charged"), "{error}");
    assert!(!error.contains("new request_id"), "{error}");
    assert!(!error.contains("machines list"), "{error}");
    assert!(
        error.contains(&format!(
            "Retrying chariox.machines.acquire with request_id {request_id} is safe"
        )),
        "{error}"
    );
}

/// A refusal of a request the service closed: nothing is held under it and nothing ever will be,
/// so a new request_id is safe.
fn assert_closed(error: &str, request_id: &str) {
    assert!(
        error.contains(&format!(
            "The service closed request_id {request_id}: nothing was allocated or charged under it"
        )),
        "{error}"
    );
    assert!(error.contains("Use a new request_id"), "{error}");
    assert!(!error.contains("machines list"), "{error}");
}

fn now() -> u64 {
    crate::session::unix_epoch_ms()
}

fn choose_big(state: &KernelRuntimeState, agent: &str, request_id: &str, at_ms: u64) {
    assert!(state
        .owned
        .durable_state_store
        .write_acquire_choice(
            agent,
            request_id,
            crate::durable_state::acquire_choices::AcquireChoiceWrite::Choose {
                option: AcquireOption::Named {
                    class_id: "big".into(),
                    region: Some("south".into()),
                },
            },
            at_ms,
        )
        .unwrap());
}

fn acquire_args(request_id: &str) -> serde_json::Value {
    serde_json::json!({ "request_id": request_id, "reason": "Run the build" })
}

/// The stock the retries below start from: two options in stock.
fn two_in_stock() -> Vec<MachineOption> {
    vec![option("small", "north", true), option("big", "south", true)]
}

/// The agent whose run holds `token`, as the machine tools resolve it.
fn agent_of(state: &KernelRuntimeState, token: &str) -> String {
    let runs = state
        .owned
        .provider_store
        .get_runs_by_runtime_mcp_auth_token(token);
    state
        .reach_for_id(&format!(
            "{}{}",
            super::super::reach::REACH_PREFIX,
            runs[0].id()
        ))
        .unwrap()
        .agent_id
}

#[tokio::test]
async fn an_ambiguous_failure_keeps_the_option_and_the_retry_recovers_the_allocation() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let failures = [
        Answer::FailAfterCommit("500 Internal Server Error", Some("internal_error")),
        Answer::FailAfterCommit("503 Service Unavailable", Some("dependency_unavailable")),
        Answer::FailAfterCommit("429 Too Many Requests", Some("rate_limited")),
        Answer::FailAfterCommit("502 Bad Gateway", None),
        Answer::FailAfterCommit("402 Payment Required", Some("subscription_required")),
        Answer::LoseResponse,
    ];
    for (index, failure) in failures.into_iter().enumerate() {
        let request_id = format!("r{index}");
        *service.offers.lock().unwrap() = two_in_stock();
        service.answers.lock().unwrap().push_back(failure);
        let sent = service.acquires.lock().unwrap().len();
        // The service committed the allocation, then the answer failed.
        let error = state
            .machine_tool(
                &token,
                "chariox.machines.acquire",
                acquire_args(&request_id),
            )
            .await
            .unwrap_err();
        // The option sold out before the retry.
        service.offers.lock().unwrap()[0].available = false;
        let retried = state
            .machine_tool(
                &token,
                "chariox.machines.acquire",
                acquire_args(&request_id),
            )
            .await
            .unwrap_or_else(|retry| panic!("{request_id} after {error}: {retry}"));
        assert_eq!(
            retried.payload["allocation"]["allocation_ref"],
            format!("allocation-{}", index + 1),
            "{request_id} recovers the allocation the failed try committed"
        );
        let acquires = service.acquires.lock().unwrap();
        let (first, retry) = (&acquires[sent], acquires.last().unwrap());
        assert_eq!(
            (&retry["class_id"], &retry["region"]),
            (&first["class_id"], &first["region"])
        );
        assert_eq!(retry["class_id"], "small", "{request_id}");
        assert_eq!(retry["request_id"], first["request_id"], "{request_id}");
    }
}

#[tokio::test]
async fn a_refusal_after_a_lost_response_keeps_the_option_and_the_retry_recovers_it() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let agent = agent_of(&state, &token);
    let refusals = [
        ("429 Too Many Requests", "capacity_exceeded"),
        ("429 Too Many Requests", "budget_exceeded"),
        ("400 Bad Request", "invalid_request"),
    ];
    for (index, (status, code)) in refusals.into_iter().enumerate() {
        let request_id = format!("r{index}");
        let acquire = || {
            state.machine_tool(
                &token,
                "chariox.machines.acquire",
                acquire_args(&request_id),
            )
        };
        *service.offers.lock().unwrap() = two_in_stock();
        // The first try times out after the service committed; a refusal of an overlapping
        // try that the service judged before that commit arrives next.
        service
            .answers
            .lock()
            .unwrap()
            .extend([Answer::LoseResponse, Answer::Refuse(status, code)]);
        assert!(acquire().await.is_err());
        let refused = acquire().await.unwrap_err().to_string();
        if code == "capacity_exceeded" {
            assert_retry_the_same_request(&refused, &request_id);
        }
        let row = state
            .owned
            .durable_state_store
            .acquire_choice(&agent, &request_id, now())
            .unwrap()
            .unwrap();
        assert_eq!(row.state, AcquireChoiceState::Pending, "{code}");
        service.offers.lock().unwrap()[0].available = false;
        let retried = acquire()
            .await
            .unwrap_or_else(|error| panic!("{code}: {error}"));
        assert_eq!(
            retried.payload["allocation"]["allocation_ref"],
            format!("allocation-{}", index + 1),
            "{code}"
        );
        assert_eq!(retried.payload["option"]["class_id"], "small", "{code}");
    }
}

/// Two tries of one request overlap: the service refuses the first before the second commits
/// and takes the last stock, the second's response is lost, and the first's refusal arrives last.
#[tokio::test]
async fn a_late_refusal_of_an_overlapping_try_keeps_the_option_the_committed_try_sent() {
    // A sold-out option, and one that left the listing while another is in stock.
    let after_commit: [(&str, &str, Vec<MachineOption>); 2] = [
        (
            "429 Too Many Requests",
            "capacity_exceeded",
            vec![
                option("big", "north", false),
                option("small", "south", true),
            ],
        ),
        (
            "400 Bad Request",
            "invalid_request",
            vec![option("small", "south", true)],
        ),
    ];
    for (status, code, listing) in after_commit {
        let service = service(vec![option("big", "north", true)], &[]).await;
        let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
        let agent = agent_of(&state, &token);
        let acquire = || state.machine_tool(&token, "chariox.machines.acquire", acquire_args("r1"));
        let refusal = Arc::new(tokio::sync::Notify::new());
        service.answers.lock().unwrap().extend([
            Answer::RefuseLater(status, code, refusal.clone()),
            Answer::LoseResponse,
        ]);
        let overlap = async {
            tokio::join!(acquire(), async {
                while service.acquires.lock().unwrap().is_empty() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                let second = acquire().await;
                *service.offers.lock().unwrap() = listing;
                refusal.notify_one();
                second
            })
        };
        let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(60), overlap)
            .await
            .expect("both tries of r1 end");
        let first = first.unwrap_err().to_string();
        assert!(second.is_err(), "{code}");
        if code == "capacity_exceeded" {
            assert_retry_the_same_request(&first, "r1");
        }
        let recovered = acquire()
            .await
            .unwrap_or_else(|error| panic!("{code} after {first}: {error}"));
        assert_eq!(
            recovered.payload["allocation"]["allocation_ref"], "allocation-1",
            "{code}"
        );
        let acquires = service.acquires.lock().unwrap();
        assert_eq!(acquires.len(), 3, "{code}");
        assert!(
            acquires.iter().all(|sent| sent["class_id"] == "big"
                && sent["region"] == "north"
                && sent["request_id"] == acquires[0]["request_id"]),
            "{code}"
        );
        assert_eq!(
            state
                .owned
                .durable_state_store
                .acquire_choice(&agent, "r1", now())
                .unwrap()
                .unwrap()
                .state,
            AcquireChoiceState::Accepted {
                allocation_ref: "allocation-1".into()
            }
        );
    }
}

/// The capability answers for one try that waits right before its acquire is sent: its own
/// capability call answers, the one right before the acquire waits for `release`.
fn hold_before_send(service: &Service) -> (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
    let arrived = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    service.capability_failures.lock().unwrap().extend([
        CapabilityAnswer::Answer,
        CapabilityAnswer::Hold {
            arrived: arrived.clone(),
            release: release.clone(),
        },
    ]);
    (arrived, release)
}

/// A try that chose its option but has not been sent yet (its capability call or the owner's
/// decision still runs) can allocate under its request_id after another try of it was refused.
/// A refusal that does not close the request sends the caller back to it; the caller follows that
/// while the first try still waits, and ends with one machine.
#[tokio::test]
async fn a_refusal_while_another_try_waits_to_be_sent_sends_the_caller_back_to_the_same_request() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let acquire = |request_id: &str| {
        state.machine_tool(&token, "chariox.machines.acquire", acquire_args(request_id))
    };
    let (arrived, release) = hold_before_send(&service);
    service
        .answers
        .lock()
        .unwrap()
        .push_back(Answer::Refuse("429 Too Many Requests", "capacity_exceeded"));
    let interleaving = async {
        tokio::join!(acquire("r1"), async {
            arrived.notified().await;
            let refused = acquire("r1").await.unwrap_err().to_string();
            assert_retry_the_same_request(&refused, "r1");
            // The caller follows the guidance while the first try still waits to be sent.
            let retried = acquire("r1").await;
            release.notify_one();
            retried
        })
    };
    let (waited, retried) = tokio::time::timeout(std::time::Duration::from_secs(60), interleaving)
        .await
        .expect("every try of r1 ends");
    let (waited, retried) = (waited.unwrap(), retried.unwrap());
    assert_eq!(
        retried.payload["allocation"]["allocation_ref"],
        "allocation-1"
    );
    assert_eq!(
        waited.payload["allocation"]["allocation_ref"], "allocation-1",
        "the waiting try replays the one machine"
    );
    let acquires = service.acquires.lock().unwrap();
    assert_eq!(acquires.len(), 3);
    assert!(acquires
        .iter()
        .all(|sent| sent["request_id"] == acquires[0]["request_id"]));
}

/// The same interleaving when the service closes the refused request: the caller follows the
/// closed refusal to a new request_id while the first try still waits, and the first try, sent
/// last, is refused as closed. The caller's new machine is the only one.
#[tokio::test]
async fn a_closed_refusal_while_another_try_waits_to_be_sent_leaves_one_machine() {
    let service = service(two_in_stock(), &["small"]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let acquire =
        |args: serde_json::Value| state.machine_tool(&token, "chariox.machines.acquire", args);
    let (arrived, release) = hold_before_send(&service);
    let interleaving = async {
        tokio::join!(acquire(acquire_args("r1")), async {
            arrived.notified().await;
            let refused = acquire(acquire_args("r1")).await.unwrap_err().to_string();
            assert!(
                refused.contains("small in north is out of stock"),
                "{refused}"
            );
            assert_closed(&refused, "r1");
            // The caller follows the guidance while the first try of r1 still waits to be sent.
            let other =
                acquire(serde_json::json!({"request_id":"r2","class_id":"big","region":"south","reason":"Run the build"}))
                    .await;
            release.notify_one();
            other
        })
    };
    let (waited, other) = tokio::time::timeout(std::time::Duration::from_secs(60), interleaving)
        .await
        .expect("every try ends");
    let other = other.unwrap();
    assert_eq!(
        other.payload["allocation"]["allocation_ref"],
        "allocation-1"
    );
    let waited = waited.unwrap_err().to_string();
    assert_closed(&waited, "r1");
    let acquires = service.acquires.lock().unwrap();
    let sent: Vec<_> = acquires
        .iter()
        .map(|sent| sent["class_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        sent,
        ["small", "big", "small"],
        "the waiting try is sent last"
    );
    assert_eq!(acquires[2]["request_id"], acquires[0]["request_id"]);
}

#[tokio::test]
async fn an_accepted_request_keeps_its_option_after_a_later_refusal() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let acquire = || state.machine_tool(&token, "chariox.machines.acquire", acquire_args("r1"));
    acquire().await.unwrap();
    service
        .answers
        .lock()
        .unwrap()
        .push_back(Answer::Refuse("429 Too Many Requests", "capacity_exceeded"));
    service.offers.lock().unwrap()[0].available = false;
    let refused = acquire().await.unwrap_err().to_string();
    assert_retry_the_same_request(&refused, "r1");
    let retried = acquire().await.unwrap();
    assert_eq!(
        retried.payload["allocation"]["allocation_ref"],
        "allocation-1"
    );
    assert_eq!(service.acquires.lock().unwrap()[2]["class_id"], "small");
}

#[tokio::test]
async fn concurrent_tries_of_one_request_send_one_option() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let acquire = || state.machine_tool(&token, "chariox.machines.acquire", acquire_args("r1"));
    let (first, second) = tokio::join!(acquire(), acquire());
    assert_eq!(
        first.unwrap().payload["allocation"],
        second.unwrap().payload["allocation"]
    );
    let acquires = service.acquires.lock().unwrap();
    assert_eq!(acquires.len(), 2);
    assert_eq!(acquires[0], acquires[1]);
}

#[tokio::test]
async fn a_try_that_loses_the_choice_sends_the_winners_option() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    // A concurrent try of r1 chose "big" while "small" was out of stock.
    assert!(state
        .owned
        .durable_state_store
        .write_acquire_choice(
            &agent_of(&state, &token),
            "r1",
            crate::durable_state::acquire_choices::AcquireChoiceWrite::Choose {
                option: AcquireOption::Named {
                    class_id: "big".into(),
                    region: Some("south".into()),
                },
            },
            now(),
        )
        .unwrap());
    let result = state
        .machine_tool(&token, "chariox.machines.acquire", acquire_args("r1"))
        .await
        .unwrap();
    assert_eq!(result.payload["option"]["class_id"], "big");
    assert_eq!(service.acquires.lock().unwrap()[0]["region"], "south");
}

#[tokio::test]
async fn a_named_option_that_differs_from_the_one_a_request_took_is_refused_locally() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    service
        .answers
        .lock()
        .unwrap()
        .push_back(Answer::FailAfterCommit(
            "500 Internal Server Error",
            Some("internal_error"),
        ));
    assert!(state
        .machine_tool(&token, "chariox.machines.acquire", acquire_args("r1"))
        .await
        .is_err());
    let error = state
        .machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"reason":"Run the build","request_id":"r1","class_id":"big","region":"south"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("request_id r1 already asked for small in north"),
        "{error}"
    );
    assert!(
        error.contains("Use a new request_id only when the service closes this one"),
        "{error}"
    );
    assert!(!error.contains("machines list"), "{error}");
    assert_eq!(service.acquires.lock().unwrap().len(), 1);
    let named_same = state
        .machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"reason":"Run the build","request_id":"r1","class_id":"small"}),
        )
        .await
        .unwrap();
    assert_eq!(
        named_same.payload["allocation"]["allocation_ref"],
        "allocation-1"
    );
}

#[tokio::test]
async fn a_failure_before_the_acquire_is_sent_keeps_the_option_and_recovers_with_no_stock() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let agent = agent_of(&state, &token);
    // Even a code that refuses an acquire says nothing about the request when the capability
    // call before it fails.
    let failures = [
        ("429 Too Many Requests", "rate_limited"),
        ("400 Bad Request", "invalid_request"),
    ];
    for (index, failure) in failures.into_iter().enumerate() {
        let request_id = format!("r{index}");
        *service.offers.lock().unwrap() = two_in_stock();
        service
            .answers
            .lock()
            .unwrap()
            .push_back(Answer::LoseResponse);
        let acquire = || {
            state.machine_tool(
                &token,
                "chariox.machines.acquire",
                acquire_args(&request_id),
            )
        };
        assert!(acquire().await.is_err());
        let sent = service.acquires.lock().unwrap().len();
        // The tool's own capability call answers; the one before the acquire fails.
        service.capability_failures.lock().unwrap().extend([
            CapabilityAnswer::Answer,
            CapabilityAnswer::Fail(failure.0, failure.1),
        ]);
        let error = acquire().await.unwrap_err().to_string();
        assert!(error.contains("this try was not sent"), "{error}");
        assert_eq!(service.acquires.lock().unwrap().len(), sent);
        assert_eq!(
            state
                .owned
                .durable_state_store
                .acquire_choice(&agent, &request_id, now())
                .unwrap()
                .map(|choice| choice.state),
            Some(AcquireChoiceState::Pending),
            "{failure:?}"
        );
        for offer in service.offers.lock().unwrap().iter_mut() {
            offer.available = false;
        }
        let recovered = acquire().await.unwrap();
        assert_eq!(
            recovered.payload["allocation"]["allocation_ref"],
            format!("allocation-{}", index + 1),
            "{failure:?}"
        );
        let acquires = service.acquires.lock().unwrap();
        let (first, last) = (&acquires[sent - 1], acquires.last().unwrap());
        assert_eq!(
            (&last["class_id"], &last["region"]),
            (&first["class_id"], &first["region"])
        );
    }
}

#[tokio::test]
async fn an_accepted_default_request_replays_the_default_after_options_appear() {
    let service = service(vec![], &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let agent = agent_of(&state, &token);
    let acquire = || state.machine_tool(&token, "chariox.machines.acquire", acquire_args("r1"));
    service
        .answers
        .lock()
        .unwrap()
        .push_back(Answer::LoseResponse);
    assert!(acquire().await.is_err());
    assert_eq!(
        state
            .owned
            .durable_state_store
            .acquire_choice(&agent, "r1", now())
            .unwrap()
            .map(|choice| choice.option),
        Some(AcquireOption::ServiceDefault)
    );
    // The service now lists options, all sold out, and then one in stock: the retries still ask
    // for the default the first try took.
    *service.offers.lock().unwrap() = vec![option("small", "north", false)];
    let recovered = acquire().await.unwrap();
    assert_eq!(
        recovered.payload["allocation"]["allocation_ref"],
        "allocation-1"
    );
    service.offers.lock().unwrap()[0].available = true;
    let replayed = acquire().await.unwrap();
    assert_eq!(
        replayed.payload["allocation"]["allocation_ref"],
        "allocation-1"
    );
    let acquires = service.acquires.lock().unwrap();
    assert_eq!(acquires.len(), 3);
    assert!(acquires
        .iter()
        .all(|acquire| acquire["class_id"].is_null() && acquire["region"].is_null()));
}

const WINDOW_MS: u64 = crate::durable_state::FINISHED_RETENTION_MS;

#[tokio::test]
async fn a_retry_inside_the_replay_window_keeps_its_option_and_one_after_it_chooses_again() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let agent = agent_of(&state, &token);
    choose_big(&state, &agent, "inside", now() - WINDOW_MS + 60_000);
    choose_big(&state, &agent, "after", now() - WINDOW_MS - 1);
    for request_id in ["inside", "after"] {
        state
            .machine_tool(&token, "chariox.machines.acquire", acquire_args(request_id))
            .await
            .unwrap();
    }
    let acquires = service.acquires.lock().unwrap();
    assert_eq!(acquires[0]["class_id"], "big");
    assert_eq!(acquires[1]["class_id"], "small");
}

#[tokio::test]
async fn a_settled_acquisition_removes_the_rows_past_their_window() {
    let service = service(two_in_stock(), &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let agent = agent_of(&state, &token);
    let old = now() - WINDOW_MS - 1;
    choose_big(&state, &agent, "old", old);
    let store = &state.owned.durable_state_store;
    // Read with a clock from before the row aged out: the row is still there.
    assert!(store.acquire_choice(&agent, "old", old).unwrap().is_some());
    state
        .machine_tool(&token, "chariox.machines.acquire", acquire_args("r1"))
        .await
        .unwrap();
    assert!(store.acquire_choice(&agent, "old", old).unwrap().is_none());
    assert!(store.acquire_choice(&agent, "r1", now()).unwrap().is_some());
}

#[tokio::test]
async fn a_closed_refusal_keeps_the_request_for_a_new_window() {
    let service = service(
        vec![option("big", "south", true), option("small", "north", true)],
        &["big"],
    )
    .await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let agent = agent_of(&state, &token);
    choose_big(&state, &agent, "r1", now() - WINDOW_MS + 60_000);
    let error = state
        .machine_tool(&token, "chariox.machines.acquire", acquire_args("r1"))
        .await
        .unwrap_err()
        .to_string();
    assert_closed(&error, "r1");
    // Two minutes on, the first write's window has ended; the closed refusal's has not.
    assert!(state
        .owned
        .durable_state_store
        .acquire_choice(&agent, "r1", now() + 120_000)
        .unwrap()
        .is_some());
}

/// The service accepts a try, and before its success is recorded another write sweeps the row,
/// either a fresh one the sweep reaches because the clock has moved a whole window on, or one a
/// retry read just before its window ended. The agent still gets the machine, and the row records it.
#[tokio::test]
async fn a_success_whose_row_is_swept_while_the_try_waits_is_still_recorded() {
    for near_expiry in [false, true] {
        let service = service(two_in_stock(), &[]).await;
        let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
        let agent = agent_of(&state, &token);
        let store = &state.owned.durable_state_store;
        if near_expiry {
            choose_big(&state, &agent, "r1", now() - WINDOW_MS + 60_000);
        }
        let (arrived, release) = (
            Arc::new(tokio::sync::Notify::new()),
            Arc::new(tokio::sync::Notify::new()),
        );
        service
            .answers
            .lock()
            .unwrap()
            .push_back(Answer::AcceptLater {
                arrived: arrived.clone(),
                release: release.clone(),
            });
        let interleaving = async {
            tokio::join!(
                state.machine_tool(&token, "chariox.machines.acquire", acquire_args("r1")),
                async {
                    arrived.notified().await;
                    // Another agent's write a window on removes every row past its window.
                    choose_big(&state, "other-agent", "r9", now() + WINDOW_MS + 1);
                    assert!(
                        store
                            .acquire_choice(&agent, "r1", now() + WINDOW_MS + 2)
                            .unwrap()
                            .is_none(),
                        "the sweep removed the row"
                    );
                    release.notify_one();
                }
            )
            .0
        };
        let accepted = tokio::time::timeout(std::time::Duration::from_secs(60), interleaving)
            .await
            .expect("the acquire ends")
            .unwrap_or_else(|error| panic!("near_expiry {near_expiry}: {error}"));
        assert_eq!(
            accepted.payload["allocation"]["allocation_ref"], "allocation-1",
            "near_expiry {near_expiry}"
        );
        assert_eq!(
            store
                .acquire_choice(&agent, "r1", now())
                .unwrap()
                .map(|choice| choice.state),
            Some(AcquireChoiceState::Accepted {
                allocation_ref: "allocation-1".into()
            }),
            "near_expiry {near_expiry}"
        );
    }
}

#[tokio::test]
async fn machine_options_include_decimal_rates() {
    let service = service(vec![option("small", "north", true)], &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let result = state
        .machine_tool(&token, "chariox.machines.options", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(
        result.payload["options"][0]["rate"],
        serde_json::json!({"per_minute":"0.000284","per_hour":"0.01704","currency":"EUR"})
    );
}

fn require_owner_approval(state: &KernelRuntimeState, token: &str) -> String {
    use crate::durable_state::agent_lifecycle::Operation;
    use crate::session::{PromptQueueItem, PromptStatus};

    let mut config = state.owned.config_projection.snapshot();
    config.room_agent_tools = true;
    config.user_config.agents.offload_without_asking = Some(false);
    state.owned.config_projection.update(config);
    let runs = state
        .owned
        .provider_store
        .get_runs_by_runtime_mcp_auth_token(token);
    let run = &runs[0];
    let room = state
        .owned
        .session_store
        .get_session(run.session_id())
        .unwrap();
    let agent = run.agent_instance_id().unwrap();
    let prompt = state
        .owned
        .prompt_state_owner
        .activate_prompt(
            &room,
            PromptQueueItem::new(
                "machine-prompt",
                "attachment",
                agent,
                "Acquire a machine",
                PromptStatus::Running,
            ),
        )
        .unwrap();
    state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: room.owner_user_id().into(),
            room: room.id().into(),
            agent: agent.into(),
            prompt: prompt.id().into(),
            run: Some(run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    room.id().into()
}

#[tokio::test]
async fn direct_rental_names_the_asker_and_only_raises_one_card_for_the_id() {
    let service = service(vec![option("small", "north", true)], &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let room = require_owner_approval(&state, &token);
    let mut agent = state
        .owned
        .agent_store
        .get_agent(&agent_of(&state, &token))
        .unwrap();
    agent.set_alias(Some("builder".into()));
    state.owned.agent_store.restore_agent(agent);
    {
        let mut sessions = state.owned.session_store.write();
        let mut session = sessions.get_session(&room).unwrap();
        session.set_alias(Some("Build Room".into()));
        sessions.restore_session(session);
    }
    let acquire = |args| state.machine_tool(&token, "chariox.machines.acquire", args);
    for args in [
        serde_json::json!({"request_id":"r1"}),
        serde_json::json!({"request_id":"r1", "reason":"  "}),
        serde_json::json!({"request_id":"r1", "reason":"a\nb"}),
        serde_json::json!({"request_id":"r1", "reason":"a".repeat(161)}),
    ] {
        assert!(acquire(args)
            .await
            .unwrap_err()
            .to_string()
            .contains("Give a reason"));
    }
    let args = acquire_args("r1");
    let (first, retry) = tokio::join!(acquire(args.clone()), acquire(args.clone()));
    let first = first.unwrap().payload;
    assert_eq!(first["status"], "waiting");
    assert_eq!(retry.unwrap().payload, first);
    assert_eq!(acquire(args.clone()).await.unwrap().payload, first);
    let changed = serde_json::json!({"request_id":"r1", "reason":"Run something else"});
    assert_eq!(acquire(changed).await.unwrap().payload, first);

    let session = state.owned.session_store.get_session(&room).unwrap();
    assert_eq!(session.active_interactions().len(), 1);
    let card = &session.active_interactions()[0];
    let expected = "builder in Build Room · Run the build";
    assert_eq!(card.title(), Some(expected));
    let listed = state.list_owner_interactions(session.owner_user_id());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].interaction_id, card.id());
    assert_eq!(listed[0].title.as_deref(), Some(expected));
    assert!(listed[0].person_only.is_none());

    state
        .resolve_terminal_runtime_interaction(
            &room,
            card.id(),
            "approve",
            None,
            Some(session.owner_user_id()),
        )
        .await
        .unwrap();
    let sent = acquire(args).await.unwrap();
    assert_eq!(sent.payload["allocation"]["allocation_ref"], "allocation-1");
    assert!(state
        .list_owner_interactions(session.owner_user_id())
        .is_empty());
    let acquires = service.acquires.lock().unwrap();
    assert_eq!(acquires.len(), 1);
    assert!(acquires[0].get("reason").is_none());
}

#[tokio::test]
async fn unanswered_direct_acquire_returns_a_room_wait_and_expires_once() {
    let service = service(vec![option("small", "north", true)], &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let room = require_owner_approval(&state, &token);
    let acquire = |request_id| {
        state.machine_tool(&token, "chariox.machines.acquire", acquire_args(request_id))
    };
    let result = tokio::time::timeout(Duration::from_secs(2), acquire("r1"))
        .await
        .expect("a pending decision must return without waiting for the person")
        .unwrap();
    assert_eq!(result.payload["status"], "waiting");
    assert!(result.payload["registration_id"].is_string());
    assert_eq!(acquire("r1").await.unwrap().payload, result.payload);
    let id;
    {
        let mut sessions = state.owned.session_store.write();
        let mut session = sessions.get_session(&room).unwrap();
        assert_eq!(session.active_interactions().len(), 1);
        let card = &session.active_interactions()[0];
        assert!(card
            .message()
            .contains("0.000284 EUR per minute (0.01704 EUR per hour)"));
        id = card.id().to_owned();
        let mut card = serde_json::json!(card);
        card["requested_at_ms"] = serde_json::json!(1);
        session.replace_active_interaction(serde_json::from_value(card).unwrap());
        sessions.restore_session(session);
    }
    state
        .tick_interaction_wakes(result.payload["expires_at_ms"].as_u64().unwrap())
        .unwrap();
    let session = state.owned.session_store.get_session(&room).unwrap();
    assert!(session.active_interactions().is_empty());
    assert_eq!(
        session.machine_approval_receipts()[0].state,
        crate::session::MachineApprovalReceiptState::Expired
    );
    let wake = state
        .owned
        .durable_state_store
        .agent_wake(&id)
        .unwrap()
        .unwrap();
    let (event, _) = state
        .owned
        .durable_state_store
        .agent_inbox_event(wake.last_sequence.unwrap())
        .unwrap()
        .unwrap();
    let outcome = &event.payload["public_answer"];
    assert_eq!(outcome["status"], "expired");
    assert_eq!(outcome["expired_at_ms"], 300001);
    assert!(outcome["message"].as_str().unwrap().contains("Expired at"));
    let error = acquire("r1").await.unwrap_err().to_string();
    assert!(error.contains("Expired at"), "{error}");
    assert!(error.contains("Use a new request_id"), "{error}");
    assert_eq!(acquire("r2").await.unwrap().payload["status"], "waiting");
    assert!(service.acquires.lock().unwrap().is_empty());
}

#[tokio::test]
async fn machine_wait_denial_names_the_person_and_approval_stays_visible_until_sent() {
    let service = service(vec![option("small", "north", true)], &[]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let room = require_owner_approval(&state, &token);
    let acquire = |request_id| {
        state.machine_tool(&token, "chariox.machines.acquire", acquire_args(request_id))
    };
    let answer = |choice| {
        let session = state.owned.session_store.get_session(&room).unwrap();
        let id = session.active_interactions()[0].id().to_owned();
        let owner = session.owner_user_id().to_owned();
        let state = &state;
        let room = &room;
        async move {
            state
                .resolve_terminal_runtime_interaction(room, &id, choice, None, Some(&owner))
                .await
                .unwrap();
        }
    };
    assert_eq!(acquire("deny").await.unwrap().payload["status"], "waiting");
    answer("deny").await;
    let error = acquire("deny").await.unwrap_err().to_string();
    assert!(error.contains("Denied by user@example.test at "), "{error}");
    assert!(service.acquires.lock().unwrap().is_empty());

    assert_eq!(
        acquire("approve").await.unwrap().payload["status"],
        "waiting"
    );
    let session = state.owned.session_store.get_session(&room).unwrap();
    let listed = state.list_owner_interactions(session.owner_user_id());
    assert!(listed.iter().any(
        |card| card.interaction_id == session.active_interactions()[0].id()
            && card.person_only.is_none()
    ));
    answer("approve").await;
    let read_status = || {
        state
            .owned
            .session_store
            .get_session(&room)
            .unwrap()
            .machine_approval_receipts()[0]
            .state
    };
    assert_eq!(
        read_status(),
        crate::session::MachineApprovalReceiptState::Approving
    );
    let arrived = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    service
        .answers
        .lock()
        .unwrap()
        .push_back(Answer::AcceptLater {
            arrived: arrived.clone(),
            release: release.clone(),
        });
    let acquisition = acquire("approve");
    tokio::pin!(acquisition);
    tokio::select! {
        _ = arrived.notified() => {}
        result = &mut acquisition => panic!("service response was not held: {:?}", result.map(|result| result.payload)),
    }
    assert_eq!(
        read_status(),
        crate::session::MachineApprovalReceiptState::Approving
    );
    release.notify_one();
    assert!(acquisition.await.unwrap().payload["allocation"]["allocation_ref"].is_string());
    assert_eq!(
        read_status(),
        crate::session::MachineApprovalReceiptState::Sent
    );
    assert_eq!(service.acquires.lock().unwrap().len(), 1);
}
