//! A kernel reuses the provider logins already on the machine, without a client asking.
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::Duration;

use crate::account_profile::{ProviderAccountAuthState, ProviderAccountProfile};
use crate::local::test_support::LocalRouterTestHarness;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};

const OWNER: &str = crate::session::DEFAULT_LOCAL_USER_ID;

/// A Claude CLI that reports who is signed in per configuration directory.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  --version) printf 'Claude test version\n' ;;
  auth)
    case "${CLAUDE_CONFIG_DIR:-}" in
      *.claude-work) printf '%s\n' '{"loggedIn":true,"email":"bob@example.com","subscriptionType":"pro"}' ;;
      *.claude-signed-out) printf '%s\n' '{"loggedIn":false}' ;;
      *) printf '%s\n' '{"loggedIn":true,"email":"alice@example.com","subscriptionType":"max"}' ;;
    esac ;;
  *) exit 1 ;;
esac
"#;

/// A machine with one Claude login on it. Codex and OpenCode are not installed.
struct Machine {
    home: PathBuf,
}

impl Machine {
    fn new(label: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "chariox-provider-discovery-{label}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        let bin = home.join("fake-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let claude = bin.join("claude");
        std::fs::write(&claude, FAKE_CLAUDE).unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("CHARIOX_CLAUDE_BIN", &claude);
        std::env::set_var("CHARIOX_CODEX_BIN", home.join("not-installed-codex"));
        std::env::set_var("CHARIOX_OPENCODE_BIN", home.join("not-installed-opencode"));
        for name in ["CODEX_HOME", "CLAUDE_CONFIG_DIR", "OPENCODE_CONFIG_DIR"] {
            std::env::remove_var(name);
        }
        Self { home }
    }

    /// Another Claude configuration directory of this user, as `claude` would create it.
    fn claude_config_dir(&self, name: &str, with_login: bool) -> PathBuf {
        let dir = self.home.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        if with_login {
            std::fs::write(dir.join(".credentials.json"), "{}").unwrap();
        }
        dir
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn request(value: serde_json::Value) -> LocalDaemonRequest {
    serde_json::from_value(value).expect("the request has the public shape")
}

fn claude_profiles(harness: &LocalRouterTestHarness) -> Vec<ProviderAccountProfile> {
    harness
        .runtime_state()
        .provider_account_profile_registry()
        .list(OWNER, Some("claude"))
        .unwrap()
}

fn default_claude(harness: &LocalRouterTestHarness) -> ProviderAccountProfile {
    claude_profiles(harness)
        .into_iter()
        .find(|profile| profile.is_default)
        .expect("a fresh kernel registers the machine's default Claude account")
}

#[test]
fn a_first_start_reuses_the_machine_login_and_warms_the_model_catalog() {
    crate::test_support::isolated_env_test!();
    let _machine = Machine::new("first-start");
    let harness = LocalRouterTestHarness::new();
    assert_eq!(
        default_claude(&harness).auth_state,
        ProviderAccountAuthState::Unknown
    );
    assert!(harness.warmed_default_catalog().is_none());

    harness.run_provider_discovery_pass();

    let profile = default_claude(&harness);
    assert_eq!(profile.auth_state, ProviderAccountAuthState::Authenticated);
    assert_eq!(
        profile.identity_summary.as_deref(),
        Some("alice@example.com")
    );
    let catalog = harness
        .warmed_default_catalog()
        .expect("the catalog is ready before any client asks for it");
    assert!(catalog
        .all
        .iter()
        .any(|provider| provider.id.starts_with("claude") && !provider.models.is_empty()));
}

#[test]
fn empty_native_defaults_start_servers_only_when_a_provider_is_requested() {
    crate::test_support::isolated_env_test!();
    let machine = Machine::new("empty-native-defaults");
    let starts = machine.home.join("server-starts");
    for (provider, variable) in [
        ("codex", "CHARIOX_CODEX_BIN"),
        ("opencode", "CHARIOX_OPENCODE_BIN"),
    ] {
        let executable = machine.home.join("fake-bin").join(provider);
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'test version' ;;\n  auth) exit 0 ;;\n  app-server|serve) echo \"$1\" >> '{}' ; exit 1 ;;\n  *) exit 1 ;;\nesac\n",
                starts.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var(variable, executable);
    }
    let harness = LocalRouterTestHarness::new();

    harness.run_provider_discovery_pass();

    assert!(harness.warmed_default_catalog().is_some());
    assert!(
        !starts.exists(),
        "discovering and warming empty native defaults must not start provider servers"
    );
    for (provider, command) in [("codex", "app-server"), ("opencode", "serve")] {
        let _ = harness.dispatch(request(serde_json::json!({"GetProviderCatalog": {
            "provider": provider,
        }})));
        let launched = std::fs::read_to_string(&starts).unwrap();
        assert!(
            launched.lines().any(|line| line == command),
            "an explicit {provider} catalog request still starts its native server"
        );
    }

    std::fs::remove_file(&starts).unwrap();
    std::fs::write(machine.home.join(".codex/auth.json"), "{}").unwrap();
    harness.run_provider_discovery_pass();
    assert!(
        std::fs::read_to_string(&starts)
            .unwrap()
            .lines()
            .any(|line| line == "app-server"),
        "a native login appearing later is still checked by the official server"
    );
}

#[test]
fn a_second_kernel_on_the_machine_reuses_the_same_login_at_its_first_start() {
    crate::test_support::isolated_env_test!();
    let _machine = Machine::new("second-kernel");
    let first = LocalRouterTestHarness::new();
    first.run_provider_discovery_pass();
    let second = LocalRouterTestHarness::new();
    assert_eq!(
        default_claude(&second).auth_state,
        ProviderAccountAuthState::Unknown
    );

    second.run_provider_discovery_pass();

    let profile = default_claude(&second);
    assert_eq!(profile.auth_state, ProviderAccountAuthState::Authenticated);
    assert_eq!(
        profile.identity_summary.as_deref(),
        Some("alice@example.com")
    );
    assert!(second.warmed_default_catalog().is_some());
}

#[test]
fn a_login_one_kernel_linked_in_a_machine_location_is_found_by_another() {
    crate::test_support::isolated_env_test!();
    let machine = Machine::new("linked-location");
    let work = machine.claude_config_dir(".claude-work", false);
    let first = LocalRouterTestHarness::new();
    first.run_provider_discovery_pass();
    first
        .dispatch(request(serde_json::json!({"LinkProviderAccountProfile": {
            "provider": "claude", "label": "Work", "path": work.display().to_string(),
        }})))
        .unwrap();

    let second = LocalRouterTestHarness::new();
    second.run_provider_discovery_pass();

    let adopted = claude_profiles(&second)
        .into_iter()
        .find(|profile| profile.identity_summary.as_deref() == Some("bob@example.com"))
        .expect("the linked login is registered without any step in the second kernel");
    assert_eq!(adopted.auth_state, ProviderAccountAuthState::Authenticated);

    // Removing it again is the user's decision; the next pass does not undo it.
    second
        .dispatch(request(
            serde_json::json!({"RemoveProviderAccountProfile": {
                "provider": "claude", "account_profile": adopted.profile_id,
            }}),
        ))
        .unwrap();
    second.run_provider_discovery_pass();
    assert!(claude_profiles(&second)
        .iter()
        .all(|profile| profile.identity_summary.as_deref() != Some("bob@example.com")));
}

#[test]
fn other_claude_config_directories_holding_a_login_are_discovered() {
    crate::test_support::isolated_env_test!();
    let machine = Machine::new("claude-scan");
    machine.claude_config_dir(".claude-work", true);
    machine.claude_config_dir(".claude-signed-out", true);
    machine.claude_config_dir(".claude-never-logged-in", false);
    let harness = LocalRouterTestHarness::new();

    harness.run_provider_discovery_pass();

    let identities: Vec<_> = claude_profiles(&harness)
        .into_iter()
        .filter(|profile| profile.auth_state == ProviderAccountAuthState::Authenticated)
        .filter_map(|profile| profile.identity_summary)
        .collect();
    assert!(identities.contains(&"alice@example.com".to_string()));
    assert!(identities.contains(&"bob@example.com".to_string()));
    assert_eq!(
        claude_profiles(&harness).len(),
        2,
        "a signed-out or never-used directory is not registered"
    );
}

#[test]
fn a_profile_becoming_authenticated_changes_what_every_client_is_told() {
    crate::test_support::isolated_env_test!();
    let _machine = Machine::new("revision");
    let harness = LocalRouterTestHarness::new();
    harness.run_provider_discovery_pass();
    let snapshot = |known| match harness
        .dispatch(LocalDaemonRequest::GetClientCatalogSnapshot(
            crate::local::GetClientCatalogSnapshotRequest {
                scope: Default::default(),
                known,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::ClientCatalogSnapshot { snapshot } => snapshot,
        other => panic!("unexpected response {other:?}"),
    };
    let before = snapshot(None);
    let (stop, task) = harness.spawn_provider_discovery();
    // Let the task's own first pass settle so only the change below is measured.
    std::thread::sleep(Duration::from_millis(500));
    let sequence = harness.client_catalog_change_sequence();

    // Signed out and back in outside every request handler that invalidates the catalog.
    let registry = harness
        .runtime_state()
        .provider_account_profile_registry()
        .clone();
    let profile = default_claude(&harness);
    registry
        .mark_logged_out(OWNER, "claude", &profile.profile_id)
        .unwrap();
    wait_until(|| harness.client_catalog_change_sequence() != sequence);

    let after = snapshot(Some(before.revisions.clone()));
    assert_ne!(
        after.revisions.provider_accounts,
        before.revisions.provider_accounts
    );
    let accounts = after.provider_accounts.expect("changed accounts are sent");
    assert!(accounts
        .iter()
        .any(|account| account.auth_state == ProviderAccountAuthState::NotConfigured));
    let _ = stop.send(true);
    harness.block_on_test_task(task).unwrap();
}

fn wait_until(mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "the kernel did not announce the change within 20 s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
