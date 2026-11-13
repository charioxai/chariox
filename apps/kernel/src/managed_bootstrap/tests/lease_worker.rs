use super::*;
use crate::config::write_private_file;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::managed_context::outbound_service::ManagedContextTransferTicket;
use crate::runtime::command::KernelCommand;
use chariox_relay::{RelayAction, RelayAuthVerifier, RelayTokenClaims, ScopedTokenVerifier};
use std::collections::BTreeMap;
use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
#[tokio::test(flavor = "current_thread")]
async fn enrolled_lease_worker_materializes_selected_project_via_public_context_transfer() {
    crate::test_support::isolated_env_test!();
    use std::sync::Arc;

    use chariox_relay::{RelayConfig, RelayServer};
    use tokio::sync::{oneshot, watch, Mutex};
    use tokio::time::{sleep, timeout};

    use crate::app::DaemonApp;
    use crate::config::KernelRuntimeRole;
    use crate::local::{
        GetManagedContextTransferStatusRequest, StartManagedContextTransferRequest,
        UpdateProjectWorkspacesRequest,
    };
    use crate::managed_context::development::DevelopmentRepositoryRole;
    use crate::managed_context::outbound_service::{
        ManagedContextOutboundOperationPhase, ManagedContextTransferTarget,
    };
    use crate::session::{CreateSessionRequest, SessionAgentDefaults, SessionProjectSelection};

    let _env = crate::env_lock::lock();
    let fixture = super::super::tests::Fixture::new("disposable-project-transfer");
    let task_root = fixture
        .config
        .chariox_home
        .parent()
        .expect("fixture home parent")
        .to_path_buf();
    let source_git = task_root.join("selected-git");
    let source_directory = task_root.join("selected-directory");
    let home_root = task_root.join("home-daemon");
    let worker_root = task_root.join("worker-daemon");
    let target_workspace_parent = worker_root.join("state").join("managed-context-workspaces");
    let unrelated_worker_file = target_workspace_parent.join("unrelated-preserved.txt");
    let worker_started_marker = fixture
        .config
        .chariox_home
        .join("disposable-worker/kernel-started");

    let previous_home = env::var_os("HOME");
    let previous_chariox_home = env::var_os("CHARIOX_HOME");
    let previous_local_auth = env::var_os("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE");
    let previous_started_marker = env::var_os("CHARIOX_KERNEL_STARTED_MARKER");
    env::set_var("CHARIOX_HOME", &fixture.config.chariox_home);
    env::set_var("HOME", &task_root);
    env::set_var("CHARIOX_KERNEL_STARTED_MARKER", &worker_started_marker);

    let mut worker_app_config =
        isolated_worker_test_config(&worker_root, "worker-kernel", "worker-machine", 43119);
    let worker_private_key = worker_app_config.relay_private_key.clone();
    let worker_public_key = worker_app_config.relay_public_key.clone();
    init_worker_test_identity_with_key(
        &fixture.config.chariox_home,
        "worker-kernel",
        "worker-machine",
        &worker_private_key,
        &worker_public_key,
    );

    let home_machine_id = "source-machine-test";
    let home_kernel_id = "home-kernel";
    let mut home_app_config =
        isolated_worker_test_config(&home_root, home_kernel_id, home_machine_id, 43120);
    let home_public_key = home_app_config.relay_public_key.clone();
    let home_caller = crate::config::LeaseWorkerHomeCaller {
        user_id: "local".to_string(),
        realm_id: "default".to_string(),
        kernel_id: home_kernel_id.to_string(),
        relay_public_key: home_public_key.clone(),
    };

    init_test_repository(&source_git, "selected.txt", "selected source\n");
    fs::create_dir_all(&source_directory).expect("selected directory should exist");
    fs::write(
        source_directory.join("supporting.txt"),
        "selected supporting source\n",
    )
    .expect("selected directory file should exist");
    fs::create_dir_all(&target_workspace_parent).expect("target workspace parent");
    fs::write(&unrelated_worker_file, "untouched worker state\n")
        .expect("unrelated worker state should exist");

    let cloud_api = TestCloudApi::bind();
    let relay_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("relay listener should bind");
    let relay_address = relay_listener.local_addr().expect("relay address");
    home_app_config.relay_url = Some(format!("ws://{relay_address}"));
    home_app_config.relay_token = Some("relay-home-token".to_string());
    home_app_config.cloud_relay = Some(test_cloud_profile(
        cloud_api.url.clone(),
        home_machine_id,
        "mcred_home_test",
        "default",
    ));
    worker_app_config.relay_url = Some(format!("ws://{relay_address}"));
    worker_app_config.relay_token = Some("relay-worker-token".to_string());
    worker_app_config.cloud_relay = Some(test_cloud_profile(
        cloud_api.url.clone(),
        "worker-machine",
        &format!("mcred_{}", "w".repeat(40)),
        "default",
    ));
    worker_app_config.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
    worker_app_config.accept_remote_leases = true;
    worker_app_config.remote_lease_capacity = Some(1);
    worker_app_config.lease_worker_home_caller = Some(home_caller.clone());

    let relay_auth = test_relay_auth(
        (
            "relay-home-token",
            home_kernel_id,
            home_machine_id,
            &home_public_key,
        ),
        (
            "relay-worker-token",
            &worker_app_config.daemon_id,
            &worker_app_config.host_machine_id,
            &worker_public_key,
        ),
    );
    let relay = Arc::new(RelayServer::with_auth_verifier(
        RelayConfig {
            host: relay_address.ip().to_string(),
            port: relay_address.port(),
            shared_token: None,
        },
        relay_auth,
    ));
    let relay_registry = relay.registry();
    let (relay_shutdown_tx, relay_shutdown_rx) = oneshot::channel();
    let relay_task = {
        let relay = Arc::clone(&relay);
        tokio::spawn(async move {
            relay
                .run_listener_until(relay_listener, async {
                    let _ = relay_shutdown_rx.await;
                })
                .await
                .expect("test relay should run");
        })
    };

    let home_app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(home_app_config.clone()).expect("home app should bootstrap"),
    ));
    // Use the same explicit enrollment and home binding as a lease-worker launch.
    env::set_var("CHARIOX_KERNEL_RUNTIME_ROLE", "remote_lease_worker");
    env::set_var("CHARIOX_ACCEPT_REMOTE_LEASES", "1");
    env::set_var("CHARIOX_REMOTE_LEASE_CAPACITY", "1");
    env::set_var(
        "CHARIOX_LEASE_WORKER_HOME_CALLER",
        serde_json::to_string(&home_caller.clone()).expect("encode worker home caller"),
    );
    let worker_app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(worker_app_config.clone()).expect("worker app should bootstrap"),
    ));
    let home_router = Arc::new(
        crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::clone(&home_app), 1),
    );
    let worker_router = Arc::new(
        crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::clone(&worker_app),
            1,
        ),
    );
    let home_state = home_app.lock().await.relay_client_state();
    let worker_state = worker_app.lock().await.relay_client_state();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let home_connector = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
            Arc::clone(&home_router),
            home_state,
            shutdown_rx.clone(),
            format!("ws://{relay_address}"),
            "relay-home-token".to_string(),
        ),
    );
    let worker_connector = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
            Arc::clone(&worker_router),
            worker_state,
            shutdown_rx,
            format!("ws://{relay_address}"),
            "relay-worker-token".to_string(),
        ),
    );
    timeout(Duration::from_secs(5), async {
        loop {
            let registered = relay_registry.read().await;
            let ready = registered.daemon(home_kernel_id).is_some()
                && registered.daemon(&worker_app_config.daemon_id).is_some();
            drop(registered);
            if ready {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("home and confirmed worker should register on the relay");

    let seeded = dispatch_public(
        &home_router,
        LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                source_git.display().to_string(),
                source_git.display().to_string(),
            )
            .with_project_selection(SessionProjectSelection::New { name: None })
            .with_agent_defaults(SessionAgentDefaults::new("default")),
        ),
    )
    .await
    .expect("home should create the selected Project");
    let (seed_session_id, project_id) = match seeded {
        LocalDaemonResponse::SessionCreated { session, .. } => {
            (session.id().to_string(), session.project_id().to_string())
        }
        other => panic!("unexpected Project seed response: {other:?}"),
    };
    let updated = dispatch_public(
        &home_router,
        LocalDaemonRequest::UpdateProjectWorkspaces(UpdateProjectWorkspacesRequest {
            project_id: project_id.clone(),
            workspace_ids: vec![
                source_git.display().to_string(),
                source_directory.display().to_string(),
            ],
        }),
    )
    .await
    .expect("home should select both Project workspaces");
    match updated {
        LocalDaemonResponse::ProjectWorkspacesUpdated { project } => assert_eq!(
            project.workspace_ids(),
            &[
                source_git.display().to_string(),
                source_directory.display().to_string()
            ]
        ),
        other => panic!("unexpected Project update response: {other:?}"),
    }

    let context_plan = ManagedKernelContextPlan::source_project_with_repositories_for_tests(
        "context-disposable-project",
        "default",
        home_machine_id,
        home_kernel_id,
        &crate::runtime::terminal_pairings::public_key_thumbprint(&home_public_key),
        &project_id,
        vec![
            (
                DevelopmentRepositoryRole::Primary,
                source_git.display().to_string(),
                None,
            ),
            (
                DevelopmentRepositoryRole::Supporting,
                source_directory.display().to_string(),
                None,
            ),
        ],
    );
    let ticket = ManagedContextTransferTicket {
        environment_id: "environment-disposable-project".to_string(),
        context_plan,
        target: ManagedContextTransferTarget {
            relay_realm_id: "default".to_string(),
            machine_id: worker_app_config.host_machine_id.clone(),
            kernel_id: worker_app_config.daemon_id.clone(),
            relay_public_key: worker_public_key.clone(),
            key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                &worker_public_key,
            ),
        },
    };
    cloud_api.set_ticket(ticket.clone());

    let started = dispatch_public(
        &home_router,
        LocalDaemonRequest::StartManagedContextTransfer(Box::new(
            StartManagedContextTransferRequest {
                owner_managed: None,
                interactive: false,
                ticket: Some(ticket.clone()),
                launch_id: None,
            },
        )),
    )
    .await;
    let mut final_status = None;
    if started.is_ok() {
        for _ in 0..200 {
            let status = dispatch_public(
                &home_router,
                LocalDaemonRequest::GetManagedContextTransferStatus(
                    GetManagedContextTransferStatusRequest {
                        context_id: ticket.context_plan.context_id().to_string(),
                    },
                ),
            )
            .await;
            if let Ok(LocalDaemonResponse::ManagedContextTransferStatus { status }) = status {
                if matches!(
                    status.phase,
                    ManagedContextOutboundOperationPhase::Completed
                        | ManagedContextOutboundOperationPhase::Failed
                ) {
                    final_status = Some(status);
                    break;
                }
            }
            sleep(Duration::from_millis(25)).await;
        }
    }

    let launch_result = if final_status
        .as_ref()
        .is_some_and(|status| status.phase == ManagedContextOutboundOperationPhase::Completed)
    {
        Some(
            worker_app
                .lock()
                .await
                .managed_context_transfer_store()
                .launch_target(
                    ticket.context_plan.context_id(),
                    &ticket.context_plan.package_binding().plan_digest,
                ),
        )
    } else {
        None
    };

    let _ = shutdown_tx.send(true);
    timeout(Duration::from_secs(5), home_connector)
        .await
        .expect("home connector should stop")
        .expect("home connector should join");
    timeout(Duration::from_secs(5), worker_connector)
        .await
        .expect("worker connector should stop")
        .expect("worker connector should join");
    let _ = relay_shutdown_tx.send(());
    timeout(Duration::from_secs(5), relay_task)
        .await
        .expect("relay task should stop")
        .expect("relay task should join");
    let cloud_requests = cloud_api.stop();

    restore_worker_test_env("HOME", previous_home);
    restore_worker_test_env("CHARIOX_HOME", previous_chariox_home);
    restore_worker_test_env("CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE", previous_local_auth);
    restore_worker_test_env("CHARIOX_KERNEL_STARTED_MARKER", previous_started_marker);
    env::remove_var("CHARIOX_KERNEL_RUNTIME_ROLE");
    env::remove_var("CHARIOX_ACCEPT_REMOTE_LEASES");
    env::remove_var("CHARIOX_REMOTE_LEASE_CAPACITY");
    env::remove_var("CHARIOX_LEASE_WORKER_HOME_CALLER");

    assert!(
        started.is_ok(),
        "public managed-worker transfer should start from the enrolled home: {started:?}"
    );
    let final_status = final_status.expect("public transfer should reach a terminal status");
    assert_eq!(
        final_status.phase,
        ManagedContextOutboundOperationPhase::Completed,
        "confirmed disposable worker should accept the selected Project transfer: {final_status:?}"
    );
    assert!(
        cloud_requests
            .iter()
            .any(|(path, _)| path.ends_with("/context/ticket")),
        "home must fetch the authoritative context ticket"
    );
    assert!(
        cloud_requests
            .iter()
            .any(|(path, _)| path.ends_with("/context/complete")),
        "worker must report the completed import to Cloud"
    );
    let ticket_request = cloud_requests
        .iter()
        .find(|(path, _)| path.ends_with("/context/ticket"))
        .expect("home must request an authoritative ticket");
    assert_eq!(ticket_request.1["machineId"], home_machine_id);
    assert_eq!(ticket_request.1["kernelId"], home_kernel_id);
    assert_eq!(ticket_request.1["relayRealmId"], "default");
    let Some(Ok(target)) = launch_result else {
        panic!("worker should retain the completed launch target: {launch_result:?}");
    };
    let crate::local::ManagedContextDevelopmentLaunchTarget::FromSource { repositories, .. } =
        target.development
    else {
        panic!("selected Project should produce source repositories");
    };
    assert_eq!(repositories.len(), 2);
    for repository in repositories {
        let path = std::path::PathBuf::from(repository.workspace_path);
        match repository.role {
            DevelopmentRepositoryRole::Primary => assert_eq!(
                fs::read_to_string(path.join("selected.txt"))
                    .expect("selected Git file should be copied"),
                "selected source\n"
            ),
            DevelopmentRepositoryRole::Supporting => assert_eq!(
                fs::read_to_string(path.join("supporting.txt"))
                    .expect("selected directory file should be copied"),
                "selected supporting source\n"
            ),
        }
    }
    assert_eq!(
        fs::read_to_string(&unrelated_worker_file).expect("unrelated worker state"),
        "untouched worker state\n"
    );
    assert_eq!(
        fs::read_to_string(source_git.join("selected.txt"))
            .expect("source Git file should remain intact"),
        "selected source\n"
    );
    assert_eq!(
        fs::read_to_string(source_directory.join("supporting.txt"))
            .expect("source directory file should remain intact"),
        "selected supporting source\n"
    );
    assert!(
        !seed_session_id.is_empty(),
        "the public Project selection must have created a home session"
    );
    fixture.cleanup();
}

struct TestCloudApi {
    url: String,
    ticket: std::sync::Arc<std::sync::Mutex<Option<ManagedContextTransferTicket>>>,
    requests: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    shutdown: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TestCloudApi {
    fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Cloud fixture should bind");
        listener
            .set_nonblocking(true)
            .expect("Cloud fixture should be nonblocking");
        let address = listener.local_addr().expect("Cloud fixture address");
        let ticket = std::sync::Arc::new(std::sync::Mutex::new(None));
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let thread_ticket = std::sync::Arc::clone(&ticket);
        let thread_requests = std::sync::Arc::clone(&requests);
        let (shutdown, shutdown_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            while shutdown_rx.try_recv().is_err() {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if let Some((path, body)) = read_worker_http_request(&mut stream) {
                            thread_requests
                                .lock()
                                .expect("Cloud request lock")
                                .push((path.clone(), body.clone()));
                            let (status, response) = if path.ends_with("/context/ticket") {
                                match thread_ticket.lock().expect("Cloud ticket lock").clone() {
                                    Some(ticket) => (
                                        "200 OK",
                                        serde_json::to_vec(&ticket).expect("encode Cloud ticket"),
                                    ),
                                    None => ("503 Service Unavailable", b"{}".to_vec()),
                                }
                            } else if path.ends_with("/context/complete") {
                                let digest = body
                                    .get("contextManifestDigest")
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Null);
                                (
                                    "200 OK",
                                    serde_json::to_vec(&serde_json::json!({
                                        "ready": true,
                                        "observedState": "ready",
                                        "contextManifestDigest": digest,
                                    }))
                                    .expect("encode Cloud completion"),
                                )
                            } else {
                                ("404 Not Found", b"{}".to_vec())
                            };
                            let header = format!(
                                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                    response.len()
                                );
                            let _ = stream.write_all(header.as_bytes());
                            let _ = stream.write_all(&response);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url: format!("http://{address}"),
            ticket,
            requests,
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
    }

    fn set_ticket(&self, ticket: ManagedContextTransferTicket) {
        *self.ticket.lock().expect("Cloud ticket lock") = Some(ticket);
    }

    fn stop(mut self) -> Vec<(String, serde_json::Value)> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().expect("Cloud fixture should stop");
        }
        self.requests.lock().expect("Cloud request lock").clone()
    }
}

fn read_worker_http_request(stream: &mut TcpStream) -> Option<(String, serde_json::Value)> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("Cloud fixture timeout");
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        request.extend_from_slice(&chunk[..read]);
        let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })?;
        if request.len() < header_end + 4 + content_length {
            continue;
        }
        let path = headers
            .lines()
            .next()?
            .split_whitespace()
            .nth(1)?
            .to_string();
        let body =
            serde_json::from_slice(&request[header_end + 4..header_end + 4 + content_length])
                .ok()?;
        return Some((path, body));
    }
}

fn isolated_worker_test_config(
    root: &std::path::Path,
    daemon_id: &str,
    machine_id: &str,
    websocket_port: u16,
) -> DaemonConfig {
    let mut config = DaemonConfig::for_tests();
    config.user_config_path = root.join("config.toml");
    config.daemon_id = daemon_id.to_string();
    config.host_machine_id = machine_id.to_string();
    config.local_socket_path =
        DaemonConfig::default_local_socket_path(&format!("fixture:{}", root.display()));
    config = config.with_session_history_root(root.join("sessions"));
    config.kernel_websocket_host = "127.0.0.1".to_string();
    config.kernel_websocket_port = websocket_port;
    config.runtime_mcp_host = "127.0.0.1".to_string();
    config.runtime_mcp_port = websocket_port.saturating_add(1);
    config.user_config.history.operational.path =
        Some(root.join("history.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config.user_config.state.path = Some(root.join("state/state.db").display().to_string());
    config.user_config.credential_vault.path = root.join("vault.json").display().to_string();
    config
}

fn init_worker_test_identity_with_key(
    home: &std::path::Path,
    daemon_id: &str,
    machine_id: &str,
    private_key: &str,
    public_key: &str,
) {
    write_private_file(
        &home.join("machine/identity.json"),
        &serde_json::to_vec(&serde_json::json!({
            "machine_id": machine_id,
        }))
        .expect("encode machine identity"),
    )
    .expect("write machine identity");
    write_private_file(
        &home.join("state/daemon/identity.json"),
        &serde_json::to_vec(&serde_json::json!({
            "daemon_id": daemon_id,
            "machine_id": machine_id,
            "relay_public_key": public_key,
            "relay_private_key": private_key,
        }))
        .expect("encode worker runtime identity"),
    )
    .expect("write worker runtime identity");
}

fn test_cloud_profile(
    api_url: String,
    machine_id: &str,
    machine_credential: &str,
    realm_id: &str,
) -> PersistedCloudRelayProfile {
    PersistedCloudRelayProfile {
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
        api_url,
        email: "owner@example.test".to_string(),
        account_id: "account-1".to_string(),
        user_id: "local".to_string(),
        account_slug: "account-1".to_string(),
        realm_id: realm_id.to_string(),
        relay_url: "wss://relay.example.test".to_string(),
        issuer_id: "issuer-test".to_string(),
        client_id: None,
        client_alias: None,
        machine_id: Some(machine_id.to_string()),
        machine_credential: Some(machine_credential.to_string()),
        cloud_session_token: None,
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: None,
    }
}

fn test_relay_auth(
    home: (&str, &str, &str, &str),
    worker: (&str, &str, &str, &str),
) -> RelayAuthVerifier {
    let claims = [home, worker]
        .into_iter()
        .map(|(token, subject, machine_id, public_key)| {
            (
                token.to_string(),
                RelayTokenClaims {
                    issuer: "disposable-worker-test".to_string(),
                    subject: subject.to_string(),
                    subject_kind: chariox_relay::RelaySubjectKind::Kernel,
                    realm_id: "default".to_string(),
                    allowed_actions: vec![
                        RelayAction::DaemonRegister,
                        RelayAction::DaemonHeartbeat,
                        RelayAction::PeerRequest,
                        RelayAction::PeerEvent,
                        RelayAction::ClientMetadataRead,
                        RelayAction::PacketRoute,
                    ],
                    allowed_targets: None,
                    issued_at_ms: 1,
                    expires_at_ms: u64::MAX,
                    token_id: token.to_string(),
                    account_id: Some("account-1".to_string()),
                    organization_id: None,
                    user_id: Some("local".to_string()),
                    device_id: None,
                    machine_id: Some(machine_id.to_string()),
                    client_id: None,
                    session_id: None,
                    public_key_thumbprint: Some(
                        crate::runtime::terminal_pairings::public_key_thumbprint(public_key),
                    ),
                    entitlements_version: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), None))
}

fn dispatch_public(
    router: &crate::runtime::router::CommandRouter,
    request: LocalDaemonRequest,
) -> impl std::future::Future<Output = Result<LocalDaemonResponse, DaemonError>> + '_ {
    let command = KernelCommand::from_local_request(
        format!("disposable-project-{}", rand::random::<u64>()),
        None,
        None,
        &request,
    );
    // Construct and box the large router future outside the test's poll
    // frame so public requests run on the default test stack.
    Box::pin(router.dispatch(command, request))
}

fn restore_worker_test_env(name: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => env::set_var(name, value),
        None => env::remove_var(name),
    }
}

fn init_test_repository(root: &std::path::Path, filename: &str, contents: &str) {
    fs::create_dir_all(root).expect("test repository should exist");
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.email", "chariox@example.test"]);
    run_git(root, &["config", "user.name", "Chariox Test"]);
    fs::write(root.join(filename), contents).expect("test repository file should exist");
    run_git(root, &["add", filename]);
    run_git(root, &["commit", "-m", "selected source"]);
}

fn run_git(root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git should be available for the materialization regression");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
