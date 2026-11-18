//! The agent's `chariox.machines.acquire` tool against a stand-in connected service.
use super::*;
use crate::local::{MachineAllocationCapability, MachineOption};
use crate::provider::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Service {
    api_url: String,
    acquires: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    /// The options the service lists; a test changes them to change the stock.
    offers: Arc<std::sync::Mutex<Vec<MachineOption>>>,
}

/// A connected service that lists `offers` and refuses to rent any class named in `sold_out`.
async fn service(offers: Vec<MachineOption>, sold_out: &'static [&'static str]) -> Service {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let acquires = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = acquires.clone();
    let offers = Arc::new(std::sync::Mutex::new(offers));
    let listed = offers.clone();
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
            let (status, response) = match request["action"].as_str().unwrap() {
                "capabilities" => {
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
                    match request["class_id"].as_str() {
                        Some(class) if sold_out.contains(&class) => (
                            "429 Too Many Requests",
                            serde_json::json!({"error":{"code":"capacity_exceeded","message":"No capacity"}}),
                        ),
                        _ => (
                            "200 OK",
                            serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
                                "allocation_ref":"allocation-1","state":"creating","kernel_target":null,
                                "access_state":"pending","access_reason":null}),
                        ),
                    }
                }
                other => panic!("unexpected control-plane action {other}"),
            };
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
    });
    Service {
        api_url,
        acquires,
        offers,
    }
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
    assert_eq!(acquires[1]["request_id"], "r1");
}

#[tokio::test]
async fn a_retry_after_a_refusal_chooses_again() {
    let service = service(
        vec![option("big", "north", true), option("small", "south", true)],
        &["big"],
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
    // The listing still shows "big" in stock, but the service has sold it.
    assert!(acquire().await.is_err());
    service.offers.lock().unwrap()[0].available = false;
    acquire().await.unwrap();
    let acquires = service.acquires.lock().unwrap();
    assert_eq!(acquires[1]["class_id"], "small");
    assert_eq!(acquires[1]["region"], "south");
}
