// Actual Bubblewrap boundary controls with synthetic accounts only.
#[cfg(target_os = "linux")]
#[test]
fn protected_slice_accounts_and_github_remain_inside_selected_namespace() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if std::fs::metadata("/proc/self")
        .expect("process metadata")
        .uid()
        != 0
        || !Path::new(BWRAP_PATH).is_file()
    {
        eprintln!(
            "requires Linux root and installed Bubblewrap for actual protected-account boundary"
        );
        return;
    }
    let _env = crate::env_lock::lock();
    // /tmp and /var/tmp are independently masked. This synthetic /var/lib
    // fixture makes the new private-root mask, rather than a temp mask, decisive.
    let root = PathBuf::from(format!(
        "/var/lib/chariox-protected-account-test-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir(&root).expect("unique synthetic root");
    // The runtime HOME is deliberately under /home: this exercises the
    // admitted late workspace rebind over the synthetic /home namespace.
    let runtime_home = PathBuf::from(format!(
        "/home/chariox-nss-alias-test-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir(&runtime_home).expect("unique synthetic runtime home");
    let names = [
        MANAGED_PROVIDER_ISOLATION_ENV,
        MANAGED_PROVIDER_HOME_ENV,
        "CHARIOX_HOME",
        "HOME",
        "CHARIOX_SLICE_PRIVATE_ROOT",
        "GH_CONFIG_DIR",
        MANAGED_PROVIDER_BWRAP_ENV,
    ];
    let previous = names
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect::<Vec<_>>();
    struct Cleanup {
        root: PathBuf,
        runtime_home: PathBuf,
        previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for (name, value) in self.previous.drain(..) {
                restore_env(name, value);
            }
            let _ = std::fs::remove_dir_all(&self.runtime_home); // Only this newly created synthetic home.
            let _ = std::fs::remove_dir_all(&self.root); // Only this newly created synthetic fixture.
        }
    }
    let _cleanup = Cleanup {
        root: root.clone(),
        runtime_home: runtime_home.clone(),
        previous,
    };
    let private = root.join("private");
    let home = private.join("provider-home");
    let selected = private.join("provider-accounts/person-a/codex/profile");
    let sibling = private.join("provider-accounts/person-b/codex/profile");
    // Default accounts live in the private root, outside the shared provider
    // HOME that every managed run binds at /home/chariox.
    let default_codex = private.join("provider-default/codex");
    let default_claude = private.join("provider-default/claude");
    let kernel = private.join("kernel");
    let nss = private.join("nssdb");
    let workspace = runtime_home.join(".local");
    let nss_alias = runtime_home.join(".local/share/pki/nssdb");
    for directory in [
        &home,
        &selected,
        &sibling,
        &default_codex,
        &default_claude,
        &kernel,
        &nss,
        &nss_alias,
        &workspace,
    ] {
        std::fs::create_dir_all(directory).expect("synthetic directories");
    }
    std::fs::create_dir_all(home.join(".config/gh")).expect("synthetic GitHub config");
    std::fs::create_dir(home.join("bin")).expect("synthetic utility path");
    std::fs::write(selected.join("auth.json"), "selected-synthetic-account").unwrap();
    std::fs::write(sibling.join("auth.json"), "sibling-synthetic-account").unwrap();
    std::fs::write(default_codex.join("auth.json"), "default-synthetic-account").unwrap();
    std::fs::write(
        default_claude.join(".credentials.json"),
        "default-synthetic-claude-account",
    )
    .unwrap();
    std::fs::write(kernel.join("private-sentinel"), "synthetic-kernel-private").unwrap();
    std::fs::write(nss.join("private-sentinel"), "synthetic-nss-private").unwrap();
    std::fs::write(nss_alias.join("private-sentinel"), "synthetic-nss-private").unwrap();
    std::fs::write(
        runtime_home.join(".local/ordinary-user-data"),
        "ordinary-synthetic",
    )
    .unwrap();
    std::fs::write(
        home.join(".config/gh/hosts.yml"),
        "synthetic-github-account",
    )
    .unwrap();
    let gh = home.join("bin/gh");
    std::fs::write(
        &gh,
        r#"#!/bin/sh
set -eu
case "$GH_CONFIG_DIR" in /home/chariox/.config/gh|/home/chariox/.provider-account/root-8) ;; *) exit 92 ;; esac
test "$(cat "$GH_CONFIG_DIR/hosts.yml")" = synthetic-github-account
case "${1-}" in
  auth) printf 'username=synthetic\npassword=synthetic\n\n' ;;
  *) exit 91 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(home.join(".gitconfig"), "[credential \"https://synthetic.invalid\"]\n\thelper = !/home/chariox/bin/gh auth git-credential\n").unwrap();
    let bwrap = root.join("bwrap");
    std::fs::copy(BWRAP_PATH, &bwrap).unwrap();
    std::fs::set_permissions(&bwrap, std::fs::Permissions::from_mode(0o755)).unwrap();
    for name in names {
        std::env::remove_var(name);
    }
    std::env::set_var(MANAGED_PROVIDER_ISOLATION_ENV, "1");
    std::env::set_var(MANAGED_PROVIDER_HOME_ENV, &home);
    std::env::set_var("CHARIOX_HOME", &kernel);
    std::env::set_var("CHARIOX_SLICE_PRIVATE_ROOT", &private);
    std::env::set_var("HOME", &runtime_home);
    std::env::set_var("GH_CONFIG_DIR", home.join(".config/gh"));
    std::env::set_var(MANAGED_PROVIDER_BWRAP_ENV, &bwrap);
    let private_runtime = managed_isolated_utility_launch(
        "/bin/true",
        Vec::new(),
        BTreeMap::from([(
            "CHARIOX_CLAUDE_NATIVE_SYNTHETIC".into(),
            private.display().to_string(),
        )]),
        Some(workspace.clone()),
        "private-runtime-refusal",
    );
    assert!(
        private_runtime.is_err(),
        "late runtime roots must not re-expose private storage"
    );
    let script = r#"
set -eu
test "$(cat "$CODEX_HOME/auth.json")" = selected-synthetic-account
test ! -r "$1/provider-accounts/person-a/codex/profile/auth.json"
test ! -r "$1/provider-accounts/person-b/codex/profile/auth.json"
test ! -r "$1/provider-default/codex/auth.json"
test ! -r "$1/provider-default/claude/.credentials.json"
test ! -e /home/chariox/.codex
test ! -e /home/chariox/.claude
test ! -r "$1/kernel/private-sentinel"
test ! -r "$1/nssdb/private-sentinel"
test ! -r "$2/.local/share/pki/nssdb/private-sentinel"
test "$(cat "$2/.local/ordinary-user-data")" = ordinary-synthetic
test -z "${CHARIOX_SLICE_PRIVATE_ROOT+x}"
test "$(/home/chariox/bin/gh auth status | head -n 1)" = username=synthetic
printf 'protocol=https\nhost=synthetic.invalid\n\n' | git credential fill | grep -qx password=synthetic
printf 'PROTECTED_ACCOUNT_AND_GITHUB_BOUNDARY_PASS\n'
"#;
    let mut launch = managed_isolated_utility_launch(
        "/bin/sh",
        vec![
            "-ec".into(),
            script.into(),
            "protected-account-probe".into(),
            private.display().to_string(),
            runtime_home.display().to_string(),
        ],
        BTreeMap::from([("CODEX_HOME".into(), selected.display().to_string())]),
        Some(workspace.clone()),
        "protected-account-boundary",
    )
    .expect("namespace launch");
    // Model the actual NSS second mount at the namespace boundary, after
    // .local has been exposed and immediately before its credential mask.
    let mask = launch
        .pty_args
        .windows(2)
        .position(|args| args[0] == "--tmpfs" && args[1] == nss_alias.display().to_string())
        .expect("NSS alias mask follows runtime .local bind");
    assert!(
        launch
            .pty_args
            .windows(3)
            .filter(|args| args[0] == "--bind"
                && args[1] == workspace.display().to_string()
                && args[2] == workspace.display().to_string())
            .count()
            >= 2,
        "the /home workspace must be rebound after its runtime anchor"
    );
    let workspace_rebind = launch
        .pty_args
        .windows(3)
        .rposition(|args| {
            args[0] == "--bind"
                && args[1] == workspace.display().to_string()
                && args[2] == workspace.display().to_string()
        })
        .expect("actual /home runtime workspace rebind");
    assert!(
        workspace_rebind < mask,
        "NSS mask must follow the late workspace bind"
    );
    launch.pty_args.splice(
        mask..mask,
        [
            "--bind".to_string(),
            nss.display().to_string(),
            nss_alias.display().to_string(),
        ],
    );
    let mut alias_negative = launch.clone();
    let final_mask = alias_negative
        .pty_args
        .windows(2)
        .position(|args| args[0] == "--tmpfs" && args[1] == nss_alias.display().to_string())
        .expect("final NSS mask");
    alias_negative.pty_args.drain(final_mask..final_mask + 2);
    assert!(
        !command_from_provider_launch(alias_negative)
            .unwrap()
            .status()
            .unwrap()
            .success(),
        "removing only the NSS alias mask must expose the synthetic second mount"
    );
    let output = command_from_provider_launch(launch)
        .expect("namespace command")
        .output()
        .expect("namespace execution");
    assert!(
        output.status.success(),
        "synthetic boundary failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "PROTECTED_ACCOUNT_AND_GITHUB_BOUNDARY_PASS"
    );
    // Runtime negative control: without the new whole-tree boundary, the
    // original account paths remain readable and the same denial script fails.
    std::env::remove_var("CHARIOX_SLICE_PRIVATE_ROOT");
    let negative = managed_isolated_utility_launch(
        "/bin/sh",
        vec![
            "-ec".into(),
            script.into(),
            "protected-account-negative".into(),
            private.display().to_string(),
            runtime_home.display().to_string(),
        ],
        BTreeMap::from([("CODEX_HOME".into(), selected.display().to_string())]),
        Some(workspace.clone()),
        "unmasked-account-negative",
    )
    .unwrap();
    assert!(!command_from_provider_launch(negative)
        .unwrap()
        .status()
        .unwrap()
        .success());
    std::env::set_var("CHARIOX_SLICE_PRIVATE_ROOT", &private);
    std::env::set_var("GH_CONFIG_DIR", sibling.join("unexpected-github"));
    assert!(managed_inherited_github_config(&home, &BTreeMap::new()).is_err());
    std::env::set_var("GH_CONFIG_DIR", home.join(".config/gh"));

    // Explicit GitHub selection is re-bound like every other selected account.
    let explicit_gh = private.join("provider-accounts/person-a/github/profile");
    std::fs::create_dir_all(&explicit_gh).unwrap();
    std::fs::write(explicit_gh.join("hosts.yml"), "synthetic-github-account").unwrap();
    let launch = managed_isolated_utility_launch("/bin/sh", vec!["-ec".into(),
        r#"test "$GH_CONFIG_DIR" = /home/chariox/.provider-account/root-8; /home/chariox/bin/gh auth status >/dev/null"#.into()],
        BTreeMap::from([("GH_CONFIG_DIR".into(), explicit_gh.display().to_string())]),
        Some(workspace.clone()), "explicit-github-boundary").unwrap();
    assert!(command_from_provider_launch(launch)
        .unwrap()
        .status()
        .unwrap()
        .success());
    // The default account is selected like any other account: only a run that
    // selects it receives it, through its own account binding.
    let launch = managed_isolated_utility_launch(
        "/bin/sh",
        vec![
            "-ec".into(),
            r#"case "$CODEX_HOME" in /home/chariox/.provider-account/root-*) ;; *) exit 93 ;; esac
test "$(cat "$CODEX_HOME/auth.json")" = default-synthetic-account
test ! -r "$1/provider-accounts/person-a/codex/profile/auth.json"
test ! -r "$1/provider-default/claude/.credentials.json""#
                .into(),
            "default-account-probe".into(),
            private.display().to_string(),
        ],
        BTreeMap::from([("CODEX_HOME".into(), default_codex.display().to_string())]),
        Some(workspace.clone()),
        "default-account-boundary",
    )
    .unwrap();
    assert!(command_from_provider_launch(launch)
        .unwrap()
        .status()
        .unwrap()
        .success());
    // Negative control for default roots inside the shared provider HOME: the
    // /home/chariox alias exposes them to a run that selected another account.
    std::fs::create_dir(home.join(".codex")).unwrap();
    std::fs::write(home.join(".codex/auth.json"), "default-synthetic-account").unwrap();
    let launch = managed_isolated_utility_launch(
        "/bin/sh",
        vec![
            "-ec".into(),
            r#"test "$(cat /home/chariox/.codex/auth.json)" = default-synthetic-account"#.into(),
        ],
        BTreeMap::from([("CODEX_HOME".into(), selected.display().to_string())]),
        Some(workspace.clone()),
        "provider-home-default-alias-negative",
    )
    .unwrap();
    assert!(
        command_from_provider_launch(launch)
            .unwrap()
            .status()
            .unwrap()
            .success(),
        "a default root inside the provider HOME must be visible through its alias"
    );
    // Remove only the synthetic negative fixture.
    std::fs::remove_dir_all(home.join(".codex")).unwrap();
    // Before login, the inherited directory may be absent. Translating HOME's
    // config path must not require or manufacture credentials to launch.
    std::fs::rename(
        home.join(".config/gh"),
        home.join(".config/gh-retained-synthetic"),
    )
    .unwrap();
    let launch = managed_isolated_utility_launch(
        "/bin/sh",
        vec![
            "-ec".into(),
            r#"test "$GH_CONFIG_DIR" = /home/chariox/.config/gh; test ! -e "$GH_CONFIG_DIR""#
                .into(),
        ],
        BTreeMap::new(),
        Some(workspace),
        "before-github-login",
    )
    .unwrap();
    assert!(command_from_provider_launch(launch)
        .unwrap()
        .status()
        .unwrap()
        .success());
    println!(
        "PROTECTED_ACCOUNT_ISOLATION_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.protected_account_isolation_probe.v1",
            "actualNamespaceExecuted": true, "selectedAccountReadable": true,
            "originalAccountTreeHidden": true, "siblingAccountHidden": true,
            "kernelAndNssHidden": true, "namespaceNssSecondMountHidden": true, "privateRootControlScrubbed": true,
            "maskRemovalNegativeFailed": true, "ambientGitHubAndGitWorked": true,
            "explicitGitHubWorked": true, "preLoginDirectoryMayBeAbsent": true,
            "outsideHomeAmbientGitHubRefused": true,
            "defaultAccountHiddenFromOtherSelection": true, "defaultAccountSelectedReadable": true,
            "providerHomeDefaultAliasNegativeExposed": true,
        })
    );
}
