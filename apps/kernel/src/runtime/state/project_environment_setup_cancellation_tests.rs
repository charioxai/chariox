//! Cancel through the public kernel request while real worker validation runs.
use super::*;

#[cfg(unix)]
#[tokio::test]
async fn cancel_request_stops_in_flight_worker_validation() {
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};
    use crate::runtime::router::CommandRouter;

    let _environment_lock = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-validation-cancel-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let home = root.join("kernel-home");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let receipt = root.join("receipt.json");
    std::fs::write(
        &receipt,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1, "status": "confirmed", "allocationId": "worker-1",
            "machineId": "worker-machine", "kernelId": "worker-kernel",
            "relayPublicKey": "worker-public-key",
            "runtimeReleaseDigest": format!("sha256:{}", "a".repeat(64)),
            "homeCaller": {"accountId": "account-1", "userId": "user-1",
                "realmId": "realm-1", "machineId": "home-machine",
                "kernelId": "home-kernel", "relayPublicKey": "home-public-key"},
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
            "api_url": "https://staging.chariox.com", "email": "owner@example.test",
            "account_id": "account-1", "user_id": "user-1", "account_slug": "account-1",
            "realm_id": "realm-1", "relay_url": "wss://relay.example.test",
            "issuer_id": "issuer-1", "machine_id": "worker-machine",
            "machine_credential": format!("mcred_{}", "c".repeat(40))
        }))
        .unwrap(),
    );
    ensure_worker_validation_boundary(&config).expect("fixture is a confirmed worker");
    let app = crate::DaemonApp::bootstrap(config).unwrap();
    let mut execution = super::tests::execution();
    execution.workspace_id = workspace.display().to_string();
    execution.target_worker_id = "worker-machine".into();
    execution.target_platform = actual_worker_platform();
    let mut definition = execution.definition.clone().unwrap();
    definition.target_platform = execution.target_platform.clone();
    definition.validation_commands = vec!["touch started; sleep 2; touch should-not-exist".into()];
    execution.definition = Some(definition.clone());
    let request = LaunchProviderRequest::new("session-1", "codex", "codex", "default", "default")
        .with_agent_id("agent-1")
        .with_owner_user_id("user-1");
    let mut provider_run = RuntimeProviderRun::new(
        "validation-run",
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "worker-fixture".into(),
            pty_target: None,
            pty_program: Some("/bin/sh".into()),
            pty_args: Vec::new(),
            pty_env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            pty_env_remove: Vec::new(),
            working_directory: Some(workspace.clone()),
            structured_endpoint: None,
        },
    );
    provider_run.mark_running();
    app.providers_mut()
        .insert_run_for_test(provider_run.clone());
    let runtime =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 1)
            .runtime_state();
    runtime
        .owned
        .project_environment_setups
        .begin(execution.clone())
        .unwrap();
    runtime
        .owned
        .project_environment_setups
        .update("setup-1", 1, |entry| {
            entry.status.phase = ProjectEnvironmentSetupPhase::Validating;
        });
    let validation_runtime = runtime.clone();
    let validation = tokio::spawn(async move {
        validation_runtime
            .validate_definition_on_worker(&execution, 1, &definition, &provider_run)
            .await
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !workspace.join("started").exists()
        && !validation.is_finished()
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let started = workspace.join("started").exists();
    let cancelled_at = Instant::now();
    let response = runtime
        .execute_project_environment_setup_request(
            LocalDaemonRequest::CancelProjectEnvironmentSetup(
                CancelProjectEnvironmentSetupRequest {
                    operation_id: "setup-1".into(),
                    session_id: "session-1".into(),
                },
            ),
            "user-1",
        )
        .await;
    // Always join the bounded command before asserting, including on the red path.
    let result = validation.await.unwrap();
    let elapsed = cancelled_at.elapsed();
    assert!(started, "validation never reached the command: {result:?}");
    assert!(matches!(
        response,
        Ok(LocalDaemonResponse::ProjectEnvironmentSetupCancelled { .. })
    ));
    assert!(
        !workspace.join("should-not-exist").exists(),
        "cancelled validation still mutated the workspace"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "cancellation waited for the command: {elapsed:?}"
    );
}
