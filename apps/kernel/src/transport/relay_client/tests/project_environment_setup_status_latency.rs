#![allow(unused_imports)]

use super::support::*;

use chariox_relay::protocol::{DaemonRegistration, RelayEnvelope, RelayError};
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::oneshot;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::agent::RemoteAgentBinding;
use crate::local::{
    GetProjectEnvironmentSetupStatusRequest, LocalDaemonRequest, LocalDaemonResponse,
    ProjectEnvironmentDefinition, ProjectEnvironmentDefinitionOrigin,
    ProjectEnvironmentDefinitionSource, ProjectEnvironmentSetupPhase,
    ProjectEnvironmentSetupStatus, ProjectEnvironmentSetupStep, ProjectEnvironmentSetupStepKind,
    StartProjectEnvironmentSetupRequest,
};
use crate::session::CreateSessionRequest;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RelayProjectEnvironmentSetupStatus,
    PROJECT_ENVIRONMENT_SETUP_NOT_FOUND_CODE, PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE,
    RELAY_PEER_PROTOCOL_VERSION,
};

const PUBLIC_STATUS_RESPONSE_DEADLINE: Duration = Duration::from_millis(300);

#[test]
fn authenticated_public_setup_status_request_has_a_bounded_worker_response_deadline() {
    run_async_with_large_test_stack(
        "public-project-environment-setup-status-latency",
        authenticated_public_setup_status_request_has_a_bounded_worker_response_deadline_async,
    );
}

async fn authenticated_public_setup_status_request_has_a_bounded_worker_response_deadline_async() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _test_home = RelayTestHome::new();

    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener
        .local_addr()
        .expect("relay listener should have an address");
    let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = "daemon-status-latency-home".to_string();
    config_home.daemon_alias = Some("status-latency-home".to_string());
    config_home.host_machine_id = "machine-status-latency-home".to_string();
    config_home.relay_url = Some(relay_url.clone());
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;

    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-status-latency-worker".to_string();
    config_worker.daemon_alias = Some("status-latency-worker".to_string());
    config_worker.host_machine_id = "machine-status-latency-worker".to_string();
    config_worker.host_machine_alias = Some("status-latency-worker".to_string());
    config_worker.relay_url = Some(relay_url.clone());
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;

    let (worker_registration, worker_private_key) = {
        let mut worker_app = DaemonApp::bootstrap(config_worker.clone())
            .expect("worker registration fixture should bootstrap");
        (
            worker_app.relay_registration(),
            config_worker.relay_private_key.clone(),
        )
    };
    let (
        shutdown_worker_tx,
        release_status_tx,
        worker_start_seen,
        worker_status_seen,
        worker_start_count,
        worker_task,
    ) = spawn_withheld_status_worker(
        relay_url.clone(),
        worker_registration,
        worker_private_key,
        config_home.relay_public_key.clone(),
    );
    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");

    let (session_id, project_id, agent_id) = {
        let mut app = app_home.lock().await;
        let (session, agent) = app
            .create_session(CreateSessionRequest::new(
                "workspace-status-latency",
                "worktree-status-latency",
            ))
            .expect("home session should be created");
        (
            session.id().to_string(),
            session.project_id().to_string(),
            agent.id().to_string(),
        )
    };
    {
        let mut app = app_home.lock().await;
        app.agents()
            .bind_remote_execution(
                &agent_id,
                RemoteAgentBinding {
                    worker_kernel_id: config_worker.daemon_id.clone(),
                    worker_machine_id: config_worker.host_machine_id.clone(),
                    execution_lease_id: "lease-status-latency".to_string(),
                    leased_agent_id: "leased-agent-status-latency".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: Some(relay_url.clone()),
                    relay_token: Some("secret".to_string()),
                    relay_peer_protocol_version: Some(RELAY_PEER_PROTOCOL_VERSION),
                },
            )
            .expect("home agent should bind to the fixture worker");
    }

    let (mut client_socket, _) = connect_async(&relay_url)
        .await
        .expect("public client should connect to relay");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config_home.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let home_public_key = expect_client_connected(&mut client_socket).await;

    let target_platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let definition = ProjectEnvironmentDefinition {
        schema_version: 1,
        origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
        source: ProjectEnvironmentDefinitionSource::Commands,
        target_platform: target_platform.clone(),
        source_path: None,
        setup_steps: vec![ProjectEnvironmentSetupStep {
            kind: ProjectEnvironmentSetupStepKind::Command,
            command: "true".to_string(),
        }],
        validation_commands: vec!["true".to_string()],
    };
    let operation_id = "setup-status-latency".to_string();
    let start_private_key = send_client_request(
        &mut client_socket,
        "setup-status-latency-start",
        &config_home.daemon_id,
        &home_public_key,
        LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
            operation_id: operation_id.clone(),
            project_id: project_id.clone(),
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
            target_worker_id: config_worker.host_machine_id.clone(),
            target_platform,
            definition: Some(definition),
            validation_commands: Vec::new(),
        }),
    )
    .await;
    let start_response = expect_client_response(
        &mut client_socket,
        "setup-status-latency-start",
        &start_private_key,
    )
    .await;
    assert!(matches!(
        start_response,
        LocalDaemonResponse::ProjectEnvironmentSetupStarted { status }
            if status.operation_id == operation_id && status.attempt == 1
    ));
    tokio::time::timeout(Duration::from_secs(2), worker_start_seen)
        .await
        .expect("fixture worker should observe authenticated Start")
        .expect("fixture Start barrier should remain available");

    let _status_private_key = send_client_request(
        &mut client_socket,
        "setup-status-latency-get",
        &config_home.daemon_id,
        &home_public_key,
        LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
            GetProjectEnvironmentSetupStatusRequest {
                operation_id: operation_id.clone(),
            },
        ),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), worker_status_seen)
        .await
        .expect("fixture worker should observe authenticated Get")
        .expect("fixture Get barrier should remain available");

    let bounded_status = tokio::time::timeout(
        PUBLIC_STATUS_RESPONSE_DEADLINE,
        expect_client_response_error(
            &mut client_socket,
            "setup-status-latency-get",
        ),
    )
    .await
    .expect(
        "public setup status must settle within the explicit deadline when the worker withholds its reply",
    );
    assert_eq!(
        bounded_status.code, "local_transport_error",
        "a withheld worker reply is transport uncertainty, not a setup terminal result: {bounded_status:?}"
    );
    assert!(
        bounded_status.retryable,
        "a withheld worker reply must remain recoverable: {bounded_status:?}"
    );
    assert_eq!(
        worker_start_count.load(Ordering::Acquire),
        1,
        "a bounded status observation must not redispatch Start"
    );

    let _ = release_status_tx.send(());
    let _ = client_socket.close(None).await;

    let (mut recovery_socket, _) = connect_async(&relay_url)
        .await
        .expect("recovery client should connect to relay");
    send_client_envelope(
        &mut recovery_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config_home.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let recovery_home_public_key = expect_client_connected(&mut recovery_socket).await;
    let recovery_private_key = send_client_request(
        &mut recovery_socket,
        "setup-status-latency-recovery-get",
        &config_home.daemon_id,
        &recovery_home_public_key,
        LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
            GetProjectEnvironmentSetupStatusRequest { operation_id },
        ),
    )
    .await;
    let recovery_response = tokio::time::timeout(
        Duration::from_secs(2),
        expect_client_response(
            &mut recovery_socket,
            "setup-status-latency-recovery-get",
            &recovery_private_key,
        ),
    )
    .await
    .expect("released worker status should remain publicly recoverable");
    let recovered_status = match recovery_response {
        LocalDaemonResponse::ProjectEnvironmentSetupStatus { status } => status,
        other => panic!("unexpected recovered setup response: {other:?}"),
    };
    assert_eq!(recovered_status.operation_id, "setup-status-latency");
    assert_eq!(recovered_status.attempt, 1);
    assert_eq!(
        recovered_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert!(recovered_status.retryable);
    assert_eq!(
        worker_start_count.load(Ordering::Acquire),
        1,
        "recovery must not create a second Start dispatch"
    );

    let _ = shutdown_worker_tx.send(());
    worker_task
        .await
        .expect("withheld-status worker should stop");
    let _ = shutdown_home_tx.send(true);
    connector_home
        .await
        .expect("home relay connector should stop");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("relay server should stop");
}

fn spawn_withheld_status_worker(
    relay_url: String,
    registration: DaemonRegistration,
    worker_private_key: String,
    home_public_key: String,
) -> (
    oneshot::Sender<()>,
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (release_status_tx, release_status_rx) = oneshot::channel();
    let (start_seen_tx, start_seen_rx) = oneshot::channel();
    let (status_seen_tx, status_seen_rx) = oneshot::channel();
    let start_count = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn(run_withheld_status_worker(
        relay_url,
        registration,
        worker_private_key,
        home_public_key,
        shutdown_rx,
        release_status_rx,
        start_seen_tx,
        status_seen_tx,
        Arc::clone(&start_count),
    ));
    (
        shutdown_tx,
        release_status_tx,
        start_seen_rx,
        status_seen_rx,
        start_count,
        task,
    )
}

async fn run_withheld_status_worker(
    relay_url: String,
    registration: DaemonRegistration,
    worker_private_key: String,
    home_public_key: String,
    mut shutdown_rx: oneshot::Receiver<()>,
    mut release_status_rx: oneshot::Receiver<()>,
    start_seen_tx: oneshot::Sender<()>,
    status_seen_tx: oneshot::Sender<()>,
    start_count: Arc<AtomicUsize>,
) {
    let (mut socket, _) =
        match tokio::time::timeout(Duration::from_secs(3), connect_async(&relay_url)).await {
            Ok(Ok(connection)) => connection,
            _ => return,
        };
    let register = RelayEnvelope::DaemonRegister { registration };
    let Ok(register_payload) = serde_json::to_string(&register) else {
        return;
    };
    if socket
        .send(Message::Text(register_payload.into()))
        .await
        .is_err()
    {
        return;
    }

    let mut start_seen_tx = Some(start_seen_tx);
    let mut status_seen_tx = Some(status_seen_tx);
    let mut held_setup: Option<RelayProjectEnvironmentSetupStatus> = None;
    let mut status_released = false;
    loop {
        let message = tokio::select! {
            _ = &mut shutdown_rx => {
                let _ = socket.close(None).await;
                return;
            }
            message = socket.next() => message,
        };
        let Some(Ok(message)) = message else {
            return;
        };
        let envelope = match message {
            Message::Text(text) => match serde_json::from_str::<RelayEnvelope>(&text) {
                Ok(envelope) => envelope,
                Err(_) => return,
            },
            Message::Ping(payload) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    return;
                }
                continue;
            }
            Message::Close(_) => return,
            _ => continue,
        };
        let RelayEnvelope::DaemonIncomingPeerRequest {
            relay_request_id,
            encrypted_request,
            ..
        } = envelope
        else {
            continue;
        };
        let decrypted = match relay_crypto::decrypt_payload_for_private_key(
            &worker_private_key,
            &encrypted_request,
        ) {
            Ok(decrypted) => decrypted,
            Err(_) => return,
        };
        let request = match serde_json::from_slice::<RelayPeerRequest>(&decrypted.plaintext) {
            Ok(request) => request,
            Err(_) => return,
        };
        let (encrypted_response, error) = match request {
            RelayPeerRequest::StartLeasedProjectEnvironmentSetup {
                operation_id,
                attempt,
                project_id,
                home_session_id,
                home_agent_id,
                target_worker_id,
                target_platform,
                definition,
                ..
            } => {
                start_count.fetch_add(1, Ordering::AcqRel);
                if let Some(start_seen_tx) = start_seen_tx.take() {
                    let _ = start_seen_tx.send(());
                }
                let setup = RelayProjectEnvironmentSetupStatus {
                    status: ProjectEnvironmentSetupStatus {
                        operation_id,
                        project_id,
                        session_id: home_session_id,
                        agent_id: home_agent_id,
                        worker_id: target_worker_id,
                        platform: target_platform,
                        phase: ProjectEnvironmentSetupPhase::Requested,
                        attempt,
                        progress_percent: 0,
                        definition_digest: definition
                            .as_ref()
                            .map(ProjectEnvironmentDefinition::digest),
                        validation: None,
                        message: Some("fixture worker accepted setup".to_string()),
                        failure_code: None,
                        failure_message: None,
                        retryable: true,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    },
                    definition,
                };
                held_setup = Some(setup.clone());
                (
                    Some(encrypt_worker_response(
                        &worker_private_key,
                        &home_public_key,
                        RelayPeerResponse::LeasedProjectEnvironmentSetupStarted { setup },
                    )),
                    None,
                )
            }
            RelayPeerRequest::GetLeasedProjectEnvironmentSetupStatus { .. } => {
                if let Some(status_seen_tx) = status_seen_tx.take() {
                    let _ = status_seen_tx.send(());
                }
                if held_setup.is_none() {
                    (
                        None,
                        Some(RelayError {
                            code: PROJECT_ENVIRONMENT_SETUP_NOT_FOUND_CODE.to_string(),
                            message: "setup operation was not found".to_string(),
                            retryable: false,
                        }),
                    )
                } else {
                    if !status_released {
                        tokio::select! {
                            _ = &mut shutdown_rx => return,
                            _ = &mut release_status_rx => status_released = true,
                        }
                    }
                    (
                        Some(encrypt_worker_response(
                            &worker_private_key,
                            &home_public_key,
                            RelayPeerResponse::LeasedProjectEnvironmentSetupStatus {
                                setup: held_setup
                                    .clone()
                                    .expect("held setup should remain available"),
                            },
                        )),
                        None,
                    )
                }
            }
            _ => (
                None,
                Some(RelayError {
                    code: PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE.to_string(),
                    message: "fixture worker rejected an unsupported setup request".to_string(),
                    retryable: false,
                }),
            ),
        };
        let response = RelayEnvelope::DaemonIncomingPeerResponse {
            relay_request_id,
            encrypted_response,
            error,
        };
        let Ok(response_payload) = serde_json::to_string(&response) else {
            return;
        };
        if socket
            .send(Message::Text(response_payload.into()))
            .await
            .is_err()
        {
            return;
        }
    }
}

fn encrypt_worker_response(
    worker_private_key: &str,
    home_public_key: &str,
    response: RelayPeerResponse,
) -> chariox_relay::protocol::EncryptedRelayPayload {
    let plaintext = serde_json::to_vec(&response).expect("fixture response should serialize");
    relay_crypto::encrypt_payload_for_peer(worker_private_key, home_public_key, &plaintext)
        .expect("fixture response should encrypt for the authenticated home")
}
