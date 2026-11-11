//! Discovery against a registry that is closed and reopened, as a kernel restart does.
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use super::*;
use crate::account_profile::ProviderAccountProfileRegistry;
use crate::config::DaemonConfig;

const OWNER: &str = crate::session::DEFAULT_LOCAL_USER_ID;

/// A Claude CLI whose answer per configuration directory is a file the test writes:
/// no file is a CLI that fails, as a timeout or an unavailable Keychain does.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  --version) printf 'Claude test version\n' ;;
  auth)
    case "${CLAUDE_CONFIG_DIR:-}" in
      ""|*/.claude) name=default ;;
      *) name=$(basename "$CLAUDE_CONFIG_DIR") ;;
    esac
    printf 'probe\n' >> "$HOME/fake-claude/probes$name"
    if [ -f "$HOME/fake-claude/login$name" ]; then cat "$HOME/fake-claude/login$name"; else exit 3; fi ;;
  *) exit 1 ;;
esac
"#;

const ALICE: &str = r#"{"loggedIn":true,"email":"alice@example.com","subscriptionType":"max"}"#;
const BOB: &str = r#"{"loggedIn":true,"email":"bob@example.com","subscriptionType":"pro"}"#;
const SIGNED_OUT: &str = r#"{"loggedIn":false}"#;

struct Machine {
    home: PathBuf,
    registry_path: PathBuf,
}

impl Machine {
    fn new(label: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "chariox-discovery-restart-{label}-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        let bin = home.join("fake-bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(home.join("fake-claude")).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
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
        let registry_path = home.join("state").join("provider-accounts.json");
        std::fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        Self {
            home,
            registry_path,
        }
    }

    /// What the CLI answers for `name` (`default` or a directory name such as `.claude-work`).
    fn cli_answers(&self, name: &str, reply: &str) {
        std::fs::write(
            self.home.join("fake-claude").join(format!("login{name}")),
            reply,
        )
        .unwrap();
    }

    fn cli_fails(&self, name: &str) {
        let _ = std::fs::remove_file(self.home.join("fake-claude").join(format!("login{name}")));
    }

    fn probes(&self, name: &str) -> usize {
        std::fs::read_to_string(self.home.join("fake-claude").join(format!("probes{name}")))
            .map_or(0, |log| log.lines().count())
    }

    fn claude_config_dir(&self, name: &str) -> PathBuf {
        let dir = self.home.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(dir.join(".credentials.json"), "{}").unwrap();
        dir
    }

    /// A kernel start: the registry file is read again, nothing else is remembered.
    fn open_registry(&self) -> ProviderAccountProfileRegistry {
        ProviderAccountProfileRegistry::open(&self.registry_path).unwrap()
    }

    fn discover(&self, registry: &ProviderAccountProfileRegistry) -> DiscoveryOutcome {
        discover_machine_providers(registry, OWNER, &self.home, &DaemonConfig::for_tests())
    }

    /// The kernel was down for `hours`: every recorded check is that much older.
    fn age_recorded_checks(&self, hours: u64) {
        let mut document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&self.registry_path).unwrap()).unwrap();
        let aged = crate::session::unix_epoch_ms() - hours * 3_600_000;
        for profile in document["profiles"].as_array_mut().unwrap() {
            profile["last_validated_at_ms"] = serde_json::json!(aged);
        }
        std::fs::write(&self.registry_path, serde_json::to_vec(&document).unwrap()).unwrap();
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn identities(registry: &ProviderAccountProfileRegistry) -> Vec<Option<String>> {
    registry
        .list(OWNER, Some("claude"))
        .unwrap()
        .into_iter()
        .filter(|profile| profile.auth_state == ProviderAccountAuthState::Authenticated)
        .map(|profile| profile.identity_summary)
        .collect()
}

#[test]
fn a_probe_that_found_no_login_does_not_hide_a_later_login_at_the_same_path() {
    crate::test_support::isolated_env_test!();
    for first_answer in [None, Some(SIGNED_OUT)] {
        let machine = Machine::new("later-login");
        machine.claude_config_dir(".claude-work");
        match first_answer {
            Some(reply) => machine.cli_answers(".claude-work", reply),
            None => machine.cli_fails(".claude-work"),
        }
        let registry = machine.open_registry();
        let outcome = machine.discover(&registry);
        assert!(outcome.adopted.is_empty(), "{first_answer:?}: {outcome:?}");
        assert!(registry.list(OWNER, Some("claude")).unwrap().is_empty());
        drop(registry);

        machine.cli_answers(".claude-work", BOB);
        let restarted = machine.open_registry();
        let outcome = machine.discover(&restarted);

        assert_eq!(
            outcome.adopted.len(),
            1,
            "{first_answer:?}: the location was claimed by the failed probe"
        );
        assert_eq!(
            identities(&restarted),
            vec![Some("bob@example.com".to_string())]
        );
    }
}

#[test]
fn removing_a_discovered_account_is_still_not_undone_after_a_restart() {
    crate::test_support::isolated_env_test!();
    let machine = Machine::new("removed");
    machine.claude_config_dir(".claude-work");
    machine.cli_answers(".claude-work", BOB);
    let registry = machine.open_registry();
    let adopted = machine.discover(&registry).adopted;
    assert_eq!(adopted.len(), 1);
    registry
        .remove_registration(OWNER, "claude", &adopted[0])
        .unwrap();
    drop(registry);

    let restarted = machine.open_registry();
    let outcome = machine.discover(&restarted);

    assert!(outcome.adopted.is_empty());
    assert!(restarted.list(OWNER, Some("claude")).unwrap().is_empty());
    assert_eq!(
        machine.probes(".claude-work"),
        1,
        "removed means not asked again"
    );
}

#[test]
fn a_native_account_signed_in_as_someone_else_outside_chariox_replaces_the_one_shown() {
    crate::test_support::isolated_env_test!();
    for (reply, expected) in [
        (BOB, ProviderAccountAuthState::Authenticated),
        (SIGNED_OUT, ProviderAccountAuthState::NotConfigured),
    ] {
        let machine = Machine::new("switched");
        std::fs::create_dir_all(machine.home.join(".claude")).unwrap();
        std::fs::set_permissions(
            machine.home.join(".claude"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        machine.cli_answers("default", ALICE);
        let registry = machine.open_registry();
        registry
            .migrate_effective_defaults(OWNER, &machine.home)
            .unwrap();
        machine.discover(&registry);
        assert_eq!(
            identities(&registry),
            vec![Some("alice@example.com".to_string())]
        );
        drop(registry);

        machine.cli_answers("default", reply);
        machine.age_recorded_checks(1);
        let restarted = machine.open_registry();
        machine.discover(&restarted);

        let shown = restarted
            .list(OWNER, Some("claude"))
            .unwrap()
            .into_iter()
            .find(|profile| profile.is_default)
            .unwrap();
        assert_eq!(shown.auth_state, expected, "{reply}");
        assert_eq!(
            shown.identity_summary.as_deref(),
            (expected == ProviderAccountAuthState::Authenticated).then_some("bob@example.com"),
            "{reply}: the waiting room still shows the account that signed out"
        );
    }
}

#[test]
fn only_claude_and_opencode_authenticated_logins_are_checked_again_when_stale() {
    crate::test_support::isolated_env_test!();
    let machine = Machine::new("stale");
    machine.cli_answers("default", ALICE);
    let registry = machine.open_registry();
    registry
        .migrate_effective_defaults(OWNER, &machine.home)
        .unwrap();
    machine.discover(&registry);
    let mut profile = registry
        .list(OWNER, Some("claude"))
        .unwrap()
        .into_iter()
        .find(|profile| profile.is_default)
        .unwrap();
    assert_eq!(profile.auth_state, ProviderAccountAuthState::Authenticated);
    let checked = profile.last_validated_at_ms.unwrap();
    let a_day_later = checked + 24 * 3_600_000;

    assert!(!needs_validation(&profile, checked), "just checked");
    assert!(needs_validation(&profile, a_day_later), "claude");
    profile.provider = "opencode".to_string();
    assert!(needs_validation(&profile, a_day_later), "opencode");
    profile.provider = "codex".to_string();
    assert!(
        !needs_validation(&profile, a_day_later),
        "codex only reloads its login on an explicit refresh"
    );
}

#[test]
fn an_unusable_location_is_probed_again_after_a_growing_pause() {
    crate::test_support::isolated_env_test!();
    let machine = Machine::new("backoff");
    machine.claude_config_dir(".claude-work");
    let registry = machine.open_registry();
    let config = DaemonConfig::for_tests();
    let pass = |now_ms: u64| discover_at(&registry, OWNER, &machine.home, &config, now_ms);
    let start = crate::session::unix_epoch_ms();
    let second = 1000;

    pass(start);
    assert_eq!(machine.probes(".claude-work"), 1);
    pass(start + 30 * second);
    assert_eq!(
        machine.probes(".claude-work"),
        1,
        "30 s after the first failure"
    );
    pass(start + 61 * second);
    assert_eq!(
        machine.probes(".claude-work"),
        2,
        "a minute after the first failure"
    );
    pass(start + 61 * second + 119 * second);
    assert_eq!(
        machine.probes(".claude-work"),
        2,
        "the second pause is two minutes"
    );

    machine.cli_answers(".claude-work", BOB);
    let outcome = pass(start + 61 * second + 121 * second);
    assert_eq!(
        outcome.adopted.len(),
        1,
        "the login is found once the pause is over"
    );
    assert_eq!(machine.probes(".claude-work"), 3);
}

#[test]
fn the_pause_between_probes_of_an_unusable_location_stops_growing_at_half_an_hour() {
    crate::test_support::isolated_env_test!();
    let machine = Machine::new("backoff-cap");
    machine.claude_config_dir(".claude-work");
    let registry = machine.open_registry();
    let config = DaemonConfig::for_tests();
    let minute = 60 * 1000;
    let mut now_ms = crate::session::unix_epoch_ms();
    for probes in 1..=12 {
        discover_at(&registry, OWNER, &machine.home, &config, now_ms);
        assert_eq!(machine.probes(".claude-work"), probes);
        now_ms += 31 * minute;
    }

    discover_at(
        &registry,
        OWNER,
        &machine.home,
        &config,
        now_ms - 3 * minute,
    );
    assert_eq!(
        machine.probes(".claude-work"),
        12,
        "29 min after the last probe"
    );
    discover_at(&registry, OWNER, &machine.home, &config, now_ms);
    assert_eq!(
        machine.probes(".claude-work"),
        13,
        "31 min after the last probe"
    );
}

fn claude_profile(
    registry: &ProviderAccountProfileRegistry,
    linked: bool,
) -> ProviderAccountProfile {
    registry
        .list(OWNER, Some("claude"))
        .unwrap()
        .into_iter()
        .find(|profile| profile.is_default != linked)
        .unwrap()
}

#[test]
fn a_linked_account_rejected_while_the_default_login_was_the_same_account_returns_after_the_switch_back(
) {
    crate::test_support::isolated_env_test!();
    let machine = Machine::new("collision");
    std::fs::create_dir_all(machine.home.join(".claude")).unwrap();
    std::fs::set_permissions(
        machine.home.join(".claude"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    machine.claude_config_dir(".claude-work");
    machine.cli_answers("default", ALICE);
    machine.cli_answers(".claude-work", BOB);
    let registry = machine.open_registry();
    registry
        .migrate_effective_defaults(OWNER, &machine.home)
        .unwrap();
    machine.discover(&registry);
    let linked = claude_profile(&registry, true);
    assert_eq!(linked.auth_state, ProviderAccountAuthState::Authenticated);
    assert_eq!(linked.identity_summary.as_deref(), Some("bob@example.com"));
    drop(registry);

    // The default CLI is signed in as Bob outside Chariox: the default wins the
    // duplicate-identity check and the linked Bob profile is rejected.
    machine.cli_answers("default", BOB);
    machine.age_recorded_checks(1);
    let restarted = machine.open_registry();
    machine.discover(&restarted);
    assert_eq!(
        claude_profile(&restarted, true).auth_state,
        ProviderAccountAuthState::Error,
        "the collision is what rejects the linked profile"
    );
    drop(restarted);

    // Signed back in as Alice, across restarts: the linked Bob login was never
    // wrong, so a bounded recheck validates it again.
    machine.cli_answers("default", ALICE);
    for _ in 0..2 {
        machine.age_recorded_checks(1);
        let restarted = machine.open_registry();
        machine.discover(&restarted);
    }
    let reopened = machine.open_registry();
    let linked = claude_profile(&reopened, true);
    assert_eq!(linked.auth_state, ProviderAccountAuthState::Authenticated);
    assert_eq!(linked.identity_summary.as_deref(), Some("bob@example.com"));
    assert_eq!(
        claude_profile(&reopened, false).identity_summary.as_deref(),
        Some("alice@example.com")
    );
}

#[test]
fn a_native_profile_in_error_is_rechecked_like_an_authenticated_one_except_codex() {
    crate::test_support::isolated_env_test!();
    let machine = Machine::new("error-recheck");
    machine.cli_answers("default", ALICE);
    let registry = machine.open_registry();
    registry
        .migrate_effective_defaults(OWNER, &machine.home)
        .unwrap();
    machine.discover(&registry);
    let mut profile = claude_profile(&registry, false);
    profile.auth_state = ProviderAccountAuthState::Error;
    profile.last_validated_at_ms = Some(1_000_000);
    let a_day_later = 1_000_000 + 24 * 3_600_000;

    assert!(!needs_validation(&profile, 1_000_000), "just checked");
    assert!(needs_validation(&profile, a_day_later), "claude");
    profile.provider = "opencode".to_string();
    assert!(needs_validation(&profile, a_day_later), "opencode");
    profile.provider = "codex".to_string();
    assert!(!needs_validation(&profile, a_day_later), "codex");
}
