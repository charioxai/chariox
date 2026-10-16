use super::*;
use crate::durable_state::worker_prompt_receipts::WorkerPromptReceiptPhase;
use crate::transport::relay_peer::RemoteGitTurnContext;

/// A worker whose account needs a new sign-in refuses the prompt before launching a run and
/// records the rejection, so the home gets a definitive answer instead of an unresolved admission.
#[tokio::test]
async fn leased_prompt_is_rejected_before_launch_when_the_account_needs_sign_in() {
    crate::test_support::isolated_env_test!();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "home-session",
            "home-agent",
            false,
            crate::session::DEFAULT_LOCAL_USER_ID,
        )
        .unwrap();
    let leased = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "codex",
            "default",
            Some("gpt-6-luna".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let profile = crate::transport::relay_peer::RelayAgentExecutionProfile::from(
        &app.agents().get_agent(&leased.backing_agent_id).unwrap(),
    );
    let context = RemoteGitTurnContext {
        home_session_id: lease.home_session_id.clone(),
        home_agent_id: lease.home_agent_id.clone(),
        home_prompt_id: "home-prompt-1".to_string(),
        home_turn_id: "home-turn-1".to_string(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "try once more".to_string(),
    };
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(app.clone(), 2);
    let error = router
        .relay_submit_leased_prompt(
            &leased.id,
            profile,
            "try once more",
            "",
            Vec::new(),
            None,
            Some(context),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
            None,
        )
        .await
        .expect_err("an unauthenticated worker account must refuse the prompt");
    assert!(
        error.to_string().contains("is not authenticated"),
        "{error}"
    );
    let app = app.lock().await;
    let receipt = app
        .worker_prompt_receipts
        .get(&leased.id, "home-prompt-1")
        .expect("the refused admission is recorded");
    assert_eq!(receipt.receipt.phase, WorkerPromptReceiptPhase::Rejected);
    assert!(app
        .providers()
        .get_run_for_agent(&leased.backing_session_id, &leased.backing_agent_id)
        .is_none());
}
