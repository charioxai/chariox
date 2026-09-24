//! Test-only public project-environment setup lifecycle harness.
//!
//! The provider is a simulated local OpenCode HTTP endpoint and the confirmed
//! worker receipt is fabricated. The validation step still executes an actual
//! `/bin/sh` command, but this is not real VM provisioning or project-toolchain
//! installation acceptance.

#[cfg(unix)]
use sha2::{Digest, Sha256};
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
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use chariox_relay::protocol::{DaemonRegistration, RelayEnvelope, RelayError};
#[cfg(unix)]
use chariox_relay::{
    RelayAction, RelayAuthVerifier, RelayConfig, RelayServer, RelaySubjectKind, RelayTokenClaims,
    ScopedTokenVerifier,
};
#[cfg(unix)]
use futures_util::{SinkExt, StreamExt};
#[cfg(unix)]
use tokio::sync::{oneshot, watch};
#[cfg(unix)]
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::*;

#[cfg(unix)]
use crate::config::{DaemonConfig, KernelRuntimeRole};
#[cfg(unix)]
use crate::local::{
    CancelProjectEnvironmentSetupRequest, GetProjectEnvironmentSetupStatusRequest,
    LocalDaemonRequest, LocalDaemonResponse, ProjectEnvironmentCommandResult,
    ProjectEnvironmentDefinition, ProjectEnvironmentDefinitionOrigin,
    ProjectEnvironmentDefinitionSource, ProjectEnvironmentInput, ProjectEnvironmentInputKind,
    ProjectEnvironmentPathBase, ProjectEnvironmentPathEntry, ProjectEnvironmentSetupPhase,
    ProjectEnvironmentSetupStatus, ProjectEnvironmentSetupStep, ProjectEnvironmentSetupStepKind,
    ProjectEnvironmentValidation, RetryProjectEnvironmentSetupRequest,
    StartProjectEnvironmentSetupRequest,
};
#[cfg(unix)]
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult,
    ProviderLifecycleFailureInjection, ProviderLifecycleFailureStage, RuntimeProviderRun,
};
#[cfg(unix)]
use crate::runtime::router::CommandRouter;
#[cfg(unix)]
use crate::session::CreateSessionRequest;
#[cfg(unix)]
use crate::transport::relay_client::TestPeerRequestObservation;
#[cfg(unix)]
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RelayProjectEnvironmentSetupStatus,
    PROJECT_ENVIRONMENT_SETUP_NOT_FOUND_CODE, PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE,
};

#[cfg(unix)]
const MAX_PROVIDER_FIXTURE_TRACE_ENTRIES: usize = 64;

#[cfg(unix)]
const SETUP_TRANSPORT_RECOVERY_REALM: &str = "realm-transport-recovery";

#[cfg(unix)]
const SETUP_TRANSPORT_RECOVERY_RELAY_REQUEST_TIMEOUT_MS: u64 = 500;

#[cfg(unix)]
struct WorkerCommandGateCleanup {
    setup_release: PathBuf,
    validation_release: PathBuf,
}

#[cfg(unix)]
impl Drop for WorkerCommandGateCleanup {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.setup_release, b"");
        let _ = std::fs::write(&self.validation_release, b"");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_validates_supplied_definition_through_worker_boundary() {
    exercise_public_setup_lifecycle(DefinitionScenario::Supplied).await;
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_runs_on_an_ordinary_local_worker_without_cloud_receipt() {
    exercise_public_setup_lifecycle_at_role(
        DefinitionScenario::Supplied,
        KernelRuntimeRole::General,
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_generates_definition_through_worker_boundary() {
    exercise_public_setup_lifecycle(DefinitionScenario::Generated).await;
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_reuses_unchanged_recipe_and_lockfile_inputs_without_utility() {
    exercise_public_setup_lifecycle(DefinitionScenario::SuppliedInputReuse).await;
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_repairs_stale_and_missing_inputs_before_ready() {
    exercise_public_setup_lifecycle(DefinitionScenario::SuppliedStaleInputs).await;
    exercise_public_setup_lifecycle(DefinitionScenario::SuppliedMissingInputs).await;
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_repairs_legacy_unattested_definition_before_ready() {
    exercise_public_setup_lifecycle(DefinitionScenario::SuppliedLegacyUnattested).await;
}

#[cfg(unix)]
#[tokio::test]
async fn public_setup_lifecycle_repairs_definition_for_followup_worker_without_utility() {
    exercise_public_setup_lifecycle(DefinitionScenario::SuppliedSetupFailure).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn public_setup_cancellation_restores_ordinary_provider_without_retry() {
    exercise_public_setup_lifecycle_with_options(DefinitionScenario::Supplied, false, None).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pr364_generated_validation_failure_restores_previous_provider_snapshot() {
    exercise_public_setup_lifecycle_with_candidate_failure(CandidateFailureMode::Validation).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pr364_generated_validation_cancellation_restores_previous_provider_snapshot() {
    exercise_public_setup_lifecycle_with_candidate_failure(CandidateFailureMode::Cancellation)
        .await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pr364_public_setup_second_restart_spawn_failure_restores_the_previous_provider_child() {
    exercise_public_setup_lifecycle_with_second_restart_failure(
        ProviderLifecycleFailureStage::Spawn,
    )
    .await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pr364_public_setup_second_restart_binding_failure_restores_the_previous_provider_child() {
    exercise_public_setup_lifecycle_with_second_restart_failure(
        ProviderLifecycleFailureStage::Bind,
    )
    .await;
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum DefinitionScenario {
    Supplied,
    SuppliedSetupFailure,
    Generated,
    SuppliedInputReuse,
    SuppliedStaleInputs,
    SuppliedMissingInputs,
    SuppliedLegacyUnattested,
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum CandidateFailureMode {
    Validation,
    Cancellation,
}

#[cfg(unix)]
async fn exercise_public_setup_lifecycle(scenario: DefinitionScenario) {
    exercise_public_setup_lifecycle_at_role(scenario, KernelRuntimeRole::RemoteLeaseWorker).await;
}

#[cfg(unix)]
async fn exercise_public_setup_lifecycle_at_role(
    scenario: DefinitionScenario,
    runtime_role: KernelRuntimeRole,
) {
    exercise_public_setup_lifecycle_with_failure_timing(
        scenario,
        true,
        None,
        false,
        None,
        runtime_role,
    )
    .await;
}

#[cfg(unix)]
async fn exercise_public_setup_lifecycle_with_options(
    scenario: DefinitionScenario,
    retry_after_cancel: bool,
    failure_stage: Option<ProviderLifecycleFailureStage>,
) {
    exercise_public_setup_lifecycle_with_failure_timing(
        scenario,
        retry_after_cancel,
        failure_stage,
        false,
        None,
        KernelRuntimeRole::RemoteLeaseWorker,
    )
    .await;
}

#[cfg(target_os = "linux")]
async fn exercise_public_setup_lifecycle_with_second_restart_failure(
    failure_stage: ProviderLifecycleFailureStage,
) {
    exercise_public_setup_lifecycle_with_failure_timing(
        DefinitionScenario::Supplied,
        true,
        Some(failure_stage),
        true,
        None,
        KernelRuntimeRole::RemoteLeaseWorker,
    )
    .await;
}

#[cfg(target_os = "linux")]
async fn exercise_public_setup_lifecycle_with_candidate_failure(failure: CandidateFailureMode) {
    exercise_public_setup_lifecycle_with_failure_timing(
        DefinitionScenario::Generated,
        false,
        None,
        false,
        Some(failure),
        KernelRuntimeRole::RemoteLeaseWorker,
    )
    .await;
}

#[cfg(unix)]
async fn exercise_public_setup_lifecycle_with_failure_timing(
    scenario: DefinitionScenario,
    retry_after_cancel: bool,
    failure_stage: Option<ProviderLifecycleFailureStage>,
    fail_on_second_restart: bool,
    candidate_failure: Option<CandidateFailureMode>,
    runtime_role: KernelRuntimeRole,
) {
    let _environment_lock = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-project-environment-lifecycle-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let home = root.join("kernel-home");
    let provider_home = root.join("provider-home");
    let workspace = root.join("worker-worktree");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&provider_home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let input_scenario = matches!(
        scenario,
        DefinitionScenario::SuppliedInputReuse
            | DefinitionScenario::SuppliedStaleInputs
            | DefinitionScenario::SuppliedMissingInputs
            | DefinitionScenario::SuppliedLegacyUnattested
    );
    let recipe_path = ".devcontainer/devcontainer.json";
    let recipe_contents: &[u8] = br#"{"image":"mcr.microsoft.com/devcontainers/base:ubuntu"}
"#;
    let lockfile_path = "Cargo.lock";
    let lockfile_contents: &[u8] = b"version = 3\n\n[[package]]\nname = \"fixture\"\n";
    let stale_recipe_contents: &[u8] = br#"{"image":"stale-target"}
"#;
    let stale_lockfile_contents: &[u8] = b"version = 3\n\n[[package]]\nname = \"stale-fixture\"\n";
    if input_scenario {
        std::fs::create_dir_all(workspace.join(".devcontainer")).unwrap();
        match scenario {
            DefinitionScenario::SuppliedMissingInputs => {}
            DefinitionScenario::SuppliedLegacyUnattested => {
                std::fs::write(workspace.join(recipe_path), recipe_contents).unwrap();
                std::fs::write(workspace.join(lockfile_path), lockfile_contents).unwrap();
            }
            DefinitionScenario::SuppliedStaleInputs => {
                std::fs::write(workspace.join(recipe_path), stale_recipe_contents).unwrap();
                std::fs::write(workspace.join(lockfile_path), stale_lockfile_contents).unwrap();
            }
            DefinitionScenario::SuppliedInputReuse => {
                std::fs::write(workspace.join(recipe_path), recipe_contents).unwrap();
                std::fs::write(workspace.join(lockfile_path), lockfile_contents).unwrap();
            }
            _ => unreachable!("non-input scenario entered input materialization"),
        }
    }
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
    if runtime_role == KernelRuntimeRole::RemoteLeaseWorker {
        std::env::set_var("CHARIOX_DISPOSABLE_WORKER_RECEIPT", &receipt);
    } else {
        std::env::remove_var("CHARIOX_DISPOSABLE_WORKER_RECEIPT");
    }

    let validation_release = workspace.join("validation-release");
    let command = match candidate_failure {
        Some(CandidateFailureMode::Validation) => {
            "touch validation-started; while [ ! -f validation-release ]; do sleep 0.01; done; false"
                .to_string()
        }
        Some(CandidateFailureMode::Cancellation) | None => {
            "touch validation-started; while [ ! -f validation-release ]; do sleep 0.01; done; command -v sh"
                .to_string()
        }
    };
    let setup_command = match scenario {
        DefinitionScenario::SuppliedSetupFailure => "false".to_string(),
        DefinitionScenario::Supplied
        | DefinitionScenario::Generated
        | DefinitionScenario::SuppliedInputReuse
        | DefinitionScenario::SuppliedStaleInputs
        | DefinitionScenario::SuppliedMissingInputs
        | DefinitionScenario::SuppliedLegacyUnattested => {
            "touch setup-started; command -v sh".to_string()
        }
    };
    let target_platform = actual_worker_platform();
    let definition = ProjectEnvironmentDefinition {
        schema_version: 1,
        origin: match scenario {
            DefinitionScenario::Supplied
            | DefinitionScenario::SuppliedSetupFailure
            | DefinitionScenario::SuppliedInputReuse
            | DefinitionScenario::SuppliedStaleInputs
            | DefinitionScenario::SuppliedMissingInputs
            | DefinitionScenario::SuppliedLegacyUnattested => {
                ProjectEnvironmentDefinitionOrigin::UserAuthored
            }
            DefinitionScenario::Generated => ProjectEnvironmentDefinitionOrigin::UtilityGenerated,
        },
        source: if input_scenario {
            ProjectEnvironmentDefinitionSource::Devcontainer
        } else {
            ProjectEnvironmentDefinitionSource::Commands
        },
        target_platform: target_platform.clone(),
        source_path: input_scenario.then(|| recipe_path.to_string()),
        inputs: if input_scenario
            && !matches!(scenario, DefinitionScenario::SuppliedLegacyUnattested)
        {
            vec![
                ProjectEnvironmentInput {
                    kind: ProjectEnvironmentInputKind::Recipe,
                    path: recipe_path.to_string(),
                    sha256: format!("sha256:{:x}", Sha256::digest(recipe_contents)),
                },
                ProjectEnvironmentInput {
                    kind: ProjectEnvironmentInputKind::Lockfile,
                    path: lockfile_path.to_string(),
                    sha256: format!("sha256:{:x}", Sha256::digest(lockfile_contents)),
                },
            ]
        } else {
            Vec::new()
        },
        path_entries: if candidate_failure.is_some() {
            vec![ProjectEnvironmentPathEntry {
                base: ProjectEnvironmentPathBase::Workspace,
                path: ".".to_string(),
            }]
        } else {
            Vec::new()
        },
        setup_steps: vec![ProjectEnvironmentSetupStep {
            kind: ProjectEnvironmentSetupStepKind::Command,
            command: setup_command,
        }],
        validation_commands: if input_scenario {
            vec![format!(
                "touch validation-started; while [ ! -f validation-release ]; do sleep 0.01; done; command -v sh; test -f {recipe_path} && test -f {lockfile_path}"
            )]
        } else {
            vec![command.clone()]
        },
    };
    let utility_definition = match scenario {
        DefinitionScenario::SuppliedSetupFailure
        | DefinitionScenario::SuppliedStaleInputs
        | DefinitionScenario::SuppliedMissingInputs
        | DefinitionScenario::SuppliedLegacyUnattested => {
            let mut repaired = definition.clone();
            repaired.setup_steps[0].command = "touch setup-repaired; command -v sh".to_string();
            if matches!(scenario, DefinitionScenario::SuppliedLegacyUnattested) {
                repaired.inputs = vec![
                    ProjectEnvironmentInput {
                        kind: ProjectEnvironmentInputKind::Recipe,
                        path: recipe_path.to_string(),
                        sha256: format!("sha256:{:x}", Sha256::digest(recipe_contents)),
                    },
                    ProjectEnvironmentInput {
                        kind: ProjectEnvironmentInputKind::Lockfile,
                        path: lockfile_path.to_string(),
                        sha256: format!("sha256:{:x}", Sha256::digest(lockfile_contents)),
                    },
                ];
            }
            repaired
        }
        DefinitionScenario::Supplied
        | DefinitionScenario::Generated
        | DefinitionScenario::SuppliedInputReuse => definition.clone(),
    };
    let provider_fixture = if matches!(
        scenario,
        DefinitionScenario::SuppliedStaleInputs
            | DefinitionScenario::SuppliedMissingInputs
            | DefinitionScenario::SuppliedLegacyUnattested
    ) {
        UtilityProviderFixture::start_with_repairs(
            utility_definition,
            Some(workspace.clone()),
            BTreeMap::from([
                (recipe_path.to_string(), recipe_contents.to_vec()),
                (lockfile_path.to_string(), lockfile_contents.to_vec()),
                ("setup-repaired".to_string(), Vec::new()),
            ]),
        )
    } else {
        UtilityProviderFixture::start(utility_definition)
    };

    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "worker-kernel".into();
    config.host_machine_id = "worker-machine".into();
    config.relay_public_key = "worker-public-key".into();
    config.kernel_runtime_role = runtime_role;
    if runtime_role == KernelRuntimeRole::RemoteLeaseWorker {
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
    }
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
            pty_program: Some("/bin/sh".into()),
            pty_args: vec!["-c".into(), "sleep 60".into()],
            pty_env: BTreeMap::from([
                ("HOME".into(), provider_home.display().to_string()),
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("GIT_SSH_COMMAND".into(), "selected-ssh".into()),
                ("SSH_AUTH_SOCK".into(), "selected-agent".into()),
            ]),
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
    let _first_restart_failure_injection = if fail_on_second_restart {
        None
    } else {
        failure_stage
            .map(|stage| ProviderLifecycleFailureInjection::install("utility-provider-run", stage))
    };
    if matches!(
        scenario,
        DefinitionScenario::Supplied
            | DefinitionScenario::SuppliedSetupFailure
            | DefinitionScenario::SuppliedInputReuse
            | DefinitionScenario::SuppliedStaleInputs
            | DefinitionScenario::SuppliedMissingInputs
    ) {
        runtime
            .update_project_environment_definition(&project_id, definition.clone(), "user-1")
            .expect("the selected project should retain its reusable definition");
    } else if matches!(scenario, DefinitionScenario::SuppliedLegacyUnattested) {
        let mut project = runtime
            .owned
            .session_store
            .get_project(&project_id)
            .expect("legacy fixture project should remain available");
        project.set_environment_definition(definition.clone());
        runtime.owned.session_store.restore_projects(vec![project]);
    }
    let start = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
                operation_id: "setup-lifecycle".into(),
                project_id: project_id.clone(),
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                target_worker_id: "worker-machine".into(),
                target_platform: target_platform.clone(),
                definition: None,
                validation_commands: match scenario {
                    DefinitionScenario::Supplied
                    | DefinitionScenario::SuppliedSetupFailure
                    | DefinitionScenario::SuppliedInputReuse
                    | DefinitionScenario::SuppliedStaleInputs
                    | DefinitionScenario::SuppliedMissingInputs
                    | DefinitionScenario::SuppliedLegacyUnattested => Vec::new(),
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
    if fail_on_second_restart {
        let failure_stage =
            failure_stage.expect("second-restart coverage requires a failure stage");
        let discovery_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = response_status(
                get_setup_status(
                    &runtime,
                    "setup-lifecycle",
                    "user-1",
                    "second-restart discovery polling",
                )
                .await,
            );
            if status.phase == ProjectEnvironmentSetupPhase::Validating
                && validation_marker.exists()
            {
                let discovery_run = runtime
                    .owned
                    .provider_store
                    .get_run("utility-provider-run")
                    .expect("discovery provider run should remain available");
                assert!(
                    discovery_run.read_only_discovery(),
                    "failure must be injected after the ordinary-to-discovery restart"
                );
                break;
            }
            assert!(
                !matches!(
                    status.phase,
                    ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Ready
                ),
                "setup did not reach the discovery validation barrier: {status:?}"
            );
            assert!(
                Instant::now() < discovery_deadline,
                "setup did not reach the discovery validation barrier: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let _second_restart_failure_injection =
            ProviderLifecycleFailureInjection::install("utility-provider-run", failure_stage);
        std::fs::write(&validation_release, b"").unwrap();

        let failure_deadline = Instant::now() + Duration::from_secs(10);
        let failed_status = loop {
            let status = response_status(
                get_setup_status(
                    &runtime,
                    "setup-lifecycle",
                    "user-1",
                    "second-restart failure polling",
                )
                .await,
            );
            if status.phase == ProjectEnvironmentSetupPhase::Failed {
                break status;
            }
            assert_ne!(
                status.phase,
                ProjectEnvironmentSetupPhase::Ready,
                "injected second provider restart failure must fail setup, not report Ready"
            );
            assert!(
                Instant::now() < failure_deadline,
                "injected second provider restart failure did not settle: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(
            failed_status.failure_code.as_deref(),
            Some("provider_environment_bind_failed")
        );
        let (provider_run, _provider_pid, provider_environment) =
            wait_for_ordinary_provider_child(&runtime, "utility-provider-run").await;
        assert_eq!(
            provider_run.state(),
            crate::provider::ProviderRunState::Running
        );
        assert!(
            !provider_run.read_only_discovery(),
            "second-restart recovery must restore the ordinary provider snapshot"
        );
        assert_eq!(
            provider_environment.get("HOME"),
            provider_run.pty_env().get("HOME")
        );
        assert_eq!(
            provider_environment.get("PATH"),
            provider_run.pty_env().get("PATH")
        );
        assert_eq!(
            provider_environment
                .get("GIT_SSH_COMMAND")
                .map(String::as_str),
            Some("selected-ssh")
        );
        assert_eq!(
            provider_environment
                .get("SSH_AUTH_SOCK")
                .map(String::as_str),
            Some("selected-agent")
        );
        drop(provider_fixture);
        return;
    }
    if failure_stage.is_some() {
        let failure_deadline = Instant::now() + Duration::from_secs(10);
        let failed_status = loop {
            let status = response_status(
                get_setup_status(&runtime, "setup-lifecycle", "user-1", "failure polling").await,
            );
            if status.phase == ProjectEnvironmentSetupPhase::Failed {
                break status;
            }
            assert_ne!(
                status.phase,
                ProjectEnvironmentSetupPhase::Ready,
                "injected provider restart failure must fail setup, not report Ready"
            );
            assert!(
                Instant::now() < failure_deadline,
                "injected provider restart failure did not settle: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(
            failed_status.failure_code.as_deref(),
            Some("provider_environment_bind_failed")
        );
        let (provider_run, _provider_pid, provider_environment) =
            wait_for_ordinary_provider_child(&runtime, "utility-provider-run").await;
        assert_eq!(
            provider_run.state(),
            crate::provider::ProviderRunState::Running
        );
        assert!(!provider_run.read_only_discovery());
        assert_eq!(
            provider_environment.get("HOME"),
            provider_run.pty_env().get("HOME")
        );
        assert_eq!(
            provider_environment.get("PATH"),
            provider_run.pty_env().get("PATH")
        );
        assert_eq!(
            provider_environment
                .get("GIT_SSH_COMMAND")
                .map(String::as_str),
            Some("selected-ssh")
        );
        assert_eq!(
            provider_environment
                .get("SSH_AUTH_SOCK")
                .map(String::as_str),
            Some("selected-agent")
        );
        drop(provider_fixture);
        return;
    }
    if let Some(failure) = candidate_failure {
        let validation_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = response_status(
                get_setup_status(
                    &runtime,
                    "setup-lifecycle",
                    "user-1",
                    "candidate validation polling",
                )
                .await,
            );
            if status.phase == ProjectEnvironmentSetupPhase::Validating
                && validation_marker.exists()
            {
                break;
            }
            assert!(
                !matches!(
                    status.phase,
                    ProjectEnvironmentSetupPhase::Failed
                        | ProjectEnvironmentSetupPhase::Cancelled
                        | ProjectEnvironmentSetupPhase::Ready
                ),
                "candidate setup did not reach the validation barrier: {status:?}"
            );
            assert!(
                Instant::now() < validation_deadline,
                "candidate setup did not reach the validation barrier: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let failure_code = match failure {
            CandidateFailureMode::Validation => {
                std::fs::write(&validation_release, b"").unwrap();
                let failure_deadline = Instant::now() + Duration::from_secs(10);
                let failed_status = loop {
                    let status = response_status(
                        get_setup_status(
                            &runtime,
                            "setup-lifecycle",
                            "user-1",
                            "candidate validation failure polling",
                        )
                        .await,
                    );
                    if status.phase == ProjectEnvironmentSetupPhase::Failed {
                        break status;
                    }
                    assert_ne!(
                        status.phase,
                        ProjectEnvironmentSetupPhase::Ready,
                        "failed candidate validation must not report Ready"
                    );
                    assert!(
                        Instant::now() < failure_deadline,
                        "failed candidate validation did not settle: {status:?}"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                };
                failed_status.failure_code
            }
            CandidateFailureMode::Cancellation => {
                let cancelled = runtime
                    .execute_project_environment_setup_request(
                        LocalDaemonRequest::CancelProjectEnvironmentSetup(
                            CancelProjectEnvironmentSetupRequest {
                                operation_id: "setup-lifecycle".to_string(),
                                session_id: session.id().to_string(),
                            },
                        ),
                        "user-1",
                    )
                    .await
                    .expect("public candidate setup cancellation should settle");
                let cancelled_status = response_status(cancelled);
                assert_eq!(
                    cancelled_status.phase,
                    ProjectEnvironmentSetupPhase::Cancelled
                );
                std::fs::write(&validation_release, b"").unwrap();
                cancelled_status.failure_code
            }
        };
        let (provider_run, _provider_pid, provider_environment) =
            wait_for_ordinary_provider_child(&runtime, "utility-provider-run").await;
        assert_eq!(
            provider_run.state(),
            crate::provider::ProviderRunState::Running
        );
        assert!(
            !provider_run.read_only_discovery(),
            "candidate failure must restore the ordinary provider snapshot"
        );
        let expected_home = provider_home.display().to_string();
        assert_eq!(
            provider_run.pty_env().get("HOME").map(String::as_str),
            Some(expected_home.as_str())
        );
        assert_eq!(
            provider_run.pty_env().get("PATH").map(String::as_str),
            Some("/usr/bin:/bin")
        );
        assert_eq!(
            provider_environment.get("HOME").map(String::as_str),
            Some(expected_home.as_str())
        );
        assert_eq!(
            provider_environment.get("PATH").map(String::as_str),
            Some("/usr/bin:/bin")
        );
        assert_eq!(
            provider_environment
                .get("GIT_SSH_COMMAND")
                .map(String::as_str),
            Some("selected-ssh")
        );
        assert_eq!(
            provider_environment
                .get("SSH_AUTH_SOCK")
                .map(String::as_str),
            Some("selected-agent")
        );
        match failure {
            CandidateFailureMode::Validation => {
                assert_eq!(failure_code.as_deref(), Some("validation_failed"));
            }
            CandidateFailureMode::Cancellation => {
                assert!(failure_code.is_none());
            }
        }
        drop(provider_fixture);
        return;
    }
    let validation_deadline = Instant::now() + Duration::from_secs(10);
    let initial_status = loop {
        let response =
            get_setup_status(&runtime, "setup-lifecycle", "user-1", "validation polling").await;
        let status = response_status(response);
        if status.phase == ProjectEnvironmentSetupPhase::Validating && validation_marker.exists() {
            break status;
        }
        if status.phase == ProjectEnvironmentSetupPhase::Ready {
            assert!(
                validation_marker.exists(),
                "Ready must include the completed worker validation marker"
            );
            break status;
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
    };

    let ready = if matches!(
        scenario,
        DefinitionScenario::SuppliedStaleInputs
            | DefinitionScenario::SuppliedMissingInputs
            | DefinitionScenario::SuppliedLegacyUnattested
    ) {
        if initial_status.phase == ProjectEnvironmentSetupPhase::Validating {
            std::fs::write(&validation_release, b"").unwrap();
            wait_for_ready(&runtime, "setup-lifecycle", &provider_fixture).await
        } else {
            initial_status
        }
    } else {
        assert_eq!(
            initial_status.phase,
            ProjectEnvironmentSetupPhase::Validating,
            "cancellation coverage requires the worker validation barrier"
        );
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

        if !retry_after_cancel {
            let (provider_run, _provider_pid, provider_environment) =
                wait_for_ordinary_provider_child(&runtime, "utility-provider-run").await;
            assert_eq!(
                provider_run.state(),
                crate::provider::ProviderRunState::Running
            );
            assert!(!provider_run.read_only_discovery());
            assert_eq!(
                provider_environment.get("HOME"),
                provider_run.pty_env().get("HOME")
            );
            assert_eq!(
                provider_environment.get("PATH"),
                provider_run.pty_env().get("PATH")
            );
            assert!(
                runtime
                    .owned
                    .provider_store
                    .structured_runtime_state_bound_for_tests(provider_run.id()),
                "cancellation without retry must retain a bound provider runtime"
            );
            assert_eq!(
                provider_environment
                    .get("GIT_SSH_COMMAND")
                    .map(String::as_str),
                Some("selected-ssh")
            );
            assert_eq!(
                provider_environment
                    .get("SSH_AUTH_SOCK")
                    .map(String::as_str),
                Some("selected-agent")
            );
            drop(provider_fixture);
            return;
        }

        std::fs::write(&validation_release, b"").unwrap();

        let retried = runtime
            .execute_project_environment_setup_request(
                LocalDaemonRequest::RetryProjectEnvironmentSetup(
                    RetryProjectEnvironmentSetupRequest {
                        operation_id: "setup-lifecycle".into(),
                        session_id: session.id().to_string(),
                    },
                ),
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

        wait_for_ready(&runtime, "setup-lifecycle", &provider_fixture).await
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

    match scenario {
        DefinitionScenario::Supplied | DefinitionScenario::SuppliedInputReuse => {
            assert!(
                workspace.join("setup-started").exists(),
                "a reusable definition must be applied by the worker before validation"
            );
            assert_eq!(
                provider_fixture.diagnostics(),
                "<no requests observed>",
                "a passing reusable definition must not invoke the utility agent"
            );
        }
        DefinitionScenario::SuppliedSetupFailure
        | DefinitionScenario::Generated
        | DefinitionScenario::SuppliedStaleInputs
        | DefinitionScenario::SuppliedMissingInputs
        | DefinitionScenario::SuppliedLegacyUnattested => {
            assert_ne!(
                provider_fixture.diagnostics(),
                "<no requests observed>",
                "missing or failed setup must invoke the existing utility agent"
            );
        }
    }

    let persisted = runtime
        .owned
        .session_store
        .get_project(&project_id)
        .expect("project should remain available")
        .environment_definition()
        .cloned()
        .expect("Ready setup should persist the measured definition");
    if matches!(
        scenario,
        DefinitionScenario::SuppliedSetupFailure
            | DefinitionScenario::SuppliedStaleInputs
            | DefinitionScenario::SuppliedMissingInputs
            | DefinitionScenario::SuppliedLegacyUnattested
    ) {
        assert_eq!(
            persisted.setup_steps[0].command, "touch setup-repaired; command -v sh",
            "a successful repair must persist the repeatable definition returned by the utility"
        );

        if input_scenario {
            assert_eq!(
                std::fs::read(workspace.join(recipe_path)).unwrap(),
                recipe_contents,
                "repair must materialize the attested recipe bytes on the target worker"
            );
            assert_eq!(
                std::fs::read(workspace.join(lockfile_path)).unwrap(),
                lockfile_contents,
                "repair must materialize the attested lockfile bytes on the target worker"
            );
            assert_eq!(persisted.inputs.len(), 2);
            assert_eq!(
                persisted.inputs[0].sha256,
                format!("sha256:{:x}", Sha256::digest(recipe_contents))
            );
            assert_eq!(
                persisted.inputs[1].sha256,
                format!("sha256:{:x}", Sha256::digest(lockfile_contents))
            );
        }
        if matches!(
            scenario,
            DefinitionScenario::SuppliedStaleInputs
                | DefinitionScenario::SuppliedMissingInputs
                | DefinitionScenario::SuppliedLegacyUnattested
        ) {
            assert!(
                workspace.join("setup-repaired").exists(),
                "utility repair must execute the repeatable setup steps before validation"
            );
        }

        let utility_trace = provider_fixture.diagnostics();
        let materialized_inputs: Vec<(&str, &[u8])> = if input_scenario {
            vec![
                (recipe_path, recipe_contents),
                (lockfile_path, lockfile_contents),
            ]
        } else {
            Vec::new()
        };
        let (second_ready, second_workspace) = run_repaired_definition_on_fresh_worker(
            &root,
            persisted.clone(),
            target_platform.clone(),
            &provider_fixture,
            &materialized_inputs,
        )
        .await;
        assert_eq!(
            second_ready
                .validation
                .as_ref()
                .expect("follow-up Ready requires worker validation")
                .commands[0]
                .exit_code,
            0
        );
        assert_ne!(
            second_ready
                .validation
                .as_ref()
                .expect("fresh-worker Ready requires worker validation")
                .worker_id,
            ready
                .validation
                .as_ref()
                .expect("initial Ready requires worker validation")
                .worker_id,
            "repaired recipe coverage must cross a fresh worker identity"
        );
        assert!(
            second_workspace.join("setup-repaired").exists(),
            "the follow-up worker must apply the repaired reusable definition"
        );
        if input_scenario {
            assert_eq!(
                std::fs::read(second_workspace.join(recipe_path)).unwrap(),
                recipe_contents,
                "fresh worker must validate the transferred recipe bytes"
            );
            assert_eq!(
                std::fs::read(second_workspace.join(lockfile_path)).unwrap(),
                lockfile_contents,
                "fresh worker must validate the transferred lockfile bytes"
            );
        }
        assert_eq!(
            provider_fixture.diagnostics(),
            utility_trace,
            "the follow-up worker must reuse the repaired definition without utility"
        );
    }
    assert_eq!(
        persisted.origin,
        match scenario {
            DefinitionScenario::Supplied
            | DefinitionScenario::SuppliedSetupFailure
            | DefinitionScenario::SuppliedInputReuse
            | DefinitionScenario::SuppliedStaleInputs
            | DefinitionScenario::SuppliedMissingInputs
            | DefinitionScenario::SuppliedLegacyUnattested => {
                ProjectEnvironmentDefinitionOrigin::UserAuthored
            }
            DefinitionScenario::Generated => ProjectEnvironmentDefinitionOrigin::UtilityGenerated,
        }
    );
    drop(provider_fixture);
}

#[cfg(unix)]
async fn wait_for_ready(
    runtime: &crate::runtime::state::KernelRuntimeState,
    operation_id: &str,
    provider_fixture: &UtilityProviderFixture,
) -> ProjectEnvironmentSetupStatus {
    let ready_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = response_status(
            get_setup_status(runtime, operation_id, "user-1", "ready polling").await,
        );
        match status.phase {
            ProjectEnvironmentSetupPhase::Ready => return status,
            ProjectEnvironmentSetupPhase::Failed => {
                panic!(
                    "setup failed before Ready: {status:?}; provider trace: {}",
                    provider_fixture.diagnostics()
                )
            }
            _ => {
                assert!(
                    Instant::now() < ready_deadline,
                    "setup did not reach Ready: {status:?}; provider trace: {}",
                    provider_fixture.diagnostics()
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
}

#[cfg(unix)]
async fn wait_for_ordinary_provider_child(
    runtime: &crate::runtime::state::KernelRuntimeState,
    provider_run_id: &str,
) -> (RuntimeProviderRun, u32, BTreeMap<String, String>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let provider_run = runtime
            .owned
            .provider_store
            .get_run(provider_run_id)
            .expect("provider run should remain available while setup settles");
        let process_id = {
            let app = runtime.app.lock().await;
            app.pty().process_id(provider_run_id).ok().flatten()
        };
        if provider_run.state() == crate::provider::ProviderRunState::Running
            && !provider_run.read_only_discovery()
            && runtime
                .owned
                .provider_store
                .structured_runtime_state_bound_for_tests(provider_run_id)
        {
            if let Some(process_id) = process_id {
                let environment = provider_child_environment(process_id);
                if environment
                    .get("GIT_SSH_COMMAND")
                    .is_some_and(|value| value == "selected-ssh")
                    && environment
                        .get("SSH_AUTH_SOCK")
                        .is_some_and(|value| value == "selected-agent")
                {
                    return (provider_run, process_id, environment);
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "ordinary provider child did not return after setup lifecycle: {provider_run:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[cfg(target_os = "linux")]
fn provider_child_environment(process_id: u32) -> BTreeMap<String, String> {
    std::fs::read(format!("/proc/{process_id}/environ"))
        .unwrap_or_default()
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let entry = std::str::from_utf8(entry).ok()?;
            let (name, value) = entry.split_once('=')?;
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn provider_child_environment(_process_id: u32) -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[cfg(unix)]
async fn run_repaired_definition_on_fresh_worker(
    root: &std::path::Path,
    definition: ProjectEnvironmentDefinition,
    target_platform: String,
    provider_fixture: &UtilityProviderFixture,
    materialized_inputs: &[(&str, &[u8])],
) -> (ProjectEnvironmentSetupStatus, PathBuf) {
    let fresh_root = root.join("fresh-worker");
    let fresh_home = fresh_root.join("kernel-home");
    let fresh_workspace = fresh_root.join("worker-worktree");
    let fresh_receipt = fresh_root.join("receipt.json");
    std::fs::create_dir_all(&fresh_home).unwrap();
    std::fs::create_dir_all(&fresh_workspace).unwrap();
    for &(relative_path, contents) in materialized_inputs {
        let target = fresh_workspace.join(relative_path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(target, contents).unwrap();
    }
    std::fs::write(fresh_workspace.join("validation-release"), b"").unwrap();
    std::fs::write(
        &fresh_receipt,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "status": "confirmed",
            "allocationId": "worker-fresh",
            "machineId": "worker-machine-fresh",
            "kernelId": "worker-kernel-fresh",
            "relayPublicKey": "worker-public-key-fresh",
            "runtimeReleaseDigest": format!("sha256:{}", "b".repeat(64)),
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
    std::env::set_var("CHARIOX_HOME", &fresh_home);
    std::env::set_var("CHARIOX_DISPOSABLE_WORKER_RECEIPT", &fresh_receipt);

    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "worker-kernel-fresh".into();
    config.host_machine_id = "worker-machine-fresh".into();
    config.relay_public_key = "worker-public-key-fresh".into();
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
            "machine_id": "worker-machine-fresh",
            "machine_credential": format!("mcred_{}", "d".repeat(40))
        }))
        .unwrap(),
    );
    ensure_worker_validation_boundary(&config).expect("fresh worker receipt should be confirmed");

    let mut app = crate::DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = app
        .create_session(
            CreateSessionRequest::new(
                fresh_workspace.display().to_string(),
                fresh_workspace.display().to_string(),
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
        "utility-provider-run-fresh",
        &launch_request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode-project-environment-fresh-worker-fixture".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec!["-c".into(), "sleep 60".into()],
            pty_env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            pty_env_remove: Vec::new(),
            working_directory: Some(fresh_workspace.clone()),
            structured_endpoint: Some(provider_fixture.address()),
        },
    );
    provider_run.mark_running();
    app.providers_mut().insert_run_for_test(provider_run);
    let runtime =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 1)
            .runtime_state();
    runtime
        .update_project_environment_definition(&project_id, definition, "user-1")
        .expect("fresh worker should retain the repaired definition");

    let start = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
                operation_id: "setup-lifecycle-fresh-worker".into(),
                project_id: project_id.clone(),
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                target_worker_id: "worker-machine-fresh".into(),
                target_platform,
                definition: None,
                validation_commands: Vec::new(),
            }),
            "user-1",
        )
        .await
        .expect("fresh worker setup should be accepted");
    let LocalDaemonResponse::ProjectEnvironmentSetupStarted { status } = start else {
        panic!("unexpected fresh worker setup start response: {start:?}");
    };
    assert_eq!(status.phase, ProjectEnvironmentSetupPhase::Requested);
    assert_eq!(status.attempt, 1);

    let ready_deadline = Instant::now() + Duration::from_secs(10);
    let ready = loop {
        let status = response_status(
            get_setup_status(
                &runtime,
                "setup-lifecycle-fresh-worker",
                "user-1",
                "fresh worker polling",
            )
            .await,
        );
        match status.phase {
            ProjectEnvironmentSetupPhase::Ready => break status,
            ProjectEnvironmentSetupPhase::Failed => {
                panic!("fresh worker setup failed: {status:?}")
            }
            _ => {
                assert!(
                    Instant::now() < ready_deadline,
                    "fresh worker setup did not reach Ready: {status:?}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };
    assert_eq!(
        ready
            .validation
            .as_ref()
            .expect("fresh worker Ready requires validation")
            .worker_id,
        "worker-machine-fresh"
    );
    (ready, fresh_workspace)
}

#[cfg(unix)]
#[test]
fn public_setup_status_transport_recovery_and_missing_dispatch_replay_preserve_operation() {
    std::thread::Builder::new()
        .name("project-environment-setup-transport-recovery".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("transport-recovery test runtime should build")
                .block_on(
                    public_setup_status_transport_recovery_and_missing_dispatch_replay_preserve_operation_inner(),
                );
        })
        .expect("transport-recovery test thread should spawn")
        .join()
        .expect("transport-recovery test thread should not panic");
}

#[cfg(unix)]
async fn public_setup_status_transport_recovery_and_missing_dispatch_replay_preserve_operation_inner(
) {
    let _environment_lock = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-project-environment-transport-recovery-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let worker_home = root.join("worker-home");
    let provider_home = root.join("provider-home");
    let workspace = root.join("worker-worktree");
    let receipt = root.join("receipt.json");
    std::fs::create_dir_all(&worker_home).unwrap();
    std::fs::create_dir_all(&provider_home).unwrap();
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
    config_home.relay_token = Some(home_relay_token.clone());
    config_home.relay_heartbeat_ms = 50;
    config_home.relay_request_timeout_ms = SETUP_TRANSPORT_RECOVERY_RELAY_REQUEST_TIMEOUT_MS;

    let mut config_worker = DaemonConfig::for_tests();
    let worker_relay_token = "setup-transport-recovery-worker-token".to_string();
    config_worker.daemon_id = "worker-kernel-transport-recovery".to_string();
    config_worker.host_machine_id = "worker-machine-transport-recovery".to_string();
    config_worker.relay_url = Some(relay_url.clone());
    config_worker.relay_token = Some(worker_relay_token.clone());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.relay_request_timeout_ms = SETUP_TRANSPORT_RECOVERY_RELAY_REQUEST_TIMEOUT_MS;
    config_worker.kernel_runtime_role = KernelRuntimeRole::RemoteLeaseWorker;
    config_worker.accept_remote_leases = true;
    config_worker.remote_lease_capacity = Some(1);
    config_worker.lease_worker_home_caller = Some(crate::config::LeaseWorkerHomeCaller {
        kernel_id: config_home.daemon_id.clone(),
        realm_id: SETUP_TRANSPORT_RECOVERY_REALM.to_string(),
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
            "realm_id": SETUP_TRANSPORT_RECOVERY_REALM,
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
                "realmId": SETUP_TRANSPORT_RECOVERY_REALM,
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
    let app_worker = {
        let app = crate::DaemonApp::bootstrap(config_worker.clone()).unwrap();
        let provider_profiles = app.provider_account_profile_registry();
        let provider_account_owner =
            crate::account_profile::provider_account_authority_owner_user_id(
                &config_worker,
                "user-1",
            );
        let profiles = provider_profiles
            .migrate_effective_defaults(&provider_account_owner, &workspace)
            .unwrap();
        for profile in profiles {
            crate::test_support::authenticate_provider_account(
                &provider_profiles,
                &provider_account_owner,
                &profile.provider,
                &profile.profile_id,
            )
            .unwrap();
        }
        Arc::new(tokio::sync::Mutex::new(app))
    };
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
            realm_id: SETUP_TRANSPORT_RECOVERY_REALM.to_string(),
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
                "opencode",
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
                    // Remote setup status uses the home daemon's relay
                    // identity for metadata discovery and its temporary peer
                    // registration. The listener seed's `secret` is not an
                    // accepted scoped token; the verifier below intentionally
                    // knows only the home and worker tokens.
                    relay_token: Some(home_relay_token.clone()),
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("home agent should bind to the worker");
    }

    let target_platform = actual_worker_platform();
    let validation_command = "sleep 1; command -v sh".to_string();
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
            command: validation_command.clone(),
        }],
        validation_commands: vec![validation_command.clone()],
    };
    let repair_recipe_path = ".devcontainer/devcontainer.json";
    let repair_recipe_contents: &[u8] = br#"{"image":"mcr.microsoft.com/devcontainers/base:ubuntu"}
"#;
    let repair_lockfile_path = "Cargo.lock";
    let repair_lockfile_contents: &[u8] =
        b"version = 3\n\n[[package]]\nname = \"remote-fixture\"\n";
    let mut repairable_definition = definition.clone();
    repairable_definition.source = ProjectEnvironmentDefinitionSource::Devcontainer;
    repairable_definition.source_path = Some(repair_recipe_path.to_string());
    repairable_definition.inputs.clear();
    repairable_definition.setup_steps[0].command = concat!(
        "touch remote-setup-started; ",
        "while [ ! -f remote-setup-release ]; do sleep 0.01; done; ",
        "command -v sh"
    )
    .to_string();
    repairable_definition.validation_commands = vec![format!(
        "touch remote-validation-started; \
         while [ ! -f remote-validation-release ]; do sleep 0.01; done; \
         command -v sh; test -f {repair_recipe_path}; test -f {repair_lockfile_path}"
    )];
    let mut repaired_definition = repairable_definition.clone();
    repaired_definition.inputs = vec![
        ProjectEnvironmentInput {
            kind: ProjectEnvironmentInputKind::Recipe,
            path: repair_recipe_path.to_string(),
            sha256: format!("sha256:{:x}", Sha256::digest(repair_recipe_contents)),
        },
        ProjectEnvironmentInput {
            kind: ProjectEnvironmentInputKind::Lockfile,
            path: repair_lockfile_path.to_string(),
            sha256: format!("sha256:{:x}", Sha256::digest(repair_lockfile_contents)),
        },
    ];
    let (provider_fixture, release_utility) =
        UtilityProviderFixture::start_with_repairs_and_prompt_gate(
            repaired_definition.clone(),
            Some(workspace.clone()),
            BTreeMap::from([
                (
                    repair_recipe_path.to_string(),
                    repair_recipe_contents.to_vec(),
                ),
                (
                    repair_lockfile_path.to_string(),
                    repair_lockfile_contents.to_vec(),
                ),
            ]),
        );
    let provider_run = {
        let launch_request = LaunchProviderRequest::new(
            &backing_session_id,
            "opencode",
            "opencode",
            "default",
            "opencode/test-model",
        )
        .with_agent_id(&backing_agent_id)
        .with_owner_user_id("user-1")
        .with_variant(Some("fixture".to_string()));
        let mut provider_run = RuntimeProviderRun::new(
            "setup-transport-recovery-provider-run",
            &launch_request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "opencode-project-environment-transport-fixture".into(),
                pty_target: None,
                pty_program: Some("/bin/sh".into()),
                pty_args: vec!["-c".into(), "sleep 60".into()],
                pty_env: BTreeMap::from([
                    ("HOME".into(), provider_home.display().to_string()),
                    ("PATH".into(), "/usr/bin:/bin".into()),
                ]),
                pty_env_remove: Vec::new(),
                working_directory: Some(workspace.clone()),
                structured_endpoint: Some(provider_fixture.address()),
            },
        );
        provider_run.mark_running();
        assert_eq!(
            provider_run.endpoint_mode(),
            AgentEndpointMode::Managed,
            "project setup requires a managed OpenCode server that can restart with the prepared worker environment"
        );
        assert!(
            provider_run.adapter_key() == "opencode"
                && matches!(provider_run.adapter_key(), "codex" | "opencode"),
            "the managed test provider must use the restartable OpenCode server adapter"
        );
        assert_eq!(
            provider_run.state(),
            crate::provider::ProviderRunState::Running,
            "the prepared worker provider context must remain live for replay"
        );
        provider_run
    };
    let worker_router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app_worker),
        1,
    ));
    let worker_runtime = worker_router.runtime_state();
    let expected_worker_workspace_id = workspace.display().to_string();
    let setup_target = {
        let mut app = app_worker.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app)
            .project_environment_setup_target(
                &leased_agent_id,
                &session_id,
                &agent_id,
                Some(&expected_worker_workspace_id),
            )
            .expect("relay setup target must resolve the seeded worker lease identities")
    };
    assert_eq!(setup_target.backing_session_id, backing_session_id);
    assert_eq!(setup_target.backing_agent_id, backing_agent_id);
    assert_eq!(setup_target.workspace_id, expected_worker_workspace_id);
    // Seed through the same provider store instance used by the worker's
    // agent_utility_provider_run lookup after relay dispatch.
    worker_runtime
        .owned
        .provider_store
        .write()
        .insert_run_for_test(provider_run.clone());
    {
        let mut app = app_worker.lock().await;
        crate::app::ProviderLaunchProcessRuntime::new(&mut app)
            .spawn_for_launch(&provider_run)
            .expect("managed worker fixture must have a live provider process");
    }
    let backing_session = worker_runtime
        .owned
        .session_store
        .get_session(&backing_session_id)
        .expect("leased backing session must be visible to the worker runtime");
    let backing_agent = worker_runtime
        .owned
        .agent_store
        .get_session_agents(&backing_session_id)
        .into_iter()
        .find(|agent| agent.id() == backing_agent_id.as_str())
        .expect("leased backing agent must belong to the worker session");
    assert_eq!(backing_agent.session_id(), backing_session.id());
    assert!(
        backing_agent.remote_execution().is_none(),
        "worker utility lookup must resolve the local backing agent, not its home remote agent"
    );
    assert!(
        worker_runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&backing_session, &backing_agent_id)
            .is_none(),
        "worker backing agent must not already have an active prompt"
    );
    let seeded_provider_run = worker_runtime
        .owned
        .provider_store
        .get_run_for_agent(&backing_session_id, &backing_agent_id)
        .expect("managed provider run must be indexed under the leased backing identity");
    assert_eq!(
        seeded_provider_run.session_id(),
        backing_session_id.as_str()
    );
    assert_eq!(
        seeded_provider_run.agent_instance_id(),
        Some(backing_agent_id.as_str())
    );
    assert_eq!(seeded_provider_run.owner_user_id(), "user-1");
    assert_eq!(
        seeded_provider_run.state(),
        crate::provider::ProviderRunState::Running
    );
    let (resolved_backing_agent, resolved_provider_run) = worker_runtime
        .agent_utility_provider_run(
            &backing_session_id,
            &backing_agent_id,
            "project environment setup fixture preflight",
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "worker utility lookup should resolve its seeded backing agent and provider run: {error}"
            )
        });
    assert_eq!(resolved_backing_agent.id(), backing_agent_id.as_str());
    assert_eq!(resolved_provider_run.id(), seeded_provider_run.id());
    let worker_execution = super::SetupExecution {
        owner_user_id: "user-1".to_string(),
        operation_id: "setup-transport-recovery".to_string(),
        project_id: project_id.clone(),
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        execution_session_id: backing_session_id.clone(),
        execution_agent_id: backing_agent_id.clone(),
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
            if registry
                .read()
                .await
                .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, daemon_id)
                .is_some()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let registry_snapshot = registry.read().await;
        // `daemon()` is intentionally the default-realm lookup. A missing
        // entry here must not hide a successful scoped-token registration in
        // the fixture's explicit realm, or turn a real auth rejection into a
        // timeout-only diagnostic.
        assert!(
            registry_snapshot
                .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, daemon_id)
                .is_some(),
            "daemon {daemon_id} should register in relay realm {SETUP_TRANSPORT_RECOVERY_REALM}; default-realm-entry={}, registered-daemon-count={}",
            registry_snapshot.daemon(daemon_id).is_some(),
            registry_snapshot.daemon_count(),
        );
    }
    let provider_run_after_relay_registration = worker_runtime
        .owned
        .provider_store
        .get_run_for_agent(&backing_session_id, &backing_agent_id)
        .expect("relay registration must retain the worker backing provider run");
    assert_eq!(
        provider_run_after_relay_registration.id(),
        resolved_provider_run.id()
    );
    assert_eq!(
        provider_run_after_relay_registration.state(),
        crate::provider::ProviderRunState::Running,
        "the managed worker fixture must remain running through relay registration"
    );
    let (_, utility_run_after_relay_registration) = worker_runtime
        .agent_utility_provider_run(
            &backing_session_id,
            &backing_agent_id,
            "project environment setup relay fixture preflight",
        )
        .await
        .unwrap_or_else(|error| {
            panic!("relay-connected worker utility lookup should resolve its seeded run: {error}")
        });
    assert_eq!(
        utility_run_after_relay_registration.id(),
        provider_run_after_relay_registration.id()
    );

    let home_router = CommandRouter::with_interactive_capacity_from_app(Arc::clone(&app_home), 1);
    let runtime = home_router.runtime_state();

    // A selected legacy file-backed definition is admitted by both kernels,
    // but remains unattested until the worker utility materializes its inputs.
    // Hold that utility response so the home must accept the worker's real
    // pre-Ready status before the repaired definition exists.
    let repair_operation_id = "setup-remote-repairable-definition";
    let repair_started = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
                operation_id: repair_operation_id.to_string(),
                project_id: project_id.clone(),
                session_id: session_id.clone(),
                agent_id: agent_id.clone(),
                target_worker_id: config_worker.host_machine_id.clone(),
                target_platform: target_platform.clone(),
                definition: Some(repairable_definition.clone()),
                validation_commands: Vec::new(),
            }),
            "user-1",
        )
        .await
        .expect("home should admit a repairable selected definition");
    let repair_started_status = response_status(repair_started);
    assert_eq!(
        repair_started_status.phase,
        ProjectEnvironmentSetupPhase::Requested,
        "home admission should return the immediate worker-boundary status"
    );

    let utility_prompt_deadline = Instant::now() + Duration::from_secs(3);
    while !provider_fixture.diagnostics().contains("prompt_async") {
        if Instant::now() >= utility_prompt_deadline {
            let status = runtime
                .execute_project_environment_setup_request(
                    LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                        GetProjectEnvironmentSetupStatusRequest {
                            operation_id: repair_operation_id.to_string(),
                        },
                    ),
                    "user-1",
                )
                .await
                .map(response_status);
            panic!(
                "worker did not reach the gated utility request; status: {status:?}; provider trace: {}",
                provider_fixture.diagnostics()
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let immediate_repair_status = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest {
                    operation_id: repair_operation_id.to_string(),
                },
            ),
            "user-1",
        )
        .await
        .expect("home should accept the worker's repairable pre-Ready status");
    let immediate_repair_status = response_status(immediate_repair_status);
    assert!(
        matches!(
            immediate_repair_status.phase,
            ProjectEnvironmentSetupPhase::Requested
                | ProjectEnvironmentSetupPhase::Preparing
                | ProjectEnvironmentSetupPhase::Validating
        ),
        "repairable worker status must remain non-terminal before utility completion: {immediate_repair_status:?}"
    );

    // Hold commands in the relay-connected target worker. Ready must come
    // from the worker after both command barriers are released.
    release_utility.store(true, Ordering::Release);
    let setup_started = workspace.join("remote-setup-started");
    let setup_release = workspace.join("remote-setup-release");
    let validation_started = workspace.join("remote-validation-started");
    let validation_release = workspace.join("remote-validation-release");
    let _gate_cleanup = WorkerCommandGateCleanup {
        setup_release: setup_release.clone(),
        validation_release: validation_release.clone(),
    };
    let held_setup = wait_for_connected_worker_setup_phase(
        &runtime,
        repair_operation_id,
        ProjectEnvironmentSetupPhase::Validating,
        &setup_started,
        "setup",
    )
    .await;
    assert_eq!(held_setup.phase, ProjectEnvironmentSetupPhase::Validating);
    assert!(!validation_started.exists());
    std::fs::write(&setup_release, b"").unwrap();

    let held_validation = wait_for_connected_worker_setup_phase(
        &runtime,
        repair_operation_id,
        ProjectEnvironmentSetupPhase::Validating,
        &validation_started,
        "validation",
    )
    .await;
    assert_eq!(
        held_validation.phase,
        ProjectEnvironmentSetupPhase::Validating
    );
    assert!(!validation_release.exists());
    std::fs::write(&validation_release, b"").unwrap();

    let repair_ready_deadline = Instant::now() + Duration::from_secs(10);
    let repair_ready = loop {
        let observation = runtime
            .execute_project_environment_setup_request(
                LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                    GetProjectEnvironmentSetupStatusRequest {
                        operation_id: repair_operation_id.to_string(),
                    },
                ),
                "user-1",
            )
            .await;
        match observation {
            Ok(response) => {
                let status = response_status(response);
                match status.phase {
                    ProjectEnvironmentSetupPhase::Ready => break status,
                    ProjectEnvironmentSetupPhase::Failed
                    | ProjectEnvironmentSetupPhase::Cancelled => panic!(
                        "repairable worker setup became terminal before Ready: {status:?}; provider trace: {}",
                        provider_fixture.diagnostics()
                    ),
                    ProjectEnvironmentSetupPhase::Requested
                    | ProjectEnvironmentSetupPhase::Preparing
                    | ProjectEnvironmentSetupPhase::Validating => {}
                }
            }
            Err(error) => {
                let (_, status, _) = runtime
                    .owned
                    .project_environment_setups
                    .get_entry_with_cancellation(repair_operation_id, "user-1")
                    .expect("repair operation should remain owned during status recovery");
                assert!(
                    !matches!(
                        status.phase,
                        ProjectEnvironmentSetupPhase::Failed
                            | ProjectEnvironmentSetupPhase::Cancelled
                    ),
                    "repairable worker status error must not manufacture terminal failure: {error}; status={status:?}"
                );
            }
        }
        assert!(
            Instant::now() < repair_ready_deadline,
            "repairable worker setup did not reach Ready: provider trace: {}",
            provider_fixture.diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(repair_ready.attempt, 1);
    assert_eq!(
        repair_ready.definition_digest.as_deref(),
        Some(repaired_definition.digest().as_str()),
        "Ready must carry the utility-repaired definition identity"
    );
    let repair_validation = repair_ready
        .validation
        .as_ref()
        .expect("Ready must carry target-worker validation");
    assert!(
        repair_validation.passed(),
        "Ready must carry passing target-worker validation"
    );
    assert_eq!(repair_validation.worker_id, config_worker.host_machine_id);
    assert_eq!(repair_validation.platform, target_platform);
    assert_eq!(repair_validation.commands.len(), 1);
    assert_eq!(repair_validation.commands[0].exit_code, 0);
    assert!(
        setup_started.exists() && setup_release.exists(),
        "the worker must execute the repaired repeatable setup steps"
    );
    assert!(
        validation_started.exists() && validation_release.exists(),
        "Ready requires the gated worker validation command to complete"
    );
    assert_eq!(
        std::fs::read(workspace.join(repair_recipe_path)).unwrap(),
        repair_recipe_contents,
        "utility repair must materialize the recipe on the worker"
    );
    assert_eq!(
        std::fs::read(workspace.join(repair_lockfile_path)).unwrap(),
        repair_lockfile_contents,
        "utility repair must materialize the lockfile on the worker"
    );
    let persisted_repair = runtime
        .owned
        .session_store
        .get_project(&project_id)
        .expect("selected project should remain available")
        .environment_definition()
        .cloned()
        .expect("Ready should persist the repaired selected definition");
    assert_eq!(persisted_repair, repaired_definition);
    assert!(
        provider_fixture.diagnostics().contains("prompt_async"),
        "the existing worker utility must have repaired the target: {}",
        provider_fixture.diagnostics()
    );

    let start_request = StartProjectEnvironmentSetupRequest {
        operation_id: "setup-transport-recovery".to_string(),
        project_id: project_id.clone(),
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
    let initial = get_setup_status(
        &runtime,
        "setup-transport-recovery",
        "user-1",
        "post-registration status before interruption",
    )
    .await;
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
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
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

    for round in 0..4 {
        let reader_a = runtime.execute_project_environment_setup_request(
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest {
                    operation_id: "setup-transport-recovery".to_string(),
                },
            ),
            "user-1",
        );
        let reader_b = runtime.execute_project_environment_setup_request(
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest {
                    operation_id: "setup-transport-recovery".to_string(),
                },
            ),
            "user-1",
        );
        let (reader_a, reader_b) = tokio::join!(reader_a, reader_b);
        for (reader, observation) in [("reader-a", reader_a), ("reader-b", reader_b)] {
            let error = observation.expect_err(&format!(
                "transport observation {round} from {reader} must remain unavailable"
            ));
            assert!(
                error.to_string().contains("relay") || error.to_string().contains("transport"),
                "transport uncertainty should remain observable from {reader}: {error:?}"
            );
        }
    }
    let (_, uncertain) = runtime
        .owned
        .project_environment_setups
        .get_entry("setup-transport-recovery", "user-1")
        .expect("uncertain setup should remain owned by the caller");
    assert_eq!(
        uncertain.phase,
        ProjectEnvironmentSetupPhase::Requested,
        "transport observations must not manufacture worker-authoritative failure"
    );
    assert_eq!(
        uncertain.attempt, 1,
        "transport observations must preserve the active setup attempt"
    );
    assert!(
        uncertain.retryable,
        "an uncertain active setup must retain its original retryability until a worker status arrives"
    );
    let retry = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::RetryProjectEnvironmentSetup(RetryProjectEnvironmentSetupRequest {
                operation_id: "setup-transport-recovery".to_string(),
                session_id: session_id.clone(),
            }),
            "user-1",
        )
        .await;
    assert!(
        retry.is_err(),
        "uncertain setup must not be retried before worker status is confirmed: {retry:?}"
    );

    let dispatch_never_arrived_operation_id = "setup-dispatch-never-arrived";
    let dispatch_never_arrived = StartProjectEnvironmentSetupRequest {
        operation_id: dispatch_never_arrived_operation_id.to_string(),
        ..start_request.clone()
    };
    let started_while_worker_was_disconnected = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(dispatch_never_arrived),
            "user-1",
        )
        .await
        .expect("home should retain a setup whose dispatch was interrupted");
    assert_eq!(
        response_status(started_while_worker_was_disconnected).phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    let cancel_during_missing_status_operation_id = "setup-cancel-during-missing-status";
    let started_cancel_during_missing_status = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
                operation_id: cancel_during_missing_status_operation_id.to_string(),
                ..start_request.clone()
            }),
            "user-1",
        )
        .await
        .expect("home should retain the setup that will be cancelled during recovery");
    assert_eq!(
        response_status(started_cancel_during_missing_status).phase,
        ProjectEnvironmentSetupPhase::Requested
    );

    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (worker_request_tx, mut worker_request_rx) =
        tokio::sync::mpsc::unbounded_channel::<TestPeerRequestObservation>();
    state_worker
        .write()
        .await
        .test_set_authenticated_peer_request_observer(worker_request_tx);
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
            Arc::clone(&worker_router),
            Arc::clone(&state_worker),
            shutdown_worker_rx,
            relay_url.clone(),
            worker_relay_token.clone(),
        ),
    );
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
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
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
            .is_some(),
        "worker should re-register before recovery is queried"
    );

    // The worker relay fixture emits an authenticated, operation-scoped
    // receipt before dispatching each setup request and holds its response
    // behind the receipt's release channel. This makes the public Get/Cancel
    // ordering explicit without observing the home setup store.
    let get_runtime = runtime.clone();
    let delayed_status = tokio::spawn(async move {
        get_runtime
            .execute_project_environment_setup_request(
                LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                    GetProjectEnvironmentSetupStatusRequest {
                        operation_id: cancel_during_missing_status_operation_id.to_string(),
                    },
                ),
                "user-1",
            )
            .await
    });
    let get_release = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match worker_request_rx.recv().await {
                Some(TestPeerRequestObservation::GetProjectEnvironmentSetupStatus {
                    operation_id,
                    release,
                }) if operation_id == cancel_during_missing_status_operation_id => {
                    break release;
                }
                Some(TestPeerRequestObservation::StartProjectEnvironmentSetup { operation_id }) => {
                    panic!(
                        "unexpected worker setup start before cancellation intent: {operation_id}"
                    )
                }
                Some(_) => continue,
                None => panic!("worker request observer closed before public Get was received"),
            }
        }
    })
    .await
    .expect("authenticated worker should receive the public Get request");

    let cancel_runtime = runtime.clone();
    let cancel_session_id = session_id.clone();
    let cancellation = tokio::spawn(async move {
        cancel_runtime
            .execute_project_environment_setup_request(
                LocalDaemonRequest::CancelProjectEnvironmentSetup(
                    CancelProjectEnvironmentSetupRequest {
                        operation_id: cancel_during_missing_status_operation_id.to_string(),
                        session_id: cancel_session_id,
                    },
                ),
                "user-1",
            )
            .await
    });

    let cancel_release = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match worker_request_rx.recv().await {
                Some(TestPeerRequestObservation::CancelProjectEnvironmentSetup {
                    operation_id,
                    release,
                }) if operation_id == cancel_during_missing_status_operation_id => {
                    break release;
                }
                Some(TestPeerRequestObservation::StartProjectEnvironmentSetup { operation_id }) => {
                    panic!(
                    "worker setup was redispatched before cancellation response: {operation_id}"
                )
                }
                Some(_) => continue,
                None => {
                    panic!("worker request observer closed before public Cancel was received")
                }
            }
        }
    })
    .await
    .expect("authenticated worker should receive the public Cancel request");
    assert!(
        !cancellation.is_finished(),
        "public Cancel should remain at the worker boundary until the delayed status is released"
    );

    // Cancel receipt proves the public home request has recorded intent. Let
    // the worker report the missing operation first, then release Cancel so
    // no response-order assumption can hide a redispatched Start.
    get_release
        .send(())
        .expect("public Get worker response gate should remain open");

    let delayed_status = delayed_status
        .await
        .expect("delayed public status task should join")
        .expect_err("cancelled missing operation must not return a fabricated worker status");
    assert!(
        delayed_status.to_string().contains("project_environment_setup_not_found"),
        "the public status request should expose the authenticated worker absence: {delayed_status:?}"
    );

    let mut redispatched_start = None;
    while let Ok(observation) = worker_request_rx.try_recv() {
        if let TestPeerRequestObservation::StartProjectEnvironmentSetup { operation_id } =
            observation
        {
            redispatched_start = Some(operation_id);
            break;
        }
    }
    assert_eq!(
        redispatched_start, None,
        "public Get must not redispatch Start after public Cancel intent"
    );

    cancel_release
        .send(())
        .expect("public Cancel worker response gate should remain open");
    let cancelled = cancellation
        .await
        .expect("cancellation task should join")
        .expect("missing worker operation should let public cancellation settle safely");
    let cancelled_status = response_status(cancelled);
    assert_eq!(
        cancelled_status.phase,
        ProjectEnvironmentSetupPhase::Cancelled,
        "cancellation intent must win over missing-status redispatch"
    );
    assert_eq!(
        cancelled_status.message.as_deref(),
        Some("setup cancellation completed because the worker had no matching operation"),
        "an authenticated missing-worker response must settle the public cancellation"
    );
    assert!(
        cancelled_status.retryable,
        "a cancellation settled without a worker must remain retryable"
    );
    state_worker
        .write()
        .await
        .test_clear_authenticated_peer_request_observer();

    let recovered_missing_operation = get_setup_status(
        &runtime,
        dispatch_never_arrived_operation_id,
        "user-1",
        "status after the worker reports the dispatch-never-arrived operation missing",
    )
    .await;
    let recovered_missing_status = response_status(recovered_missing_operation);
    assert_eq!(
        recovered_missing_status.operation_id,
        dispatch_never_arrived_operation_id
    );
    assert_eq!(
        recovered_missing_status.attempt, 1,
        "same-operation recovery must not manufacture a retry attempt"
    );
    assert!(
        matches!(
            recovered_missing_status.phase,
            ProjectEnvironmentSetupPhase::Requested
                | ProjectEnvironmentSetupPhase::Preparing
                | ProjectEnvironmentSetupPhase::Validating
                | ProjectEnvironmentSetupPhase::Ready
        ),
        "worker-missing recovery must return an active or measured status, not a heuristic terminal result: {recovered_missing_status:?}"
    );
    let worker_progress_deadline = Instant::now() + Duration::from_secs(10);
    let recovered_worker_status = loop {
        let status = response_status(
            get_setup_status(
                &runtime,
                dispatch_never_arrived_operation_id,
                "user-1",
                "authenticated status polling after same-operation redispatch",
            )
            .await,
        );
        assert_eq!(
            status.operation_id, dispatch_never_arrived_operation_id,
            "authenticated redispatch must retain the requested operation identity"
        );
        assert_eq!(
            status.attempt, 1,
            "authenticated redispatch must retain the original operation attempt"
        );
        // Preparing and Validating still have a live worker task holding the
        // runtime state. Wait for worker-authoritative Ready before replacing
        // the worker so that task has returned and released its state owner.
        if status.phase == ProjectEnvironmentSetupPhase::Ready {
            break status;
        }
        assert!(
            !matches!(
                status.phase,
                ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Cancelled
            ),
            "worker status must not fabricate terminal recovery state: {status:?}; provider trace: {}",
            provider_fixture.diagnostics()
        );
        assert!(
            Instant::now() < worker_progress_deadline,
            "authenticated worker status never advanced after redispatch: {status:?}; provider trace: {}",
            provider_fixture.diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        recovered_worker_status.operation_id,
        dispatch_never_arrived_operation_id
    );
    assert_eq!(
        recovered_worker_status.attempt, 1,
        "the authenticated worker status must retain the original operation attempt"
    );

    let replayed = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(start_request.clone()),
            "user-1",
        )
        .await
        .expect("replayed public start should remain idempotent");
    let replayed_status = response_status(replayed);
    assert_eq!(
        replayed_status.phase,
        ProjectEnvironmentSetupPhase::Requested,
        "replayed public start must not settle an uncertain operation or dispatch a duplicate attempt"
    );
    assert_eq!(replayed_status.attempt, 1);
    let ready = get_setup_status(
        &runtime,
        "setup-transport-recovery",
        "user-1",
        "status after worker re-registration",
    )
    .await;
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

    // This is deliberately separate from the process-restart case below. A
    // connector interruption leaves the worker process and its attempt-one
    // record alive while home Retry advances the authoritative attempt to 2.
    // Recovery must replay through the same authenticated binding after the
    // connector returns; it must not accept a stale Ready/attempt-one record.
    // Hold attempt one at the worker's active setup boundary until Cancel is
    // authoritative. A fixed sleep made the retry's setup and validation
    // phases race the two-second observation deadline.
    let reconnect_operation_id = "setup-retry-before-worker-reconnect";
    let reconnect_release = workspace.join("reconnect-release");
    let reconnect_command =
        "while [ ! -f reconnect-release ]; do sleep 0.01; done; command -v sh".to_string();
    let mut reconnect_definition = definition.clone();
    reconnect_definition.setup_steps[0].command = reconnect_command.clone();
    reconnect_definition.validation_commands = vec![reconnect_command];
    let reconnect_start_request = StartProjectEnvironmentSetupRequest {
        operation_id: reconnect_operation_id.to_string(),
        definition: Some(reconnect_definition),
        ..start_request.clone()
    };
    let reconnect_started = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(reconnect_start_request),
            "user-1",
        )
        .await
        .expect("public connector-recovery Start should be accepted");
    let reconnect_started_status = response_status(reconnect_started);
    assert_eq!(reconnect_started_status.attempt, 1);
    assert_eq!(
        reconnect_started_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    let reconnect_active_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = response_status(
            get_setup_status(
                &runtime,
                reconnect_operation_id,
                "user-1",
                "worker acceptance before connector interruption",
            )
            .await,
        );
        assert_eq!(status.attempt, 1);
        assert!(!matches!(
            status.phase,
            ProjectEnvironmentSetupPhase::Failed
                | ProjectEnvironmentSetupPhase::Cancelled
                | ProjectEnvironmentSetupPhase::Ready
        ));
        if matches!(
            status.phase,
            ProjectEnvironmentSetupPhase::Preparing | ProjectEnvironmentSetupPhase::Validating
        ) {
            break;
        }
        assert!(
            Instant::now() < reconnect_active_deadline,
            "worker did not expose an active setup before connector interruption: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let reconnect_cancelled = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::CancelProjectEnvironmentSetup(
                CancelProjectEnvironmentSetupRequest {
                    operation_id: reconnect_operation_id.to_string(),
                    session_id: session_id.clone(),
                },
            ),
            "user-1",
        )
        .await
        .expect("public connector-recovery Cancel should be acknowledged by the worker");
    let reconnect_cancelled_status = response_status(reconnect_cancelled);
    assert_eq!(
        reconnect_cancelled_status.phase,
        ProjectEnvironmentSetupPhase::Cancelled
    );
    assert_eq!(reconnect_cancelled_status.attempt, 1);
    assert!(reconnect_cancelled_status.retryable);
    std::fs::write(&reconnect_release, b"")
        .expect("connector-recovery worker should be released after authoritative Cancel");

    let _ = shutdown_worker_tx.send(true);
    connector_worker
        .await
        .expect("worker connector should stop before connector-only Retry");
    let worker_disconnected = async {
        for _ in 0..200 {
            if registry
                .read()
                .await
                .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
                .is_none()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }
    .await;
    assert!(
        worker_disconnected,
        "worker connector should be absent before connector-only Retry"
    );
    let reconnect_retry = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::RetryProjectEnvironmentSetup(RetryProjectEnvironmentSetupRequest {
                operation_id: reconnect_operation_id.to_string(),
                session_id: session_id.clone(),
            }),
            "user-1",
        )
        .await
        .expect("home should accept connector-only Retry while delivery is interrupted");
    let reconnect_retry_status = response_status(reconnect_retry);
    assert_eq!(
        reconnect_retry_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert_eq!(reconnect_retry_status.attempt, 2);
    let (_, reconnect_home_status, _) = runtime
        .owned
        .project_environment_setups
        .get_entry_with_cancellation(reconnect_operation_id, "user-1")
        .expect("connector-recovery home operation should remain owned");
    assert_eq!(reconnect_home_status.attempt, 2);
    assert_eq!(
        reconnect_home_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );

    // Reconnect the same worker app/router without replacing its process or
    // durable store. Its authenticated binding and attempt-one record remain
    // the only worker state available to this scenario.
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
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
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
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
            .is_some(),
        "same worker should re-register after connector-only interruption"
    );
    let reconnect_deadline = Instant::now() + Duration::from_secs(2);
    let reconnect_first_status = loop {
        let result = runtime
            .execute_project_environment_setup_request(
                LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                    GetProjectEnvironmentSetupStatusRequest {
                        operation_id: reconnect_operation_id.to_string(),
                    },
                ),
                "user-1",
            )
            .await;
        match result {
            Ok(response) => {
                let status = response_status(response);
                assert_eq!(
                    status.attempt, 2,
                    "connector recovery must never return stale attempt-one status"
                );
                break status;
            }
            Err(error) => {
                assert!(
                    Instant::now() < reconnect_deadline,
                    "same worker remained unreachable after connector re-registration: {error}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };
    assert_eq!(reconnect_first_status.attempt, 2);
    let reconnect_recovery_deadline = Instant::now() + Duration::from_secs(2);
    let reconnect_recovered = loop {
        let result = runtime
            .execute_project_environment_setup_request(
                LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                    GetProjectEnvironmentSetupStatusRequest {
                        operation_id: reconnect_operation_id.to_string(),
                    },
                ),
                "user-1",
            )
            .await;
        match result {
            Ok(response) => {
                let status = response_status(response);
                assert_eq!(status.attempt, 2);
                if status.phase == ProjectEnvironmentSetupPhase::Ready
                    || (status.phase == ProjectEnvironmentSetupPhase::Failed && status.retryable)
                {
                    break status;
                }
            }
            Err(error) => {
                let (_, status, _) = runtime
                    .owned
                    .project_environment_setups
                    .get_entry_with_cancellation(reconnect_operation_id, "user-1")
                    .expect("connector recovery home operation should remain owned");
                assert_eq!(status.attempt, 2);
                assert_ne!(
                    status.phase,
                    ProjectEnvironmentSetupPhase::Ready,
                    "connector recovery must not project stale Ready state"
                );
                assert!(
                    Instant::now() < reconnect_recovery_deadline,
                    "connector recovery remained unreachable: {error}; status={status:?}"
                );
            }
        }
        assert!(
            Instant::now() < reconnect_recovery_deadline,
            "connector recovery did not reach attempt two or an explicit retryable failure"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(reconnect_recovered.attempt, 2);
    assert!(
        reconnect_recovered.phase == ProjectEnvironmentSetupPhase::Ready
            || (reconnect_recovered.phase == ProjectEnvironmentSetupPhase::Failed
                && reconnect_recovered.retryable),
        "connector recovery must end Ready or in an explicit retryable failure: {reconnect_recovered:?}"
    );

    // Exercise the public retry boundary while the authenticated worker is
    // still real and connected after the connector-only recovery. Retry is accepted by the home before its
    // asynchronous worker dispatch is observed; interrupt that dispatch,
    // then restore the same worker durable state with its prior attempt.
    // Use the same explicit release protocol for the restart branch so its
    // second attempt is measured after, rather than during, worker execution.
    let worker_registration = {
        let mut app = app_worker.lock().await;
        app.relay_registration()
    };
    let restart_operation_id = "setup-retry-before-worker-restart";
    let restart_release = workspace.join("restart-release");
    let restart_command =
        "while [ ! -f restart-release ]; do sleep 0.01; done; command -v sh".to_string();
    let mut restart_definition = definition.clone();
    restart_definition.setup_steps[0].command = restart_command.clone();
    restart_definition.validation_commands = vec![restart_command];
    let restart_start_request = StartProjectEnvironmentSetupRequest {
        operation_id: restart_operation_id.to_string(),
        definition: Some(restart_definition),
        ..start_request.clone()
    };
    let restart_started = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(restart_start_request),
            "user-1",
        )
        .await
        .expect("public restart-recovery Start should be accepted");
    let restart_started_status = response_status(restart_started);
    assert_eq!(restart_started_status.attempt, 1);
    assert_eq!(
        restart_started_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    let active_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = response_status(
            get_setup_status(
                &runtime,
                restart_operation_id,
                "user-1",
                "worker acceptance before retry interruption",
            )
            .await,
        );
        assert_eq!(status.attempt, 1);
        assert!(!matches!(
            status.phase,
            ProjectEnvironmentSetupPhase::Failed
                | ProjectEnvironmentSetupPhase::Cancelled
                | ProjectEnvironmentSetupPhase::Ready
        ));
        if matches!(
            status.phase,
            ProjectEnvironmentSetupPhase::Preparing | ProjectEnvironmentSetupPhase::Validating
        ) {
            break;
        }
        assert!(
            Instant::now() < active_deadline,
            "worker did not expose an active setup before retry interruption: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let cancelled = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::CancelProjectEnvironmentSetup(
                CancelProjectEnvironmentSetupRequest {
                    operation_id: restart_operation_id.to_string(),
                    session_id: session_id.clone(),
                },
            ),
            "user-1",
        )
        .await
        .expect("public restart-recovery Cancel should be acknowledged by the worker");
    let cancelled_status = response_status(cancelled);
    assert_eq!(
        cancelled_status.phase,
        ProjectEnvironmentSetupPhase::Cancelled
    );
    assert_eq!(cancelled_status.attempt, 1);
    assert!(cancelled_status.retryable);
    std::fs::write(&restart_release, b"")
        .expect("restart-recovery worker should be released after authoritative Cancel");

    let _ = shutdown_worker_tx.send(true);
    connector_worker
        .await
        .expect("worker connector should stop before retry interruption");
    let worker_disconnected = async {
        for _ in 0..200 {
            if registry
                .read()
                .await
                .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
                .is_none()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }
    .await;
    assert!(
        worker_disconnected,
        "worker transport should be interrupted before public Retry"
    );

    // The connector is gone, but its router and app still hold the durable
    // state owner. Release every original worker handle before accepting the
    // home Retry and reopening the same durable state path.
    {
        let mut app = app_worker.lock().await;
        crate::app::ProviderLaunchProcessRuntime::new(&mut app)
            .remove_run(provider_run.id())
            .expect("original managed provider fixture process should stop");
    }
    drop(worker_runtime);
    drop(worker_router);
    drop(app_worker);

    let retry_accepted = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::RetryProjectEnvironmentSetup(RetryProjectEnvironmentSetupRequest {
                operation_id: restart_operation_id.to_string(),
                session_id: session_id.clone(),
            }),
            "user-1",
        )
        .await
        .expect("home must accept a legitimate retry before worker dispatch");
    let retry_accepted_status = response_status(retry_accepted);
    assert_eq!(
        retry_accepted_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert_eq!(retry_accepted_status.attempt, 2);

    let disconnected_get = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest {
                    operation_id: restart_operation_id.to_string(),
                },
            ),
            "user-1",
        )
        .await;
    assert!(
        disconnected_get.is_err(),
        "public Get must observe the controlled worker interruption: {disconnected_get:?}"
    );
    let (_, after_retry, _) = runtime
        .owned
        .project_environment_setups
        .get_entry_with_cancellation(restart_operation_id, "user-1")
        .expect("home retry state should remain owned");
    assert_eq!(after_retry.attempt, 2);
    assert_eq!(after_retry.phase, ProjectEnvironmentSetupPhase::Requested);

    // Rebootstrap the same worker from the same durable state path. No setup
    // entry is fabricated: the public Cancel above persisted the worker's
    // attempt-one state, which this fresh app must restore.
    let restarted_worker_app = Arc::new(tokio::sync::Mutex::new(
        crate::DaemonApp::bootstrap(config_worker.clone())
            .expect("worker should restore its durable setup state"),
    ));
    let restarted_worker_router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&restarted_worker_app),
        1,
    ));
    let restarted_worker_state = restarted_worker_app.lock().await.relay_client_state();
    let (restarted_shutdown_worker_tx, restarted_shutdown_worker_rx) = watch::channel(false);
    let restarted_connector_worker = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
            Arc::clone(&restarted_worker_router),
            restarted_worker_state,
            restarted_shutdown_worker_rx,
            relay_url.clone(),
            worker_relay_token.clone(),
        ),
    );
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
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
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
            .is_some(),
        "restored worker should re-register before public recovery"
    );

    // A restarted worker loses its ephemeral lease ownership. The first
    // public Get must either perform the kernel-owned refresh and return
    // attempt 2, or reject stale attempt-1 state while leaving home on
    // attempt 2 for the bounded recovery poll below.
    let first_post_restart = tokio::time::timeout(
        Duration::from_secs(2),
        runtime.execute_project_environment_setup_request(
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest {
                    operation_id: restart_operation_id.to_string(),
                },
            ),
            "user-1",
        ),
    )
    .await
    .expect("first post-restart Get must remain within the observation deadline");
    match first_post_restart {
        Ok(response) => {
            let status = response_status(response);
            assert_eq!(
                status.attempt, 2,
                "restart recovery must never return stale attempt-one status"
            );
        }
        Err(error) => {
            let (_, status, _) = runtime
                .owned
                .project_environment_setups
                .get_entry_with_cancellation(restart_operation_id, "user-1")
                .expect("home setup must remain owned during restart recovery");
            assert_eq!(status.attempt, 2);
            assert_ne!(
                status.phase,
                ProjectEnvironmentSetupPhase::Ready,
                "stale worker status must never be accepted as Ready"
            );
            assert!(
                error.to_string().contains("remote")
                    || error.to_string().contains("relay")
                    || error.to_string().contains("setup"),
                "restart rejection should remain a scoped setup/remote diagnostic: {error}"
            );
        }
    }

    let recovery_deadline = Instant::now() + Duration::from_secs(2);
    let recovered = loop {
        let result = tokio::time::timeout(
            recovery_deadline.saturating_duration_since(Instant::now()),
            runtime.execute_project_environment_setup_request(
                LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                    GetProjectEnvironmentSetupStatusRequest {
                        operation_id: restart_operation_id.to_string(),
                    },
                ),
                "user-1",
            ),
        )
        .await
        .expect("post-restart recovery Get must remain within the total deadline");
        match result {
            Ok(response) => {
                let status = response_status(response);
                assert_eq!(
                    status.attempt, 2,
                    "public recovery must never return a stale worker attempt"
                );
                if status.phase == ProjectEnvironmentSetupPhase::Ready
                    || (status.phase == ProjectEnvironmentSetupPhase::Failed && status.retryable)
                {
                    break status;
                }
            }
            Err(error) => {
                let (_, status, _) = runtime
                    .owned
                    .project_environment_setups
                    .get_entry_with_cancellation(restart_operation_id, "user-1")
                    .expect("home setup must remain owned during recovery");
                assert_eq!(status.attempt, 2);
                assert_ne!(
                    status.phase,
                    ProjectEnvironmentSetupPhase::Ready,
                    "stale worker status must never be accepted as Ready"
                );
                if status.phase == ProjectEnvironmentSetupPhase::Failed && status.retryable {
                    break status;
                }
                assert!(
                    Instant::now() < recovery_deadline,
                    "home setup remained permanently unreachable after worker restart: {error}; status={status:?}"
                );
            }
        }
        assert!(
            Instant::now() < recovery_deadline,
            "home setup did not progress or expose an explicit recoverable failure"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(recovered.attempt, 2);
    assert!(
        recovered.phase == ProjectEnvironmentSetupPhase::Ready
            || (recovered.phase == ProjectEnvironmentSetupPhase::Failed && recovered.retryable),
        "recovery must end Ready or in an explicit retryable failure: {recovered:?}"
    );

    let _ = restarted_shutdown_worker_tx.send(true);
    restarted_connector_worker
        .await
        .expect("restored worker connector should stop");
    let restarted_worker_disconnected = async {
        for _ in 0..200 {
            if registry
                .read()
                .await
                .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
                .is_none()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }
    .await;
    assert!(
        restarted_worker_disconnected,
        "restored worker should be absent before the retained-worker fixture registers"
    );

    // Start a separate operation through an authenticated external worker
    // fixture. Its first Get is worker-authoritative Cancelled, allowing the
    // public Retry to advance the home's attempt to 2 without touching either
    // kernel's private setup store.
    let retry_operation_id = "setup-worker-attempt-recovery";
    let (fake_worker_shutdown, fake_worker_start_seen, fake_worker_retry_seen, fake_worker_task) =
        spawn_external_worker_fixture(
            relay_url.clone(),
            worker_registration.clone(),
            config_worker.relay_private_key.clone(),
            config_home.relay_public_key.clone(),
            ExternalWorkerFixtureMode::Stateful,
        );
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
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
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
            .is_some(),
        "external worker should register before public setup dispatch"
    );

    let retry_started = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
                operation_id: retry_operation_id.to_string(),
                ..start_request.clone()
            }),
            "user-1",
        )
        .await
        .expect("public retry-regression start should be accepted");
    let retry_started_status = response_status(retry_started);
    assert_eq!(
        retry_started_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert_eq!(retry_started_status.attempt, 1);
    tokio::time::timeout(Duration::from_secs(3), fake_worker_start_seen)
        .await
        .expect("external worker should observe the public Start")
        .expect("external worker Start barrier should remain available");

    let first_cancelled = response_status(
        get_setup_status(
            &runtime,
            retry_operation_id,
            "user-1",
            "worker-authoritative cancellation before public Retry",
        )
        .await,
    );
    assert_eq!(
        first_cancelled.phase,
        ProjectEnvironmentSetupPhase::Cancelled
    );
    assert_eq!(first_cancelled.attempt, 1);
    assert!(first_cancelled.retryable);

    let retry_accepted = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::RetryProjectEnvironmentSetup(RetryProjectEnvironmentSetupRequest {
                operation_id: retry_operation_id.to_string(),
                session_id: session_id.clone(),
            }),
            "user-1",
        )
        .await
        .expect("public retry-regression Retry should be accepted");
    let retry_accepted_status = response_status(retry_accepted);
    assert_eq!(
        retry_accepted_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert_eq!(
        retry_accepted_status.attempt, 2,
        "home Retry must advance the authoritative operation attempt"
    );
    tokio::time::timeout(Duration::from_secs(3), fake_worker_retry_seen)
        .await
        .expect("external worker should observe the public Retry")
        .expect("external worker Retry barrier should remain available");
    let second_attempt = response_status(
        get_setup_status(
            &runtime,
            retry_operation_id,
            "user-1",
            "public Get after worker-observed Retry",
        )
        .await,
    );
    assert_eq!(
        second_attempt.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert_eq!(second_attempt.attempt, 2);

    let _ = fake_worker_shutdown.send(());
    fake_worker_task
        .await
        .expect("stateful external worker should stop before simulated loss");
    let fake_worker_disconnected = async {
        for _ in 0..200 {
            if registry
                .read()
                .await
                .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
                .is_none()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }
    .await;
    assert!(
        fake_worker_disconnected,
        "external worker should be absent before replacement with empty state"
    );

    // A fresh external connection reuses only the authenticated registration;
    // it has no setup record. The desired public result is successful
    // same-operation recovery at the home's attempt 2.
    let (
        empty_worker_shutdown,
        _empty_worker_start_seen,
        _empty_worker_retry_seen,
        empty_worker_task,
    ) = spawn_external_worker_fixture(
        relay_url.clone(),
        worker_registration.clone(),
        config_worker.relay_private_key.clone(),
        config_home.relay_public_key.clone(),
        ExternalWorkerFixtureMode::Empty,
    );
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
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
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
            .is_some(),
        "replacement worker should register before public recovery"
    );
    let recovered = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                GetProjectEnvironmentSetupStatusRequest {
                    operation_id: retry_operation_id.to_string(),
                },
            ),
            "user-1",
        )
        .await
        .expect("public Get should recover the lost worker operation");
    let recovered_status = response_status(recovered);
    assert_eq!(recovered_status.operation_id, retry_operation_id);
    assert_eq!(
        recovered_status.attempt, 2,
        "lost-worker redispatch must preserve the home-authoritative attempt"
    );
    assert_eq!(
        recovered_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );

    let _ = empty_worker_shutdown.send(());
    empty_worker_task
        .await
        .expect("empty external worker should stop");
    let empty_worker_disconnected = async {
        for _ in 0..200 {
            if registry
                .read()
                .await
                .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
                .is_none()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }
    .await;
    assert!(
        empty_worker_disconnected,
        "empty replacement worker should be absent before the withheld-retry worker registers"
    );

    // Exercise the distinct stale-attempt path through the authenticated
    // public Start/Cancel/Retry/Get seam. The retained worker keeps its
    // attempt-one Cancelled record while the home accepts Retry attempt two;
    // the first recovery Retry response is withheld so a concurrent Get must
    // observe the shared same-attempt reservation rather than dispatching a
    // duplicate. The old accepted Retry response is released only after the
    // recovery response, proving late settlement cannot rewind the home.
    let (
        withheld_worker_shutdown,
        withheld_retry_release_tx,
        withheld_start_seen,
        withheld_retry_seen,
        withheld_recovery_retry_seen,
        withheld_second_get_seen,
        withheld_retry_release_applied,
        withheld_retry_count,
        withheld_worker_task,
    ) = spawn_external_worker_fixture_with_withheld_retry(
        relay_url.clone(),
        worker_registration,
        config_worker.relay_private_key.clone(),
        config_home.relay_public_key.clone(),
    );
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
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
            .daemon_in_realm(SETUP_TRANSPORT_RECOVERY_REALM, &config_worker.daemon_id)
            .is_some(),
        "withheld-retry worker should register before public stale-attempt recovery"
    );

    let stale_attempt_operation_id = "setup-stale-attempt-recovery";
    let stale_attempt_started = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
                operation_id: stale_attempt_operation_id.to_string(),
                ..start_request.clone()
            }),
            "user-1",
        )
        .await
        .expect("public stale-attempt Start should be accepted");
    let stale_attempt_started_status = response_status(stale_attempt_started);
    assert_eq!(stale_attempt_started_status.attempt, 1);
    assert_eq!(
        stale_attempt_started_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    tokio::time::timeout(Duration::from_secs(2), withheld_start_seen)
        .await
        .expect("withheld-retry worker should observe the public Start before its first Get")
        .expect("withheld-retry Start barrier should remain available");

    let stale_attempt_cancelled = response_status(
        get_setup_status(
            &runtime,
            stale_attempt_operation_id,
            "user-1",
            "retained worker attempt-one cancellation before public Retry",
        )
        .await,
    );
    assert_eq!(
        stale_attempt_cancelled.phase,
        ProjectEnvironmentSetupPhase::Cancelled
    );
    assert_eq!(stale_attempt_cancelled.attempt, 1);
    assert!(stale_attempt_cancelled.retryable);

    let stale_attempt_retry = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::RetryProjectEnvironmentSetup(RetryProjectEnvironmentSetupRequest {
                operation_id: stale_attempt_operation_id.to_string(),
                session_id: session_id.clone(),
            }),
            "user-1",
        )
        .await
        .expect("home should accept stale-attempt Retry");
    let stale_attempt_retry_status = response_status(stale_attempt_retry);
    assert_eq!(stale_attempt_retry_status.attempt, 2);
    assert_eq!(
        stale_attempt_retry_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    tokio::time::timeout(Duration::from_secs(2), withheld_retry_seen)
        .await
        .expect("worker should observe the accepted Retry before its response is withheld")
        .expect("withheld Retry barrier should remain available");

    let first_stale_get = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            runtime
                .execute_project_environment_setup_request(
                    LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                        GetProjectEnvironmentSetupStatusRequest {
                            operation_id: stale_attempt_operation_id.to_string(),
                        },
                    ),
                    "user-1",
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), withheld_recovery_retry_seen)
        .await
        .expect("stale-attempt recovery should reach the worker Retry request")
        .expect("recovery Retry barrier should remain available");

    let second_stale_get = tokio::spawn({
        let runtime = runtime.clone();
        async move {
            runtime
                .execute_project_environment_setup_request(
                    LocalDaemonRequest::GetProjectEnvironmentSetupStatus(
                        GetProjectEnvironmentSetupStatusRequest {
                            operation_id: stale_attempt_operation_id.to_string(),
                        },
                    ),
                    "user-1",
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), withheld_second_get_seen)
        .await
        .expect("concurrent stale-attempt polling should reach the worker")
        .expect("concurrent Get barrier should remain available");
    let second_stale_get_error = tokio::time::timeout(Duration::from_secs(2), second_stale_get)
        .await
        .expect("concurrent stale-attempt Get should remain bounded")
        .expect("concurrent stale-attempt Get should not panic")
        .expect_err("the concurrent Get must be fenced while recovery Retry is in flight");
    assert!(
        second_stale_get_error
            .to_string()
            .contains("same-attempt replay is already in flight"),
        "concurrent stale-attempt polling should expose the shared reservation: {second_stale_get_error}"
    );

    let _ = withheld_retry_release_tx.send(());
    tokio::time::timeout(Duration::from_secs(2), withheld_retry_release_applied)
        .await
        .expect("withheld Retry responses should be released after concurrent polling")
        .expect("withheld Retry release barrier should remain available");
    let first_stale_get_status = response_status(
        tokio::time::timeout(Duration::from_secs(2), first_stale_get)
            .await
            .expect("stale-attempt recovery Get should remain bounded")
            .expect("stale-attempt recovery Get should not panic")
            .expect("stale-attempt recovery Get should reconcile the current attempt"),
    );
    assert_eq!(
        first_stale_get_status.operation_id,
        stale_attempt_operation_id
    );
    assert_eq!(
        first_stale_get_status.attempt, 2,
        "stale worker attempt-one status must never be returned after home Retry"
    );
    assert_eq!(
        first_stale_get_status.phase,
        ProjectEnvironmentSetupPhase::Requested
    );
    assert_eq!(
        withheld_retry_count.load(Ordering::Acquire),
        2,
        "one accepted Retry and one shared stale-attempt recovery Retry are allowed; concurrent Get must not add another"
    );

    let _ = withheld_worker_shutdown.send(());
    withheld_worker_task
        .await
        .expect("withheld-retry external worker should stop");
    let _ = shutdown_home_tx.send(true);
    connector_home.await.expect("home connector should stop");
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
                realm_id: SETUP_TRANSPORT_RECOVERY_REALM.to_string(),
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
                realm_id: SETUP_TRANSPORT_RECOVERY_REALM.to_string(),
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExternalWorkerFixtureMode {
    Stateful,
    Empty,
    WithheldRetry,
}

#[cfg(unix)]
#[derive(Clone)]
struct ExternalWorkerSetupState {
    operation_id: String,
    project_id: String,
    home_session_id: String,
    home_agent_id: String,
    target_worker_id: String,
    target_platform: String,
    definition: Option<ProjectEnvironmentDefinition>,
    attempt: u32,
}

#[cfg(unix)]
struct ExternalWorkerRetryControl {
    release_retry_rx: oneshot::Receiver<()>,
    release_retry_applied_tx: Option<oneshot::Sender<()>>,
    recovery_retry_seen_tx: Option<oneshot::Sender<()>>,
    second_get_seen_tx: Option<oneshot::Sender<()>>,
    retry_count: Arc<AtomicUsize>,
}

#[cfg(unix)]
fn spawn_external_worker_fixture(
    relay_url: String,
    registration: DaemonRegistration,
    worker_private_key: String,
    home_public_key: String,
    mode: ExternalWorkerFixtureMode,
) -> (
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    tokio::task::JoinHandle<()>,
) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (start_seen_tx, start_seen_rx) = oneshot::channel();
    let (retry_seen_tx, retry_seen_rx) = oneshot::channel();
    let task = tokio::spawn(run_external_worker_fixture(
        relay_url,
        registration,
        worker_private_key,
        home_public_key,
        mode,
        shutdown_rx,
        start_seen_tx,
        retry_seen_tx,
        None,
    ));
    (shutdown_tx, start_seen_rx, retry_seen_rx, task)
}

#[cfg(unix)]
fn spawn_external_worker_fixture_with_withheld_retry(
    relay_url: String,
    registration: DaemonRegistration,
    worker_private_key: String,
    home_public_key: String,
) -> (
    oneshot::Sender<()>,
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (release_retry_tx, release_retry_rx) = oneshot::channel();
    let (release_retry_applied_tx, release_retry_applied_rx) = oneshot::channel();
    let (start_seen_tx, start_seen_rx) = oneshot::channel();
    let (retry_seen_tx, retry_seen_rx) = oneshot::channel();
    let (recovery_retry_seen_tx, recovery_retry_seen_rx) = oneshot::channel();
    let (second_get_seen_tx, second_get_seen_rx) = oneshot::channel();
    let retry_count = Arc::new(AtomicUsize::new(0));
    let retry_control = ExternalWorkerRetryControl {
        release_retry_rx,
        release_retry_applied_tx: Some(release_retry_applied_tx),
        recovery_retry_seen_tx: Some(recovery_retry_seen_tx),
        second_get_seen_tx: Some(second_get_seen_tx),
        retry_count: Arc::clone(&retry_count),
    };
    let task = tokio::spawn(run_external_worker_fixture(
        relay_url,
        registration,
        worker_private_key,
        home_public_key,
        ExternalWorkerFixtureMode::WithheldRetry,
        shutdown_rx,
        start_seen_tx,
        retry_seen_tx,
        Some(retry_control),
    ));
    (
        shutdown_tx,
        release_retry_tx,
        start_seen_rx,
        retry_seen_rx,
        recovery_retry_seen_rx,
        second_get_seen_rx,
        release_retry_applied_rx,
        retry_count,
        task,
    )
}

#[cfg(unix)]
async fn run_external_worker_fixture(
    relay_url: String,
    registration: DaemonRegistration,
    worker_private_key: String,
    home_public_key: String,
    mode: ExternalWorkerFixtureMode,
    mut shutdown_rx: oneshot::Receiver<()>,
    start_seen_tx: oneshot::Sender<()>,
    retry_seen_tx: oneshot::Sender<()>,
    retry_control: Option<ExternalWorkerRetryControl>,
) {
    let (mut socket, _) =
        match tokio::time::timeout(Duration::from_secs(3), connect_async(&relay_url)).await {
            Ok(Ok(connection)) => connection,
            _ => return,
        };
    let register = RelayEnvelope::DaemonRegister { registration };
    let register_payload = match serde_json::to_string(&register) {
        Ok(payload) => payload,
        Err(_) => return,
    };
    if socket
        .send(Message::Text(register_payload.into()))
        .await
        .is_err()
    {
        return;
    }

    let mut start_seen_tx = Some(start_seen_tx);
    let mut retry_seen_tx = Some(retry_seen_tx);
    let mut retry_control = retry_control;
    let mut state: Option<ExternalWorkerSetupState> = None;
    let mut held_retry_responses: Vec<(String, RelayProjectEnvironmentSetupStatus)> = Vec::new();
    let mut retry_responses_released = false;
    loop {
        let message = tokio::select! {
            _ = &mut shutdown_rx => {
                let _ = socket.close(None).await;
                return;
            }
            _ = async {
                if let Some(control) = retry_control.as_mut() {
                    let _ = (&mut control.release_retry_rx).await;
                } else {
                    std::future::pending::<()>().await;
                }
            }, if mode == ExternalWorkerFixtureMode::WithheldRetry
                && !retry_responses_released
                && !held_retry_responses.is_empty() => {
                retry_responses_released = true;
                if let Some(setup_state) = state.as_mut() {
                    setup_state.attempt = 2;
                }
                // The recovery Retry is appended after the accepted public
                // Retry. Release it first so the accepted response is truly
                // late and cannot be mistaken for the recovery observation.
                let held_retry_responses = std::mem::take(&mut held_retry_responses);
                for (relay_request_id, setup) in held_retry_responses.into_iter().rev() {
                    let response = RelayEnvelope::DaemonIncomingPeerResponse {
                        relay_request_id,
                        encrypted_response: Some(encrypt_external_worker_response(
                            &worker_private_key,
                            &home_public_key,
                            RelayPeerResponse::LeasedProjectEnvironmentSetupRetried { setup },
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
                }
                if let Some(control) = retry_control.as_mut() {
                    if let Some(release_retry_applied_tx) = control.release_retry_applied_tx.take() {
                        let _ = release_retry_applied_tx.send(());
                    }
                }
                continue;
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
        let decrypted = match crate::transport::relay_crypto::decrypt_payload_for_private_key(
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
        // Follow the serialized home-authoritative attempt at this external
        // relay boundary; do not pin a valid recovery implementation to the
        // fresh worker's default attempt 1.
        let request_attempt = external_worker_request_attempt(&request);
        let (encrypted_response, error) = match request {
            RelayPeerRequest::StartLeasedProjectEnvironmentSetup {
                operation_id,
                project_id,
                home_session_id,
                home_agent_id,
                target_worker_id,
                target_platform,
                definition,
                ..
            } => {
                if let Some(start_seen_tx) = start_seen_tx.take() {
                    let _ = start_seen_tx.send(());
                }
                let setup_state = ExternalWorkerSetupState {
                    operation_id,
                    project_id,
                    home_session_id,
                    home_agent_id,
                    target_worker_id,
                    target_platform,
                    definition,
                    attempt: request_attempt,
                };
                if matches!(
                    mode,
                    ExternalWorkerFixtureMode::Stateful | ExternalWorkerFixtureMode::WithheldRetry
                ) {
                    state = Some(setup_state.clone());
                }
                let setup = external_worker_setup_status(
                    &setup_state,
                    ProjectEnvironmentSetupPhase::Requested,
                    request_attempt,
                );
                let response = RelayPeerResponse::LeasedProjectEnvironmentSetupStarted { setup };
                (
                    Some(encrypt_external_worker_response(
                        &worker_private_key,
                        &home_public_key,
                        response,
                    )),
                    None,
                )
            }
            RelayPeerRequest::GetLeasedProjectEnvironmentSetupStatus { operation_id, .. } => {
                match state.as_ref().filter(|setup_state| {
                    matches!(
                        mode,
                        ExternalWorkerFixtureMode::Stateful
                            | ExternalWorkerFixtureMode::WithheldRetry
                    ) && setup_state.operation_id == operation_id
                }) {
                    Some(setup_state) => {
                        if mode == ExternalWorkerFixtureMode::WithheldRetry
                            && setup_state.attempt == 1
                            && held_retry_responses.len() >= 2
                        {
                            if let Some(control) = retry_control.as_mut() {
                                if let Some(second_get_seen_tx) = control.second_get_seen_tx.take()
                                {
                                    let _ = second_get_seen_tx.send(());
                                }
                            }
                        }
                        let phase = if setup_state.attempt == 1 {
                            ProjectEnvironmentSetupPhase::Cancelled
                        } else {
                            ProjectEnvironmentSetupPhase::Requested
                        };
                        let attempt = setup_state.attempt;
                        let setup = external_worker_setup_status(setup_state, phase, attempt);
                        let response =
                            RelayPeerResponse::LeasedProjectEnvironmentSetupStatus { setup };
                        (
                            Some(encrypt_external_worker_response(
                                &worker_private_key,
                                &home_public_key,
                                response,
                            )),
                            None,
                        )
                    }
                    None => (
                        None,
                        Some(RelayError {
                            code: PROJECT_ENVIRONMENT_SETUP_NOT_FOUND_CODE.to_string(),
                            message: "setup operation was not found".to_string(),
                            retryable: false,
                        }),
                    ),
                }
            }
            RelayPeerRequest::RetryLeasedProjectEnvironmentSetup { operation_id, .. }
                if matches!(
                    mode,
                    ExternalWorkerFixtureMode::Stateful | ExternalWorkerFixtureMode::WithheldRetry
                ) && state
                    .as_ref()
                    .is_some_and(|setup_state| setup_state.operation_id == operation_id) =>
            {
                let setup_state = state.as_mut().expect("stateful setup should be present");
                if mode == ExternalWorkerFixtureMode::WithheldRetry {
                    let retry_number = retry_control
                        .as_ref()
                        .expect("withheld-retry mode should have control")
                        .retry_count
                        .fetch_add(1, Ordering::AcqRel)
                        + 1;
                    if retry_number > 2 {
                        (
                            None,
                            Some(RelayError {
                                code: PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE.to_string(),
                                message: "duplicate stale-attempt Retry reached fixture"
                                    .to_string(),
                                retryable: false,
                            }),
                        )
                    } else {
                        if retry_number == 2 {
                            if let Some(control) = retry_control.as_mut() {
                                if let Some(recovery_retry_seen_tx) =
                                    control.recovery_retry_seen_tx.take()
                                {
                                    let _ = recovery_retry_seen_tx.send(());
                                }
                            }
                        }
                        if retry_number == 1 {
                            if let Some(retry_seen_tx) = retry_seen_tx.take() {
                                let _ = retry_seen_tx.send(());
                            }
                        }
                        let mut response_state = setup_state.clone();
                        response_state.attempt = 2;
                        let setup = external_worker_setup_status(
                            &response_state,
                            ProjectEnvironmentSetupPhase::Requested,
                            2,
                        );
                        held_retry_responses.push((relay_request_id, setup));
                        continue;
                    }
                } else {
                    setup_state.attempt = 2;
                    if let Some(retry_seen_tx) = retry_seen_tx.take() {
                        let _ = retry_seen_tx.send(());
                    }
                    let attempt = setup_state.attempt;
                    let setup = external_worker_setup_status(
                        setup_state,
                        ProjectEnvironmentSetupPhase::Requested,
                        attempt,
                    );
                    let response =
                        RelayPeerResponse::LeasedProjectEnvironmentSetupRetried { setup };
                    (
                        Some(encrypt_external_worker_response(
                            &worker_private_key,
                            &home_public_key,
                            response,
                        )),
                        None,
                    )
                }
            }
            _ => (
                None,
                Some(RelayError {
                    code: PROJECT_ENVIRONMENT_SETUP_REJECTED_CODE.to_string(),
                    message: "external worker fixture rejected unsupported setup request"
                        .to_string(),
                    retryable: false,
                }),
            ),
        };
        let response = RelayEnvelope::DaemonIncomingPeerResponse {
            relay_request_id,
            encrypted_response,
            error,
        };
        let response_payload = match serde_json::to_string(&response) {
            Ok(payload) => payload,
            Err(_) => return,
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

#[cfg(unix)]
fn external_worker_request_attempt(request: &RelayPeerRequest) -> u32 {
    serde_json::to_value(request)
        .ok()
        .and_then(|value| value.get("attempt").and_then(serde_json::Value::as_u64))
        .and_then(|attempt| u32::try_from(attempt).ok())
        .filter(|attempt| *attempt > 0)
        .unwrap_or(1)
}

#[cfg(unix)]
fn encrypt_external_worker_response(
    worker_private_key: &str,
    home_public_key: &str,
    response: RelayPeerResponse,
) -> chariox_relay::protocol::EncryptedRelayPayload {
    let plaintext = serde_json::to_vec(&response).expect("fixture response should serialize");
    crate::transport::relay_crypto::encrypt_payload_for_peer(
        worker_private_key,
        home_public_key,
        &plaintext,
    )
    .expect("fixture response should encrypt for the authenticated home")
}

#[cfg(unix)]
fn external_worker_setup_status(
    state: &ExternalWorkerSetupState,
    phase: ProjectEnvironmentSetupPhase,
    attempt: u32,
) -> RelayProjectEnvironmentSetupStatus {
    RelayProjectEnvironmentSetupStatus {
        status: ProjectEnvironmentSetupStatus {
            operation_id: state.operation_id.clone(),
            project_id: state.project_id.clone(),
            session_id: state.home_session_id.clone(),
            agent_id: state.home_agent_id.clone(),
            worker_id: state.target_worker_id.clone(),
            platform: state.target_platform.clone(),
            phase,
            attempt,
            progress_percent: 0,
            definition_digest: state
                .definition
                .as_ref()
                .map(|definition| definition.digest()),
            validation: None,
            message: Some(
                match phase {
                    ProjectEnvironmentSetupPhase::Cancelled => {
                        "external worker cancelled the first attempt"
                    }
                    ProjectEnvironmentSetupPhase::Requested => {
                        "external worker accepted the attempt"
                    }
                    _ => "external worker reported setup status",
                }
                .to_string(),
            ),
            failure_code: None,
            failure_message: None,
            retryable: matches!(
                phase,
                ProjectEnvironmentSetupPhase::Requested
                    | ProjectEnvironmentSetupPhase::Preparing
                    | ProjectEnvironmentSetupPhase::Validating
                    | ProjectEnvironmentSetupPhase::Cancelled
            ),
            created_at_ms: 1,
            updated_at_ms: u64::from(attempt),
        },
        definition: state.definition.clone(),
    }
}

#[cfg(unix)]
async fn get_setup_status(
    runtime: &crate::runtime::state::KernelRuntimeState,
    operation_id: &str,
    caller_user_id: &str,
    phase: &str,
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
        .unwrap_or_else(|error| {
            panic!(
                "public setup status should be available during {phase} (operation {operation_id}, caller {caller_user_id}): {error}"
            )
        })
}

#[cfg(unix)]
async fn wait_for_connected_worker_setup_phase(
    runtime: &crate::runtime::state::KernelRuntimeState,
    operation_id: &str,
    expected_phase: ProjectEnvironmentSetupPhase,
    command_started: &std::path::Path,
    command_name: &str,
) -> ProjectEnvironmentSetupStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = response_status(
            get_setup_status(
                runtime,
                operation_id,
                "user-1",
                "connected worker setup readiness barrier",
            )
            .await,
        );
        if status.phase == expected_phase && command_started.exists() {
            return status;
        }
        assert_ne!(
            status.phase,
            ProjectEnvironmentSetupPhase::Ready,
            "connected target worker became Ready before its {command_name} command passed"
        );
        assert!(
            !matches!(
                status.phase,
                ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Cancelled
            ),
            "connected worker setup terminated before the {command_name} barrier: {status:?}"
        );
        assert!(
            Instant::now() < deadline,
            "connected worker did not reach the {command_name} barrier (marker present: {}): {status:?}",
            command_started.exists()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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
        Self::start_with_repairs(definition, None, BTreeMap::new())
    }

    fn start_with_repairs(
        definition: ProjectEnvironmentDefinition,
        repair_workspace: Option<PathBuf>,
        repair_files: BTreeMap<String, Vec<u8>>,
    ) -> Self {
        Self::start_with_options(definition, repair_workspace, repair_files, None)
    }

    fn start_with_repairs_and_prompt_gate(
        definition: ProjectEnvironmentDefinition,
        repair_workspace: Option<PathBuf>,
        repair_files: BTreeMap<String, Vec<u8>>,
    ) -> (Self, Arc<AtomicBool>) {
        let release_prompt = Arc::new(AtomicBool::new(false));
        let fixture = Self::start_with_options(
            definition,
            repair_workspace,
            repair_files,
            Some(Arc::clone(&release_prompt)),
        );
        (fixture, release_prompt)
    }

    fn start_with_options(
        definition: ProjectEnvironmentDefinition,
        repair_workspace: Option<PathBuf>,
        repair_files: BTreeMap<String, Vec<u8>>,
        release_prompt: Option<Arc<AtomicBool>>,
    ) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("provider fixture should bind");
        listener
            .set_nonblocking(true)
            .expect("provider fixture should be nonblocking");
        let address = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(UtilityProviderState::default()));
        let stop_for_server = stop.clone();
        let state_for_server = state.clone();
        let repairs = repair_workspace.map(|workspace| (workspace, repair_files));
        let join = thread::spawn(move || {
            while !stop_for_server.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let stop = stop_for_server.clone();
                        let state = state_for_server.clone();
                        let definition = definition.clone();
                        let repairs = repairs.clone();
                        let release_prompt = release_prompt.clone();
                        thread::spawn(move || {
                            serve_provider_request(
                                stream,
                                stop,
                                state,
                                definition,
                                repairs,
                                release_prompt,
                            );
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

    fn trace_entries(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("provider fixture state should not poison")
            .trace
            .clone()
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

#[cfg(target_os = "linux")]
#[tokio::test]
async fn pr364_opencode_discovery_rejects_source_mcp_and_restores_ordinary_config() {
    let _environment_lock = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-opencode-discovery-mcp-lifecycle-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let home = root.join("kernel-home");
    let provider_home = root.join("provider-home");
    let workspace = root.join("worker-worktree");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&provider_home).unwrap();
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

    let validation_command =
        "touch validation-started; while [ ! -f validation-release ]; do sleep 0.01; done; command -v sh"
            .to_string();
    let target_platform = actual_worker_platform();
    let definition = ProjectEnvironmentDefinition {
        schema_version: 1,
        origin: ProjectEnvironmentDefinitionOrigin::UtilityGenerated,
        source: ProjectEnvironmentDefinitionSource::Commands,
        target_platform: target_platform.clone(),
        source_path: None,
        inputs: Vec::new(),
        path_entries: Vec::new(),
        setup_steps: vec![ProjectEnvironmentSetupStep {
            kind: ProjectEnvironmentSetupStepKind::Command,
            command: "touch setup-started; command -v sh".to_string(),
        }],
        validation_commands: vec![validation_command.clone()],
    };
    let (provider_fixture, release_prompt) =
        UtilityProviderFixture::start_with_repairs_and_prompt_gate(
            definition.clone(),
            None,
            BTreeMap::new(),
        );

    let runtime_mcp_url = "http://127.0.0.1:43120/mcp";
    let granted_mcp_url = "http://127.0.0.1:43121/mcp";
    let original_provider_home = provider_home.display().to_string();
    let original_provider_path = "/usr/bin:/bin".to_string();
    let source_mcp_config = serde_json::json!({
        "mcp": {
            "chariox": {
                "type": "remote",
                "url": runtime_mcp_url,
                "enabled": true,
                "oauth": false,
                "timeout": 300000,
                "headers": {"Authorization": "Bearer token-123"}
            },
            "mutating-tool": {
                "type": "remote",
                "url": granted_mcp_url,
                "enabled": true,
                "oauth": false,
                "timeout": 45000,
                "headers": {}
            }
        }
    })
    .to_string();

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
    let launch_request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "opencode",
        "opencode",
        "default",
        "opencode/test-model",
    )
    .with_agent_id(agent.id())
    .with_owner_user_id("user-1")
    .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
        runtime_mcp_url,
        "token-123",
    ))
    .with_mcp_servers(vec![crate::mcp::CharioxMcpServerConfig::streamable_http(
        "mutating-tool",
        granted_mcp_url,
    )]);
    let mut pty_env = BTreeMap::from([
        ("HOME".into(), original_provider_home.clone()),
        ("PATH".into(), original_provider_path.clone()),
        ("GIT_SSH_COMMAND".into(), "selected-ssh".into()),
        ("SSH_AUTH_SOCK".into(), "selected-agent".into()),
    ]);
    pty_env.insert("OPENCODE_CONFIG_CONTENT".into(), source_mcp_config.clone());
    let mut provider_run = RuntimeProviderRun::new(
        "utility-provider-run",
        &launch_request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode-discovery-mcp-fixture".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: vec!["-c".into(), "sleep 60".into()],
            pty_env,
            pty_env_remove: Vec::new(),
            working_directory: Some(workspace.clone()),
            structured_endpoint: Some(provider_fixture.address()),
        },
    );
    provider_run.mark_running();
    assert_eq!(
        provider_opencode_mcp_names(provider_run.pty_env()),
        vec!["chariox".to_string(), "mutating-tool".to_string()],
        "the source ordinary provider run must begin with both MCP servers"
    );
    assert_eq!(
        provider_run.runtime_mcp_server_url(),
        Some(runtime_mcp_url),
        "the source ordinary provider run must retain its runtime MCP binding"
    );
    app.providers_mut().insert_run_for_test(provider_run);
    let runtime =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 1)
            .runtime_state();

    let start = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::StartProjectEnvironmentSetup(StartProjectEnvironmentSetupRequest {
                operation_id: "setup-opencode-discovery-mcp".into(),
                project_id,
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                target_worker_id: "worker-machine".into(),
                target_platform,
                definition: None,
                validation_commands: vec![validation_command],
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

    let prompt_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let trace = provider_fixture.trace_entries();
        if trace.iter().any(|entry| entry.contains("prompt_async")) {
            break;
        }
        let status = response_status(
            get_setup_status(
                &runtime,
                "setup-opencode-discovery-mcp",
                "user-1",
                "OpenCode discovery prompt gate",
            )
            .await,
        );
        assert!(
            !matches!(
                status.phase,
                ProjectEnvironmentSetupPhase::Failed | ProjectEnvironmentSetupPhase::Ready
            ),
            "setup settled before its discovery prompt: {status:?}"
        );
        assert!(
            Instant::now() < prompt_deadline,
            "OpenCode discovery prompt did not reach the fixture: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let discovery_run = runtime
        .owned
        .provider_store
        .get_run("utility-provider-run")
        .expect("discovery provider run should remain available while prompt is gated");
    assert!(
        discovery_run.read_only_discovery(),
        "the gated utility prompt must run on the read-only discovery restart"
    );
    let discovery_pid = {
        let app = runtime.app.lock().await;
        app.pty()
            .process_id("utility-provider-run")
            .ok()
            .flatten()
            .expect("discovery provider child should be running")
    };
    let discovery_environment = provider_child_environment(discovery_pid);
    assert_eq!(
        provider_opencode_mcp_names(&discovery_environment),
        Vec::<String>::new(),
        "read-only discovery must not expose source runtime/granted MCP config"
    );
    let discovery_trace = provider_fixture.trace_entries();
    assert!(
        discovery_trace
            .iter()
            .all(|entry| !(entry.starts_with("POST /mcp/") && entry.ends_with("/connect"))),
        "read-only discovery must not connect source MCP servers before its prompt: {discovery_trace:?}"
    );
    let mcp_connect_count = |trace: &[String]| {
        trace
            .iter()
            .filter(|entry| entry.starts_with("POST /mcp/") && entry.ends_with("/connect"))
            .count()
    };
    assert_eq!(
        mcp_connect_count(&discovery_trace),
        0,
        "read-only discovery must have zero MCP connections: {discovery_trace:?}"
    );

    release_prompt.store(true, Ordering::Release);
    let validation_started = workspace.join("validation-started");
    let validation_start_deadline = Instant::now() + Duration::from_secs(10);
    while !validation_started.exists() {
        assert!(
            Instant::now() < validation_start_deadline,
            "gated validation did not start after discovery: {}",
            provider_fixture.diagnostics()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (gated_ordinary_run, _gated_ordinary_pid, gated_environment) =
        wait_for_ordinary_provider_child(&runtime, "utility-provider-run").await;
    assert_eq!(
        gated_environment.get("HOME").map(String::as_str),
        Some(original_provider_home.as_str()),
        "ordinary restoration must restore the original HOME while validation is gated"
    );
    assert_eq!(
        gated_environment.get("PATH").map(String::as_str),
        Some(original_provider_path.as_str()),
        "ordinary restoration must restore the original PATH while validation is gated"
    );
    assert_eq!(
        gated_ordinary_run.pty_env().get("HOME").map(String::as_str),
        Some(original_provider_home.as_str())
    );
    assert_eq!(
        gated_ordinary_run.pty_env().get("PATH").map(String::as_str),
        Some(original_provider_path.as_str())
    );
    let gated_trace = provider_fixture.trace_entries();
    assert_eq!(
        mcp_connect_count(&gated_trace),
        2,
        "the original ordinary restore should reconnect both source MCP servers before validation completes: {gated_trace:?}"
    );

    let validation_release = workspace.join("validation-release");
    std::fs::write(&validation_release, b"").unwrap();
    let ready = wait_for_ready(&runtime, "setup-opencode-discovery-mcp", &provider_fixture).await;
    assert_eq!(
        ready.phase,
        ProjectEnvironmentSetupPhase::Ready,
        "ordinary restoration must complete setup"
    );

    let (ordinary_run, _ordinary_pid, ordinary_environment) =
        wait_for_ordinary_provider_child(&runtime, "utility-provider-run").await;
    assert!(
        !ordinary_run.read_only_discovery(),
        "ordinary provider restoration must leave discovery mode"
    );
    assert_eq!(
        provider_opencode_mcp_names(&ordinary_environment),
        vec!["chariox".to_string(), "mutating-tool".to_string()],
        "ordinary provider child must regain the source MCP config"
    );
    assert_eq!(
        provider_opencode_mcp_names(ordinary_run.pty_env()),
        vec!["chariox".to_string(), "mutating-tool".to_string()],
        "ordinary provider run must retain the source MCP config"
    );
    let (prepared_home, prepared_path) = ordinary_run
        .preparation_environment()
        .expect("Ready ordinary run must carry the validated preparation environment");
    let prepared_home_path = std::path::Path::new(&prepared_home);
    assert!(
        prepared_home_path.starts_with(&root),
        "validated preparation HOME must remain in the worker root: {prepared_home}"
    );
    assert_ne!(
        prepared_home, original_provider_home,
        "Ready must rebind the ordinary provider away from its original HOME"
    );
    let prepared_path_prefix = format!("{prepared_home}/.local/bin:{prepared_home}/.cargo/bin:");
    assert!(
        prepared_path.starts_with(&prepared_path_prefix),
        "validated PATH must expose preparation-home tool directories: {prepared_path}"
    );
    assert_eq!(
        ordinary_environment.get("HOME").map(String::as_str),
        Some(prepared_home.as_str()),
        "Ready ordinary child must see the validated preparation HOME"
    );
    assert_eq!(
        ordinary_environment.get("PATH").map(String::as_str),
        Some(prepared_path.as_str()),
        "Ready ordinary child must see the validated preparation PATH"
    );
    assert_eq!(
        ordinary_run.pty_env().get("HOME").map(String::as_str),
        Some(prepared_home.as_str())
    );
    assert_eq!(
        ordinary_run.pty_env().get("PATH").map(String::as_str),
        Some(prepared_path.as_str())
    );
    let final_trace = provider_fixture.trace_entries();
    let prompt_index = final_trace
        .iter()
        .position(|entry| entry.contains("prompt_async"))
        .expect("the gated discovery prompt should be recorded");
    let reconnects = final_trace
        .iter()
        .enumerate()
        .filter(|(index, entry)| {
            *index > prompt_index && entry.starts_with("POST /mcp/") && entry.ends_with("/connect")
        })
        .count();
    assert_eq!(
        reconnects, 4,
        "ordinary restoration and the validated Ready rebind should reconnect both source MCP servers per phase: {final_trace:?}"
    );
    assert_eq!(
        mcp_connect_count(&final_trace),
        4,
        "the integrated lifecycle should emit two reconnects for each ordinary phase: {final_trace:?}"
    );

    drop(provider_fixture);
}

#[cfg(target_os = "linux")]
fn provider_opencode_mcp_names(environment: &BTreeMap<String, String>) -> Vec<String> {
    let Some(config) = environment
        .get("OPENCODE_CONFIG_CONTENT")
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
    else {
        return Vec::new();
    };
    let mut names = config
        .get("mcp")
        .and_then(serde_json::Value::as_object)
        .map(|mcp| mcp.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    names.sort();
    names
}

#[cfg(unix)]
fn serve_provider_request(
    mut stream: TcpStream,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<UtilityProviderState>>,
    definition: ProjectEnvironmentDefinition,
    repairs: Option<(PathBuf, BTreeMap<String, Vec<u8>>)>,
    release_prompt: Option<Arc<AtomicBool>>,
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
        ("POST", path) if path.starts_with("/mcp/") && path.ends_with("/connect") => {
            write_json_response(&mut stream, 200, &serde_json::json!(true));
        }
        ("GET", "/mcp") => {
            write_json_response(
                &mut stream,
                200,
                &serde_json::json!({
                    "chariox": {"status": "connected"},
                    "mutating-tool": {"status": "connected"}
                }),
            );
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
            if let Some(release_prompt) = release_prompt {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !release_prompt.load(Ordering::Acquire)
                    && !stop.load(Ordering::Acquire)
                    && Instant::now() < deadline
                {
                    thread::sleep(Duration::from_millis(2));
                }
                if !release_prompt.load(Ordering::Acquire) {
                    return;
                }
            }
            if let Some((workspace, files)) = repairs.as_ref() {
                for (relative_path, contents) in files {
                    let target = workspace.join(relative_path);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)
                            .expect("repair fixture should create target parent directories");
                    }
                    std::fs::write(&target, contents)
                        .expect("repair fixture should materialize target input bytes");
                    state
                        .lock()
                        .expect("provider fixture state should not poison")
                        .record(format!("repair {relative_path}"));
                }
            }
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
