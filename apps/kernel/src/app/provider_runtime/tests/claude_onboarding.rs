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
        assert_eq!(
            state["projects"][worktree.path().canonicalize().unwrap().to_str().unwrap()]
                ["hasTrustDialogAccepted"],
            true,
            "detached={detached}: the agent's own workspace is trusted before Claude starts"
        );
        assert_eq!(
            state["projects"].as_object().unwrap().len(),
            1,
            "detached={detached}: no other folder is trusted"
        );
        assert!(
            !ambient.join(".claude.json").exists(),
            "detached={detached}: ambient HOME must stay untouched"
        );
        std::fs::remove_file(bound.join(".claude.json")).unwrap();
    }
    app.shutdown_cleanup().unwrap();
}

#[test]
fn a_launch_trusts_only_the_agents_workspace_and_keeps_what_claude_wrote() {
    crate::test_support::isolated_env_test!();
    let _claude = crate::test_support::TestClaudeExecutable::new();
    let root = crate::test_support::TestWorktree::new("trust-only-workspace");
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join(".claude.json"),
        r#"{"theme":"dark","projects":{"/elsewhere":{"hasTrustDialogAccepted":false,"allowedTools":["Bash"]}}}"#,
    )
    .unwrap();
    let mut app = crate::test_support::bootstrap_authenticated_app(DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let worktree = crate::test_support::TestWorktree::new("trust-only-workspace-session");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            worktree
                .session_request()
                .with_agent_defaults(SessionAgentDefaults::new("claude").with_model("haiku")),
        )
        .expect("session create should succeed");

    let other = root.path().join("other-folder");
    std::fs::create_dir_all(&other).unwrap();
    let refused = app
        .launch_provider(
            bound_home_request(session.id(), agent.id(), &home).with_working_directory(other),
        )
        .expect_err("a launch outside the agent's workspace must not start");
    assert!(
        refused.to_string().contains("not the agent's workspace"),
        "{refused}"
    );
    let untouched = std::fs::read_to_string(home.join(".claude.json")).unwrap();
    assert!(!untouched.contains("other-folder"), "{untouched}");

    app.launch_provider(bound_home_request(session.id(), agent.id(), &home))
        .expect("launch in the agent's own workspace should start");
    let trusted_notice = format!(
        "Chariox trusted {} for Claude Code.",
        worktree.path().canonicalize().unwrap().display()
    );
    let notices = |app: &crate::app::DaemonApp| -> usize {
        let session = app.sessions.get_session(session.id()).unwrap();
        app.load_session_history_entries(&session, Some(agent.id()))
            .unwrap()
            .into_iter()
            .filter(|entry| entry.text == trusted_notice)
            .count()
    };
    assert_eq!(notices(&app), 1, "the person sees the folder being trusted");
    app.launch_provider_detached(bound_home_request(session.id(), agent.id(), &home))
        .expect("a later cold start should start");
    assert_eq!(
        notices(&app),
        1,
        "an already trusted folder is not announced again"
    );
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(state["theme"], "dark");
    assert_eq!(
        state["projects"]["/elsewhere"],
        serde_json::json!({"hasTrustDialogAccepted": false, "allowedTools": ["Bash"]})
    );
    let own = worktree.path().canonicalize().unwrap();
    assert_eq!(
        state["projects"][own.to_str().unwrap()],
        serde_json::json!({"hasTrustDialogAccepted": true})
    );
    app.shutdown_cleanup().unwrap();
}
