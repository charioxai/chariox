//! A run on a leased worker uses the Vault of the home kernel, so the worker's own
//! account identity has no say over it.

const RUN_TOKEN: &str = "leased-vault-run-token";

#[tokio::test]
async fn an_agent_on_a_slice_reaches_the_home_vault_whatever_account_the_slice_runs_as() {
    crate::test_support::isolated_env_test!();
    let workspace = crate::test_support::TestWorktree::new("vault-leased-workspace");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    // The slice is enrolled as a different account than the Room's owner.
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "slice-account".into(),
        ..Default::default()
    });
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let leased = {
        let mut leases = crate::app::RemoteLeaseRuntime::new(&mut app);
        let lease = leases
            .create_execution_lease("home", "home-session", "home-agent", false, "room-owner")
            .unwrap();
        leases
            .create_leased_agent_from_base_directory(
                workspace.path(),
                &lease.id,
                "dev-stub",
                "default",
                Some("default".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap()
    };
    let launch = crate::provider::LaunchProviderRequest::new(
        &leased.backing_session_id,
        "dev-stub",
        "dev-stub",
        "default",
        "default",
    )
    .with_agent_id(&leased.backing_agent_id);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "leased-vault-run",
        &launch,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: Some(workspace.path().to_owned()),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    run.set_runtime_mcp_auth_token(Some(RUN_TOKEN.into()));
    app.providers_mut().insert_run_for_test(run.clone());
    let state = super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
    state.owned.provider_run_projection.update(run);

    let outcome = state
        .dispatch_authenticated_runtime_tool_call(
            RUN_TOKEN,
            "chariox.vault.use_env",
            serde_json::json!({
                "argv": ["true"],
                "origin": "https://api.example.com",
                "env": {"API_KEY": "home_key"},
            }),
        )
        .await;
    let message = match outcome {
        Err(error) => error.to_string(),
        Ok(result) => panic!("the home kernel is out of reach, yet the call ended: {result:?}"),
    };
    assert!(
        !message.contains("no longer belongs"),
        "the slice's own account identity refused a Vault use the home kernel owns: {message}"
    );
}
