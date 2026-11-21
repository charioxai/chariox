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
    use std::os::unix::fs::PermissionsExt;
    let root = crate::test_support::TestWorktree::new("slow-model-refresh");
    let root = root.path();
    let marker = root.join("refresh-started");
    let release = root.join("refresh-may-finish");
    let executable = root.join("opencode");
    std::fs::write(
        &executable,
        format!(
            r#"#!/bin/sh
set -eu
if [ "$1" = "auth" ]; then printf '%s\n' '1 credential'; exit 0; fi
if [ "$1" = "--version" ]; then printf '%s\n' 'opencode 1.2.3'; exit 0; fi
if [ "$1" = "models" ] && [ "${{2-}}" = "--refresh" ]; then
  touch '{}'
  while [ ! -f '{}' ]; do sleep 0.02; done
  printf '%s\n' 'opencode-go/claude-haiku-5-5'
  exit 0
fi
exit 2
"#,
            marker.display(),
            release.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("CHARIOX_OPENCODE_BIN", &executable);
    // The Go profile has a synthetic key. Keep its verification offline so the
    // real provisioning path can reach the native catalog refresh.
    std::env::set_var("CHARIOX_TEST_OPENCODE_GO_USAGE_STATUS", "200");
    std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.user_config.state.path = Some(root.join("kernel/state.db").display().to_string());
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
    let mut provision = tokio::spawn(async move {
        provision_state
            .ensure_relay_remote_provider_account(context, materialization)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !marker.exists() {
            tokio::select! {
                result = &mut provision => panic!("provisioning finished before the native refresh: {result:?}"),
                () = tokio::time::sleep(std::time::Duration::from_millis(5)) => {}
            }
        }
    })
    .await
    .expect("the native catalog refresh must actually run");
    let responsive = tokio::time::timeout(
        std::time::Duration::from_millis(150),
        state.with_app_side_effect(|_| ()),
    )
    .await
    .is_ok();
    if remove_lease && responsive {
        state
            .with_app_side_effect(|app| {
                RemoteLeaseRuntime::new(app).destroy_execution_lease(&lease_id)
            })
            .await
            .expect("remove the lease through its normal cleanup path");
    }
    std::fs::write(release, []).unwrap();
    let provision_result = provision.await.unwrap();
    assert!(
        responsive,
        "cold native account preparation held the shared kernel app lock"
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
}
