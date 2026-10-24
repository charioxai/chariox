// MP-08 / MP-10 / MP-11: admitted accounts remain visible and stable across clients.
use super::*;

#[tokio::test]
async fn new_session_binds_stable_default_account_before_provider_launch() {
    let worktree = crate::test_support::TestWorktree::new("stable-session-account");
    let app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let registry = app.provider_account_profile_registry();
    let first = registry
        .create_managed("local", "claude", "First session account")
        .unwrap();
    registry
        .set_default("local", "claude", &first.profile_id)
        .unwrap();
    let state = super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
    let create = || {
        crate::session::CreateSessionRequest::new(
            worktree.path().display().to_string(),
            worktree.path().display().to_string(),
        )
        .with_agent_defaults(crate::session::SessionAgentDefaults::new("claude"))
    };
    let response = state.create_session_response(create()).await.unwrap();
    let crate::local::LocalDaemonResponse::SessionCreated { session, agent } = response else {
        panic!("created session")
    };
    assert_eq!(agent.account_profile(), Some(first.profile_id.as_str()),
        "native and ordinary clients must receive a stable account binding, not the hidden default sentinel");
    let second = registry
        .create_managed("local", "claude", "Second session account")
        .unwrap();
    registry
        .set_default("local", "claude", &second.profile_id)
        .unwrap();
    assert_eq!(
        state
            .owned
            .agent_store
            .get_agent(agent.id())
            .unwrap()
            .account_profile(),
        Some(first.profile_id.as_str())
    );
    let spawned = state
        .spawn_agent(crate::agent::CreateAgentRequest::new(
            session.id(),
            "claude",
        ))
        .await
        .unwrap();
    assert_eq!(spawned.account_profile(), Some(second.profile_id.as_str()));
}
