use super::*;

#[derive(Clone, Copy)]
enum ReceivingAccount {
    NewCopy,
    CurrentPhysicalCopy,
    UnverifiedCopy,
}

#[tokio::test]
async fn leased_prompt_profile_change_does_not_reseed_the_old_home_thread() {
    assert_codex_copy_admission(ReceivingAccount::NewCopy).await;
}

#[tokio::test]
async fn leased_codex_copy_keeps_the_current_physical_profile_session() {
    assert_codex_copy_admission(ReceivingAccount::CurrentPhysicalCopy).await;
}

#[tokio::test]
async fn leased_codex_copy_requires_an_authenticated_receiving_profile() {
    assert_codex_copy_admission(ReceivingAccount::UnverifiedCopy).await;
}

async fn assert_codex_copy_admission(receiving: ReceivingAccount) {
    crate::test_support::isolated_env_test!();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    // Hold native handshaking until assertions inspect the admitted Codex run.
    config.provider_runtime_init_delay_ms = 60_000;
    let executable_dir = crate::test_support::TestTempDir::new("codex-account-admission");
    let executable = executable_dir.path().join("codex");
    // The existing delivery probe consumes process I/O; the real adapter still plans
    // its executable, endpoint, account environment and native checkpoint.
    std::fs::write(&executable, "#!/bin/sh\nexit 97\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("CHARIOX_CODEX_BIN", &executable);
    let process_delivery =
        crate::provider::ProviderCredentialDeliveryProbe::install("provider-run-1", &[]);
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let registry = app.provider_account_profile_registry();
    let original = registry
        .create_managed(owner, "codex", "home account")
        .unwrap();
    let copied = registry
        .create_managed(owner, "codex", "receiving account")
        .unwrap();
    if !matches!(receiving, ReceivingAccount::UnverifiedCopy) {
        registry
            .update_observation(
                owner,
                "codex",
                &copied.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
    }
    let leased = {
        let mut runtime = RemoteLeaseRuntime::new(&mut app);
        let execution = runtime
            .create_execution_lease("home-kernel", "home-session", "home-agent", false, owner)
            .unwrap();
        runtime
            .create_leased_agent(
                &execution.id,
                "codex",
                &original.profile_id,
                Some("model".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap()
    };
    let previous_thread = crate::provider::ProviderResumeState::from_codex_thread_id(
        "previous-physical-worker-thread",
    );
    app.agents_mut()
        .set_agent_provider_resume_state(&leased.backing_agent_id, previous_thread.clone())
        .unwrap();
    let previous_account = if matches!(receiving, ReceivingAccount::CurrentPhysicalCopy) {
        &copied.profile_id
    } else {
        &original.profile_id
    };
    let request = crate::provider::LaunchProviderRequest::new(
        &leased.backing_session_id,
        "codex",
        "codex",
        previous_account,
        "model",
    )
    .with_agent_id(&leased.backing_agent_id)
    .with_owner_user_id(owner)
    .with_resume_state(previous_thread.clone());
    let mut previous = crate::provider::RuntimeProviderRun::new(
        "previous-codex-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "previous-codex-account-fixture".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    previous.mark_running();
    app.providers_mut().insert_run_for_test(previous);
    let receipt = crate::account_profile::ManagedContextProviderAccountReceipt {
        context_id: "copy".into(),
        package_sha256: "package".into(),
        materialization_sha256: "materialization".into(),
        provider: "codex".into(),
        profile_id: copied.profile_id.clone(),
    };
    registry
        .record_transfer_origin(owner, &receipt, "home-kernel", &original.profile_id)
        .unwrap();
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile {
        provider: "codex".into(),
        account_profile: original.profile_id.clone(),
        model: Some("model".into()),
        effort: None,
    };
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let result = router
        .relay_submit_leased_prompt(
            &leased.id,
            profile,
            Some(crate::provider::ProviderResumeState::from_codex_thread_id(
                "stale-home-thread",
            )),
            "continue on the receiving Codex account",
            "",
            Vec::new(),
            None,
            None,
            Vec::new(),
            crate::extension::RemoteExtensionManifest::default(),
            Default::default(),
        )
        .await;
    if matches!(receiving, ReceivingAccount::UnverifiedCopy) {
        assert!(
            matches!(result, Err(crate::error::DaemonError::LocalTransport { operation, .. })
            if operation == crate::transport::relay_peer::ADMIT_LEASED_PROMPT_ACCOUNT_OPERATION),
            "native Codex authentication must refuse the unverified receiving account"
        );
        assert!(
            app.lock()
                .await
                .providers()
                .get_run("provider-run-1")
                .is_err(),
            "an unverified copy must not create the receiving provider run"
        );
        return;
    }
    let (run_id, _) = result.unwrap();
    let app = app.lock().await;
    let run = app.providers().get_run(&run_id).unwrap();
    assert_eq!(run.provider(), "codex");
    assert_eq!(run.adapter_key(), "codex");
    assert_eq!(
        run.endpoint_mode(),
        crate::provider::AgentEndpointMode::Managed
    );
    assert_eq!(
        run.pty_program(),
        Some(executable.to_string_lossy().as_ref())
    );
    assert!(run.structured_endpoint().is_some());
    assert!(
        process_delivery.observed_exactly("pty_spawn"),
        "real Codex launch must reach the process delivery boundary"
    );
    assert_eq!(
        run.account_profile(),
        copied.profile_id,
        "the home selection must resolve through the real transfer binding"
    );
    if matches!(receiving, ReceivingAccount::CurrentPhysicalCopy) {
        assert_eq!(
            run.resume_state(),
            &previous_thread,
            "the receiving profile keeps its valid worker session over stale home state"
        );
    } else {
        assert!(
            run.resume_state().is_empty(),
            "the incoming home thread belongs to the previous physical Codex profile"
        );
    }
}
