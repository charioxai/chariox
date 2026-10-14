//! MP-08 / MP-10 / MP-11: sanitized probes keep a vetted script interpreter.
use super::*;
#[cfg(unix)]
#[test]
fn check_interpreter_trust_and_health_use_selected_script_runtime() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    use std::os::unix::fs::PermissionsExt;
    let tools = crate::test_support::TestWorktree::new("review1301-tools");
    let workspace = crate::test_support::TestWorktree::new("review1301-workspace");
    let node = super::check_inventory::executable("node").expect("fixture requires installed Node");
    std::fs::write(
        tools.path().join("node"),
        format!(
            "#!/bin/sh\nexport CHARIOX_SELECTED_RUNTIME=1\nexec '{}' \"$@\"\n",
            node.display()
        ),
    )
    .unwrap();
    std::fs::write(tools.path().join("npm"), "#!/usr/bin/env node\nconsole.log(process.env.CHARIOX_SELECTED_RUNTIME === '1' ? '98.7.6' : '11.2.3'); process.exit(process.env.CHARIOX_SELECTED_RUNTIME === '1' ? 0 : 1);\n").unwrap();
    for name in ["node", "npm"] {
        std::fs::set_permissions(
            tools.path().join(name),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let prior = std::env::var_os("PATH").unwrap();
    std::env::set_var(
        "PATH",
        std::env::join_paths(
            std::iter::once(tools.path().to_path_buf()).chain(std::env::split_paths(&prior)),
        )
        .unwrap(),
    );
    let version = super::check_inventory::version("npm", &[workspace.path().to_path_buf()]);
    let probe = EnvironmentProbe::Executable {
        name: "npm".into(),
        arguments: vec!["--version".into()],
        timeout_ms: 2000,
    };
    let health = super::check_inventory::health(&probe, &None, &[workspace.path().to_path_buf()]);
    std::env::set_var("PATH", prior);
    assert_eq!(
        version,
        Ok(Some("98.7.6".into())),
        "selected npm must use its vetted Node instead of the fixed PATH's runtime"
    );
    assert_eq!(health, Ok(true));
}

#[cfg(unix)]
#[test]
fn review1301_script_runtime_rejects_workspace_interpreters_and_cycles() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    use std::os::unix::fs::PermissionsExt;
    let tools = crate::test_support::TestWorktree::new("review1301-trust-tools");
    let workspace = crate::test_support::TestWorktree::new("review1301-untrusted-runtime");
    let executable = tools.path().join("npm");
    std::fs::write(&executable, "#!/usr/bin/env node\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let runtime = workspace.path().join("node");
    std::fs::write(&runtime, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink(&runtime, tools.path().join("node")).unwrap();
    let previous = std::env::var_os("PATH").unwrap();
    std::env::set_var("PATH", tools.path());
    assert_eq!(
        super::check_executable::command(&executable, &[workspace.path().to_path_buf()]).err(),
        Some("untrusted_executable")
    );
    std::fs::remove_file(tools.path().join("node")).unwrap();
    std::fs::write(tools.path().join("node"), "#!/usr/bin/env npm\n").unwrap();
    std::fs::set_permissions(
        tools.path().join("node"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(
        super::check_executable::command(&executable, &[workspace.path().to_path_buf()]).err(),
        Some("interpreter_cycle")
    );
    std::env::set_var("PATH", previous);
}

// MP-08 / MP-10 / MP-11: env chains must retain each shim's invocation name.
#[cfg(unix)]
#[test]
fn env_shebang_chain_preserves_symlink_runtime_name() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    use std::os::unix::fs::PermissionsExt;
    let tools = crate::test_support::TestWorktree::new("env-shim-chain");
    let workspace = crate::test_support::TestWorktree::new("env-shim-workspace");
    std::os::unix::fs::symlink("/bin/bash", tools.path().join("sh")).unwrap();
    let prior = std::env::var_os("PATH").unwrap();
    std::env::set_var("PATH", tools.path());
    for shebang in [
        "#!/usr/bin/env sh",
        "#!/usr/bin/env -S sh -e",
        "#!/usr/bin/env node",
    ] {
        std::fs::write(
            tools.path().join("node"),
            "#!/usr/bin/env sh\nif shopt -oq posix; then printf '98.7.6\\n'; else exit 1; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(
            tools.path().join("node"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(
            tools.path().join("npm"),
            format!("{shebang}\nif shopt -oq posix; then printf '98.7.6\\n'; else exit 1; fi\n"),
        )
        .unwrap();
        std::fs::set_permissions(
            tools.path().join("npm"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let result = super::check_inventory::version("npm", &[workspace.path().to_path_buf()]);
        assert_eq!(result, Ok(Some("98.7.6".into())), "{shebang}");
    }
    std::env::set_var("PATH", prior);
}

#[cfg(unix)]
#[test]
fn font_and_apt_inventory_reject_canonical_workspace_executables() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    use std::os::unix::fs::PermissionsExt;
    let tools = crate::test_support::TestWorktree::new("inventory-trust-tools");
    let workspace = crate::test_support::TestWorktree::new("inventory-trust-workspace");
    let marker = workspace.path().join("executed");
    for name in ["fc-list", "dpkg-query"] {
        std::fs::write(
            workspace.path().join(name),
            format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(
            workspace.path().join(name),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::os::unix::fs::symlink(workspace.path().join(name), tools.path().join(name)).unwrap();
    }
    let prior = std::env::var_os("PATH").unwrap();
    std::env::set_var("PATH", tools.path());
    for source in ["font", "apt"] {
        assert_eq!(
            super::check_inventory::software_version(
                "public-tool",
                Some(source),
                None,
                &[workspace.path().canonicalize().unwrap()]
            ),
            Err("untrusted_executable"),
            "{source}"
        );
        assert!(!marker.exists(), "untrusted probe must not run");
    }
    // A PATH-directory alias must also fail before executing anything.
    std::os::unix::fs::symlink(workspace.path(), tools.path().join("alias")).unwrap();
    std::env::set_var("PATH", tools.path().join("alias"));
    for source in ["font", "apt"] {
        assert_eq!(
            super::check_inventory::software_version(
                "public-tool",
                Some(source),
                None,
                &[workspace.path().canonicalize().unwrap()]
            ),
            Err("untrusted_executable")
        );
    }
    std::env::set_var("PATH", prior);
}

// MP-08 / MP-10 / MP-11: canonical host binaries must not inherit a workspace venv.
#[cfg(unix)]
#[test]
fn workspace_invocation_paths_cannot_select_trusted_host_interpreters() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    use std::os::unix::fs::{symlink, PermissionsExt};
    let tools = crate::test_support::TestWorktree::new("host-interpreter-tools");
    let workspace = crate::test_support::TestWorktree::new("workspace-venv");
    let bin = workspace.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    symlink("/usr/bin/python3", bin.join("python3")).unwrap();
    symlink(&bin, tools.path().join("alias")).unwrap();
    symlink("/usr/bin", workspace.path().join("host-bin")).unwrap();
    let script = tools.path().join("npm");
    std::fs::write(&script, "#!/usr/bin/env python3\nprint('1.2.3')\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let roots = [workspace.path().canonicalize().unwrap()];
    let prior = std::env::var_os("PATH").unwrap();
    std::env::set_var("PATH", &bin);
    let interpreter = super::check_executable::command(&script, &roots).err();
    let alias = super::check_executable::command(&tools.path().join("alias/python3"), &roots).err();
    let directory_alias =
        super::check_executable::command(&workspace.path().join("host-bin/python3"), &roots).err();
    std::env::set_var("PATH", prior);
    assert_eq!(interpreter, Some("untrusted_executable"));
    assert_eq!(alias, Some("untrusted_executable"));
    assert_eq!(directory_alias, Some("untrusted_executable"));
}
