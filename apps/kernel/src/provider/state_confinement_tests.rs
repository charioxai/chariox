//! Real processes started through the kernel's own launch paths, against a
//! kernel home of dummy secrets.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::test_temp_dir::TestTempDir;

/// A kernel home with a dummy secret in every private place, and the folders
/// an agent owns.
pub(crate) struct DummyKernelHome {
    _root: TestTempDir,
    _env: crate::env_lock::EnvGuard,
    pub(crate) home: PathBuf,
    pub(crate) profile: PathBuf,
    pub(crate) scratch: PathBuf,
    pub(crate) workspace: PathBuf,
}

pub(crate) const SECRET: &str = "dummy-kernel-secret";

/// Secrets as an agent would name them, relative to the kernel home.
pub(crate) const SECRET_FILES: &[&str] = &[
    "state/cloud-relay.json",
    "state/kernel.db",
    "state/provider-accounts/local/claude/other/claude/.credentials.json",
    "vault.json",
    "vault.json.managed-context-key-0.json",
    "vault/vault.json",
    "credentials/cap-1.yaml",
    "machine/identity.json",
    "daemon/config.json",
    "kernels/kernel-1/state.db",
    "history/operational.db",
    "sessions/room-1.jsonl",
];

impl DummyKernelHome {
    pub(crate) fn new() -> Self {
        let env = crate::env_lock::lock();
        let root = TestTempDir::new("state-confinement");
        let home = root.path().join("home");
        for file in SECRET_FILES {
            write(&home.join(file), SECRET);
        }
        let profile = home.join("state/provider-accounts/local/claude/own/claude");
        write(&profile.join(".credentials.json"), "own-login");
        write(
            &home.join("state/kernel-local-auth/7000.token"),
            "local-token",
        );
        write(&home.join("kernels/active/kernel-1.json"), "{}");
        write(&home.join("config.toml"), "# settings");
        let scratch = home.join("t/room1/agent1");
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::env::set_var("CHARIOX_HOME", &home);
        std::env::remove_var(crate::provider::MANAGED_PROVIDER_ISOLATION_ENV);
        Self {
            _root: root,
            _env: env,
            home,
            profile,
            scratch,
            workspace,
        }
    }

    pub(crate) fn agent_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                self.profile.display().to_string(),
            ),
            ("TMPDIR".to_string(), self.scratch.display().to_string()),
            (
                "PATH".to_string(),
                std::env::var("PATH").unwrap_or_default(),
            ),
        ])
    }

    /// A script that reports, one line each, what the agent can reach.
    pub(crate) fn probe_script(&self) -> String {
        let mut script = String::new();
        for file in SECRET_FILES {
            script.push_str(&format!(
                "if cat '{0}' >/dev/null 2>&1; then echo 'secret-open {1}'; else echo 'secret-closed {1}'; fi\n",
                self.home.join(file).display(),
                file
            ));
        }
        script.push_str(&format!(
            "echo \"state: $(ls '{state}' | tr '\\n' ' ')\"\n\
             cat '{profile}/.credentials.json' && echo\n\
             echo written > '{profile}/settings.json' && echo profile-writable\n\
             echo written > '{workspace}/file' && echo workspace-writable\n\
             echo written > \"$TMPDIR/file\" && echo scratch-writable\n\
             cat '{home}/state/kernel-local-auth/7000.token' && echo\n\
             echo x 2>/dev/null > '{home}/state/kernel-local-auth/7000.token' || echo local-auth-read-only\n\
             cat '{home}/config.toml' && echo\n",
            state = self.home.join("state").display(),
            profile = self.profile.display(),
            workspace = self.workspace.display(),
            home = self.home.display(),
        ));
        script
    }

    /// Fails unless `output` shows every secret closed and the agent's own folders open.
    pub(crate) fn assert_confined(&self, output: &str) {
        for file in SECRET_FILES {
            assert!(
                output.contains(&format!("secret-closed {file}")),
                "{file} must be out of the agent's reach:\n{output}"
            );
        }
        assert!(
            !output.contains(SECRET),
            "a secret reached the agent:\n{output}"
        );
        for reachable in [
            "state: kernel-local-auth provider-accounts",
            "own-login",
            "profile-writable",
            "workspace-writable",
            "scratch-writable",
            "local-token",
            "local-auth-read-only",
            "# settings",
        ] {
            assert!(
                output.contains(reachable),
                "the agent must keep {reachable:?}:\n{output}"
            );
        }
    }
}

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[test]
fn a_command_the_kernel_starts_for_an_agent_cannot_read_the_kernels_secrets() {
    let kernel = DummyKernelHome::new();
    let launch = crate::provider::managed_isolated_utility_launch(
        "/bin/sh",
        vec!["-c".to_string(), kernel.probe_script()],
        kernel.agent_environment(),
        Some(kernel.workspace.clone()),
        "agent-watched-process",
    )
    .expect("an agent command launch is planned");
    let output = crate::provider::command_from_provider_launch(launch)
        .expect("an agent command is built")
        .output()
        .expect("an agent command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    kernel.assert_confined(&stdout);
}

#[test]
fn an_agent_folder_cannot_be_the_kernels_state_folder() {
    let kernel = DummyKernelHome::new();
    let mut environment = kernel.agent_environment();
    environment.insert(
        "TMPDIR".to_string(),
        kernel.home.join("state").display().to_string(),
    );
    let error = crate::provider::managed_isolated_utility_launch(
        "/bin/true",
        Vec::new(),
        environment,
        Some(kernel.workspace.clone()),
        "agent-watched-process",
    )
    .expect_err("the kernel's state folder is no agent's workspace");
    assert!(
        error.to_string().contains("kernel's private folder"),
        "{error}"
    );
}

#[test]
fn a_fresh_provider_child_keeps_the_confinement() {
    let kernel = DummyKernelHome::new();
    let launch = crate::provider::managed_isolated_utility_launch(
        "/bin/echo",
        vec!["first".to_string()],
        kernel.agent_environment(),
        Some(kernel.workspace.clone()),
        "provider",
    )
    .expect("a provider launch is planned");
    let args = crate::provider::managed_isolation::fresh_provider_child_args(
        &launch.pty_args,
        vec!["second".to_string()],
    );
    let prefix = super::confinement_prefix_len(&launch.pty_args).expect("the launch is confined");
    assert_eq!(args[..prefix], launch.pty_args[..prefix]);
    assert_eq!(args[prefix..], ["second".to_string()]);
    assert_eq!(args[prefix - 1], "/bin/echo");
}

#[test]
fn an_endpoint_the_kernel_does_not_start_is_left_alone() {
    let _kernel = DummyKernelHome::new();
    let launch = crate::provider::ProviderLaunchResult {
        endpoint_mode: crate::provider::AgentEndpointMode::External,
        process_label: "external".to_string(),
        pty_target: None,
        pty_program: None,
        pty_args: Vec::new(),
        pty_env: BTreeMap::new(),
        pty_env_remove: Vec::new(),
        working_directory: None,
        structured_endpoint: Some("http://127.0.0.1:1".to_string()),
    };
    assert_eq!(super::confine_launch(launch.clone()).unwrap(), launch);
}
