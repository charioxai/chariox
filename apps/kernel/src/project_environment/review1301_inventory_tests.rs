//! MP-08 / MP-10 / MP-11: sanitized probes keep a vetted script interpreter.
use super::*;
#[cfg(unix)]
#[test]
fn review1301_inventory_and_health_use_selected_script_runtime() {
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
