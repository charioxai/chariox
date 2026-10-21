// MP-08 / MP-10 / MP-11: native quota observation must not block output or cross account ownership.
use super::*;

fn usage_run(owner: &str, profile: &str, path: &std::path::Path) -> RuntimeProviderRun {
    let request = crate::provider::LaunchProviderRequest::new(
        "native-usage-session",
        "claude",
        "claude",
        profile,
        "haiku",
    )
    .with_owner_user_id(owner);
    RuntimeProviderRun::new(
        "native-usage-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "native usage observation".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: std::collections::BTreeMap::from([(
                "CHARIOX_CLAUDE_USAGE_FILE".into(),
                path.display().to_string(),
            )]),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    )
}

fn write_usage(path: &std::path::Path) {
    fs::write(
        path,
        serde_json::json!({"rate_limits": {"five_hour": {"used_percentage": 55.0}}}).to_string(),
    )
    .unwrap();
}

#[test]
fn native_usage_aliases_only_the_home_cloud_owner() {
    crate::test_support::isolated_env_test!();
    let root = std::env::temp_dir().join(format!(
        "chariox-native-usage-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "home-cloud-owner".into(),
        ..Default::default()
    });
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let registry = app.provider_account_profile_registry();
    for (runtime_owner, account_owner) in [
        ("home-cloud-owner", "local"),
        ("collaborator", "collaborator"),
    ] {
        let profile = registry
            .create_managed(account_owner, "claude", "Native usage account")
            .unwrap();
        let path = root.join(runtime_owner);
        write_usage(&path);
        let run = usage_run(runtime_owner, &profile.profile_id, &path);
        ProviderOutputClaudeNativeBridge::new(&mut app)
            .process_claude_account_usage(&run)
            .expect(
                "native output must retain its authoritative home/collaborator account namespace",
            );
        let updated = registry
            .get(account_owner, "claude", &profile.profile_id)
            .unwrap();
        assert_eq!(updated.usage.source, "claude.status_line");
        assert_eq!(updated.usage.meters.len(), 1);
        assert!(!path.exists(), "native status-line sample is consumed once");
    }
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_worker_usage_never_requires_or_mutates_a_home_profile() {
    crate::test_support::isolated_env_test!();
    let root = std::env::temp_dir().join(format!(
        "chariox-native-worker-usage-{}-{}",
        std::process::id(),
        timestamp_millis()
    ));
    fs::create_dir_all(&root).unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        user_id: "home-cloud-owner".into(),
        ..Default::default()
    });
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let registry = app.provider_account_profile_registry();
    let local = registry
        .create_managed("local", "claude", "Worker own account")
        .unwrap();
    let before = registry
        .get("local", "claude", &local.profile_id)
        .unwrap()
        .usage;
    let path = root.join("leased-usage");
    write_usage(&path);
    // Deliberately reuse the local profile id: worker quota must never alter
    // that account, even when home and worker enroll under the same Cloud owner.
    let run = usage_run("home-cloud-owner", &local.profile_id, &path);
    app.mark_leased_provider_run(run.id());
    ProviderOutputClaudeNativeBridge::new(&mut app)
        .process_claude_account_usage(&run)
        .expect("leased native output must not resolve the home's account on the worker");
    let after = registry
        .get("local", "claude", &local.profile_id)
        .unwrap()
        .usage;
    assert_eq!(
        after, before,
        "worker quota cannot change the worker's own account"
    );
    drop(app);
    fs::remove_dir_all(root).unwrap();
}
