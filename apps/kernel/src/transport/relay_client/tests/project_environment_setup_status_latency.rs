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
    CancelProjectEnvironmentSetupRequest, GetProjectEnvironmentSetupStatusRequest,
    LocalDaemonRequest, LocalDaemonResponse, ProjectEnvironmentDefinition,
    ProjectEnvironmentDefinitionOrigin, ProjectEnvironmentDefinitionSource,
    ProjectEnvironmentSetupPhase, ProjectEnvironmentSetupStatus, ProjectEnvironmentSetupStep,
    ProjectEnvironmentSetupStepKind, RetryProjectEnvironmentSetupRequest,
    StartProjectEnvironmentSetupRequest,
};
use crate::runtime::router::CommandRouter;
use crate::session::CreateSessionRequest;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RelayProjectEnvironmentSetupStatus,
    PROJECT_ENVIRONMENT_SETUP_NOT_FOUND_CODE, PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE,
    RELAY_PEER_PROTOCOL_VERSION,
};

const PUBLIC_STATUS_RESPONSE_DEADLINE: Duration = Duration::from_millis(300);

#[derive(Debug)]
struct ClientResponseObservation {
    response: Option<LocalDaemonResponse>,
    error: Option<RelayError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FreshGetWorkerObservation {
    Status,
    NotFound,
}

async fn expect_client_response_observation<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    request_id: &str,
    client_private_key: &str,
) -> ClientResponseObservation
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(payload))) => {
                match serde_json::from_str::<RelayEnvelope>(&payload)
                    .expect("relay envelope should parse")
                {
                    RelayEnvelope::ClientResponse {
                        request_id: response_request_id,
                        encrypted_response,
                        error,
                    } => {
                        assert_eq!(response_request_id, request_id);
                        let response = encrypted_response.map(|encrypted_response| {
                            let decrypted = relay_crypto::decrypt_payload_for_private_key(
                                client_private_key,
                                &encrypted_response,
                            )
                            .expect("client response should decrypt with its request key");
                            serde_json::from_slice(&decrypted.plaintext)
                                .expect("encrypted local response should deserialize")
                        });
                        return ClientResponseObservation { response, error };
                    }
                    RelayEnvelope::ClientEvent { .. } => {}
                    other => {
                        panic!("unexpected envelope while awaiting client response: {other:?}")
                    }
                }
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            other => panic!("unexpected relay message while awaiting client response: {other:?}"),
        }
    }
}

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
    config_home.relay_request_timeout_ms = 100;

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
    let (home_setup_scope_tx, home_setup_scope_rx) = oneshot::channel();
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
        home_setup_scope_rx,
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
    home_setup_scope_tx
        .send((
            "leased-agent-status-latency".to_string(),
            session_id.clone(),
            agent_id.clone(),
        ))
        .expect("fixture worker should receive its exact home setup binding");

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
        inputs: Vec::new(),
        path_entries: Vec::new(),
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

#[test]
fn authenticated_public_concurrent_missing_setup_polls_share_a_bounded_recovery() {
    run_async_with_large_test_stack(
        "public-project-environment-setup-concurrent-recovery",
        authenticated_public_concurrent_missing_setup_polls_share_a_bounded_recovery_async,
    );
}

#[test]
fn authenticated_public_dropped_setup_get_recovers_through_cancel_and_retry() {
    run_async_with_large_test_stack(
        "public-project-environment-setup-dropped-get-recovery",
        authenticated_public_dropped_setup_get_recovers_through_cancel_and_retry_async,
    );
}

async fn authenticated_public_dropped_setup_get_recovers_through_cancel_and_retry_async() {
    run_authenticated_public_concurrent_missing_setup_polls_with_mode(true, false, false).await;
}

async fn authenticated_public_concurrent_missing_setup_polls_share_a_bounded_recovery_async() {
    run_authenticated_public_concurrent_missing_setup_polls_with_mode(false, false, false).await;
}

#[test]
fn authenticated_public_worker_loss_after_acknowledged_replay_reopens_same_attempt_recovery() {
    run_async_with_large_test_stack(
        "public-project-environment-setup-acknowledged-worker-loss",
        authenticated_public_worker_loss_after_acknowledged_replay_reopens_same_attempt_recovery_async,
    );
}

async fn authenticated_public_worker_loss_after_acknowledged_replay_reopens_same_attempt_recovery_async(
) {
    run_authenticated_public_concurrent_missing_setup_polls_with_mode(false, true, false).await;
}

#[test]
fn authenticated_public_lost_replay_status_then_second_worker_loss_reopens_same_attempt_recovery() {
    run_async_with_large_test_stack(
        "public-project-environment-setup-lost-replay-status-second-loss",
        authenticated_public_lost_replay_status_then_second_worker_loss_reopens_same_attempt_recovery_async,
    );
}

async fn authenticated_public_lost_replay_status_then_second_worker_loss_reopens_same_attempt_recovery_async(
) {
    run_authenticated_public_concurrent_missing_setup_polls_with_mode(false, true, true).await;
}

async fn run_authenticated_public_concurrent_missing_setup_polls_with_mode(
    drop_inflight_get: bool,
    reopen_after_acknowledged_worker_loss: bool,
    lose_replay_response: bool,
) {
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
        .expect("relay server should have an address");
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
    config_home.daemon_id = "daemon-status-concurrent-home".to_string();
    config_home.daemon_alias = Some("status-concurrent-home".to_string());
    config_home.host_machine_id = "machine-status-concurrent-home".to_string();
    config_home.relay_url = Some(relay_url.clone());
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    // Keep the public regression fast with the existing lower relay timeout;
    // production's dedicated setup-status policy defaults to five seconds.
    config_home.relay_request_timeout_ms = 100;

    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-status-concurrent-worker".to_string();
    config_worker.daemon_alias = Some("status-concurrent-worker".to_string());
    config_worker.host_machine_id = "machine-status-concurrent-worker".to_string();
    config_worker.host_machine_alias = Some("status-concurrent-worker".to_string());
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
    let (home_setup_scope_tx, home_setup_scope_rx) = oneshot::channel();
    let (
        shutdown_worker_tx,
        release_replay_tx,
        release_replay_applied,
        retry_seen,
        lose_setup_tx,
        initial_start_seen,
        mut missing_get_seen,
        fresh_get_worker_observation,
        replay_start_seen,
        worker_status_seen,
        setup_loss_applied,
        post_loss_get_seen,
        post_loss_start_seen,
        worker_start_count,
        worker_task,
    ) = spawn_missing_then_withheld_replay_worker(
        relay_url.clone(),
        worker_registration,
        worker_private_key,
        config_home.relay_public_key.clone(),
        lose_replay_response,
        home_setup_scope_rx,
    );
    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let home_router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app_home),
        1,
    ));
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector_with_router(
        Arc::clone(&home_router),
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
                "workspace-status-concurrent",
                "worktree-status-concurrent",
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
                    execution_lease_id: "lease-status-concurrent".to_string(),
                    leased_agent_id: "leased-agent-status-concurrent".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: Some(relay_url.clone()),
                    relay_token: Some("secret".to_string()),
                    relay_peer_protocol_version: Some(RELAY_PEER_PROTOCOL_VERSION),
                },
            )
            .expect("home agent should bind to the fixture worker");
    }
    home_setup_scope_tx
        .send((
            "leased-agent-status-concurrent".to_string(),
            session_id.clone(),
            agent_id.clone(),
        ))
        .expect("fixture worker should receive its exact home setup binding");

    let (mut first_client, _) = connect_async(&relay_url)
        .await
        .expect("first public client should connect to relay");
    send_client_envelope(
        &mut first_client,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config_home.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let first_home_public_key = expect_client_connected(&mut first_client).await;

    let target_platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let operation_id = "setup-status-concurrent-recovery".to_string();
    let definition = ProjectEnvironmentDefinition {
        schema_version: 1,
        origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
        source: ProjectEnvironmentDefinitionSource::Commands,
        target_platform: target_platform.clone(),
        source_path: None,
        inputs: Vec::new(),
        path_entries: Vec::new(),
        setup_steps: vec![ProjectEnvironmentSetupStep {
            kind: ProjectEnvironmentSetupStepKind::Command,
            command: "true".to_string(),
        }],
        validation_commands: vec!["true".to_string()],
    };
    let start_private_key = send_client_request(
        &mut first_client,
        "setup-status-concurrent-start",
        &config_home.daemon_id,
        &first_home_public_key,
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
        &mut first_client,
        "setup-status-concurrent-start",
        &start_private_key,
    )
    .await;
    assert!(matches!(
        start_response,
        LocalDaemonResponse::ProjectEnvironmentSetupStarted { status }
            if status.operation_id == operation_id && status.attempt == 1
    ));
    tokio::time::timeout(Duration::from_secs(2), initial_start_seen)
        .await
        .expect("fixture worker should observe the initial Start")
        .expect("initial Start barrier should remain available");

    if drop_inflight_get {
        // This internal cancellation probe uses the same runtime coordinator
        // as the relay connector, but is not a client-socket disconnect. The
        // public relay path is exercised separately below.
        let runtime = home_router.runtime_state();
        let mut dropped_get = tokio::spawn({
            let runtime = runtime.clone();
            let operation_id = operation_id.clone();
            async move {
                runtime
                    .execute_project_environment_setup_request(
                        LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                            GetProjectEnvironmentSetupStatusRequest { operation_id },
                        ),
                        crate::session::DEFAULT_LOCAL_USER_ID,
                    )
                    .await
            }
        });
        let dropped_get_or_barrier = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                barrier = &mut missing_get_seen => {
                    barrier.expect("dropped Get barrier should remain available");
                    None
                }
                task_result = &mut dropped_get => Some(task_result),
            }
        })
        .await;
        match dropped_get_or_barrier {
            Ok(None) => {}
            Ok(Some(task_result)) => {
                let task_result = task_result
                    .expect("dropped Get task must not panic before reaching the worker");
                panic!(
                    "dropped Get completed before the worker request barrier; result={task_result:?}; authenticated Start and the shared home router were already established"
                );
            }
            Err(_) => {
                dropped_get.abort();
                let task_result = dropped_get.await;
                panic!(
                    "fixture worker did not observe the dropped Get within 2s; task_result={task_result:?}; authenticated Start and the shared home router were already established"
                );
            }
        }
        tokio::time::timeout(Duration::from_secs(2), replay_start_seen)
            .await
            .expect("fixture worker should observe the replay before the Get is dropped")
            .expect("replay Start barrier should remain available");
        dropped_get.abort();
        let dropped_result = dropped_get
            .await
            .expect_err("the in-flight setup Get must be cancelled by the observer");
        assert!(
            dropped_result.is_cancelled(),
            "the dropped setup Get must not settle after its observer is gone"
        );

        // The initial Start reconciliation can race with the recovery
        // reservation made by the dropped Get. Decode the fresh response so
        // a current home-visible status is not mistaken for an error-only
        // envelope; the worker barriers and late-response checks below still
        // establish whether the worker actually acknowledged the replay.
        let fresh_get_private_key = send_client_request(
            &mut first_client,
            "setup-status-dropped-get-fresh-poll",
            &config_home.daemon_id,
            &first_home_public_key,
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest {
                    operation_id: operation_id.clone(),
                },
            ),
        )
        .await;
        let fresh_get = tokio::time::timeout(
            PUBLIC_STATUS_RESPONSE_DEADLINE,
            expect_client_response_observation(
                &mut first_client,
                "setup-status-dropped-get-fresh-poll",
                &fresh_get_private_key,
            ),
        )
        .await
        .expect("fresh public polling must remain bounded after the dropped observation");
        let fresh_worker_observation =
            tokio::time::timeout(Duration::from_secs(2), fresh_get_worker_observation)
                .await
                .expect("fixture worker must classify the fresh public Get")
                .expect("fresh worker observation barrier should remain available");
        match (fresh_get.response, fresh_get.error) {
            (
                Some(LocalDaemonResponse::ProjectEnvironmentSetupStatus { status }),
                None,
            ) => {
                // A successful status is valid only when this fresh Get was
                // answered by the worker. A home status can otherwise be a
                // stale acknowledgement of the racing initial Start.
                assert_eq!(
                    fresh_worker_observation,
                    FreshGetWorkerObservation::Status,
                    "fresh public poll returned an encrypted status without a worker status response; status={status:?}"
                );
                assert_eq!(status.operation_id, operation_id);
                assert_eq!(status.session_id, session_id);
                assert_eq!(status.agent_id, agent_id);
                assert_eq!(status.attempt, 1);
                assert_eq!(status.phase, ProjectEnvironmentSetupPhase::Requested);
                assert!(status.retryable);
            }
            (None, Some(fresh_get_error)) => {
                assert_eq!(
                    fresh_worker_observation,
                    FreshGetWorkerObservation::NotFound,
                    "fresh public poll returned a transport error although the worker reported a status: error={fresh_get_error:?}"
                );
                assert_eq!(fresh_get_error.code, "local_transport_error");
                assert!(fresh_get_error.retryable);
                assert!(
                    fresh_get_error
                        .message
                        .ends_with("same-attempt replay result is unresolved"),
                    "a dropped observation must become explicit unresolved state, not stale InFlight: {fresh_get_error:?}"
                );
            }
            (Some(response), Some(error)) => panic!(
                "fresh public poll returned both encrypted response and relay error: response={response:?}, error={error:?}"
            ),
            (None, None) => panic!("fresh public poll returned neither response nor relay error"),
            (Some(response), None) => {
                panic!("unexpected encrypted fresh setup response: {response:?}")
            }
        }
        assert_eq!(
            worker_start_count.load(Ordering::Acquire),
            2,
            "bounded unresolved polling must not duplicate the withheld Start"
        );

        // Disconnect this public client and reconnect through the same
        // authenticated relay target. This verifies that socket loss is a
        // separate concern from the internal observation cancellation above.
        let _ = first_client.close(None).await;
        let (mut recovery_client, _) = connect_async(&relay_url)
            .await
            .expect("recovery public client should connect to relay");
        send_client_envelope(
            &mut recovery_client,
            &RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config_home.daemon_id.clone()),
                    daemon_alias: None,
                },
            },
        )
        .await;
        let recovery_home_public_key = expect_client_connected(&mut recovery_client).await;

        let cancel_private_key = send_client_request(
            &mut recovery_client,
            "setup-status-dropped-get-cancel",
            &config_home.daemon_id,
            &recovery_home_public_key,
            LocalDaemonRequest::CancelProjectEnvironmentSetup(
                CancelProjectEnvironmentSetupRequest {
                    operation_id: operation_id.clone(),
                    session_id: session_id.clone(),
                },
            ),
        )
        .await;
        let cancel_response = tokio::time::timeout(
            PUBLIC_STATUS_RESPONSE_DEADLINE,
            expect_client_response(
                &mut recovery_client,
                "setup-status-dropped-get-cancel",
                &cancel_private_key,
            ),
        )
        .await
        .expect("public Cancel must recover the dropped observation");
        let cancelled_status = match cancel_response {
            LocalDaemonResponse::ProjectEnvironmentSetupCancelled { status } => status,
            other => panic!("unexpected cancellation response: {other:?}"),
        };
        assert_eq!(cancelled_status.operation_id, operation_id);
        assert_eq!(cancelled_status.attempt, 1);
        assert_eq!(
            cancelled_status.phase,
            ProjectEnvironmentSetupPhase::Cancelled
        );
        assert!(cancelled_status.retryable);

        let retry_private_key = send_client_request(
            &mut recovery_client,
            "setup-status-dropped-get-retry",
            &config_home.daemon_id,
            &recovery_home_public_key,
            LocalDaemonRequest::RetryProjectEnvironmentSetup(RetryProjectEnvironmentSetupRequest {
                operation_id: operation_id.clone(),
                session_id: session_id.clone(),
            }),
        )
        .await;
        let retry_response = tokio::time::timeout(
            PUBLIC_STATUS_RESPONSE_DEADLINE,
            expect_client_response(
                &mut recovery_client,
                "setup-status-dropped-get-retry",
                &retry_private_key,
            ),
        )
        .await
        .expect("public Retry must be accepted after Cancel");
        let retried_status = match retry_response {
            LocalDaemonResponse::ProjectEnvironmentSetupRetried { status } => status,
            other => panic!("unexpected retry response: {other:?}"),
        };
        assert_eq!(retried_status.operation_id, operation_id);
        assert_eq!(retried_status.attempt, 2);
        assert_eq!(
            retried_status.phase,
            ProjectEnvironmentSetupPhase::Requested
        );
        assert!(retried_status.retryable);
        tokio::time::timeout(Duration::from_secs(2), retry_seen)
            .await
            .expect("fixture worker should observe the public Retry")
            .expect("Retry barrier should remain available");

        // The accepted attempt-one replay resolves only after the home has
        // advanced to attempt two. Its late response must not overwrite the
        // authoritative retry state or worker attempt.
        let _ = release_replay_tx.send(());
        tokio::time::timeout(Duration::from_secs(2), release_replay_applied)
            .await
            .expect("the old replay response should be released after Retry")
            .expect("old replay release barrier should remain available");

        let final_get_private_key = send_client_request(
            &mut recovery_client,
            "setup-status-dropped-get-final-poll",
            &config_home.daemon_id,
            &recovery_home_public_key,
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest { operation_id },
            ),
        )
        .await;
        let final_get_response = tokio::time::timeout(
            PUBLIC_STATUS_RESPONSE_DEADLINE,
            expect_client_response(
                &mut recovery_client,
                "setup-status-dropped-get-final-poll",
                &final_get_private_key,
            ),
        )
        .await
        .expect("fresh polling must remain available after public Retry");
        let final_status = match final_get_response {
            LocalDaemonResponse::ProjectEnvironmentSetupStatus { status } => status,
            other => panic!("unexpected final setup response: {other:?}"),
        };
        assert_eq!(
            final_status.operation_id,
            "setup-status-concurrent-recovery"
        );
        assert_eq!(final_status.attempt, 2);
        assert_eq!(final_status.phase, ProjectEnvironmentSetupPhase::Requested);
        assert!(final_status.retryable);
        assert_eq!(
            worker_start_count.load(Ordering::Acquire),
            2,
            "Cancel and Retry must not dispatch an obsolete Start"
        );
        let _ = recovery_client.close(None).await;

        let _ = shutdown_worker_tx.send(());
        worker_task
            .await
            .expect("dropped-observation worker should stop");
        let _ = shutdown_home_tx.send(true);
        connector_home
            .await
            .expect("home relay connector should stop");
        let _ = server_shutdown_tx.send(());
        server_task
            .await
            .expect("relay server should stop after dropped-observation recovery");
        return;
    }

    let (mut second_client, _) = connect_async(&relay_url)
        .await
        .expect("second public client should connect to relay");
    send_client_envelope(
        &mut second_client,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config_home.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let second_home_public_key = expect_client_connected(&mut second_client).await;

    let _ = send_client_request(
        &mut first_client,
        "setup-status-concurrent-get-a",
        &config_home.daemon_id,
        &first_home_public_key,
        LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
            GetProjectEnvironmentSetupStatusRequest {
                operation_id: operation_id.clone(),
            },
        ),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), missing_get_seen)
        .await
        .expect("fixture worker should observe the first missing-status Get")
        .expect("missing-status barrier should remain available");
    tokio::time::timeout(Duration::from_secs(2), replay_start_seen)
        .await
        .expect("fixture worker should observe the first authenticated replay Start")
        .expect("replay Start barrier should remain available");

    let _ = send_client_request(
        &mut second_client,
        "setup-status-concurrent-get-b",
        &config_home.daemon_id,
        &second_home_public_key,
        LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
            GetProjectEnvironmentSetupStatusRequest {
                operation_id: operation_id.clone(),
            },
        ),
    )
    .await;

    let (first_response, second_response) = tokio::join!(
        tokio::time::timeout(
            PUBLIC_STATUS_RESPONSE_DEADLINE,
            expect_client_response_error(&mut first_client, "setup-status-concurrent-get-a"),
        ),
        tokio::time::timeout(
            PUBLIC_STATUS_RESPONSE_DEADLINE,
            expect_client_response_error(&mut second_client, "setup-status-concurrent-get-b"),
        ),
    );
    let _ = release_replay_tx.send(());
    let first_error =
        first_response.expect("first public poll must settle within the bounded recovery deadline");
    let second_error = second_response
        .expect("second public poll must settle within the bounded recovery deadline");
    for error in [first_error, second_error] {
        assert_eq!(
            error.code, "local_transport_error",
            "withheld redispatch is transport uncertainty, not setup failure: {error:?}"
        );
        assert!(
            error.retryable,
            "withheld redispatch must remain recoverable: {error:?}"
        );
    }
    assert_eq!(
        worker_start_count.load(Ordering::Acquire),
        2,
        "concurrent missing-status polls must share one same-attempt replay Start"
    );

    let _ = first_client.close(None).await;
    let _ = second_client.close(None).await;
    let (mut recovery_client, _) = connect_async(&relay_url)
        .await
        .expect("recovery public client should connect to relay");
    send_client_envelope(
        &mut recovery_client,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config_home.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let recovery_home_public_key = expect_client_connected(&mut recovery_client).await;
    let recovery_private_key = send_client_request(
        &mut recovery_client,
        "setup-status-concurrent-recovery-get",
        &config_home.daemon_id,
        &recovery_home_public_key,
        LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
            GetProjectEnvironmentSetupStatusRequest {
                operation_id: operation_id.clone(),
            },
        ),
    )
    .await;
    if lose_replay_response {
        tokio::time::timeout(Duration::from_secs(2), worker_status_seen)
            .await
            .expect("fixture worker should observe the later authenticated status Get")
            .expect("worker status barrier should remain available");
    }
    let recovery_response = tokio::time::timeout(
        Duration::from_secs(2),
        expect_client_response(
            &mut recovery_client,
            "setup-status-concurrent-recovery-get",
            &recovery_private_key,
        ),
    )
    .await
    .expect("released replay should make the operation publicly recoverable");
    let recovered_status = match recovery_response {
        LocalDaemonResponse::ProjectEnvironmentSetupStatus { status } => status,
        other => panic!("unexpected recovered setup response: {other:?}"),
    };
    assert_eq!(
        recovered_status.operation_id,
        "setup-status-concurrent-recovery"
    );
    assert_eq!(recovered_status.attempt, 1);
    assert_eq!(
        recovered_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert!(recovered_status.retryable);
    assert_eq!(
        worker_start_count.load(Ordering::Acquire),
        2,
        "recovery must not create another Start after the shared replay"
    );

    if reopen_after_acknowledged_worker_loss {
        let _ = lose_setup_tx.send(());
        tokio::time::timeout(Duration::from_secs(2), setup_loss_applied)
            .await
            .expect("fixture worker should observe the controlled setup loss")
            .expect("setup-loss barrier should remain available");

        let post_loss_private_key = send_client_request(
            &mut recovery_client,
            "setup-status-acknowledged-loss-get",
            &config_home.daemon_id,
            &recovery_home_public_key,
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest { operation_id },
            ),
        )
        .await;
        tokio::time::timeout(Duration::from_secs(2), post_loss_get_seen)
            .await
            .expect("fixture worker should observe the authenticated post-loss Get")
            .expect("post-loss Get barrier should remain available");
        let post_loss_response = tokio::time::timeout(
            Duration::from_secs(2),
            expect_client_response(
                &mut recovery_client,
                "setup-status-acknowledged-loss-get",
                &post_loss_private_key,
            ),
        )
        .await
        .expect("post-loss setup recovery should remain publicly bounded");
        let post_loss_status = match post_loss_response {
            LocalDaemonResponse::ProjectEnvironmentSetupStatus { status } => status,
            other => panic!("unexpected post-loss setup response: {other:?}"),
        };
        assert_eq!(
            post_loss_status.operation_id,
            "setup-status-concurrent-recovery"
        );
        assert_eq!(post_loss_status.attempt, 1);
        assert_eq!(
            post_loss_status.phase,
            ProjectEnvironmentSetupPhase::Requested
        );
        assert!(post_loss_status.retryable);

        let redispatched_attempt = tokio::time::timeout(Duration::from_secs(2), post_loss_start_seen)
            .await
            .expect(
                "an authenticated post-loss not-found must reopen recovery after an acknowledged replay",
            )
            .expect("post-loss replay Start barrier should remain available");
        assert_eq!(
            redispatched_attempt, 1,
            "post-loss recovery must preserve the authoritative operation attempt"
        );
        assert_eq!(
            worker_start_count.load(Ordering::Acquire),
            3,
            "post-loss recovery must dispatch one additional same-attempt Start"
        );
    }

    let _ = shutdown_worker_tx.send(());
    worker_task
        .await
        .expect("missing-replay worker should stop");
    let _ = shutdown_home_tx.send(true);
    connector_home
        .await
        .expect("home relay connector should stop");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("relay server should stop");
}

fn spawn_missing_then_withheld_replay_worker(
    relay_url: String,
    registration: DaemonRegistration,
    worker_private_key: String,
    home_public_key: String,
    lose_replay_response: bool,
    home_setup_scope_rx: oneshot::Receiver<(String, String, String)>,
) -> (
    oneshot::Sender<()>,
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<FreshGetWorkerObservation>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<u32>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (release_replay_tx, release_replay_rx) = oneshot::channel();
    let (release_replay_applied_tx, release_replay_applied_rx) = oneshot::channel();
    let (retry_seen_tx, retry_seen_rx) = oneshot::channel();
    let (lose_setup_tx, lose_setup_rx) = oneshot::channel();
    let (initial_start_seen_tx, initial_start_seen_rx) = oneshot::channel();
    let (missing_get_seen_tx, missing_get_seen_rx) = oneshot::channel();
    let (fresh_get_worker_observation_tx, fresh_get_worker_observation_rx) = oneshot::channel();
    let (replay_start_seen_tx, replay_start_seen_rx) = oneshot::channel();
    let (worker_status_seen_tx, worker_status_seen_rx) = oneshot::channel();
    let (setup_loss_applied_tx, setup_loss_applied_rx) = oneshot::channel();
    let (post_loss_get_seen_tx, post_loss_get_seen_rx) = oneshot::channel();
    let (post_loss_start_seen_tx, post_loss_start_seen_rx) = oneshot::channel();
    let start_count = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn(run_missing_then_withheld_replay_worker(
        relay_url,
        registration,
        worker_private_key,
        home_public_key,
        shutdown_rx,
        release_replay_rx,
        release_replay_applied_tx,
        retry_seen_tx,
        lose_setup_rx,
        lose_replay_response,
        initial_start_seen_tx,
        missing_get_seen_tx,
        fresh_get_worker_observation_tx,
        replay_start_seen_tx,
        worker_status_seen_tx,
        setup_loss_applied_tx,
        post_loss_get_seen_tx,
        post_loss_start_seen_tx,
        Arc::clone(&start_count),
        home_setup_scope_rx,
    ));
    (
        shutdown_tx,
        release_replay_tx,
        release_replay_applied_rx,
        retry_seen_rx,
        lose_setup_tx,
        initial_start_seen_rx,
        missing_get_seen_rx,
        fresh_get_worker_observation_rx,
        replay_start_seen_rx,
        worker_status_seen_rx,
        setup_loss_applied_rx,
        post_loss_get_seen_rx,
        post_loss_start_seen_rx,
        start_count,
        task,
    )
}

async fn run_missing_then_withheld_replay_worker(
    relay_url: String,
    registration: DaemonRegistration,
    worker_private_key: String,
    home_public_key: String,
    mut shutdown_rx: oneshot::Receiver<()>,
    mut release_replay_rx: oneshot::Receiver<()>,
    release_replay_applied_tx: oneshot::Sender<()>,
    retry_seen_tx: oneshot::Sender<()>,
    mut lose_setup_rx: oneshot::Receiver<()>,
    lose_replay_response: bool,
    initial_start_seen_tx: oneshot::Sender<()>,
    missing_get_seen_tx: oneshot::Sender<()>,
    fresh_get_worker_observation_tx: oneshot::Sender<FreshGetWorkerObservation>,
    replay_start_seen_tx: oneshot::Sender<()>,
    worker_status_seen_tx: oneshot::Sender<()>,
    setup_loss_applied_tx: oneshot::Sender<()>,
    post_loss_get_seen_tx: oneshot::Sender<()>,
    post_loss_start_seen_tx: oneshot::Sender<u32>,
    start_count: Arc<AtomicUsize>,
    home_setup_scope_rx: oneshot::Receiver<(String, String, String)>,
) {
    let worker_machine_id = registration.machine_id.clone();
    let worker_platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
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
    let expected_home_setup_scope = match home_setup_scope_rx.await {
        Ok(scope) => scope,
        Err(_) => return,
    };

    let mut initial_start_seen_tx = Some(initial_start_seen_tx);
    let mut missing_get_seen_tx = Some(missing_get_seen_tx);
    let mut fresh_get_worker_observation_tx = Some(fresh_get_worker_observation_tx);
    let mut replay_start_seen_tx = Some(replay_start_seen_tx);
    let mut release_replay_applied_tx = Some(release_replay_applied_tx);
    let mut retry_seen_tx = Some(retry_seen_tx);
    let mut worker_status_seen_tx = Some(worker_status_seen_tx);
    let mut setup_loss_applied_tx = Some(setup_loss_applied_tx);
    let mut post_loss_get_seen_tx = Some(post_loss_get_seen_tx);
    let mut post_loss_start_seen_tx = Some(post_loss_start_seen_tx);
    let mut missing_get_seen = false;
    let mut replay_released = false;
    let mut loss_trigger_consumed = false;
    let mut setup_lost = false;
    let mut held_setup: Option<RelayProjectEnvironmentSetupStatus> = None;
    let mut accepted_leased_agent_id: Option<String> = None;
    let mut acknowledgment_setup: Option<RelayProjectEnvironmentSetupStatus> = None;
    let mut held_replay: Option<(String, RelayProjectEnvironmentSetupStatus)> = None;
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                let _ = socket.close(None).await;
                return;
            }
            _ = &mut release_replay_rx,
                if !replay_released
                    && (held_replay.is_some()
                        || (lose_replay_response && held_setup.is_some())) =>
            {
                if let Some((relay_request_id, setup)) = held_replay.take() {
                    // If a later public Retry already advanced the worker,
                    // releasing this old response must not rewind its
                    // authoritative record.
                    if held_setup.is_none() {
                        held_setup = Some(setup.clone());
                    }
                    let response = RelayEnvelope::DaemonIncomingPeerResponse {
                        relay_request_id,
                        encrypted_response: Some(encrypt_worker_response(
                            &worker_private_key,
                            &home_public_key,
                            RelayPeerResponse::LeasedProjectEnvironmentSetupStarted {
                                setup: RelayProjectEnvironmentSetupStatus {
                                    status: setup.status,
                                    definition: setup.definition,
                                },
                            },
                        )),
                        error: None,
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
                    if let Some(release_replay_applied_tx) = release_replay_applied_tx.take() {
                        let _ = release_replay_applied_tx.send(());
                    }
                }
                replay_released = true;
            }
            _ = &mut lose_setup_rx,
                if held_setup.is_some() && !setup_lost && !loss_trigger_consumed =>
            {
                held_setup = None;
                setup_lost = true;
                loss_trigger_consumed = true;
                if let Some(setup_loss_applied_tx) = setup_loss_applied_tx.take() {
                    let _ = setup_loss_applied_tx.send(());
                }
            }
            message = socket.next() => {
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
                    RelayPeerRequest::ResolveLeasedProjectEnvironmentSetupTarget {
                        leased_agent_id,
                        home_session_id,
                        home_agent_id,
                    } => fixture_setup_target_resolution(
                        &worker_private_key,
                        &home_public_key,
                        &expected_home_setup_scope,
                        &leased_agent_id,
                        &home_session_id,
                        &home_agent_id,
                        &worker_machine_id,
                        &worker_platform,
                    ),
                    RelayPeerRequest::StartLeasedProjectEnvironmentSetup {
                        leased_agent_id,
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
                        if accepted_leased_agent_id.is_none() {
                            accepted_leased_agent_id = Some(leased_agent_id);
                        }
                        let start_number = start_count.fetch_add(1, Ordering::AcqRel) + 1;
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
                        acknowledgment_setup = Some(setup.clone());
                        if start_number == 1 {
                            if let Some(initial_start_seen_tx) = initial_start_seen_tx.take() {
                                let _ = initial_start_seen_tx.send(());
                            }
                            (
                                Some(encrypt_worker_response(
                                    &worker_private_key,
                                    &home_public_key,
                                    RelayPeerResponse::LeasedProjectEnvironmentSetupStarted {
                                        setup,
                                    },
                                )),
                                None,
                            )
                        } else if start_number == 2 {
                            if let Some(replay_start_seen_tx) = replay_start_seen_tx.take() {
                                let _ = replay_start_seen_tx.send(());
                            }
                            if lose_replay_response {
                                held_setup = Some(setup);
                            } else {
                                held_replay = Some((relay_request_id, setup));
                            }
                            continue;
                        } else if start_number == 3 && setup_lost {
                            setup_lost = false;
                            held_setup = Some(setup.clone());
                            if let Some(post_loss_start_seen_tx) = post_loss_start_seen_tx.take() {
                                let _ = post_loss_start_seen_tx.send(attempt);
                            }
                            (
                                Some(encrypt_worker_response(
                                    &worker_private_key,
                                    &home_public_key,
                                    RelayPeerResponse::LeasedProjectEnvironmentSetupStarted {
                                        setup,
                                    },
                                )),
                                None,
                            )
                        } else {
                            (
                                None,
                                Some(RelayError {
                                    code: PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE.to_string(),
                                    message: "duplicate replay Start reached fixture".to_string(),
                                    retryable: false,
                                }),
                            )
                        }
                    }
                    RelayPeerRequest::CancelLeasedProjectEnvironmentSetup { .. } => {
                        let mut setup = held_setup
                            .clone()
                            .or_else(|| held_replay.as_ref().map(|(_, setup)| setup.clone()))
                            .expect("control request should follow an accepted setup");
                        setup.status.phase = ProjectEnvironmentSetupPhase::Cancelled;
                        setup.status.message = Some("fixture worker cancelled setup".to_string());
                        setup.status.failure_code = None;
                        setup.status.failure_message = None;
                        setup.status.retryable = true;
                        setup.status.updated_at_ms = crate::session::unix_epoch_ms();
                        held_setup = Some(setup.clone());
                        (
                            Some(encrypt_worker_response(
                                &worker_private_key,
                                &home_public_key,
                                RelayPeerResponse::LeasedProjectEnvironmentSetupCancelled {
                                    setup,
                                },
                            )),
                            None,
                        )
                    }
                    RelayPeerRequest::AcknowledgeLeasedProjectEnvironmentSetupDefinition {
                        leased_agent_id,
                        operation_id,
                        attempt,
                        project_id,
                        home_session_id,
                        home_agent_id,
                        definition_digest,
                    } => fixture_setup_definition_acknowledgment(
                        &worker_private_key,
                        &home_public_key,
                        accepted_leased_agent_id.as_deref(),
                        acknowledgment_setup.as_ref(),
                        &leased_agent_id,
                        &operation_id,
                        attempt,
                        &project_id,
                        &home_session_id,
                        &home_agent_id,
                        &definition_digest,
                    ),
                    RelayPeerRequest::RetryLeasedProjectEnvironmentSetup { .. } => {
                        let mut setup = held_setup
                            .clone()
                            .or_else(|| held_replay.as_ref().map(|(_, setup)| setup.clone()))
                            .expect("retry request should follow an accepted setup");
                        setup.status.phase = ProjectEnvironmentSetupPhase::Requested;
                        setup.status.attempt = setup.status.attempt.saturating_add(1);
                        setup.status.progress_percent = 0;
                        setup.status.message = Some("fixture worker accepted retry".to_string());
                        setup.status.failure_code = None;
                        setup.status.failure_message = None;
                        setup.status.retryable = true;
                        setup.status.updated_at_ms = crate::session::unix_epoch_ms();
                        held_setup = Some(setup.clone());
                        if let Some(retry_seen_tx) = retry_seen_tx.take() {
                            let _ = retry_seen_tx.send(());
                        }
                        (
                            Some(encrypt_worker_response(
                                &worker_private_key,
                                &home_public_key,
                                RelayPeerResponse::LeasedProjectEnvironmentSetupRetried {
                                    setup,
                                },
                            )),
                            None,
                        )
                    }
                    RelayPeerRequest::GetLeasedProjectEnvironmentSetupStatus { .. }
                        if held_setup.is_some()
                            && !setup_lost
                            && (!lose_replay_response || replay_released) =>
                    {
                        if missing_get_seen {
                            if let Some(fresh_get_worker_observation_tx) =
                                fresh_get_worker_observation_tx.take()
                            {
                                let _ = fresh_get_worker_observation_tx
                                    .send(FreshGetWorkerObservation::Status);
                            }
                        }
                        if let Some(worker_status_seen_tx) = worker_status_seen_tx.take() {
                            let _ = worker_status_seen_tx.send(());
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
                    RelayPeerRequest::GetLeasedProjectEnvironmentSetupStatus { .. } => {
                        if setup_lost {
                            if let Some(post_loss_get_seen_tx) = post_loss_get_seen_tx.take() {
                                let _ = post_loss_get_seen_tx.send(());
                            }
                        } else if !missing_get_seen {
                            missing_get_seen = true;
                            if let Some(missing_get_seen_tx) = missing_get_seen_tx.take() {
                                let _ = missing_get_seen_tx.send(());
                            }
                        } else if let Some(fresh_get_worker_observation_tx) =
                            fresh_get_worker_observation_tx.take()
                        {
                            let _ = fresh_get_worker_observation_tx
                                .send(FreshGetWorkerObservation::NotFound);
                        }
                        (
                            None,
                            Some(RelayError {
                                code: PROJECT_ENVIRONMENT_SETUP_NOT_FOUND_CODE.to_string(),
                                message: "setup operation was not found".to_string(),
                                retryable: false,
                            }),
                        )
                    }
                    _ => (
                        None,
                        Some(RelayError {
                            code: PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE.to_string(),
                            message: "fixture worker rejected unsupported setup request".to_string(),
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
    }
}

fn spawn_withheld_status_worker(
    relay_url: String,
    registration: DaemonRegistration,
    worker_private_key: String,
    home_public_key: String,
    home_setup_scope_rx: oneshot::Receiver<(String, String, String)>,
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
        home_setup_scope_rx,
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
    home_setup_scope_rx: oneshot::Receiver<(String, String, String)>,
) {
    let worker_machine_id = registration.machine_id.clone();
    let worker_platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
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
    let expected_home_setup_scope = match home_setup_scope_rx.await {
        Ok(scope) => scope,
        Err(_) => return,
    };

    let mut start_seen_tx = Some(start_seen_tx);
    let mut status_seen_tx = Some(status_seen_tx);
    let mut held_setup: Option<RelayProjectEnvironmentSetupStatus> = None;
    let mut accepted_leased_agent_id: Option<String> = None;
    let mut acknowledgment_setup: Option<RelayProjectEnvironmentSetupStatus> = None;
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
            RelayPeerRequest::ResolveLeasedProjectEnvironmentSetupTarget {
                leased_agent_id,
                home_session_id,
                home_agent_id,
            } => fixture_setup_target_resolution(
                &worker_private_key,
                &home_public_key,
                &expected_home_setup_scope,
                &leased_agent_id,
                &home_session_id,
                &home_agent_id,
                &worker_machine_id,
                &worker_platform,
            ),
            RelayPeerRequest::StartLeasedProjectEnvironmentSetup {
                leased_agent_id,
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
                if accepted_leased_agent_id.is_none() {
                    accepted_leased_agent_id = Some(leased_agent_id);
                }
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
                acknowledgment_setup = Some(setup.clone());
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
            RelayPeerRequest::AcknowledgeLeasedProjectEnvironmentSetupDefinition {
                leased_agent_id,
                operation_id,
                attempt,
                project_id,
                home_session_id,
                home_agent_id,
                definition_digest,
            } => fixture_setup_definition_acknowledgment(
                &worker_private_key,
                &home_public_key,
                accepted_leased_agent_id.as_deref(),
                acknowledgment_setup.as_ref(),
                &leased_agent_id,
                &operation_id,
                attempt,
                &project_id,
                &home_session_id,
                &home_agent_id,
                &definition_digest,
            ),
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

fn fixture_setup_target_resolution(
    worker_private_key: &str,
    home_public_key: &str,
    expected_home_scope: &(String, String, String),
    leased_agent_id: &str,
    home_session_id: &str,
    home_agent_id: &str,
    worker_machine_id: &str,
    worker_platform: &str,
) -> (
    Option<chariox_relay::protocol::EncryptedRelayPayload>,
    Option<RelayError>,
) {
    if expected_home_scope.0.as_str() != leased_agent_id
        || expected_home_scope.1.as_str() != home_session_id
        || expected_home_scope.2.as_str() != home_agent_id
        || worker_machine_id.is_empty()
        || worker_platform.is_empty()
    {
        return (
            None,
            Some(RelayError {
                code: PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE.to_string(),
                message: "fixture worker rejected a mismatched setup target binding".to_string(),
                retryable: false,
            }),
        );
    }
    (
        Some(encrypt_worker_response(
            worker_private_key,
            home_public_key,
            RelayPeerResponse::LeasedProjectEnvironmentSetupTargetResolved {
                worker_id: worker_machine_id.to_string(),
                platform: worker_platform.to_string(),
            },
        )),
        None,
    )
}

fn fixture_setup_definition_acknowledgment(
    worker_private_key: &str,
    home_public_key: &str,
    accepted_leased_agent_id: Option<&str>,
    setup: Option<&RelayProjectEnvironmentSetupStatus>,
    leased_agent_id: &str,
    operation_id: &str,
    attempt: u32,
    project_id: &str,
    home_session_id: &str,
    home_agent_id: &str,
    definition_digest: &str,
) -> (
    Option<chariox_relay::protocol::EncryptedRelayPayload>,
    Option<RelayError>,
) {
    let matches_setup = accepted_leased_agent_id == Some(leased_agent_id)
        && setup.is_some_and(|setup| {
            let status = &setup.status;
            status.operation_id == operation_id
                && status.attempt == attempt
                && status.project_id == project_id
                && status.session_id == home_session_id
                && status.agent_id == home_agent_id
                && !matches!(
                    status.phase,
                    ProjectEnvironmentSetupPhase::Ready
                        | ProjectEnvironmentSetupPhase::Failed
                        | ProjectEnvironmentSetupPhase::Cancelled
                )
                && status.definition_digest.as_deref() == Some(definition_digest)
                && setup.definition.as_ref().is_some_and(|definition| {
                    definition.origin == ProjectEnvironmentDefinitionOrigin::UtilityGenerated
                        && definition.digest() == definition_digest
                })
        });
    if !matches_setup {
        return (
            None,
            Some(RelayError {
                code: PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE.to_string(),
                message: "fixture worker rejected a mismatched setup definition acknowledgment"
                    .to_string(),
                retryable: false,
            }),
        );
    }
    let response = RelayPeerResponse::LeasedProjectEnvironmentSetupDefinitionAcknowledged {
        operation_id: operation_id.to_string(),
        attempt,
        project_id: project_id.to_string(),
        definition_digest: definition_digest.to_string(),
    };
    (
        Some(encrypt_worker_response(
            worker_private_key,
            home_public_key,
            response,
        )),
        None,
    )
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
