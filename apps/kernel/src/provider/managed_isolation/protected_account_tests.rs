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
        previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for (name, value) in self.previous.drain(..) {
                restore_env(name, value);
            }
            let _ = std::fs::remove_dir_all(&self.root); // Only this newly created synthetic fixture.
        }
    }
    let _cleanup = Cleanup {
        root: root.clone(),
        previous,
    };
    let private = root.join("private");
    let home = private.join("provider-home");
    let selected = private.join("provider-accounts/person-a/codex/profile");
    let sibling = private.join("provider-accounts/person-b/codex/profile");
    let kernel = private.join("kernel");
    let nss = private.join("nssdb");
    let workspace = root.join("workspace");
    for directory in [&home, &selected, &sibling, &kernel, &nss, &workspace] {
        std::fs::create_dir_all(directory).expect("synthetic directories");
    }
    std::fs::create_dir_all(home.join(".config/gh")).expect("synthetic GitHub config");
    std::fs::create_dir(home.join("bin")).expect("synthetic utility path");
    std::fs::write(selected.join("auth.json"), "selected-synthetic-account").unwrap();
    std::fs::write(sibling.join("auth.json"), "sibling-synthetic-account").unwrap();
    std::fs::write(kernel.join("private-sentinel"), "synthetic-kernel-private").unwrap();
    std::fs::write(nss.join("private-sentinel"), "synthetic-nss-private").unwrap();
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
    std::env::set_var("HOME", &root);
    std::env::set_var("GH_CONFIG_DIR", home.join(".config/gh"));
    std::env::set_var(MANAGED_PROVIDER_BWRAP_ENV, &bwrap);
    let script = r#"
set -eu
test "$(cat "$CODEX_HOME/auth.json")" = selected-synthetic-account
test ! -r "$1/provider-accounts/person-a/codex/profile/auth.json"
test ! -r "$1/provider-accounts/person-b/codex/profile/auth.json"
test ! -r "$1/kernel/private-sentinel"
test ! -r "$1/nssdb/private-sentinel"
test -z "${CHARIOX_SLICE_PRIVATE_ROOT+x}"
test "$(/home/chariox/bin/gh auth status | head -n 1)" = username=synthetic
printf 'protocol=https\nhost=synthetic.invalid\n\n' | git credential fill | grep -qx password=synthetic
printf 'PROTECTED_ACCOUNT_AND_GITHUB_BOUNDARY_PASS\n'
"#;
    let launch = managed_isolated_utility_launch(
        "/bin/sh",
        vec![
            "-ec".into(),
            script.into(),
            "protected-account-probe".into(),
            private.display().to_string(),
        ],
        BTreeMap::from([("CODEX_HOME".into(), selected.display().to_string())]),
        Some(workspace.clone()),
        "protected-account-boundary",
    )
    .expect("namespace launch");
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
    println!("PROTECTED_ACCOUNT_ISOLATION_PROBE:{}", serde_json::json!({
        "schema": "chariox.protected_account_isolation_probe.v1",
        "actualNamespaceExecuted": true, "selectedAccountReadable": true,
        "originalAccountTreeHidden": true, "siblingAccountHidden": true,
        "kernelAndNssHidden": true, "privateRootControlScrubbed": true,
        "maskRemovalNegativeFailed": true, "ambientGitHubAndGitWorked": true,
        "explicitGitHubWorked": true, "preLoginDirectoryMayBeAbsent": true,
        "outsideHomeAmbientGitHubRefused": true,
    }));

}
