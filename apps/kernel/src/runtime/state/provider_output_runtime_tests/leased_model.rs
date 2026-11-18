// The worker runs a leased agent, so the model comes from the worker's own catalog: home's can
// lack the provider or list other models.
use super::*;

#[tokio::test]
async fn a_leased_agent_runs_a_model_the_worker_serves() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("leased-model");
    let user_home = root.path().join("user");
    std::fs::create_dir_all(&user_home).unwrap();
    std::env::set_var("HOME", &user_home);
    let mut config = crate::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    crate::test_support::seed_default_account_models(
        &*app.lock().await,
        "codex",
        &["worker-model"],
    );
    let runtime = owned_runtime_state(&app).await;
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let lease = runtime
        .create_relay_execution_lease("home", "room", "agent-home-1", false, owner)
        .await
        .unwrap();
    let spawn = |model: Option<&str>| {
        runtime.create_relay_leased_agent(
            &lease.id,
            "codex",
            "default",
            model.map(str::to_string),
            Some("high".to_string()),
            None,
            None,
            None,
            Some(user_home.display().to_string()),
            None,
        )
    };
    let chosen = spawn(None).await.unwrap();
    assert_eq!(chosen.model.as_deref(), Some("worker-model"));
    let refused = spawn(Some("home-only-model"))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("home-only-model") && refused.contains("worker-model"),
        "{refused}"
    );
}
