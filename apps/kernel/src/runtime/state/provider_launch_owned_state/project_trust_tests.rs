//! MP-08 / MP-10 / MP-11: exercise the owned request path used by real clients.
use super::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn owned_claude_project_trust_is_pre_recorded_for_selected_account() {
    assert_owned_project_trust(false, false).await;
}

#[tokio::test]
async fn owned_workflow_claude_project_trust_is_pre_recorded_for_selected_account() {
    assert_owned_project_trust(true, false).await;
}

#[tokio::test]
async fn owned_claude_project_trust_redirects_home_to_selected_project() {
    assert_owned_project_trust(false, true).await;
}

async fn assert_owned_project_trust(workflow: bool, home_cwd: bool) {
    crate::test_support::isolated_env_test!();
    let folder = crate::test_support::TestWorktree::new("owned-project-trust");
    let other = crate::test_support::TestWorktree::new("owned-unchosen-project");
    let mut app =
        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(folder.session_request())
        .unwrap();
    let registry = app.provider_account_profile_registry();
    let first = registry
        .create_managed("local", "claude", "selected")
        .unwrap();
    let second = registry.create_managed("local", "claude", "other").unwrap();
    let state_path = |profile: &str| {
        let environment = registry
            .resolve_environment("local", "claude", profile)
            .unwrap();
        PathBuf::from(&environment["CLAUDE_CONFIG_DIR"]).join(".claude.json")
    };
    let first_path = state_path(&first.profile_id);
    let second_path = state_path(&second.profile_id);
    let app = Arc::new(Mutex::new(app));
    let runtime = super::tests::owned_runtime_state(&app).await;
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        &first.profile_id,
        "haiku",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let prepare = |request| {
        if workflow {
            runtime.owned.prepare_workflow_provider_launch_request(
                request,
                "http://127.0.0.1:43120/mcp".into(),
            )
        } else {
            runtime
                .owned
                .prepare_provider_launch_request(request, "http://127.0.0.1:43120/mcp".into())
        }
    };
    // Explicit unchosen cwd must never be granted trust by either caller.
    prepare(
        request
            .clone()
            .with_working_directory(other.path().to_path_buf()),
    )
    .unwrap();
    assert!(!first_path.exists());
    let request = if home_cwd {
        request.with_working_directory(PathBuf::from(std::env::var_os("HOME").unwrap()))
    } else {
        request
    };
    let prepared = prepare(request).unwrap();
    let state = std::fs::read(&first_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let projects = state.as_ref().and_then(|value| value.get("projects"));
    let trusted = projects
        .and_then(|value| value.get(folder.path().to_str().unwrap()))
        .and_then(|value| value.get("hasTrustDialogAccepted"))
        .and_then(serde_json::Value::as_bool);
    let untouched = !second_path.exists();
    app.lock().await.shutdown_cleanup().unwrap();
    assert_eq!(prepared.working_directory.as_deref(), Some(folder.path()));
    assert_eq!(
        trusted,
        Some(true),
        "selected project must be trusted on the owned launch path"
    );
    assert_eq!(
        projects
            .and_then(serde_json::Value::as_object)
            .map(|value| value.len()),
        Some(1)
    );
    assert!(untouched, "the other account config must remain untouched");
}
