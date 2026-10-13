//! MP-08 / MP-10 / MP-11: native provisioning responsiveness and lease revocation.
use super::relay_native_provider_launch_tests::runtime_state_from_app;
use super::*;

// MP-08 / MP-10 / MP-11: native provisioning must not stall relay heartbeats.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_remote_model_refresh_leaves_kernel_app_responsive() {
    assert_cold_remote_model_refresh_responsive(false).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_remote_model_refresh_never_acknowledges_a_removed_lease() {
    assert_cold_remote_model_refresh_responsive(true).await;
}

#[cfg(unix)]
async fn assert_cold_remote_model_refresh_responsive(remove_lease: bool) {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!(
        "ctxfix-slow-model-refresh-{}",
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let marker = root.join("refresh-started");
    let executable = root.join("opencode");
    std::fs::write(
        &executable,
        format!(
            r#"#!/bin/sh
set -eu
if [ "$1" = "auth" ]; then printf '%s\n' '1 credential'; exit 0; fi
if [ "$1" = "--version" ]; then printf '%s\n' 'opencode 1.2.3'; exit 0; fi
if [ "$1" = "models" ]; then
  touch '{}'
  sleep 2
  printf '%s\n' 'opencode-go/claude-haiku-5-5'
  exit 0
fi
exit 2
"#,
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("CHARIOX_OPENCODE_BIN", &executable);
    std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "home-session",
            "home-agent",
            false,
            "owner-a",
        )
        .unwrap();
    let context = crate::transport::relay_peer::RemoteProviderAccountSyncContext {
        home_kernel_id: lease.home_kernel_id,
        home_session_id: lease.home_session_id,
        home_agent_id: lease.home_agent_id,
        execution_lease_id: lease.id,
    };
    let materialization = crate::account_profile::ProviderAccountMaterialization {
        profile: crate::account_profile::ProviderAccountReplicaMetadata {
            owner_user_id: "owner-a".into(),
            provider: "opencode".into(),
            profile_id: "slow".into(),
            label: "Slow".into(),
            origin: crate::account_profile::ProviderAccountProfileOrigin::CharioxCreated,
            is_default: false,
        },
        files: vec![crate::account_profile::ProviderAccountMaterializationFile {
            relative_path: "data/opencode/auth.json".into(),
            contents_base64: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                br#"{"opencode-go":{"type":"api","key":"synthetic-test-only"}}"#,
            ),
        }],
        generated_at_ms: 1,
    };
    let state = runtime_state_from_app(app);
    let lease_id = context.execution_lease_id.clone();
    let provision_state = state.clone();
    let provision = tokio::spawn(async move {
        provision_state
            .ensure_relay_remote_provider_account(context, materialization)
            .await
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.exists() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let refresh_started = marker.exists();
    let responsive = tokio::time::timeout(
        std::time::Duration::from_millis(150),
        state.with_app_side_effect(|_| ()),
    )
    .await
    .is_ok();
    if remove_lease {
        state
            .with_app_side_effect(|app| {
                RemoteLeaseRuntime::new(app).destroy_execution_lease(&lease_id)
            })
            .await
            .expect("remove the lease through its normal cleanup path");
    }
    let provision_result = provision.await.unwrap();
    std::env::remove_var("CHARIOX_OPENCODE_BIN");
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        refresh_started,
        "the native catalog refresh must actually run"
    );
    if remove_lease {
        assert!(
            matches!(
                provision_result,
                Err(DaemonError::ExecutionLeaseNotFound { .. })
            ),
            "a removed lease must not receive a provider-account acknowledgment"
        );
    } else {
        provision_result.expect("the native model refresh should provision successfully");
    }
    assert!(
        responsive,
        "cold native account preparation held the shared kernel app lock"
    );
}
