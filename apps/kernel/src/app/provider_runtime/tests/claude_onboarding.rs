//! MP-08 / MP-09 / MP-10 / MP-11: a launch-bound HOME owns the Claude profile it starts.
use crate::config::DaemonConfig;
use crate::provider::{
    LaunchProviderRequest, ProviderClientInterface, ProviderCredentialEnvironment,
};
use crate::session::SessionAgentDefaults;

fn bound_home_request(
    session_id: &str,
    agent_id: &str,
    home: &std::path::Path,
) -> LaunchProviderRequest {
    let mut credentials = ProviderCredentialEnvironment::default();
    credentials.insert("HOME", zeroize::Zeroizing::new(home.display().to_string()));
    LaunchProviderRequest::new(session_id, "claude", "claude-headless", "default", "haiku")
        .with_agent_id(agent_id)
        .with_client_interface(ProviderClientInterface::NativeTui)
        .with_provider_credential_env(credentials)
}

#[test]
fn headless_launch_initializes_the_project_bound_home() {
    crate::test_support::isolated_env_test!();
    let _claude = crate::test_support::TestClaudeExecutable::new();
    let root = crate::test_support::TestWorktree::new("headless-bound-home");
    let ambient = root.path().join("ambient-home");
    let bound = root.path().join("bound-home");
    std::fs::create_dir_all(&ambient).unwrap();
    std::fs::create_dir_all(&bound).unwrap();
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::set_var("HOME", &ambient);
    let mut app = crate::test_support::bootstrap_authenticated_app(DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let worktree = crate::test_support::TestWorktree::new("headless-bound-home-session");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            worktree
                .session_request()
                .with_agent_defaults(SessionAgentDefaults::new("claude").with_model("haiku")),
        )
        .expect("session create should succeed");

    for detached in [false, true] {
        let request = bound_home_request(session.id(), agent.id(), &bound);
        let run = if detached {
            app.launch_provider_detached(request)
        } else {
            app.launch_provider(request)
        }
        .expect("headless launch should start");
        assert_eq!(run.provider(), "claude-headless");
        let state: serde_json::Value = serde_json::from_slice(
            &std::fs::read(bound.join(".claude.json"))
                .unwrap_or_else(|_| panic!("detached={detached}: bound HOME was not initialized")),
        )
        .unwrap();
        assert_eq!(state["hasCompletedOnboarding"], true);
        assert!(
            !ambient.join(".claude.json").exists(),
            "detached={detached}: ambient HOME must stay untouched"
        );
        std::fs::remove_file(bound.join(".claude.json")).unwrap();
    }
    app.shutdown_cleanup().unwrap();
}
