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
    "vault/vault.json",
    "vault/vault.json.managed-context-key-0.json",
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
             test ! -r '{home}/state/kernel-local-auth/7000.token' && echo local-auth-hidden\n\
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
            "state: provider-accounts",
            "own-login",
            "profile-writable",
            "workspace-writable",
            "scratch-writable",
            "local-auth-hidden",
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
        crate::provider::managed_isolation::FreshChildFolder::Empty,
    );
    let prefix = super::confinement_prefix_len(&launch.pty_args).expect("the launch is confined");
    assert_eq!(args[..prefix], launch.pty_args[..prefix]);
    assert_eq!(args[prefix..], ["second".to_string()]);
    assert_eq!(args[prefix - 1], "/bin/echo");
}

#[test]
fn a_local_provider_can_read_packages_under_a_confined_state_directory() {
    let kernel = DummyKernelHome::new();
    let packages = kernel.home.join("state/skills/packages");
    let reference = packages.join("skill-id/digest/references/phrase.txt");
    let scratch = crate::agent_temp::dir("session", "provider-test").unwrap();
    write(&reference, "bundled reference");
    let launch = crate::provider::ProviderLaunchResult {
        endpoint_mode: crate::provider::AgentEndpointMode::Managed,
        process_label: "provider".to_string(),
        pty_target: None,
        pty_program: Some("/bin/sh".to_string()),
        pty_args: vec![
            "-c".to_string(),
            format!(
                "test \"$TMPDIR\" = '{scratch}' || exit 2\n{}\ncat '{reference}' && echo && \
                 if (echo changed > '{reference}') 2>/dev/null; then exit 1; fi",
                kernel.probe_script(),
                reference = reference.display(),
                scratch = scratch.display(),
            ),
        ],
        pty_env: BTreeMap::from([(
            "CLAUDE_CONFIG_DIR".to_string(),
            kernel.profile.display().to_string(),
        )]),
        pty_env_remove: Vec::new(),
        working_directory: Some(kernel.workspace.clone()),
        structured_endpoint: None,
    };
    let mut request = crate::provider::LaunchProviderRequest::new(
        "session", "claude", "claude", "default", "model",
    );
    request.agent_id = Some("provider-test".to_string());
    let run = |request: &crate::provider::LaunchProviderRequest| {
        let confined = crate::provider::managed_isolation::apply_managed_provider_isolation(
            launch.clone(),
            request,
        )
        .expect("the provider launch is confined");
        crate::provider::command_from_provider_launch(confined)
            .unwrap()
            .output()
            .unwrap()
    };
    let hidden = run(&request);
    assert!(!hidden.status.success(), "{hidden:?}");
    assert!(!String::from_utf8_lossy(&hidden.stdout).contains("bundled reference"));

    let output = run(&request.with_skill_cache_root(packages));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{output:?}");
    assert!(
        stdout.lines().any(|line| line == "bundled reference"),
        "{output:?}"
    );
    kernel.assert_confined(&stdout);
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
    assert_eq!(super::confine_launch(launch.clone(), &[]).unwrap(), launch);
}

fn run_confined(kernel: &DummyKernelHome, script: &str) -> String {
    let launch = crate::provider::managed_isolated_utility_launch(
        "/bin/sh",
        vec!["-c".to_string(), script.to_string()],
        kernel.agent_environment(),
        Some(kernel.workspace.clone()),
        "agent-watched-process",
    )
    .expect("an agent command launch is planned");
    let output = crate::provider::command_from_provider_launch(launch)
        .expect("an agent command is built")
        .output()
        .expect("an agent command runs");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_vault_that_shares_its_folder_is_refused_at_start() {
    crate::test_support::isolated_env_test!();
    let kernel = DummyKernelHome::new();
    let folder = kernel.workspace.parent().unwrap().join("alice");
    write(&folder.join("vault.json"), SECRET);
    write(&folder.join("notes.txt"), "mine");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.path = folder.join("vault.json").display().to_string();
    let error = super::configure(&config).expect_err("a Vault beside other files is refused");
    let error = error.to_string();
    assert!(error.contains("notes.txt"), "{error}");
    assert!(error.contains("credential_vault.path"), "{error}");
    assert!(
        error.contains(
            &folder
                .join("chariox-secret/vault.json")
                .display()
                .to_string()
        ),
        "{error}"
    );
}

#[test]
fn a_vault_replaced_while_an_agent_runs_stays_hidden() {
    crate::test_support::isolated_env_test!();
    let kernel = DummyKernelHome::new();
    let folder = kernel.workspace.parent().unwrap().join("alice/vault");
    let vault = folder.join("vault.json");
    write(&vault, SECRET);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.credential_vault.path = vault.display().to_string();
    super::configure(&config).expect("a Vault in a folder of its own is accepted");
    let started = folder.parent().unwrap().join("started");
    let replaced = folder.parent().unwrap().join("replaced");
    let replacer = {
        let (vault, started, replaced) = (vault.clone(), started.clone(), replaced.clone());
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !started.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let temp = vault.with_file_name(".vault.json.next.tmp");
            std::fs::write(&temp, format!("{SECRET}-replaced")).unwrap();
            std::fs::rename(&temp, &vault).unwrap();
            std::fs::write(&replaced, "").unwrap();
        })
    };
    let output = run_confined(
        &kernel,
        &format!(
            "touch '{started}'; while [ ! -e '{replaced}' ]; do sleep 0.05; done; cat '{vault}'; ls -a '{folder}'",
            started = started.display(),
            replaced = replaced.display(),
            vault = vault.display(),
            folder = folder.display(),
        ),
    );
    replacer.join().unwrap();
    assert!(
        !output.contains(SECRET),
        "the replaced Vault reached the agent:\n{output}"
    );
    assert!(output.contains("No such file or directory"), "{output}");
}

#[test]
fn a_confined_agent_rereads_its_workflow_node_instructions() {
    let kernel = DummyKernelHome::new();
    let config = crate::config::DaemonConfig::for_tests();
    let file = crate::workflow_node_instructions::node_instruction_file(
        &config,
        "room-1",
        "run-1",
        "plan",
        Some("Write the plan."),
    )
    .expect("the instruction file is written");
    assert!(file.starts_with(&kernel.home.display().to_string()));
    let output = run_confined(&kernel, &format!("cat '{file}'"));
    assert_eq!(output, "Write the plan.", "{file}");
}

#[cfg(target_os = "linux")]
#[test]
fn a_managed_agent_rereads_its_workflow_node_instructions() {
    crate::test_support::isolated_env_test!();
    let kernel = DummyKernelHome::new();
    let home = kernel.home.join(".chariox");
    let provider_home = kernel.home.join("provider-home");
    std::fs::create_dir_all(&provider_home).unwrap();
    std::env::set_var("CHARIOX_HOME", &home);
    std::env::set_var(super::super::MANAGED_PROVIDER_ISOLATION_ENV, "1");
    std::env::set_var(
        super::super::managed_isolation::MANAGED_PROVIDER_HOME_ENV,
        &provider_home,
    );
    let private = home.join("state/witness");
    write(&private, SECRET);
    let config = crate::config::DaemonConfig::for_tests();
    let file = crate::workflow_node_instructions::node_instruction_file(
        &config,
        "room-1",
        "run-1",
        "plan",
        Some("Write the plan."),
    )
    .unwrap();
    let mut command = super::super::managed_isolated_utility_command(
        "/bin/sh",
        vec![
            "-ec".into(),
            "test \"$(cat \"$1\")\" = 'Write the plan.'; ! test -r \"$2\"; ! (echo changed > \"$1\")".into(),
            "managed-workflow-instructions".into(),
            file,
            private.display().to_string(),
        ],
        BTreeMap::new(),
        Some(kernel.workspace.clone()),
        "managed workflow instruction probe",
    )
    .unwrap();
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Runs `script` as an agent command; every listed path keeps the agent's
/// line on the host after the process ends.
fn assert_writes_reach_the_disk(kernel: &DummyKernelHome, paths: &[PathBuf]) -> String {
    let script = paths
        .iter()
        .map(|path| format!("echo agent >> '{}'\n", path.display()))
        .collect::<String>();
    let output = run_confined(kernel, &script);
    for path in paths {
        assert!(
            std::fs::read_to_string(path)
                .unwrap_or_default()
                .ends_with("agent\n"),
            "the agent's write to {} is on disk after it ends:\n{output}",
            path.display()
        );
    }
    output
}

#[test]
fn a_write_beside_the_default_state_folder_reaches_the_disk() {
    crate::test_support::isolated_env_test!();
    let kernel = DummyKernelHome::new();
    let user = kernel.workspace.parent().unwrap().join("user");
    std::env::remove_var("CHARIOX_HOME");
    std::env::remove_var("XDG_STATE_HOME");
    std::env::remove_var("XDG_CONFIG_HOME");
    std::env::set_var("HOME", &user);
    let state = user.join(".local/state/chariox");
    write(&state.join("cloud-relay.json"), SECRET);
    let notes = user.join(".local/state/chariox-notes/today.md");
    write(&notes, "mine\n");
    let output = assert_writes_reach_the_disk(&kernel, &[notes]);
    let output = output
        + &run_confined(
            &kernel,
            &format!("cat '{}'", state.join("cloud-relay.json").display()),
        );
    assert!(!output.contains(SECRET), "{output}");
}

/// Over a scratch tree with secrets at the kernel home and at configured places
/// elsewhere: exactly the declared secrets are hidden, and the agent's write to
/// every other path (name-prefix look-alikes, companion look-alikes, paths
/// reached through symlinks) is on disk after it ends.
#[test]
fn exactly_the_declared_secrets_are_hidden() {
    crate::test_support::isolated_env_test!();
    let kernel = DummyKernelHome::new();
    let root = kernel.workspace.parent().unwrap().to_path_buf();
    let home = kernel.home.clone();
    let mut config =
        crate::config::DaemonConfig::for_tests().with_session_history_root(root.join("sessions"));
    config.user_config.credential_vault.path =
        root.join("alice/vault/vault.json").display().to_string();
    config.user_config.history.operational.path =
        Some(root.join("ops/operational.db").display().to_string());
    config.user_config.state.path = Some(root.join("kstate/state.db").display().to_string());
    let secrets = [
        "alice/vault/vault.json",
        "alice/vault/vault.json.managed-context-key-ab12.json",
        "alice/vault/.vault.json.Xy9.tmp",
        "ops/operational.db",
        "ops/operational.db-wal",
        "ops/operational.db-shm",
        "ops/operational.db-journal",
        "kstate/state.db",
        "kstate/cloud-relay.json",
        "kstate/provider-accounts.json",
        "sessions/room-1.jsonl",
    ]
    .map(|secret| root.join(secret));
    for secret in &secrets {
        write(secret, SECRET);
    }
    let public = root.join("public.txt");
    write(&public, "mine\n");
    std::os::unix::fs::symlink(
        &public,
        root.join("alice/vault.json.managed-context-key-ln.json"),
    )
    .unwrap();
    std::os::unix::fs::symlink(home.join("state"), root.join("into-state")).unwrap();
    let outside = root.join("outside.txt");
    write(&outside, "mine\n");
    std::os::unix::fs::symlink(&outside, home.join("state/link-out")).unwrap();
    super::configure(&config).expect("every secret file has a folder of its own");

    let mut others = [
        "alice/vault.json",
        "alice/vault-notes/a.txt",
        "alice/vaultx",
        "alice/notes.txt",
        "ops-notes/a.txt",
        "ops.db",
        "opsx/operational.db",
        "kstate-notes/a.txt",
        "kstate.db",
        "sessions-old/a.jsonl",
        "public.txt",
        "outside.txt",
    ]
    .map(|other| root.join(other))
    .to_vec();
    for folder in [
        "state-notes",
        "statements",
        "vault-notes",
        "history-export",
        "kernels.bak",
        "credentials2",
        "machines",
        "daemons",
        "sessions-old",
    ] {
        others.push(home.join(folder).join("a.txt"));
    }
    others.push(home.join("vault.json"));
    for other in &others {
        if !other.exists() {
            write(other, "mine\n");
        }
    }
    others.push(root.join("alice/new.txt"));
    others.push(root.join("alice/public-link"));

    let output = assert_writes_reach_the_disk(&kernel, &others);
    let reads = secrets
        .iter()
        .chain([&root.join("into-state/cloud-relay.json"), &home.join("state/cloud-relay.json")])
        .map(|secret| {
            format!(
                "if cat '{0}' >/dev/null 2>&1; then echo 'secret-open {0}'; fi\ncat '{0}' 2>/dev/null\n",
                secret.display()
            )
        })
        .collect::<String>();
    let reads = run_confined(
        &kernel,
        &format!(
            "{reads}{}",
            [
                home.join("state"),
                root.join("alice/vault"),
                root.join("ops"),
                root.join("kstate")
            ]
            .iter()
            .map(|folder| format!(
                "echo agent > '{0}/new' || echo 'covered-write-refused {0}'\n",
                folder.display()
            ))
            .collect::<String>()
        ),
    );
    assert!(
        !reads.contains("secret-open") && !reads.contains(SECRET),
        "a declared secret reached the agent:\n{reads}"
    );
    for folder in [
        home.join("state"),
        root.join("alice/vault"),
        root.join("ops"),
        root.join("kstate"),
    ] {
        assert!(
            reads.contains(&format!("covered-write-refused {}", folder.display())),
            "a write into a covered folder fails:\n{reads}"
        );
        assert!(!folder.join("new").exists());
    }
    assert!(!output.contains(SECRET), "{output}");
    for secret in &secrets {
        assert_eq!(std::fs::read_to_string(secret).unwrap(), SECRET);
    }
}

/// The repo drills keep the durable state directly in the kernel home, which
/// makes the whole home a secret folder; the scratch folder the kernel gives
/// the agent there is still the agent's own. Claude creates its temp files
/// under TMPDIR first thing, so a hidden scratch folder ends every turn.
#[test]
fn a_provider_run_writes_its_scratch_when_the_kernel_home_holds_the_state() {
    crate::test_support::isolated_env_test!();
    let kernel = DummyKernelHome::new();
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.state.path = Some(kernel.home.join("state.db").display().to_string());
    config.user_config.credential_vault.path = kernel.home.join("vault.json").display().to_string();
    super::configure(&config).expect("a state folder of the kernel home is accepted");
    let executable = kernel.workspace.parent().unwrap().join("bin/opencode");
    write(
        &executable,
        "#!/bin/sh\nmkdir -p \"$TMPDIR/provider\" && echo provider > \"$TMPDIR/provider/file\" && echo \"scratch-writable $TMPDIR\"\n",
    );
    std::fs::set_permissions(
        &executable,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    std::env::set_var("CHARIOX_OPENCODE_BIN", &executable);
    let request = crate::provider::LaunchProviderRequest::new(
        "room1",
        "opencode",
        "opencode",
        "default",
        "anthropic/claude-sonnet-4",
    )
    .with_agent_id("agent-scratch1")
    .with_working_directory(kernel.workspace.clone());
    let run = crate::provider::ProviderProcessService::new()
        .start_run_provider_only(request)
        .expect("the provider run is planned")
        .run()
        .clone();
    let scratch = run
        .pty_env()
        .get("TMPDIR")
        .expect("the run has its scratch folder")
        .clone();
    assert_eq!(
        Path::new(&scratch),
        crate::agent_temp::dir("room1", "agent-scratch1").unwrap()
    );
    let output = std::process::Command::new(run.pty_program().expect("the run starts a process"))
        .args(run.pty_args())
        .envs(run.pty_env())
        .output()
        .expect("the provider process runs");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.contains(&format!("scratch-writable {scratch}")),
        "{output}"
    );
    assert_eq!(
        std::fs::read_to_string(Path::new(&scratch).join("provider/file")).unwrap_or_default(),
        "provider\n",
        "the provider's scratch write is on disk:\n{output}"
    );
}

/// Whether `pid` still runs: a zombie waiting for its reaper has ended.
fn running(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(") ")
            .is_some_and(|(_, rest)| !rest.starts_with('Z'))
    })
}

#[test]
fn killing_the_sandbox_ends_the_program_it_confines() {
    use std::io::BufRead as _;
    let kernel = DummyKernelHome::new();
    let launch = crate::provider::managed_isolated_utility_launch(
        "/bin/sh",
        vec!["-c".to_string(), "echo $$; exec sleep 300".to_string()],
        kernel.agent_environment(),
        Some(kernel.workspace.clone()),
        "agent-watched-process",
    )
    .expect("an agent command launch is planned");
    let mut leader = crate::provider::command_from_provider_launch(launch)
        .expect("an agent command is built")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("an agent command runs");
    let mut line = String::new();
    std::io::BufReader::new(leader.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let program: u32 = line.trim().parse().expect("the program reports its PID");
    assert_ne!(
        program,
        leader.id(),
        "the sandbox leads the confined program"
    );
    assert!(running(program));

    leader.kill().unwrap();
    leader.wait().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while running(program) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let survived = running(program);
    if survived {
        // SAFETY: the PID is the confined program this test started.
        unsafe { libc::kill(program as i32, libc::SIGKILL) };
    }
    assert!(!survived, "the confined program outlived its sandbox");
}
