//! "Agents may acquire machines without asking" through the agent's `chariox.machines.acquire`
//! and `chariox.machines.release` tools against a stand-in connected service.
use super::*;
use crate::durable_state::acquired_machines::AcquiredState;
use crate::local::{JobMachineRate, MachineOption};
use crate::provider::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A connected service offering a small and a big class; it accepts every acquire and lists each
/// allocation it made with its state.
struct Service {
    api_url: String,
    acquires: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    machines: Arc<std::sync::Mutex<Vec<(String, String)>>>,
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

async fn service() -> Service {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let acquires = Arc::new(std::sync::Mutex::new(Vec::new()));
    let machines = Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
    let (seen, listed) = (acquires.clone(), machines.clone());
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
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
                    break serde_json::from_slice::<serde_json::Value>(
                        &bytes[end + 4..end + 4 + length],
                    )
                    .unwrap();
                }
            };
            let request = &body["request"];
            let machine = |allocation_ref: &str, state: &str| {
                serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
                    "allocation_ref":allocation_ref,"state":state,"kernel_target":null,
                    "access_state":"pending","access_reason":null})
            };
            let response = match request["action"].as_str().unwrap() {
                "capabilities" => serde_json::json!({"kind":"capabilities","machine_allocation":{
                    "revision":"v1","actions":["acquire"],"offer":null,
                    "offers":[option("small", 2, 1_000), option("big", 8, 4_000)]}}),
                "acquire" => {
                    seen.lock().unwrap().push(request.clone());
                    let mut machines = listed.lock().unwrap();
                    let allocation_ref = format!("alloc-{}", machines.len() + 1);
                    machines.push((allocation_ref.clone(), "creating".into()));
                    machine(&allocation_ref, "creating")
                }
                "list" => serde_json::json!({"kind":"machines","saved_states":[],
                    "machines": listed.lock().unwrap().iter().map(|(allocation_ref, state)| serde_json::json!({
                        "allocation_ref":allocation_ref,"name":allocation_ref,"state":state,"revision":1,
                        "since":"2026-11-19T00:00:00Z","kernel_id":null,"machine_id":null,"available_actions":[]}))
                        .collect::<Vec<_>>()}),
                "get" => {
                    let allocation_ref = request["allocation_ref"].as_str().unwrap();
                    let state = listed
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(listed, _)| listed == allocation_ref)
                        .map(|(_, state)| state.clone())
                        .unwrap();
                    machine(allocation_ref, &state)
                }
                "release" => {
                    let allocation_ref = request["allocation_ref"].as_str().unwrap().to_owned();
                    for (listed, state) in listed.lock().unwrap().iter_mut() {
                        if *listed == allocation_ref {
                            *state = "deleted".into();
                        }
                    }
                    machine(&allocation_ref, "deleting")
                }
                other => panic!("unexpected control-plane action {other}"),
            }
            .to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    Service {
        api_url,
        acquires,
        machines,
    }
}

/// A kernel with the owner's offload settings, one running agent connected to `api_url`, and the
/// agent's tool token.
async fn agent_kernel(
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

async fn acquire(
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
async fn acquire_waits_for_a_card(
    state: &KernelRuntimeState,
    token: &str,
    room: &str,
    request_id: &str,
    class_id: &str,
) -> String {
    let (call_state, token, request_id, class_id) = (
        state.clone(),
        token.to_owned(),
        request_id.to_owned(),
        class_id.to_owned(),
    );
    let call =
        tokio::spawn(async move { acquire(&call_state, &token, &request_id, &class_id).await });
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
    service.machines.lock().unwrap()[0].1 = "deleted".into();
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
    service.machines.lock().unwrap()[0].1 = "deleted".into();
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
