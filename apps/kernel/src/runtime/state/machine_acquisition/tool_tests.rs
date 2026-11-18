//! The agent's `chariox.machines.acquire` tool against a stand-in connected service.
use super::*;
use crate::durable_state::acquire_choices::AcquireChoiceState;
use crate::local::{MachineAllocationCapability, MachineOption};
use crate::provider::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// How the stand-in service answers the next acquire.
enum Answer {
    /// Commits the allocation, then fails with this status and Cloud code (none for a bare 5xx).
    FailAfterCommit(&'static str, Option<&'static str>),
    /// Commits the allocation, then closes the connection: the response is lost.
    LoseResponse,
    /// Refuses with this status and code and commits nothing.
    Refuse(&'static str, &'static str),
    /// Refuses like `Refuse` at once, but delivers the refusal only when `gate` is notified.
    RefuseLater(&'static str, &'static str, Arc<tokio::sync::Notify>),
}

struct Service {
    api_url: String,
    acquires: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    /// The options the service lists; a test changes them to change the stock.
    offers: Arc<std::sync::Mutex<Vec<MachineOption>>>,
    /// Answers for the next acquires, before the service's own judgement.
    answers: Arc<std::sync::Mutex<std::collections::VecDeque<Answer>>>,
    capability_failures: Arc<std::sync::Mutex<CapabilityFailures>>,
}

/// For the next capability calls in order: a status and code to fail with, or none to answer.
type CapabilityFailures = std::collections::VecDeque<Option<(&'static str, &'static str)>>;

/// A connected service that lists `offers` and, like Cloud, binds each accepted request id to its
/// class and region and replays it, whatever the stock says now. A new request for a class named
/// in `sold_out` is refused.
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
    let mut accepted = BTreeMap::<String, (serde_json::Value, serde_json::Value, String)>::new();
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
            let (status, response) = match request["action"].as_str().unwrap() {
                "capabilities" => 'capabilities: {
                    if let Some(Some((status, code))) =
                        next_capability_failure.lock().unwrap().pop_front()
                    {
                        break 'capabilities (status, error(code));
                    }
                    let capability = MachineAllocationCapability {
                        revision: "v1".into(),
                        actions: vec!["acquire".into()],
                        offer: None,
                        offers: listed.lock().unwrap().clone(),
                        job_offers: vec![],
                        job_default_class_id: None,
                    };
                    (
                        "200 OK",
                        serde_json::json!({"kind":"capabilities","machine_allocation":capability}),
                    )
                }
                "acquire" => {
                    seen.lock().unwrap().push(request.clone());
                    let id = request["request_id"].as_str().unwrap().to_owned();
                    let pair = (request["class_id"].clone(), request["region"].clone());
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
                            ("429 Too Many Requests", error("capacity_exceeded"))
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
        token_expires_at_ms: None,
    });
    state.owned.config_projection.update(config);
    (state, token, worktree)
}

fn option(class_id: &str, region: &str, available: bool) -> MachineOption {
    super::tests::option(class_id, region, available)
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
            serde_json::json!({"request_id":"r1"}),
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
        .machine_tool(&token, "chariox.machines.acquire", serde_json::json!({}))
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
            serde_json::json!({"class_id":"big","region":"north"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("big in north is out of stock"), "{error}");
    assert!(error.contains("In stock now: small in south."), "{error}");
    assert!(error.contains("class_id and region"), "{error}");
    // The one try was answered and nothing accepted it: the request holds nothing.
    assert!(error.contains("none was accepted"), "{error}");
    assert!(
        error.contains("nothing was allocated or charged"),
        "{error}"
    );
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
            serde_json::json!({"class_id":"small","region":"north"}),
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
            serde_json::json!({"request_id":"r1"}),
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
    assert!(error.contains("a new request_id"), "{error}");
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

/// A refusal of a request that may hold an allocation sends the caller back to that request_id:
/// it never says nothing was charged and never offers another request_id.
fn assert_retry_the_same_request(error: &str, request_id: &str) {
    assert!(!error.contains("nothing was allocated"), "{error}");
    assert!(!error.contains("new request_id"), "{error}");
    assert!(
        error.contains("Retry chariox.machines.acquire with the same request_id"),
        "{error}"
    );
    assert!(
        error.contains(&format!(
            "request_id {request_id} may still hold an allocation"
        )),
        "{error}"
    );
}

fn acquire_args(request_id: &str) -> serde_json::Value {
    serde_json::json!({ "request_id": request_id })
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

/// A try counts as unanswered only once it is sent: a failure before the acquire leaves the
/// service nothing to hold, so it never keeps a later refusal from saying the request is empty.
#[tokio::test]
async fn a_try_that_never_reached_the_service_does_not_keep_a_refusal_from_saying_nothing_is_held()
{
    let service = service(two_in_stock(), &["small"]).await;
    let (state, token, _workspace) = agent_kernel(service.api_url.clone()).await;
    let agent = agent_of(&state, &token);
    let acquire = |request_id: &str| {
        state.machine_tool(&token, "chariox.machines.acquire", acquire_args(request_id))
    };
    // The tool's own capability call answers; the one before the acquire fails.
    let fail_before_the_acquire = || {
        service
            .capability_failures
            .lock()
            .unwrap()
            .extend([None, Some(("429 Too Many Requests", "rate_limited"))]);
    };
    fail_before_the_acquire();
    let error = acquire("r1").await.unwrap_err().to_string();
    assert!(error.contains("nothing was requested"), "{error}");
    assert!(service.acquires.lock().unwrap().is_empty());
    let unanswered = |request_id: &str| {
        state
            .owned
            .durable_state_store
            .acquire_choice(&agent, request_id)
            .unwrap()
            .unwrap()
            .unanswered
    };
    assert_eq!(unanswered("r1"), 0);
    let error = acquire("r1").await.unwrap_err().to_string();
    assert!(error.contains("small in north is out of stock"), "{error}");
    assert!(
        error.contains("nothing was allocated or charged"),
        "{error}"
    );
    assert!(error.contains("a new request_id"), "{error}");
    assert_eq!(unanswered("r1"), 0);
    // A try that was sent and never answered still sends the caller back to the same request_id.
    fail_before_the_acquire();
    assert!(acquire("r2").await.is_err());
    service
        .answers
        .lock()
        .unwrap()
        .push_back(Answer::LoseResponse);
    assert!(acquire("r2").await.is_err());
    service
        .answers
        .lock()
        .unwrap()
        .push_back(Answer::Refuse("429 Too Many Requests", "capacity_exceeded"));
    let error = acquire("r2").await.unwrap_err().to_string();
    assert_retry_the_same_request(&error, "r2");
    assert_eq!(unanswered("r2"), 1);
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
            .acquire_choice(&agent, &request_id)
            .unwrap()
            .unwrap();
        assert_eq!(row.state, AcquireChoiceState::Refused, "{code}");
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
            assert!(!first.contains("In stock now"), "{first}");
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
                .acquire_choice(&agent, "r1")
                .unwrap()
                .unwrap()
                .state,
            AcquireChoiceState::Accepted {
                allocation_ref: "allocation-1".into()
            }
        );
    }
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
            serde_json::json!({"request_id":"r1","class_id":"big","region":"south"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("request_id r1 already asked for small in north"),
        "{error}"
    );
    assert!(!error.contains("new request_id"), "{error}");
    assert_eq!(service.acquires.lock().unwrap().len(), 1);
    let named_same = state
        .machine_tool(
            &token,
            "chariox.machines.acquire",
            serde_json::json!({"request_id":"r1","class_id":"small"}),
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
        service
            .capability_failures
            .lock()
            .unwrap()
            .extend([None, Some(failure)]);
        let error = acquire().await.unwrap_err().to_string();
        assert!(error.contains("nothing was requested"), "{error}");
        assert_eq!(service.acquires.lock().unwrap().len(), sent);
        assert_eq!(
            state
                .owned
                .durable_state_store
                .acquire_choice(&agent, &request_id)
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
            .acquire_choice(&agent, "r1")
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
