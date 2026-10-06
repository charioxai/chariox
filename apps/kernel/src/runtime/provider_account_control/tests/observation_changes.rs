//! MP-08 / MP-11: refresh errors still publish account inventory changes.
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::account_profile::ProviderAccountAuthState;
use crate::app::DaemonApp;
use crate::config::DaemonConfig;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::router::CommandRouter;

struct FixtureCleanup {
    root: std::path::PathBuf,
    environment: Vec<(&'static str, Option<std::ffi::OsString>)>,
    profile_ids: Vec<String>,
}

impl Drop for FixtureCleanup {
    fn drop(&mut self) {
        for profile_id in &self.profile_ids {
            crate::provider::invalidate_codex_account_endpoint("local", profile_id);
        }
        for (name, value) in &self.environment {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn duplicate_observation_publishes_change(auth_status: bool) {
    let root = std::env::temp_dir().join(format!(
        "chariox-observation-change-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let names = [
        "HOME",
        "CHARIOX_HOME",
        "CHARIOX_CODEX_BIN",
        "CHARIOX_LOG_DIR",
        "CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT",
    ];
    let mut cleanup = FixtureCleanup {
        root: root.clone(),
        environment: names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect(),
        profile_ids: Vec::new(),
    };
    std::env::set_var("HOME", root.join("home"));
    std::env::set_var("CHARIOX_HOME", root.join("state"));
    std::env::set_var("CHARIOX_LOG_DIR", root.join("logs"));
    std::env::set_var("CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT", "1");
    let binary = root.join("codex-fixture.py");
    // Credential-free official-harness RPC fixture. No provider auth file.
    let fixture = include_str!("../../state/provider_output_runtime_tests/project_queued_environment_fixture.py")
        .replace("state_path = Path(__file__).with_suffix('.json')", "state_path = Path(os.environ['CODEX_HOME']) / 'fixture-events.json'")
        .replace("if method == 'thread/start':", r#"if method == 'account/read':
                        result = {'account': {'email': 'connected@example.test', 'planType': 'plus'}, 'requiresOpenaiAuth': True}
                    elif method == 'account/rateLimits/read':
                        result = {'rateLimits': {}}
                    elif method == 'thread/start':"#);
    std::fs::write(&binary, fixture).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", &binary);
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let router = CommandRouter::with_interactive_capacity(app.clone(), 1);
    let runtime = router.runtime_state();
    let registry = runtime.provider_account_profile_registry();
    let existing = registry
        .create_managed("local", "codex", "Connected")
        .unwrap();
    cleanup.profile_ids.push(existing.profile_id.clone());
    registry
        .update_observation(
            "local",
            "codex",
            &existing.profile_id,
            ProviderAccountAuthState::Authenticated,
            Some("connected@example.test".into()),
            Some("plus".into()),
            None,
            None,
        )
        .unwrap();
    let folder = root.join("linked");
    std::fs::create_dir(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(folder.join("fixture.txt"), "synthetic user data").unwrap();
    let link = serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
        "LinkProviderAccountProfile": {"provider": "codex", "label": "Duplicate", "path": folder},
    }))
    .unwrap();
    let LocalDaemonResponse::ProviderAccountProfile { profile: duplicate } =
        super::super::execute_provider_account_request(&runtime, "local", link)
            .await
            .unwrap()
    else {
        panic!("expected linked account");
    };
    cleanup.profile_ids.push(duplicate.profile_id.clone());
    let request = serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
        if auth_status { "GetProviderAuthStatus" } else { "RefreshProviderAccountProfile" }:
            { "provider": "codex", "account_profile": duplicate.profile_id },
    }))
    .unwrap();
    let retry = request.clone();
    app.lock()
        .await
        .provider_catalog_cache
        .set(crate::provider::OpenCodeProviderCatalog {
            all: Vec::new(),
            default: Default::default(),
            connected: vec!["codex".into()],
        });
    let sequence = runtime.waiting_room_change_sequence();
    let subscribed_runtime = runtime.clone();
    let subscriber = tokio::spawn(async move {
        subscribed_runtime
            .wait_for_waiting_room_change_after(sequence)
            .await;
    });
    tokio::task::yield_now().await;
    let result = if auth_status {
        crate::runtime::provider_auth_control::execute_provider_auth_request(
            &runtime, "local", request,
        )
        .await
    } else {
        super::super::execute_provider_account_request(&runtime, "local", request).await
    };
    let error = result
        .expect_err("duplicate identity must be refused")
        .to_string();
    assert!(
        error.contains("already connected as account `Connected` (connected@example.test)"),
        "{error}"
    );
    assert!(registry
        .get("local", "codex", &duplicate.profile_id)
        .is_err());
    assert_eq!(
        registry
            .get("local", "codex", &existing.profile_id)
            .unwrap()
            .auth_state,
        ProviderAccountAuthState::Authenticated
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("fixture.txt")).unwrap(),
        "synthetic user data"
    );
    assert!(
        runtime.waiting_room_change_sequence() > sequence,
        "duplicate rollback must publish the account inventory change even on error"
    );
    tokio::time::timeout(std::time::Duration::from_millis(200), subscriber)
        .await
        .expect("waiting-room subscriber must wake after duplicate rollback")
        .unwrap();
    assert!(
        app.lock()
            .await
            .provider_catalog_cache
            .get_fresh(std::time::Duration::from_secs(60))
            .is_none(),
        "duplicate rollback must invalidate the provider catalog"
    );
    if auth_status {
        let sequence = runtime.waiting_room_change_sequence();
        assert!(
            crate::runtime::provider_auth_control::execute_provider_auth_request(
                &runtime, "local", retry,
            )
            .await
            .is_err()
        );
        assert_eq!(
            runtime.waiting_room_change_sequence(),
            sequence,
            "a missing-profile read must not publish a spurious change"
        );
    }
}

#[tokio::test]
async fn explicit_refresh_duplicate_rollback_notifies_waiting_room_subscribers() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    duplicate_observation_publishes_change(false).await;
}

#[tokio::test]
async fn auth_status_duplicate_rollback_notifies_waiting_room_subscribers() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    duplicate_observation_publishes_change(true).await;
}
