//! Public project-environment setup lifecycle against a confirmed worker.

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
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

use super::*;

#[cfg(unix)]
use crate::config::{DaemonConfig, KernelRuntimeRole};
#[cfg(unix)]
use crate::local::{
    CancelProjectEnvironmentSetupRequest, CreateSessionRequest,
    GetProjectEnvironmentSetupStatusRequest, LocalDaemonRequest, LocalDaemonResponse,
    ProjectEnvironmentDefinition, ProjectEnvironmentDefinitionOrigin,
    ProjectEnvironmentDefinitionSource, ProjectEnvironmentSetupPhase, ProjectEnvironmentSetupStep,
    ProjectEnvironmentSetupStepKind, RetryProjectEnvironmentSetupRequest,
    StartProjectEnvironmentSetupRequest,
};
#[cfg(unix)]
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
#[cfg(unix)]
use crate::runtime::router::CommandRouter;

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_validates_supplied_definition_on_fresh_worker() {
    exercise_public_setup_lifecycle(DefinitionScenario::Supplied).await;
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_generates_definition_and_validates_fresh_worker_tooling() {
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
    let project_id = session
        .project_id()
        .expect("session should belong to the default project")
        .to_string();
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
            "setup failed before cancellation: {status:?}"
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
    join: Option<thread::JoinHandle<()>>,
}

#[cfg(unix)]
#[derive(Default)]
struct UtilityProviderState {
    next_session: u64,
    session_id: Option<String>,
    prompt_id: Option<String>,
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
            join: Some(join),
        }
    }

    fn address(&self) -> String {
        self.address.clone()
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
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
            );
            let _ = stream.write_all(headers.as_bytes());
            let _ = stream.write_all(body.as_bytes());
            let _ = stream.flush();
            let _ = prompt_id;
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(10));
            }
        }
        ("POST", path) if path.ends_with("/prompt_async") => {
            let prompt_id = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("messageID").and_then(serde_json::Value::as_str))
                .map(str::to_string);
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
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = header_text
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
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
