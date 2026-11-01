//! MP-08 / MP-09 / MP-10 / MP-11: state writes use the selected launch account.
use super::*;

#[test]
fn claude_onboarding_ignores_a_removed_ambient_profile() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("claude-selected-state");
    let ambient = root.path().join("ambient-profile");
    let selected = root.path().join("selected-home");
    let ambient_home = root.path().join("ambient-home");
    for path in [&ambient, &selected, &ambient_home] {
        fs::create_dir(path).unwrap();
    }
    std::env::set_var("CLAUDE_CONFIG_DIR", &ambient);
    std::env::set_var("HOME", &ambient_home);
    let request =
        LaunchProviderRequest::new("session", "claude", "claude-headless", "default", "haiku")
            .with_provider_account_env(BTreeMap::from([(
                "HOME".into(),
                selected.display().to_string(),
            )]))
            .with_provider_env_remove(vec!["CLAUDE_CONFIG_DIR".into()]);
    ensure_claude_headless_onboarding_state(Some(&request)).unwrap();
    assert!(
        !ambient.join(".claude.json").exists(),
        "removed ambient profile must stay untouched"
    );
    assert!(
        !ambient_home.join(".claude.json").exists(),
        "ambient HOME must stay untouched"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(selected.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(value["hasCompletedOnboarding"], true);
}

#[test]
fn claude_onboarding_uses_selected_home_without_a_config_override() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("claude-home-state");
    let selected = root.path().join("selected-home");
    let ambient = root.path().join("ambient-home");
    fs::create_dir(&selected).unwrap();
    fs::create_dir(&ambient).unwrap();
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::set_var("HOME", &ambient);
    let request =
        LaunchProviderRequest::new("session", "claude", "claude-headless", "selected", "haiku")
            .with_provider_account_env(BTreeMap::from([(
                "HOME".into(),
                selected.display().to_string(),
            )]));
    ensure_claude_headless_onboarding_state(Some(&request)).unwrap();
    assert!(
        !ambient.join(".claude.json").exists(),
        "selected HOME must own the provider state"
    );
    assert!(selected.join(".claude.json").exists());
}

#[test]
fn claude_onboarding_preserves_literal_profile_path_spaces() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("claude-profile-path-spaces");
    let selected = root.path().join("profile ");
    fs::create_dir(&selected).unwrap();
    let request =
        LaunchProviderRequest::new("session", "claude", "claude-headless", "selected", "haiku")
            .with_provider_account_env(BTreeMap::from([(
                "CLAUDE_CONFIG_DIR".into(),
                selected.display().to_string(),
            )]));
    ensure_claude_headless_onboarding_state(Some(&request)).unwrap();
    assert!(
        selected.join(".claude.json").exists(),
        "the provider uses the literal profile path"
    );
    assert!(!root.path().join("profile/.claude.json").exists());
}

#[test]
fn claude_onboarding_uses_the_launch_credential_home() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("claude-bound-home-state");
    let selected = root.path().join("selected-home");
    let ambient = root.path().join("ambient-home");
    fs::create_dir(&selected).unwrap();
    fs::create_dir(&ambient).unwrap();
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::set_var("HOME", &ambient);
    let mut environment = crate::provider::ProviderCredentialEnvironment::default();
    environment.insert(
        "HOME",
        zeroize::Zeroizing::new(selected.display().to_string()),
    );
    let request =
        LaunchProviderRequest::new("session", "claude", "claude-headless", "selected", "haiku")
            .with_provider_credential_env(environment);
    ensure_claude_headless_onboarding_state(Some(&request)).unwrap();
    assert!(
        !ambient.join(".claude.json").exists(),
        "launch-bound HOME must own the provider state"
    );
    assert!(selected.join(".claude.json").exists());
}

#[cfg(unix)]
#[test]
fn claude_onboarding_errors_do_not_echo_a_private_launch_home() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("claude-private-home-error");
    let selected = root.path().join("private-home-fixture-value");
    let ambient = root.path().join("ambient-home");
    fs::create_dir(&selected).unwrap();
    fs::create_dir(&ambient).unwrap();
    let target = root.path().join("unchosen-state");
    fs::write(&target, "unchanged").unwrap();
    std::os::unix::fs::symlink(&target, selected.join(".claude.json")).unwrap();
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    std::env::set_var("HOME", &ambient);
    let mut environment = crate::provider::ProviderCredentialEnvironment::default();
    environment.insert(
        "HOME",
        zeroize::Zeroizing::new(selected.display().to_string()),
    );
    let request =
        LaunchProviderRequest::new("session", "claude", "claude-headless", "selected", "haiku")
            .with_provider_credential_env(environment);
    let error = ensure_claude_headless_onboarding_state(Some(&request))
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a regular file"));
    assert!(!error.contains("private-home-fixture-value"));
    assert!(!ambient.join(".claude.json").exists());
    assert_eq!(fs::read_to_string(target).unwrap(), "unchanged");
}

#[cfg(unix)]
#[test]
fn claude_onboarding_errors_keep_public_selected_profile_paths() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("claude-public-profile-error");
    let selected = root.path().join("public-profile");
    fs::create_dir(&selected).unwrap();
    let target = root.path().join("unchosen-state");
    fs::write(&target, "unchanged").unwrap();
    std::os::unix::fs::symlink(&target, selected.join(".claude.json")).unwrap();
    let mut environment = crate::provider::ProviderCredentialEnvironment::default();
    environment.insert(
        "HOME",
        zeroize::Zeroizing::new("private-unused-home-fixture".into()),
    );
    let request =
        LaunchProviderRequest::new("session", "claude", "claude-headless", "selected", "haiku")
            .with_provider_account_env(BTreeMap::from([(
                "CLAUDE_CONFIG_DIR".into(),
                selected.display().to_string(),
            )]))
            .with_provider_credential_env(environment);
    let error = ensure_claude_headless_onboarding_state(Some(&request))
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a regular file"));
    assert!(error.contains(&selected.display().to_string()));
    assert!(!error.contains("private-unused-home-fixture"));
}
