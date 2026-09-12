//! Worker-side cold-launch parity: a worker profile with transferred portable
//! Claude credentials and persisted Authenticated state must launch without a
//! home Chariox-vault setup token. The home vault credential is an optional
//! fallback only: when the worker truly lacks usable provider-native
//! credentials it replies with the typed credential-required diagnostic so the
//! home can resolve and retry.

use super::*;
use crate::transport::relay_peer::{
    RelayAgentExecutionProfile, RemoteGitTurnContext,
    REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE,
};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use wait_timeout::ChildExt;

const REGULAR_PORTABLE_CHILD_ROOT: &str = "CHARIOX_COLD_CLAUDE_REGULAR_PORTABLE_ROOT";
const REGULAR_FALLBACK_CHILD_ROOT: &str = "CHARIOX_COLD_CLAUDE_REGULAR_FALLBACK_ROOT";
const NATIVE_PORTABLE_CHILD_ROOT: &str = "CHARIOX_COLD_CLAUDE_NATIVE_PORTABLE_ROOT";
const NATIVE_FALLBACK_CHILD_ROOT: &str = "CHARIOX_COLD_CLAUDE_NATIVE_FALLBACK_ROOT";

const REGULAR_PORTABLE_TEST: &str = "runtime::state::provider_output_runtime_tests::leased_claude_cold_launch::cold_claude_launch_with_portable_worker_credentials_does_not_require_home_credential";
const REGULAR_FALLBACK_TEST: &str = "runtime::state::provider_output_runtime_tests::leased_claude_cold_launch::cold_claude_launch_without_portable_worker_credentials_requires_home_fallback";
const NATIVE_PORTABLE_TEST: &str = "runtime::state::provider_output_runtime_tests::leased_claude_cold_launch::cold_native_claude_launch_with_portable_worker_credentials_does_not_require_home_credential";
const NATIVE_FALLBACK_TEST: &str = "runtime::state::provider_output_runtime_tests::leased_claude_cold_launch::cold_native_claude_launch_without_portable_worker_credentials_requires_home_fallback";

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
        std::fs::remove_dir_all(&self.root).expect("remove isolated cold-launch fixture");
    }
}

fn fixture_claude_script() -> &'static str {
    r#"#!/bin/bash
if [[ "$1" == "--version" ]]; then printf 'Claude Code 2.1.207\n'; exit 0; fi
if [[ "$1" == "auth" && "$2" == "status" && "$3" == "--json" ]]; then printf '%s\n' '{"loggedIn":true,"email":"worker@example.com","subscriptionType":"Pro"}'; exit 0; fi
: > "$CHARIOX_TEST_RECEIVED"
while IFS= read -r line; do :; done
"#
}

#[tokio::test]
async fn cold_claude_launch_with_portable_worker_credentials_does_not_require_home_credential() {
    let Some(fixture_root) = std::env::var_os(REGULAR_PORTABLE_CHILD_ROOT) else {
        run_isolated_regular_child(REGULAR_PORTABLE_CHILD_ROOT, REGULAR_PORTABLE_TEST);
        return;
    };
    run_worker_cold_launch_scenario(std::path::PathBuf::from(fixture_root), false, true).await;
}

#[tokio::test]
async fn cold_claude_launch_without_portable_worker_credentials_requires_home_fallback() {
    let Some(fixture_root) = std::env::var_os(REGULAR_FALLBACK_CHILD_ROOT) else {
        run_isolated_regular_child(REGULAR_FALLBACK_CHILD_ROOT, REGULAR_FALLBACK_TEST);
        return;
    };
    run_worker_cold_launch_scenario(std::path::PathBuf::from(fixture_root), false, false).await;
}

#[tokio::test]
async fn cold_native_claude_launch_with_portable_worker_credentials_does_not_require_home_credential(
) {
    let Some(fixture_root) = std::env::var_os(NATIVE_PORTABLE_CHILD_ROOT) else {
        run_isolated_regular_child(NATIVE_PORTABLE_CHILD_ROOT, NATIVE_PORTABLE_TEST);
        return;
    };
    run_worker_cold_launch_scenario(std::path::PathBuf::from(fixture_root), true, true).await;
}

#[tokio::test]
async fn cold_native_claude_launch_without_portable_worker_credentials_requires_home_fallback() {
    let Some(fixture_root) = std::env::var_os(NATIVE_FALLBACK_CHILD_ROOT) else {
        run_isolated_regular_child(NATIVE_FALLBACK_CHILD_ROOT, NATIVE_FALLBACK_TEST);
        return;
    };
    run_worker_cold_launch_scenario(std::path::PathBuf::from(fixture_root), true, false).await;
}

fn run_isolated_regular_child(child_root_env: &str, test_name: &str) {
    let root = std::env::temp_dir().join(format!(
        "chariox-cold-claude-{}-{}-{}",
        child_root_env,
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let executable = root.join("claude");
    std::fs::write(&executable, fixture_claude_script()).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test_name, "--exact", "--test-threads=1", "--nocapture"])
        .current_dir(&root)
        .env(child_root_env, &root)
        .env("HOME", &root)
        .env("CLAUDE_CONFIG_DIR", root.join("claude-config"))
        .env("CHARIOX_HOME", root.join("home"))
        .env("CHARIOX_LOG_DIR", root.join("logs"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("CHARIOX_CLAUDE_BIN", &executable)
        .env("CHARIOX_TEST_RECEIVED", root.join("received"))
        .env_remove("CHARIOX_MANAGED_PROVIDER_ISOLATION")
        .env_remove("CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    if child
        .wait_timeout(std::time::Duration::from_secs(60))
        .unwrap()
        .is_none()
    {
        let _ = child.kill();
        let _ = child.wait();
        panic!("isolated cold-launch fixture timed out");
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "isolated cold-launch fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn run_worker_cold_launch_scenario(fixture_root: PathBuf, native: bool, portable: bool) {
    let root = fixture_root.join("runtime");
    std::fs::create_dir_all(&root).unwrap();
    let mut cleanup = FixtureCleanup {
        root: root.clone(),
        providers: None,
    };
    let mut config = crate::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    config.provider_runtime_init_delay_ms = 100;
    config.local_socket_path = root.join("kernel.sock");
    config = config.with_session_history_root(root.join("history"));
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.join("events.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    let app = crate::app::DaemonApp::bootstrap(config).expect("worker bootstrap");
    cleanup.providers = Some(app.providers().clone());
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let lease = runtime
        .create_relay_execution_lease("home", "room", "agent", false, "owner")
        .await
        .expect("execution lease");
    // The realistic worker setup materializes the transferred portable profile,
    // which persists Authenticated state before any cold launch arrives.
    let profile_id = if portable {
        let context = crate::transport::relay_peer::RemoteProviderAccountSyncContext {
            home_kernel_id: "home".to_string(),
            home_session_id: "room".to_string(),
            home_agent_id: "agent".to_string(),
            execution_lease_id: lease.id.clone(),
        };
        let materialization = crate::account_profile::ProviderAccountMaterialization {
            profile: crate::account_profile::ProviderAccountReplicaMetadata {
                owner_user_id: "owner".to_string(),
                provider: "claude".to_string(),
                profile_id: "fixture".to_string(),
                label: "fixture".to_string(),
                origin: crate::account_profile::ProviderAccountProfileOrigin::Linked,
                is_default: false,
            },
            files: vec![crate::account_profile::ProviderAccountMaterializationFile {
                relative_path: ".credentials.json".to_string(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(br#"{"claudeAiOauth":{"refreshToken":"portable-refresh-token"}}"#),
            }],
            generated_at_ms: 1,
        };
        let profile = crate::app::RemoteLeaseRuntime::new(&mut *app.lock().await)
            .ensure_remote_provider_account(context, materialization)
            .expect("portable Claude credentials must install the worker profile");
        assert_eq!(
            profile.auth_state,
            crate::account_profile::ProviderAccountAuthState::Authenticated
        );
        profile.profile_id
    } else {
        app.lock()
            .await
            .provider_account_profile_registry()
            .create_managed("owner", "claude", "fixture")
            .expect("isolated worker account profile")
            .profile_id
    };
    let leased = runtime
        .create_relay_leased_agent(
            &lease.id,
            "claude",
            &profile_id,
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            Some(root.display().to_string()),
            None,
        )
        .await
        .expect("leased Claude agent");

    if native {
        let result = runtime
            .launch_relay_leased_native_provider_run(
                &leased.id,
                "claude",
                "claude",
                &profile_id,
                "sonnet",
                None,
                None,
                None,
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
                None,
            )
            .await;
        if portable {
            let run = result.expect(
                "portable worker Claude must cold-launch natively without a home credential",
            );
            assert_worker_provider_received(&fixture_root.join("received")).await;
            assert_eq!(run.provider(), "claude");
        } else {
            let error = result.expect_err(
                "worker without portable credentials must require the home launch credential",
            );
            assert!(
                error
                    .to_string()
                    .contains(REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE),
                "native fallback must surface the typed credential diagnostic: {error}"
            );
        }
        return;
    }

    let expected_profile = RelayAgentExecutionProfile {
        provider: leased.provider.clone(),
        account_profile: leased.account_profile.clone(),
        model: leased.model.clone(),
        effort: leased.effort.clone(),
    };
    let result = runtime
        .submit_relay_leased_prompt(
            &leased.id,
            expected_profile,
            "inspect the Room",
            "",
            Vec::new(),
            None,
            Some(RemoteGitTurnContext {
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
            None,
        )
        .await;
    if portable {
        let (run_id, _outcome) =
            result.expect("portable worker Claude must cold-launch without a home credential");
        assert!(!run_id.is_empty());
        assert_worker_provider_received(&fixture_root.join("received")).await;
    } else {
        let error = result.expect_err(
            "worker without portable credentials must require the home launch credential",
        );
        assert!(
            error
                .to_string()
                .contains(REMOTE_PROVIDER_LAUNCH_CREDENTIAL_REQUIRED_CODE),
            "regular fallback must surface the typed credential diagnostic: {error}"
        );
    }
}

async fn assert_worker_provider_received(marker: &std::path::Path) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.is_file() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        marker.is_file(),
        "the worker provider must have been spawned (received marker missing at {}",
        marker.display()
    );
}
