//! Test-only public project-environment setup lifecycle harness.
//!
//! The provider is a simulated local OpenCode HTTP endpoint and the confirmed
//! worker receipt is fabricated. The validation step still executes an actual
//! `/bin/sh` command, but this is not real VM provisioning or project-toolchain
//! installation acceptance.

#[cfg(unix)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use chariox_relay::{
    RelayAction, RelayAuthVerifier, RelayConfig, RelayServer, RelaySubjectKind, RelayTokenClaims,
    ScopedTokenVerifier,
};
#[cfg(unix)]
use tokio::sync::{oneshot, watch};

use super::*;

#[cfg(unix)]
use crate::config::{DaemonConfig, KernelRuntimeRole};
#[cfg(unix)]
use crate::local::{
    CancelProjectEnvironmentSetupRequest, GetProjectEnvironmentSetupStatusRequest,
    LocalDaemonRequest, LocalDaemonResponse, ProjectEnvironmentCommandResult,
    ProjectEnvironmentDefinition, ProjectEnvironmentDefinitionOrigin,
    ProjectEnvironmentDefinitionSource, ProjectEnvironmentSetupPhase, ProjectEnvironmentSetupStep,
    ProjectEnvironmentSetupStepKind, ProjectEnvironmentValidation,
    RetryProjectEnvironmentSetupRequest, StartProjectEnvironmentSetupRequest,
};
#[cfg(unix)]
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
#[cfg(unix)]
use crate::runtime::router::CommandRouter;
#[cfg(unix)]
use crate::session::CreateSessionRequest;

#[cfg(unix)]
const MAX_PROVIDER_FIXTURE_TRACE_ENTRIES: usize = 64;

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_validates_supplied_definition_through_worker_boundary() {
    exercise_public_setup_lifecycle(DefinitionScenario::Supplied).await;
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_generates_definition_through_worker_boundary() {
    exercise_public_setup_lifecycle(DefinitionScenario::Generated).await;
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum DefinitionScenario {
    Supplied,
    Generated,
}

#[cfg(unix)]
async fn exercise_public_setup_lifecycle(scenario: DefinitionScenario) {
    let _environment_lock = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-project-environment-lifecycle-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let home = root.join("kernel-home");
    let workspace = root.join("worker-worktree");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let receipt = root.join("receipt.json");
    std::fs::write(
        &receipt,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "status": "confirmed",
            "allocationId": "worker-1",
            "machineId": "worker-machine",
            "kernelId": "worker-kernel",
            "relayPublicKey": "worker-public-key",
            "runtimeReleaseDigest": format!("sha256:{}", "a".repeat(64)),
            "homeCaller": {
                "accountId": "account-1",
                "userId": "user-1",
                "realmId": "realm-1",
                "machineId": "home-machine",
                "kernelId": "home-kernel",
                "relayPublicKey": "home-public-key"
            },
            "confirmedAt": "2026-09-14T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();

    struct Cleanup {
        root: PathBuf,
        home: Option<std::ffi::OsString>,
        receipt: Option<std::ffi::OsString>,
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            for (key, value) in [
                ("CHARIOX_HOME", &self.home),
                ("CHARIOX_DISPOSABLE_WORKER_RECEIPT", &self.receipt),
            ] {
                if let Some(value) = value {
                    std::env::set_var(key, value);
                } else {
                    std::env::remove_var(key);
                }
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    let _cleanup = Cleanup {
        root: root.clone(),
        home: std::env::var_os("CHARIOX_HOME"),
        receipt: std::env::var_os("CHARIOX_DISPOSABLE_WORKER_RECEIPT"),
    };
    std::env::set_var("CHARIOX_HOME", &home);
    std::env::set_var("CHARIOX_DISPOSABLE_WORKER_RECEIPT", &receipt);

    let command = "touch validation-started; sleep 2; command -v sh".to_string();
    let target_platform = actual_worker_platform();
    let definition = ProjectEnvironmentDefinition {
        schema_version: 1,
        origin: match scenario {
            DefinitionScenario::Supplied => ProjectEnvironmentDefinitionOrigin::UserAuthored,
            DefinitionScenario::Generated => ProjectEnvironmentDefinitionOrigin::UtilityGenerated,
        },
        source: ProjectEnvironmentDefinitionSource::Commands,
        target_platform: target_platform.clone(),
        source_path: None,
        setup_steps: vec![ProjectEnvironmentSetupStep {
            kind: ProjectEnvironmentSetupStepKind::Command,
            command: "command -v sh".to_string(),
        }],
        validation_commands: vec![command.clone()],
    };
    let provider_fixture = UtilityProviderFixture::start(definition.clone());

    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "worker-kernel".into();
    config.host_machine_id = "worker-machine".into();
    config.relay_public_key = "worker-public-key".into();
    config.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
    config.accept_remote_leases = true;
    config.remote_lease_capacity = Some(1);
    config.lease_worker_home_caller = Some(crate::config::LeaseWorkerHomeCaller {
        kernel_id: "home-kernel".into(),
        realm_id: "realm-1".into(),
        user_id: "user-1".into(),
        relay_public_key: "home-public-key".into(),
    });
    config.cloud_relay = Some(
        serde_json::from_value(serde_json::json!({
            "api_url": "https://staging.chariox.com",
            "email": "owner@example.test",
            "account_id": "account-1",
            "user_id": "user-1",
            "account_slug": "account-1",
            "realm_id": "realm-1",
            "relay_url": "wss://relay.example.test",
            "issuer_id": "issuer-1",
            "machine_id": "worker-machine",
            "machine_credential": format!("mcred_{}", "c".repeat(40))
        }))
        .unwrap(),
    );
    ensure_worker_validation_boundary(&config).expect("fixture is a confirmed worker");

    let mut app = crate::DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = app
        .create_session(
            CreateSessionRequest::new(
                workspace.display().to_string(),
                workspace.display().to_string(),
            )
            .with_owner_user_id("user-1")
            .with_agent_defaults(crate::session::SessionAgentDefaults::new("opencode")),
        )
        .expect("fresh worker session should be created");
    let project_id = session.project_id().to_string();
    let launch_request = LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "opencode/test-model",
    )
    .with_agent_id(agent.id())
    .with_owner_user_id("user-1");
    let mut provider_run = RuntimeProviderRun::new(
        "utility-provider-run",
        &launch_request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode-project-environment-fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            pty_env_remove: Vec::new(),
            working_directory: Some(workspace.clone()),
            structured_endpoint: Some(provider_fixture.address()),
        },
    );
    provider_run.mark_running();
    app.providers_mut().insert_run_for_test(provider_run);

    let runtime =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 1)
            .runtime_state();
    let start = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
                operation_id: "setup-lifecycle".into(),
                project_id: project_id.clone(),
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                target_worker_id: "worker-machine".into(),
                target_platform: target_platform.clone(),
                definition: match scenario {
                    DefinitionScenario::Supplied => Some(definition.clone()),
                    DefinitionScenario::Generated => None,
                },
                validation_commands: match scenario {
                    DefinitionScenario::Supplied => Vec::new(),
                    DefinitionScenario::Generated => vec![command.clone()],
                },
            }),
            "user-1",
        )
        .await
        .expect("public setup start should be accepted");
    let LocalDaemonResponse::ProjectEnvironmentSetupStarted { status } = start else {
        panic!("unexpected public setup start response: {start:?}");
    };
    assert_eq!(status.phase, ProjectEnvironmentSetupPhase::Requested);
    assert_eq!(status.attempt, 1);

    let validation_marker = workspace.join("validation-started");
    let validation_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = get_setup_status(&runtime, "setup-lifecycle", "user-1").await;
        let status = response_status(response);
        if status.phase == ProjectEnvironmentSetupPhase::Validating && validation_marker.exists() {
            break;
        }
        assert!(
            !matches!(
                status.phase,
                ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Cancelled
            ),
            "setup failed before cancellation: {status:?}; provider trace: {}",
            provider_fixture.diagnostics()
        );
        assert!(
            Instant::now() < validation_deadline,
            "public Get never observed worker tooling validation: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let cancelled = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::CancelProjectEnvironmentSetup(
                CancelProjectEnvironmentSetupRequest {
                    operation_id: "setup-lifecycle".into(),
                    session_id: session.id().to_string(),
                },
            ),
            "user-1",
        )
        .await
        .expect("public setup cancellation should settle on the worker");
    let cancelled_status = response_status(cancelled);
    assert_eq!(
        cancelled_status.phase,
        ProjectEnvironmentSetupPhase::Cancelled
    );
    assert!(cancelled_status.retryable);

    let retried = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::RetryProjectEnvironmentSetup(RetryProjectEnvironmentSetupRequest {
                operation_id: "setup-lifecycle".into(),
                session_id: session.id().to_string(),
            }),
            "user-1",
        )
        .await
        .expect("public setup retry should be accepted after worker cancellation");
    let retried_status = response_status(retried);
    assert_eq!(
        retried_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert_eq!(retried_status.attempt, 2);

    let ready_deadline = Instant::now() + Duration::from_secs(10);
    let ready = loop {
        let status = response_status(get_setup_status(&runtime, "setup-lifecycle", "user-1").await);
        match status.phase {
            ProjectEnvironmentSetupPhase::Ready => break status,
            ProjectEnvironmentSetupPhase::Failed => {
                panic!("retried setup failed: {status:?}")
            }
            _ => {
                assert!(
                    Instant::now() < ready_deadline,
                    "setup did not reach Ready: {status:?}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };
    let validation = ready
        .validation
        .as_ref()
        .expect("Ready requires measured worker validation");
    assert_eq!(validation.worker_id, "worker-machine");
    assert_eq!(validation.platform, target_platform);
    assert_eq!(validation.commands.len(), 1);
    assert_eq!(validation.commands[0].exit_code, 0);
    assert!(
        validation_marker.exists(),
        "worker tooling command did not run"
    );

    let persisted = runtime
        .owned
        .session_store
        .get_project(&project_id)
        .expect("project should remain available")
        .environment_definition()
        .cloned()
        .expect("Ready setup should persist the measured definition");
    assert_eq!(
        persisted.origin,
        match scenario {
            DefinitionScenario::Supplied => ProjectEnvironmentDefinitionOrigin::UserAuthored,
            DefinitionScenario::Generated => ProjectEnvironmentDefinitionOrigin::UtilityGenerated,
        }
    );
    drop(provider_fixture);
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_status_transport_recovery_preserves_operation_until_worker_ready() {
    let _environment_lock = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-project-environment-transport-recovery-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let worker_home = root.join("worker-home");
    let workspace = root.join("worker-worktree");
    let receipt = root.join("receipt.json");
    std::fs::create_dir_all(&worker_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();

    struct Cleanup {
        root: PathBuf,
        home: Option<std::ffi::OsString>,
        receipt: Option<std::ffi::OsString>,
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            for (key, value) in [
                ("CHARIOX_HOME", &self.home),
                ("CHARIOX_DISPOSABLE_WORKER_RECEIPT", &self.receipt),
            ] {
                if let Some(value) = value {
                    std::env::set_var(key, value);
                } else {
                    std::env::remove_var(key);
                }
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    let _cleanup = Cleanup {
        root: root.clone(),
        home: std::env::var_os("CHARIOX_HOME"),
        receipt: std::env::var_os("CHARIOX_DISPOSABLE_WORKER_RECEIPT"),
    };
    std::env::set_var("CHARIOX_HOME", &worker_home);
    std::env::set_var("CHARIOX_DISPOSABLE_WORKER_RECEIPT", &receipt);

    let listener_seed = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = listener_seed
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener
        .local_addr()
        .expect("relay listener should have addr");
    let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());

    let mut config_home = DaemonConfig::for_tests();
    let home_relay_token = "setup-transport-recovery-home-token".to_string();
    config_home.daemon_id = "home-kernel-transport-recovery".to_string();
    config_home.host_machine_id = "home-machine-transport-recovery".to_string();
    config_home.relay_url = Some(relay_url.clone());
    config_home.relay_token = Some(home_relay_token);
    config_home.relay_heartbeat_ms = 50;

    let mut config_worker = DaemonConfig::for_tests();
    let worker_relay_token = "setup-transport-recovery-worker-token".to_string();
    config_worker.daemon_id = "worker-kernel-transport-recovery".to_string();
    config_worker.host_machine_id = "worker-machine-transport-recovery".to_string();
    config_worker.relay_url = Some(relay_url.clone());
    config_worker.relay_token = Some(worker_relay_token.clone());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
    config_worker.accept_remote_leases = true;
    config_worker.remote_lease_capacity = Some(1);
    config_worker.lease_worker_home_caller = Some(crate::config::LeaseWorkerHomeCaller {
        kernel_id: config_home.daemon_id.clone(),
        realm_id: "realm-transport-recovery".to_string(),
        user_id: "user-1".to_string(),
        relay_public_key: config_home.relay_public_key.clone(),
    });
    config_worker.cloud_relay = Some(
        serde_json::from_value(serde_json::json!({
            "api_url": "https://staging.chariox.com",
            "email": "owner@example.test",
            "account_id": "account-transport-recovery",
            "user_id": "user-1",
            "account_slug": "account-transport-recovery",
            "realm_id": "realm-transport-recovery",
            "relay_url": "wss://relay.example.test",
            "issuer_id": "issuer-transport-recovery",
            "machine_id": config_worker.host_machine_id,
            "machine_credential": format!("mcred_{}", "c".repeat(40))
        }))
        .unwrap(),
    );
    std::fs::write(
        &receipt,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "status": "confirmed",
            "allocationId": "allocation-transport-recovery",
            "machineId": config_worker.host_machine_id,
            "kernelId": config_worker.daemon_id,
            "relayPublicKey": config_worker.relay_public_key,
            "runtimeReleaseDigest": format!("sha256:{}", "a".repeat(64)),
            "homeCaller": {
                "accountId": "account-transport-recovery",
                "userId": "user-1",
                "realmId": "realm-transport-recovery",
                "machineId": config_home.host_machine_id,
                "kernelId": config_home.daemon_id,
                "relayPublicKey": config_home.relay_public_key
            },
            "confirmedAt": "2026-09-14T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();
    ensure_worker_validation_boundary(&config_worker).expect("fixture is a confirmed worker");

    let relay = Arc::new(RelayServer::with_auth_verifier(
        RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: None,
        },
        setup_transport_recovery_relay_auth(&config_home, &config_worker),
    ));
    let registry = relay.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let relay = Arc::clone(&relay);
        tokio::spawn(async move {
            relay
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let app_home = Arc::new(tokio::sync::Mutex::new(
        crate::DaemonApp::bootstrap(config_home.clone()).unwrap(),
    ));
    let app_worker = Arc::new(tokio::sync::Mutex::new(
        crate::DaemonApp::bootstrap(config_worker.clone()).unwrap(),
    ));
    let (session_id, agent_id, project_id) = {
        let mut app = app_home.lock().await;
        let (session, agent) = app
            .create_session(
                CreateSessionRequest::new(
                    workspace.display().to_string(),
                    workspace.display().to_string(),
                )
                .with_owner_user_id("user-1")
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
            )
            .expect("home session should be created");
        (
            session.id().to_string(),
            agent.id().to_string(),
            session.project_id().to_string(),
        )
    };

    let (lease_id, leased_agent_id, backing_session_id, backing_agent_id) = {
        let mut app = app_worker.lock().await;
        let caller = crate::app::LeaseCallerBinding {
            home_kernel_id: config_home.daemon_id.clone(),
            authenticated_machine_id: config_home.host_machine_id.clone(),
            owner_user_id: "user-1".to_string(),
            realm_id: "realm-transport-recovery".to_string(),
            public_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                &config_home.relay_public_key,
            ),
        };
        let lease = crate::app::RemoteLeaseRuntime::new(&mut app)
            .create_bound_execution_lease(
                &config_home.daemon_id,
                &session_id,
                &agent_id,
                false,
                "user-1",
                caller,
            )
            .expect("worker execution lease should be created");
        let leased_agent = crate::app::RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent_from_base_directory(
                &workspace,
                &lease.id,
                "dev-stub",
                "default",
                None,
                None,
                None,
                None,
                None,
                Some(workspace.display().to_string()),
                None,
            )
            .expect("worker leased agent should be created");
        (
            lease.id,
            leased_agent.id,
            leased_agent.backing_session_id,
            leased_agent.backing_agent_id,
        )
    };
    {
        let app = app_home.lock().await;
        app.agents()
            .bind_remote_execution(
                &agent_id,
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: config_worker.daemon_id.clone(),
                    worker_machine_id: config_worker.host_machine_id.clone(),
                    execution_lease_id: lease_id.clone(),
                    leased_agent_id: leased_agent_id.clone(),
                    active_worker_provider_run_id: None,
                    relay_url: Some(relay_url.clone()),
                    relay_token: Some("secret".to_string()),
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("home agent should bind to the worker");
    }

    let worker_router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app_worker),
        1,
    ));
    let worker_runtime = worker_router.runtime_state();
    let target_platform = actual_worker_platform();
    let validation_command = "command -v sh".to_string();
    let definition = ProjectEnvironmentDefinition {
        schema_version: 1,
        origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
        source: ProjectEnvironmentDefinitionSource::Commands,
        target_platform: target_platform.clone(),
        source_path: None,
        setup_steps: vec![ProjectEnvironmentSetupStep {
            kind: ProjectEnvironmentSetupStepKind::Command,
            command: validation_command.clone(),
        }],
        validation_commands: vec![validation_command.clone()],
    };
    let worker_execution = super::SetupExecution {
        owner_user_id: "user-1".to_string(),
        operation_id: "setup-transport-recovery".to_string(),
        project_id: project_id.clone(),
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        execution_session_id: backing_session_id,
        execution_agent_id: backing_agent_id,
        workspace_id: workspace.display().to_string(),
        target_worker_id: config_worker.host_machine_id.clone(),
        target_platform: target_platform.clone(),
        definition: Some(definition.clone()),
        validation_commands: Vec::new(),
        persist_project_definition: false,
        remote_leased_agent_id: Some(leased_agent_id.clone()),
    };
    worker_runtime
        .owned
        .project_environment_setups
        .begin(worker_execution)
        .expect("worker setup fixture should be seeded");

    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(crate::transport::relay_client::run_daemon_relay_connector(
        Arc::clone(&app_home),
        state_home,
        shutdown_home_rx,
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
            Arc::clone(&worker_router),
            state_worker,
            shutdown_worker_rx,
            relay_url.clone(),
            worker_relay_token.clone(),
        ),
    );
    for daemon_id in [&config_home.daemon_id, &config_worker.daemon_id] {
        for _ in 0..200 {
            if registry.read().await.daemon(daemon_id).is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            registry.read().await.daemon(daemon_id).is_some(),
            "daemon {daemon_id} should register with the relay"
        );
    }

    let home_router = CommandRouter::with_interactive_capacity_from_app(Arc::clone(&app_home), 1);
    let runtime = home_router.runtime_state();
    let start_request = StartProjectEnvironmentSetupRequest {
        operation_id: "setup-transport-recovery".to_string(),
        project_id,
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        target_worker_id: config_worker.host_machine_id.clone(),
        target_platform: target_platform.clone(),
        definition: Some(definition.clone()),
        validation_commands: Vec::new(),
    };
    let started = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(start_request.clone()),
            "user-1",
        )
        .await
        .expect("public remote setup start should be accepted");
    assert_eq!(
        response_status(started).phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    let initial = get_setup_status(&runtime, "setup-transport-recovery", "user-1").await;
    assert_eq!(
        response_status(initial).phase,
        ProjectEnvironmentSetupPhase::Requested,
        "the worker must be queried before it is advanced to Ready"
    );
    assert!(
        worker_runtime.owned.project_environment_setups.update(
            "setup-transport-recovery",
            1,
            |entry| {
                entry.status.phase = ProjectEnvironmentSetupPhase::Ready;
                entry.status.progress_percent = 100;
                entry.status.definition_digest = Some(definition.digest());
                entry.status.validation = Some(ProjectEnvironmentValidation {
                    worker_id: config_worker.host_machine_id.clone(),
                    platform: target_platform.clone(),
                    commands: vec![ProjectEnvironmentCommandResult {
                        command_digest: super::command_digest(&validation_command),
                        exit_code: 0,
                        stdout_bytes: 1,
                        stderr_bytes: 0,
                    }],
                });
                entry.status.message = Some("worker setup is ready".to_string());
                entry.status.retryable = false;
            }
        ),
        "worker setup should advance to its authoritative Ready status"
    );

    let _ = shutdown_worker_tx.send(true);
    connector_worker
        .await
        .expect("interrupted worker connector should stop");
    let mut worker_disconnected = false;
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon(&config_worker.daemon_id)
            .is_none()
        {
            worker_disconnected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        worker_disconnected,
        "worker transport should be interrupted"
    );

    let interrupted = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest {
                    operation_id: "setup-transport-recovery".to_string(),
                },
            ),
            "user-1",
        )
        .await;
    assert!(
        interrupted.is_err(),
        "status should observe the external worker transport interruption: {interrupted:?}"
    );
    let (_, uncertain) = runtime
        .owned
        .project_environment_setups
        .get_entry("setup-transport-recovery", "user-1")
        .expect("uncertain setup should remain owned by the caller");
    assert_ne!(
        uncertain.phase,
        ProjectEnvironmentSetupPhase::Failed,
        "a transient worker status error must not manufacture terminal failure"
    );

    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
            Arc::clone(&worker_router),
            state_worker,
            shutdown_worker_rx,
            relay_url,
            worker_relay_token,
        ),
    );
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon(&config_worker.daemon_id)
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        registry
            .read()
            .await
            .daemon(&config_worker.daemon_id)
            .is_some(),
        "worker should re-register before recovery is queried"
    );

    let replayed = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(start_request),
            "user-1",
        )
        .await
        .expect("replayed public start should remain idempotent");
    let replayed_status = response_status(replayed);
    assert_eq!(
        replayed_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert_eq!(replayed_status.attempt, 1);
    let ready = get_setup_status(&runtime, "setup-transport-recovery", "user-1").await;
    let ready = response_status(ready);
    assert_eq!(ready.phase, ProjectEnvironmentSetupPhase::Ready);
    assert_eq!(ready.attempt, 1);
    assert_eq!(
        ready
            .validation
            .as_ref()
            .expect("Ready status should retain measured validation")
            .worker_id,
        config_worker.host_machine_id
    );
    let (_, worker_status) = worker_runtime
        .owned
        .project_environment_setups
        .get_entry("setup-transport-recovery", "user-1")
        .expect("worker setup should remain available");
    assert_eq!(worker_status.phase, ProjectEnvironmentSetupPhase::Ready);
    assert_eq!(worker_status.attempt, 1);

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should stop");
    connector_worker
        .await
        .expect("restored worker connector should stop");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("relay server should stop");
}

#[cfg(unix)]
fn setup_transport_recovery_relay_auth(
    config_home: &DaemonConfig,
    config_worker: &DaemonConfig,
) -> RelayAuthVerifier {
    let allowed_actions = vec![
        RelayAction::DaemonRegister,
        RelayAction::DaemonHeartbeat,
        RelayAction::ClientMetadataRead,
        RelayAction::PacketRoute,
        RelayAction::PeerRequest,
        RelayAction::PeerEvent,
    ];
    let claims = [
        (
            "setup-transport-recovery-home-token",
            RelayTokenClaims {
                issuer: "project-environment-setup-test".to_string(),
                subject: config_home.daemon_id.clone(),
                subject_kind: RelaySubjectKind::Kernel,
                realm_id: "realm-transport-recovery".to_string(),
                allowed_actions: allowed_actions.clone(),
                allowed_targets: None,
                issued_at_ms: 1,
                expires_at_ms: u64::MAX,
                token_id: "setup-transport-recovery-home-token".to_string(),
                account_id: None,
                organization_id: None,
                user_id: Some("user-1".to_string()),
                device_id: None,
                machine_id: Some(config_home.host_machine_id.clone()),
                client_id: None,
                session_id: None,
                public_key_thumbprint: Some(
                    crate::runtime::terminal_pairings::public_key_thumbprint(
                        &config_home.relay_public_key,
                    ),
                ),
                entitlements_version: None,
            },
        ),
        (
            "setup-transport-recovery-worker-token",
            RelayTokenClaims {
                issuer: "project-environment-setup-test".to_string(),
                subject: config_worker.daemon_id.clone(),
                subject_kind: RelaySubjectKind::Kernel,
                realm_id: "realm-transport-recovery".to_string(),
                allowed_actions,
                allowed_targets: Some(vec![config_home.daemon_id.clone()]),
                issued_at_ms: 1,
                expires_at_ms: u64::MAX,
                token_id: "setup-transport-recovery-worker-token".to_string(),
                account_id: None,
                organization_id: None,
                user_id: Some("user-1".to_string()),
                device_id: None,
                machine_id: Some(config_worker.host_machine_id.clone()),
                client_id: None,
                session_id: None,
                public_key_thumbprint: Some(
                    crate::runtime::terminal_pairings::public_key_thumbprint(
                        &config_worker.relay_public_key,
                    ),
                ),
                entitlements_version: None,
            },
        ),
    ]
    .into_iter()
    .map(|(token, claims)| (token.to_string(), claims))
    .collect();
    RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(
        claims,
        std::collections::BTreeMap::new(),
        Some(10),
    ))
}

#[cfg(unix)]
async fn get_setup_status(
    runtime: &crate::runtime::state::KernelRuntimeState,
    operation_id: &str,
    caller_user_id: &str,
) -> LocalDaemonResponse {
    runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest {
                    operation_id: operation_id.to_string(),
                },
            ),
            caller_user_id,
        )
        .await
        .expect("public setup status should be available")
}

#[cfg(unix)]
fn response_status(response: LocalDaemonResponse) -> crate::local::ProjectEnvironmentSetupStatus {
    match response {
        LocalDaemonResponse::ProjectEnvironmentSetupStarted { status }
        | LocalDaemonResponse::ProjectEnvironmentSetupStatus { status }
        | LocalDaemonResponse::ProjectEnvironmentSetupCancelled { status }
        | LocalDaemonResponse::ProjectEnvironmentSetupRetried { status } => status,
        other => panic!("unexpected project setup response: {other:?}"),
    }
}

#[cfg(unix)]
struct UtilityProviderFixture {
    address: String,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<UtilityProviderState>>,
    join: Option<thread::JoinHandle<()>>,
}

#[cfg(unix)]
#[derive(Default)]
struct UtilityProviderState {
    next_session: u64,
    session_id: Option<String>,
    prompt_id: Option<String>,
    trace: Vec<String>,
}

#[cfg(unix)]
impl UtilityProviderState {
    fn record(&mut self, entry: impl Into<String>) {
        if self.trace.len() < MAX_PROVIDER_FIXTURE_TRACE_ENTRIES {
            self.trace.push(entry.into());
        }
    }
}

#[cfg(unix)]
impl UtilityProviderFixture {
    fn start(definition: ProjectEnvironmentDefinition) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("provider fixture should bind");
        listener
            .set_nonblocking(true)
            .expect("provider fixture should be nonblocking");
        let address = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(UtilityProviderState::default()));
        let stop_for_server = stop.clone();
        let state_for_server = state.clone();
        let join = thread::spawn(move || {
            while !stop_for_server.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let stop = stop_for_server.clone();
                        let state = state_for_server.clone();
                        let definition = definition.clone();
                        thread::spawn(move || {
                            serve_provider_request(stream, stop, state, definition);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            stop,
            state,
            join: Some(join),
        }
    }

    fn address(&self) -> String {
        self.address.clone()
    }

    fn diagnostics(&self) -> String {
        let state = self
            .state
            .lock()
            .expect("provider fixture state should not poison");
        if state.trace.is_empty() {
            return "<no requests observed>".to_string();
        }
        state.trace.join(" -> ")
    }
}

#[cfg(unix)]
impl Drop for UtilityProviderFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(unix)]
fn serve_provider_request(
    mut stream: TcpStream,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<UtilityProviderState>>,
    definition: ProjectEnvironmentDefinition,
) {
    let Some((request_line, body)) = read_provider_request(&mut stream) else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    state
        .lock()
        .expect("provider fixture state should not poison")
        .record(format!("{method} {path}"));
    match (method, path) {
        ("GET", "/global/health") => {
            write_json_response(&mut stream, 200, &serde_json::json!({"healthy": true}));
        }
        ("POST", "/session") => {
            let mut state = state
                .lock()
                .expect("provider fixture state should not poison");
            state.next_session += 1;
            let session_id = format!("utility-session-{}", state.next_session);
            state.session_id = Some(session_id.clone());
            state.prompt_id = None;
            write_json_response(&mut stream, 200, &serde_json::json!({"id": session_id}));
        }
        ("GET", "/event") => {
            let headers =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n";
            if stream
                .write_all(headers.as_bytes())
                .and_then(|_| stream.flush())
                .is_err()
            {
                return;
            }
            state
                .lock()
                .expect("provider fixture state should not poison")
                .record("GET /event:headers");
            let deadline = Instant::now() + Duration::from_secs(3);
            let (session_id, prompt_id) = loop {
                let state = state
                    .lock()
                    .expect("provider fixture state should not poison");
                if let (Some(session_id), Some(prompt_id)) =
                    (state.session_id.clone(), state.prompt_id.clone())
                {
                    break (session_id, prompt_id);
                }
                drop(state);
                if Instant::now() >= deadline || stop.load(Ordering::SeqCst) {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            };
            let event = serde_json::json!({
                "type": "session.status",
                "properties": {
                    "sessionID": session_id,
                "status": {"type": "idle"}
                }
            });
            let body = format!("data: {event}\n\n");
            let _ = stream.write_all(body.as_bytes());
            let _ = stream.flush();
            state
                .lock()
                .expect("provider fixture state should not poison")
                .record("GET /event:event");
            let _ = prompt_id;
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(10));
            }
        }
        ("POST", path) if path.ends_with("/prompt_async") => {
            let prompt_id = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| {
                    value
                        .get("messageID")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                });
            state
                .lock()
                .expect("provider fixture state should not poison")
                .prompt_id = prompt_id;
            write_empty_response(&mut stream, 204);
        }
        ("GET", "/session/status") => {
            write_json_response(&mut stream, 200, &serde_json::json!({}));
        }
        ("GET", path) if path.ends_with("/message") => {
            let state = state
                .lock()
                .expect("provider fixture state should not poison");
            let Some(session_id) = state.session_id.clone() else {
                write_json_response(&mut stream, 200, &serde_json::json!([]));
                return;
            };
            let Some(prompt_id) = state.prompt_id.clone() else {
                write_json_response(&mut stream, 200, &serde_json::json!([]));
                return;
            };
            let utility_output = serde_json::json!({"definition": definition});
            let response = serde_json::json!([{
                "info": {
                    "id": "utility-assistant-1",
                    "sessionID": session_id,
                    "role": "assistant",
                    "parentID": prompt_id,
                    "finish": "stop",
                    "time": {"completed": 1}
                },
                "parts": [{
                    "id": "utility-part-1",
                    "sessionID": session_id,
                    "messageID": "utility-assistant-1",
                    "type": "text",
                    "text": utility_output.to_string()
                }]
            }]);
            write_json_response(&mut stream, 200, &response);
        }
        ("POST", path) if path.ends_with("/abort") => {
            write_empty_response(&mut stream, 204);
        }
        _ => write_empty_response(&mut stream, 404),
    }
}

#[cfg(unix)]
fn read_provider_request(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 1024];
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
        if bytes.len() > 64 * 1024 {
            return None;
        }
    };
    let header_text = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let content_length = header_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then_some(value.trim())
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    while bytes.len() < body_start + content_length {
        let mut chunk = [0_u8; 1024];
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let request_line = header_text.lines().next()?.to_string();
    Some((
        request_line,
        bytes[body_start..body_start + content_length].to_vec(),
    ))
}

#[cfg(unix)]
fn write_json_response(stream: &mut TcpStream, status: u16, body: &serde_json::Value) {
    let body = body.to_string();
    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(unix)]
fn write_empty_response(stream: &mut TcpStream, status: u16) {
    let response =
        format!("HTTP/1.1 {status} OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes());
}
