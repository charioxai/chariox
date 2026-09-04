use super::*;
use crate::app::RemoteLeaseRuntime;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
};
use crate::transport::relay_peer::RelayPeerEvent;

struct FixtureCleanup {
    root: std::path::PathBuf,
    providers: Option<crate::provider::ProviderProcessServiceStore>,
}

impl Drop for FixtureCleanup {
    fn drop(&mut self) {
        if let Some(providers) = self.providers.as_mut() {
            for run in providers.list_runs() {
                providers.clear_runtime(run.id());
            }
        }
        std::fs::remove_dir_all(&self.root).expect("remove isolated leased-output fixture");
    }
}

#[tokio::test]
async fn leased_claude_failure_reaches_home_projection_without_terminal_polling() {
    let root = std::env::temp_dir().join(format!(
        "chariox-leased-output-{}-{}",
        std::process::id(),
        rand::random::<u64>(),
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut cleanup = FixtureCleanup {
        root: root.clone(),
        providers: None,
    };
    let mut config = crate::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.local_socket_path = root.join("kernel.sock");
    config = config.with_session_history_root(root.join("history"));
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.join("events.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    let mut app = DaemonApp::bootstrap(config).expect("worker bootstrap");
    cleanup.providers = Some(app.providers().clone());
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease("home", "room", "agent", false, "owner")
        .expect("execution lease");
    let leased = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent_from_base_directory(
            &root,
            &lease.id,
            "claude",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased Claude agent");

    // Replace only the external executable. Exercise real Claude stream-json
    // submission, parsing, runtime ticks and relay projection without a model.
    let received = root.join("received");
    let request = LaunchProviderRequest::new(
        &leased.backing_session_id,
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(&leased.backing_agent_id);
    let mut run = RuntimeProviderRun::new("leased-claude-fixture", &request, ProviderLaunchResult {
        // Claude's real stream-JSON launch uses External, not the PTY-managed mode.
        endpoint_mode: AgentEndpointMode::External,
        process_label: "claude-stream-fixture".to_string(),
        pty_target: None,
        pty_program: Some("/bin/sh".to_string()),
        pty_args: vec!["-c".to_string(), r#"
while IFS= read -r line; do
    : > "$CHARIOX_TEST_RECEIVED"
    printf '%s\n' '{"type":"result","subtype":"error_during_execution","is_error":true,"error":"Fixture Claude login required"}'
done
"#.to_string()],
        pty_env: [("CHARIOX_TEST_RECEIVED".to_string(), received.display().to_string())].into(),
        pty_env_remove: Vec::new(),
        working_directory: Some(root.clone()),
        structured_endpoint: Some("stdio://claude-fixture".to_string()),
    });
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.providers_mut()
        .initialize_runtime(&run)
        .expect("fixture native protocol binding");
    app.update_provider_run_projection(run.clone());
    app.sessions_mut()
        .set_active_provider_run(&leased.backing_session_id, Some(run.id().to_string()))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let (run_id, outcome) = runtime
        .submit_relay_leased_prompt(
            &leased.id,
            "inspect the Room",
            "",
            Vec::new(),
            None,
            Some(crate::transport::relay_peer::RemoteGitTurnContext {
                home_session_id: "room".to_string(),
                home_agent_id: "agent".to_string(),
                home_prompt_id: "home-prompt".to_string(),
                home_turn_id: "home-prompt".to_string(),
                source_attachment_id: None,
                workspace_live_sync_mode: None,
                prompt_origin: Some(crate::session::PromptOrigin::Chariox),
                external_provider: None,
                external_provider_session_id: None,
                external_provider_turn_id: None,
                prompt_summary: "inspect the Room".to_string(),
            }),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .await
        .expect("leased prompt accepted");
    assert_eq!(run_id, run.id());
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut projected_errors = Vec::new();
    loop {
        runtime.pump_transport_runtime().await;
        // Same drain request as the home kernel's active-prompt loop. No local
        // terminal client is attached to wake the provider-output pump.
        if let Some((_, event)) = runtime
            .drain_relay_leased_runtime_projection(&leased.id, &run_id, true, true)
            .await
            .unwrap()
        {
            let RelayPeerEvent::LeasedRuntimeProjection { output_chunks, .. } = event;
            projected_errors.extend(
                output_chunks
                    .into_iter()
                    .filter(|chunk| {
                        chunk.kind == crate::terminal::TerminalOutputKind::ProviderError
                    })
                    .map(|chunk| String::from_utf8_lossy(&chunk.bytes).into_owned()),
            );
        }
        if !projected_errors.is_empty() || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        received.is_file(),
        "native provider must have received the prompt and emitted its error; projected errors: {projected_errors:?}"
    );
    assert!(projected_errors.iter().any(|message| message.contains("Fixture Claude login required")),
        "accepted leased provider error must reach home projection without a terminal-output request: {projected_errors:?}");
    let session = runtime
        .owned
        .session_store
        .get_session(&leased.backing_session_id)
        .unwrap();
    assert!(
        session
            .active_prompt_for_agent(&leased.backing_agent_id)
            .is_none(),
        "failed turn must settle"
    );
}
