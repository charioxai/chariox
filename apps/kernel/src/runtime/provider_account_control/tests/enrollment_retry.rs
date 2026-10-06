//! MP-08 / MP-11: synthetic Codex device login through shared kernel requests.
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::config::DaemonConfig;
use crate::local::{
    LocalDaemonRequest, LocalDaemonResponse, ProviderLoginProcessState, StartProviderLoginRequest,
};
use crate::runtime::provider_auth_control::execute_start_provider_login_request;
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

#[tokio::test]
async fn abandoned_codex_enrollment_cancels_old_login_and_restarts_on_same_profile() {
    crate::test_support::isolated_env_test!();
    let _env = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-enrollment-retry-{}-{}",
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
    // Reuse the credential-free websocket fixture; only synthetic login IDs
    // and RPC method names are retained. No provider auth file is used.
    let fixture = include_str!("../../state/provider_output_runtime_tests/project_queued_environment_fixture.py")
        .replace("state_path = Path(__file__).with_suffix('.json')", "state_path = Path(os.environ['CODEX_HOME']) / 'fixture-events.json'")
        .replace(
        "if method == 'thread/start':",
        r#"if method == 'account/read':
                        result = {'account': None, 'requiresOpenaiAuth': True}
                    elif method == 'account/login/start':
                        logins = state.setdefault('logins', [])
                        login_id = Path(os.environ['CODEX_HOME']).parent.name + '-login-' + str(len(logins) + 1)
                        logins.append(login_id)
                        state.setdefault('events', []).append(['start', login_id])
                        result = {'type': 'chatgptDeviceCode', 'loginId': login_id}
                    elif method == 'account/login/cancel':
                        state.setdefault('events', []).append(['cancel', params['loginId']])
                    elif method == 'thread/start':"#,
    );
    std::fs::write(&binary, fixture).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("CHARIOX_CODEX_BIN", &binary);
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let router = CommandRouter::with_interactive_capacity(app, 1);
    let runtime = &router.runtime_state();
    let create = |label: &str| {
        serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
            "CreateProviderAccountProfile": { "provider": "codex", "label": label },
        }))
        .unwrap()
    };
    let LocalDaemonResponse::ProviderAccountProfile { profile: first } =
        super::super::execute_provider_account_request(runtime, "local", create("chariox"))
            .await
            .unwrap()
    else {
        panic!("expected account profile");
    };
    cleanup.profile_ids.push(first.profile_id.clone());
    let start = |profile_id: &str| StartProviderLoginRequest {
        provider: "codex".into(),
        account_profile: profile_id.into(),
        method: Some("device_code".into()),
    };
    let LocalDaemonResponse::ProviderLoginStarted { login: old } =
        execute_start_provider_login_request(runtime, "local", start(&first.profile_id))
            .await
            .unwrap()
    else {
        panic!("expected device login");
    };
    let LocalDaemonResponse::ProviderAccountProfile { profile: unrelated } =
        super::super::execute_provider_account_request(runtime, "local", create("Other"))
            .await
            .unwrap()
    else {
        panic!("expected other account");
    };
    cleanup.profile_ids.push(unrelated.profile_id.clone());
    let LocalDaemonResponse::ProviderLoginStarted { login: other } =
        execute_start_provider_login_request(runtime, "local", start(&unrelated.profile_id))
            .await
            .unwrap()
    else {
        panic!("expected other device login");
    };
    // The browser disappears without sending LoginCancel, then Add is retried.
    let LocalDaemonResponse::ProviderAccountProfile { profile: resumed } =
        super::super::execute_provider_account_request(runtime, "local", create("chariox"))
            .await
            .unwrap()
    else {
        panic!("expected resumed account");
    };
    assert_eq!(resumed.profile_id, first.profile_id);
    let store = runtime.provider_login_process_store();
    assert_eq!(
        store
            .record_for_owner("local", old.login_id.as_deref().unwrap())
            .unwrap()
            .state,
        ProviderLoginProcessState::Cancelled
    );
    assert_eq!(
        store
            .record_for_owner("local", other.login_id.as_deref().unwrap())
            .unwrap()
            .state,
        ProviderLoginProcessState::Running
    );
    let LocalDaemonResponse::ProviderLoginStarted { login: fresh } =
        execute_start_provider_login_request(runtime, "local", start(&resumed.profile_id))
            .await
            .unwrap()
    else {
        panic!("expected fresh device login");
    };
    assert_eq!(fresh.account_profile, first.profile_id);
    assert_ne!(fresh.login_id, old.login_id);
    assert_eq!(
        runtime
            .provider_account_profile_registry()
            .list("local", Some("codex"))
            .unwrap()
            .iter()
            .filter(|profile| profile.label == "chariox")
            .count(),
        1
    );
    let environment = runtime
        .provider_account_profile_registry()
        .resolve_environment("local", "codex", &first.profile_id)
        .unwrap();
    let events = std::path::Path::new(&environment["CODEX_HOME"]).join("fixture-events.json");
    let synthetic: serde_json::Value =
        serde_json::from_slice(&std::fs::read(events).unwrap()).unwrap();
    assert_eq!(
        synthetic["events"],
        serde_json::json!([
            ["start", old.login_id.as_ref().unwrap()],
            ["cancel", old.login_id.as_ref().unwrap()],
            ["start", fresh.login_id.as_ref().unwrap()],
        ])
    );
}
