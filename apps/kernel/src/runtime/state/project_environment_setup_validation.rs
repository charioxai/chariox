use super::*;

pub(super) const VALIDATION_COMMAND_TIMEOUT_MS: u64 = 120_000;
pub(super) const VALIDATION_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);

const WORKER_KERNEL_ENV_NAMES: &[&str] = &[
    "CHARIOX_HOME",
    "CHARIOX_MANAGED_VAULT_PATH",
    "CHARIOX_CAPABILITY_ISOLATION_ROOT",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
    "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
    "CHARIOX_MANAGED_BOOTSTRAP_PATH",
    "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
    "CHARIOX_KERNEL_HOST",
    "CHARIOX_KERNEL_PORT",
    "CHARIOX_DAEMON_ID",
    "CHARIOX_MACHINE_ID",
    "CHARIOX_MANAGED_KERNEL_BINARY",
    "CHARIOX_MANAGED_RELEASE_MANIFEST",
    "CHARIOX_MANAGED_RELEASE_SIGNATURE",
    "CHARIOX_MANAGED_RELEASE_PUBLIC_KEY",
    "CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH",
    "CHARIOX_ACCEPT_REMOTE_LEASES",
    "CHARIOX_KERNEL_RUNTIME_ROLE",
    "CHARIOX_REMOTE_LEASE_CAPACITY",
    "CHARIOX_LEASE_WORKER_HOME_CALLER",
];

// These ambient Git/SSH hooks can redirect credential lookup or host
// verification to an unselected helper. They are execution-environment
// controls, not project command syntax. The project command remains opaque and
// may still use its own tools and configuration; only automatically supplied
// credential controls are removed at this boundary.
const WORKER_UNSELECTED_CREDENTIAL_CONTROL_ENV_NAMES: &[&str] = &[
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "SSH_AGENT_PID",
];

// These paths are supplied by Chariox/provider account setup. They are not
// project tooling inputs, so opaque project commands must not inherit them.
// The command boundary below supplies a fresh HOME instead of trying to
// interpret shell syntax or maintain a repository/tool allowlist.
const WORKER_AUTOMATIC_CREDENTIAL_ENV_NAMES: &[&str] = &[
    "HOME",
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "OPENCODE_CONFIG_DIR",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_NOSYSTEM",
    "SSH_AUTH_SOCK",
    "SSH_CONFIG",
    "SSH_CONFIG_FILE",
    "SSH_KNOWN_HOSTS",
];

/// Durable per-project/per-worker HOME for setup and validation commands.
///
/// This is intentionally outside the worktree so setup does not add generated
/// user-tool state to the repository, but it is stable for the lifetime of the
/// worker and is reused by every apply/validate call for this project. It must
/// not be deleted at the end of one command: user-scoped rustup/cargo,
/// Python, npm, and other ordinary project tools may place their installed
/// state below HOME.
pub(super) struct WorkerPreparationHome {
    path: PathBuf,
}

impl WorkerPreparationHome {
    pub(super) fn for_project_worker(
        workspace_root: &Path,
        project_id: &str,
        worker_id: &str,
    ) -> Result<Self, DaemonError> {
        let state_root = workspace_root
            .parent()
            .ok_or_else(|| setup_error("worker worktree has no durable state parent"))?
            .join(".chariox-project-environment");
        let mut digest = Sha256::new();
        digest.update(workspace_root.as_os_str().to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(project_id.as_bytes());
        digest.update([0]);
        digest.update(worker_id.as_bytes());
        let path = state_root.join(format!("{:x}", digest.finalize()));

        std::fs::create_dir_all(&state_root).map_err(|error| {
            setup_error(&format!(
                "durable worker preparation state could not be created: {error}"
            ))
        })?;
        std::fs::create_dir_all(&path).map_err(|error| {
            setup_error(&format!(
                "durable worker preparation HOME could not be created: {error}"
            ))
        })?;
        for directory in [&state_root, &path] {
            let metadata = std::fs::symlink_metadata(directory).map_err(|error| {
                setup_error(&format!(
                    "durable worker preparation state could not be inspected: {error}"
                ))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(setup_error(
                    "durable worker preparation state must be real directories",
                ));
            }
        }
        #[cfg(unix)]
        for directory in [&state_root, &path] {
            std::fs::set_permissions(
                directory,
                std::os::unix::fs::PermissionsExt::from_mode(0o700),
            )
            .map_err(|error| {
                setup_error(&format!(
                    "durable worker preparation HOME permissions could not be secured: {error}"
                ))
            })?;
        }
        Ok(Self { path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

pub(super) fn ensure_worker_validation_boundary(config: &DaemonConfig) -> Result<(), DaemonError> {
    if config.kernel_runtime_role != KernelRuntimeRole::RemoteLeaseWorker {
        return Err(setup_error(
            "project environment setup is only executable by a dedicated worker kernel",
        ));
    }
    let receipt_path = std::env::var_os(crate::managed_bootstrap::worker::ACTIVITY_RECEIPT_ENV)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| setup_error("disposable worker receipt is missing"))?;
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| setup_error("disposable worker Cloud binding is missing"))?;
    crate::managed_bootstrap::worker::confirmed_activity_allocation(
        Path::new(&receipt_path),
        config,
        profile,
    )
    .map(|_| ())
    .map_err(|error| setup_error(&format!("worker boundary is not confirmed: {error}")))
}

pub(super) fn canonical_worker_workspace(
    path: impl AsRef<Path>,
    kernel_home: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, DaemonError> {
    let path = path.as_ref();
    let canonical = path
        .canonicalize()
        .map_err(|error| setup_error(&format!("worker worktree is unavailable: {error}")))?;
    if !canonical.is_dir() || canonical == Path::new("/") {
        return Err(setup_error("worker worktree must be a non-root directory"));
    }
    let kernel_home = kernel_home
        .map(PathBuf::from)
        .ok_or_else(|| setup_error("worker kernel home is not configured"))?
        .canonicalize()
        .map_err(|error| setup_error(&format!("worker kernel home is unavailable: {error}")))?;
    if canonical == kernel_home || canonical.starts_with(&kernel_home) {
        return Err(setup_error(
            "worker worktree overlaps kernel-owned home state",
        ));
    }
    Ok(canonical)
}

pub(super) fn worker_validation_environment(
    provider_run: &RuntimeProviderRun,
) -> BTreeMap<String, String> {
    worker_validation_environment_with_home(provider_run, None)
}

pub(super) fn worker_validation_environment_with_home(
    provider_run: &RuntimeProviderRun,
    preparation_home: Option<&Path>,
) -> BTreeMap<String, String> {
    // This is the credential boundary for both recipe application and
    // validation. Keep ordinary toolchain/project environment values, remove
    // Chariox-provided credential/account paths, and use a durable preparation
    // HOME for this project/worker. The command remains opaque: project tools
    // and scripts are not parsed, allowlisted, or blocked by shell substrings.
    // This boundary protects only credentials automatically supplied by
    // Chariox; it does not claim to sandbox credentials a project provides
    // itself.
    let mut removed = BTreeSet::new();
    removed.extend(
        crate::provider::managed_provider_control_env_remove()
            .into_iter()
            .collect::<BTreeSet<_>>(),
    );
    removed.extend(provider_run.pty_env_remove().iter().cloned());

    let mut environment = BTreeMap::new();
    for (name, value) in std::env::vars() {
        if worker_validation_environment_allowed(&name, &removed) {
            environment.insert(name, value);
        }
    }
    for (name, value) in provider_run.pty_env() {
        if worker_validation_environment_allowed(name, &removed) {
            environment.insert(name.clone(), value.clone());
        }
    }
    if let Some(preparation_home) = preparation_home {
        environment.insert("HOME".to_string(), preparation_home.display().to_string());
        let current_path = environment
            .get("PATH")
            .map(String::as_str)
            .unwrap_or_default();
        environment.insert(
            "PATH".to_string(),
            worker_preparation_path(preparation_home, current_path),
        );
    }
    environment
}

fn worker_preparation_path(preparation_home: &Path, current_path: &str) -> String {
    let local_bin = preparation_home.join(".local").join("bin");
    let cargo_bin = preparation_home.join(".cargo").join("bin");
    let mut entries = Vec::new();
    for entry in [local_bin, cargo_bin]
        .into_iter()
        .chain(std::env::split_paths(std::ffi::OsStr::new(current_path)))
    {
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    match std::env::join_paths(entries) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => {
            let separator = if cfg!(windows) { ';' } else { ':' };
            format!(
                "{}{}{}{}{}",
                preparation_home.join(".local").join("bin").display(),
                separator,
                preparation_home.join(".cargo").join("bin").display(),
                separator,
                current_path
            )
        }
    }
}

const MAX_PROJECT_ENVIRONMENT_INPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PROJECT_ENVIRONMENT_INPUT_BYTES_TOTAL: u64 = 128 * 1024 * 1024;

/// Resolve provider-requested input attestations against the worker's
/// already-materialized project worktree. A provider may use the kernel marker
/// when its read-only tool policy cannot run a hashing command; only this
/// function can turn that marker into a persisted exact digest.
pub(super) fn resolve_project_environment_input_attestations(
    workspace_root: &Path,
    definition: &ProjectEnvironmentDefinition,
) -> Result<ProjectEnvironmentDefinition, String> {
    definition.validate_for_utility_output()?;
    let mut resolved = definition.clone();
    let mut total_bytes_read = 0_u64;
    for input in &mut resolved.inputs {
        let candidate = workspace_root.join(&input.path);
        let metadata = std::fs::symlink_metadata(&candidate).map_err(|error| {
            format!(
                "worker project input is unavailable: {} ({error})",
                input.path
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(format!(
                "worker project input is not a regular file: {}",
                input.path
            ));
        }
        let canonical = candidate.canonicalize().map_err(|error| {
            format!(
                "worker project input could not be canonicalized: {} ({error})",
                input.path
            )
        })?;
        if !canonical.starts_with(workspace_root) {
            return Err(format!(
                "worker project input escapes the materialized worktree: {}",
                input.path
            ));
        }
        let mut file = std::fs::File::open(&canonical).map_err(|error| {
            format!(
                "worker project input could not be read: {} ({error})",
                input.path
            )
        })?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 8192];
        let mut bytes_read = 0_u64;
        loop {
            let read = file.read(&mut buffer).map_err(|error| {
                format!(
                    "worker project input could not be read: {} ({error})",
                    input.path
                )
            })?;
            if read == 0 {
                break;
            }
            bytes_read = bytes_read.saturating_add(read as u64);
            total_bytes_read = total_bytes_read.saturating_add(read as u64);
            if bytes_read > MAX_PROJECT_ENVIRONMENT_INPUT_BYTES {
                return Err(format!(
                    "worker project input exceeds the bounded attestation size: {}",
                    input.path
                ));
            }
            if total_bytes_read > MAX_PROJECT_ENVIRONMENT_INPUT_BYTES_TOTAL {
                return Err(
                    "worker project inputs exceed the bounded total attestation size".to_string(),
                );
            }
            digest.update(&buffer[..read]);
        }
        let actual = format!("sha256:{:x}", digest.finalize());
        if input.sha256 == crate::session::KERNEL_COMPUTED_INPUT_ATTESTATION {
            input.sha256 = actual;
        } else if actual != input.sha256 {
            return Err(format!(
                "worker project input does not match its attested content: {}",
                input.path
            ));
        }
    }
    resolved.validate().map_err(|message| {
        format!("worker project input attestation could not be canonicalized: {message}")
    })?;
    Ok(resolved)
}

/// Verify the content-only inputs in a definition against the worker's
/// already-materialized project worktree. This deliberately reads files but
/// never executes them or returns their contents across the setup seam.
pub(super) fn verify_project_environment_inputs(
    workspace_root: &Path,
    definition: &ProjectEnvironmentDefinition,
) -> Result<(), String> {
    let resolved = resolve_project_environment_input_attestations(workspace_root, definition)?;
    if resolved != *definition {
        return Err(
            "worker project input attestation must be kernel-resolved before verification"
                .to_string(),
        );
    }
    Ok(())
}

fn worker_validation_environment_allowed(name: &str, removed: &BTreeSet<String>) -> bool {
    !removed.contains(name)
        && !crate::secret::secret_like_env_name(name)
        && !WORKER_KERNEL_ENV_NAMES.contains(&name)
        && !WORKER_AUTOMATIC_CREDENTIAL_ENV_NAMES.contains(&name)
        && !WORKER_UNSELECTED_CREDENTIAL_CONTROL_ENV_NAMES.contains(&name)
        && !name.starts_with("GIT_CONFIG_KEY_")
        && !name.starts_with("GIT_CONFIG_VALUE_")
        && !name.starts_with("CHARIOX_")
}

pub(super) fn run_worker_validation_command(
    command_text: &str,
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    should_cancel: impl Fn() -> bool,
    overall_deadline: Option<Instant>,
) -> Result<(i32, usize, usize), String> {
    // This child is spawned only by a confirmed disposable worker kernel after
    // the provider run context and workspace have been fenced above. Keep the
    // shell local to that worker boundary and do not route through the home
    // kernel's general ShellCommandService.
    let (shell, shell_flag) = if cfg!(windows) {
        ("C:\\Windows\\System32\\cmd.exe", "/C")
    } else {
        // Preserve the provider PTY's prepared PATH and other environment
        // exactly; a login shell could source kernel-home startup files.
        ("/bin/sh", "-c")
    };
    let mut command = Command::new(shell);
    command
        .arg(shell_flag)
        .arg(command_text)
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(environment);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // Keep descendants in a worker-local process group so cancellation or
        // timeout cannot leave a compiler holding the validation pipes open.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let command_deadline = Instant::now() + Duration::from_millis(VALIDATION_COMMAND_TIMEOUT_MS);
    let deadline =
        overall_deadline.map_or(command_deadline, |deadline| deadline.min(command_deadline));
    if deadline.saturating_duration_since(Instant::now()).is_zero() {
        return Err("worker validation command exceeded the overall setup deadline".to_string());
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("worker validation command did not provide stdout capture".to_string());
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("worker validation command did not provide stderr capture".to_string());
    };
    let stdout_reader = std::thread::spawn(move || count_validation_output(stdout));
    let stderr_reader = std::thread::spawn(move || count_validation_output(stderr));
    let outcome = loop {
        if should_cancel() {
            break Err("worker validation command cancelled".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break Err("worker validation command timed out".to_string());
        }
        match child.wait_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => continue,
            Err(error) => break Err(error.to_string()),
        }
    };
    let status = match outcome {
        Ok(status) => status,
        Err(error) => {
            terminate_validation_process_group(&mut child);
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(error);
        }
    };
    terminate_validation_process_group(&mut child);
    let stdout_bytes = stdout_reader
        .join()
        .map_err(|_| "worker validation stdout reader panicked".to_string())?;
    let stdout_bytes = stdout_bytes?;
    let stderr_bytes = stderr_reader
        .join()
        .map_err(|_| "worker validation stderr reader panicked".to_string())?;
    let stderr_bytes = stderr_bytes?;
    Ok((status.code().unwrap_or(-1), stdout_bytes, stderr_bytes))
}

pub(super) fn run_worker_setup_steps(
    commands: &[String],
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    should_cancel: impl Fn() -> bool,
) -> Result<bool, String> {
    run_worker_setup_steps_until(
        commands,
        workspace_root,
        environment,
        Instant::now() + VALIDATION_TOTAL_TIMEOUT,
        should_cancel,
    )
}

fn run_worker_setup_steps_until(
    commands: &[String],
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    overall_deadline: Instant,
    should_cancel: impl Fn() -> bool,
) -> Result<bool, String> {
    for command in commands {
        if should_cancel() {
            return Err("worker setup command cancelled".to_string());
        }
        if overall_deadline
            .saturating_duration_since(Instant::now())
            .is_zero()
        {
            return Err("worker setup commands timed out".to_string());
        }
        let (exit_code, _stdout_bytes, _stderr_bytes) = run_worker_validation_command(
            command,
            workspace_root,
            environment,
            || should_cancel(),
            Some(overall_deadline),
        )?;
        if exit_code != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
pub(super) fn run_worker_setup_steps_with_total_timeout(
    commands: &[String],
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    total_timeout: Duration,
    should_cancel: impl Fn() -> bool,
) -> Result<bool, String> {
    run_worker_setup_steps_until(
        commands,
        workspace_root,
        environment,
        Instant::now() + total_timeout,
        should_cancel,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        ProjectEnvironmentDefinitionOrigin, ProjectEnvironmentDefinitionSource,
        ProjectEnvironmentInput, ProjectEnvironmentInputKind, ProjectEnvironmentSetupStep,
        ProjectEnvironmentSetupStepKind,
    };

    #[test]
    fn setup_steps_enforce_total_deadline_during_an_in_flight_command() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-total-deadline-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("deadline fixture workspace should exist");
        let started = Instant::now();
        let result = run_worker_setup_steps_with_total_timeout(
            &["sleep 0.03".to_string(), "sleep 0.20".to_string()],
            &root,
            &BTreeMap::new(),
            Duration::from_millis(75),
            || false,
        );
        assert!(
            result.is_err(),
            "a command that outlives the total setup budget must fail: {result:?}"
        );
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "an in-flight command must be interrupted by the total budget: {:?}",
            started.elapsed()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn durable_preparation_home_preserves_install_like_state_across_apply_and_validation() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-durable-home-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).expect("durable-home workspace should exist");
        let request = crate::provider::LaunchProviderRequest::new(
            "session-1",
            "codex",
            "codex",
            "default",
            "default",
        );
        let run = crate::provider::RuntimeProviderRun::new(
            "provider-run-durable-home",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "worker-provider".to_string(),
                pty_target: None,
                pty_program: Some("/bin/sh".to_string()),
                pty_args: Vec::new(),
                pty_env: BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
                pty_env_remove: Vec::new(),
                working_directory: Some(workspace.clone()),
                structured_endpoint: None,
            },
        );

        let preparation_home =
            WorkerPreparationHome::for_project_worker(&workspace, "project-1", "worker-1")
                .expect("durable preparation HOME should be created");
        let apply_environment =
            worker_validation_environment_with_home(&run, Some(preparation_home.path()));
        let installed = run_worker_setup_steps(
            &[
                "set -eu; mkdir -p \"$HOME/.local/bin\"".to_string(),
                "printf '%s\\n' project-tool > \"$HOME/.local/bin/project-tool\"; chmod 700 \"$HOME/.local/bin/project-tool\"".to_string(),
            ],
            &workspace,
            &apply_environment,
            || false,
        )
        .expect("install-like setup should execute through the worker boundary");
        assert!(installed);

        let validation_home =
            WorkerPreparationHome::for_project_worker(&workspace, "project-1", "worker-1")
                .expect("validation should reuse durable preparation HOME");
        assert_eq!(validation_home.path(), preparation_home.path());
        let validation_environment =
            worker_validation_environment_with_home(&run, Some(validation_home.path()));
        let validation = run_worker_validation_command(
            "test -x \"$HOME/.local/bin/project-tool\" && test \"$(cat \"$HOME/.local/bin/project-tool\")\" = project-tool",
            &workspace,
            &validation_environment,
            || false,
            None,
        )
        .expect("validation should see the install-like HOME artifact");
        assert_eq!(validation.0, 0);

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn prepared_home_and_path_reach_a_real_post_ready_provider_command() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-provider-home-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).expect("provider environment workspace should exist");
        let trace = root.join("provider-command.log");
        let child_script = r#"
set -eu
while IFS= read -r request; do
  tool_path="$(command -v "$PROJECT_TEST_TOOL")"
  printf 'tool=%s\n' "$tool_path" >> "$PROJECT_TEST_TRACE"
  "$tool_path" >> "$PROJECT_TEST_TRACE"
  printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"post-ready provider command succeeded"}]}}'
  printf '%s\n' '{"type":"result","subtype":"success","is_error":false}'
done
"#;
        let request = crate::provider::LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "sonnet",
        )
        .with_execution_mode(crate::provider::AgentExecutionMode::Build)
        .with_permission_level(crate::provider::AgentPermissionLevel::Yolo)
        .with_working_directory(workspace.clone());
        let run = crate::provider::RuntimeProviderRun::new(
            "provider-run-post-ready-provider",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "test-claude".to_string(),
                pty_target: None,
                pty_program: Some("/bin/sh".to_string()),
                pty_args: vec!["-c".to_string(), child_script.to_string()],
                pty_env: BTreeMap::from([
                    (
                        "HOME".to_string(),
                        root.join("old-home").display().to_string(),
                    ),
                    ("PATH".to_string(), "/usr/bin:/bin".to_string()),
                    (
                        "PROJECT_TEST_TRACE".to_string(),
                        trace.display().to_string(),
                    ),
                    ("PROJECT_TEST_TOOL".to_string(), "project-tool".to_string()),
                ]),
                pty_env_remove: Vec::new(),
                working_directory: Some(workspace.clone()),
                structured_endpoint: None,
            },
        );

        let preparation_home =
            WorkerPreparationHome::for_project_worker(&workspace, "project-1", "worker-1")
                .expect("durable provider preparation HOME should be created");
        let prepared_environment =
            worker_validation_environment_with_home(&run, Some(preparation_home.path()));
        let prepared_path = prepared_environment
            .get("PATH")
            .expect("prepared worker environment should have PATH");
        assert!(prepared_path.starts_with(
            &preparation_home
                .path()
                .join(".local")
                .join("bin")
                .display()
                .to_string()
        ));
        run_worker_setup_steps(
            &[r#"set -eu; mkdir -p "$HOME/.local/bin"; printf '%s\n' '#!/bin/sh' 'printf "%s\n" installed-from-post-ready-provider' > "$HOME/.local/bin/project-tool"; chmod 700 "$HOME/.local/bin/project-tool""#.to_string()],
            &workspace,
            &prepared_environment,
            || false,
        )
        .expect("worker setup should install the provider command");

        let mut prepared_run = run.clone();
        prepared_run
            .set_preparation_environment(
                prepared_environment
                    .get("HOME")
                    .expect("prepared worker environment should have HOME")
                    .clone(),
                prepared_path.clone(),
            )
            .expect("prepared provider environment should bind");
        let mut binding = crate::provider::initialize_claude_runtime(&run)
            .expect("real provider child should start with the original context");
        let envelope = crate::prompt_assembly::PromptEnvelope::new(
            "invoke the installed project tool",
            "",
            Vec::new(),
            crate::prompt_assembly::PromptManifest::default(),
        );
        crate::provider::submit_claude_prompt(&prepared_run, &mut binding.state, &envelope)
            .expect("post-ready provider command should use the prepared context");

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut completed = false;
        while Instant::now() < deadline {
            let batch = crate::provider::drain_claude_events(&prepared_run, &mut binding.state)
                .expect("provider child output should be readable");
            if batch.prompt_completed {
                completed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(completed, "real provider child should complete its command");
        let trace_contents = std::fs::read_to_string(&trace)
            .expect("real provider child should record its command result");
        assert!(trace_contents.contains(
            &preparation_home
                .path()
                .join(".local")
                .join("bin")
                .join("project-tool")
                .display()
                .to_string()
        ));
        assert!(trace_contents.contains("installed-from-post-ready-provider"));

        drop(binding);
        drop(preparation_home);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn script_backed_setup_keeps_project_evidence_and_private_key_fixtures_opaque() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-script-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let scripts = root.join("scripts");
        let testdata = root.join("testdata");
        std::fs::create_dir_all(&scripts).expect("script directory should exist");
        std::fs::create_dir_all(&testdata).expect("application fixture directory should exist");
        let fixture_contents = b"application test fixture, not an SSH credential\n";
        std::fs::write(testdata.join("private_key.pem"), fixture_contents)
            .expect("application private_key fixture should exist");

        // The worker executes the attested script as one opaque project
        // command. Its source may contain ordinary project evidence words and
        // tools such as ssh-keyscan or dd without the definition layer trying
        // to interpret shell syntax or reject unrelated application fixtures.
        let script = scripts.join("setup.sh");
        let script_contents = b"#!/bin/sh\nset -eu\ntest -f testdata/private_key.pem\nprintf '%s\\n' 'ssh-keyscan appears here as project evidence' | dd of=setup-evidence.txt 2>/dev/null\n";
        std::fs::write(&script, script_contents).expect("project setup script should exist");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("project setup script should be executable");

        let definition = ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
            source: ProjectEnvironmentDefinitionSource::SetupScript,
            target_platform: "linux-x86_64".to_string(),
            source_path: Some("scripts/setup.sh".to_string()),
            inputs: vec![
                ProjectEnvironmentInput {
                    kind: ProjectEnvironmentInputKind::Recipe,
                    path: "scripts/setup.sh".to_string(),
                    sha256: format!("sha256:{:x}", Sha256::digest(script_contents)),
                },
                ProjectEnvironmentInput {
                    kind: ProjectEnvironmentInputKind::Recipe,
                    path: "testdata/private_key.pem".to_string(),
                    sha256: format!("sha256:{:x}", Sha256::digest(fixture_contents)),
                },
            ],
            setup_steps: vec![ProjectEnvironmentSetupStep {
                kind: ProjectEnvironmentSetupStepKind::Command,
                command: "./scripts/setup.sh".to_string(),
            }],
            validation_commands: vec!["test -f testdata/private_key.pem".to_string()],
        };
        assert_eq!(definition.validate(), Ok(()));
        assert_eq!(
            verify_project_environment_inputs(&root, &definition),
            Ok(())
        );

        let environment = BTreeMap::from([
            (
                String::from("HOME"),
                root.join("home").display().to_string(),
            ),
            (String::from("PATH"), String::from("/usr/bin:/bin")),
        ]);
        let applied = run_worker_setup_steps(
            &["./scripts/setup.sh".to_string()],
            &root,
            &environment,
            || false,
        )
        .expect("script-backed setup should execute at the worker boundary");
        assert!(applied);
        let (exit_code, _, _) = run_worker_validation_command(
            "test -f testdata/private_key.pem",
            &root,
            &environment,
            || false,
            None,
        )
        .expect("fixture validation should execute at the worker boundary");
        assert_eq!(exit_code, 0);
        assert_eq!(
            std::fs::read_to_string(root.join("setup-evidence.txt"))
                .expect("script evidence should be written in the project worktree")
                .trim(),
            "ssh-keyscan appears here as project evidence"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn worker_input_attestation_is_content_bound_and_fails_closed() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-input-attestation-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("input fixture workspace should exist");
        let path = root.join("package.json");
        let contents = br#"{"name":"fixture"}
"#;
        std::fs::write(&path, contents).expect("input fixture should be written");
        let definition = ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
            source: ProjectEnvironmentDefinitionSource::Devcontainer,
            target_platform: "linux-x86_64".to_string(),
            source_path: Some("package.json".to_string()),
            inputs: vec![ProjectEnvironmentInput {
                kind: ProjectEnvironmentInputKind::Recipe,
                path: "package.json".to_string(),
                sha256: format!("sha256:{:x}", Sha256::digest(contents)),
            }],
            setup_steps: Vec::new(),
            validation_commands: vec!["true".to_string()],
        };
        assert_eq!(
            verify_project_environment_inputs(&root, &definition),
            Ok(())
        );
        std::fs::write(
            &path,
            br#"{"name":"changed"}
"#,
        )
        .expect("input fixture mutation should be written");
        assert!(verify_project_environment_inputs(&root, &definition)
            .unwrap_err()
            .contains("does not match"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn worker_resolves_kernel_computed_input_attestation_before_persisting() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-kernel-attestation-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("input fixture workspace should exist");
        let path = root.join(".devcontainer/devcontainer.json");
        let contents = br#"{"name":"kernel-attested"}
"#;
        std::fs::create_dir_all(path.parent().expect("fixture should have a parent"))
            .expect("recipe directory should exist");
        std::fs::write(&path, contents).expect("input fixture should be written");
        let definition = ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: ProjectEnvironmentDefinitionOrigin::UtilityGenerated,
            source: ProjectEnvironmentDefinitionSource::Devcontainer,
            target_platform: "linux-x86_64".to_string(),
            source_path: Some(".devcontainer/devcontainer.json".to_string()),
            inputs: vec![ProjectEnvironmentInput {
                kind: ProjectEnvironmentInputKind::Recipe,
                path: ".devcontainer/devcontainer.json".to_string(),
                sha256: crate::session::KERNEL_COMPUTED_INPUT_ATTESTATION.to_string(),
            }],
            setup_steps: Vec::new(),
            validation_commands: vec!["true".to_string()],
        };

        let resolved = resolve_project_environment_input_attestations(&root, &definition)
            .expect("the worker kernel should compute the exact input digest");
        assert_eq!(
            resolved.inputs[0].sha256,
            format!("sha256:{:x}", Sha256::digest(contents))
        );
        assert_eq!(verify_project_environment_inputs(&root, &resolved), Ok(()));

        std::fs::write(
            &path,
            br#"{"name":"changed"}
"#,
        )
        .expect("input fixture mutation should be written");
        assert!(verify_project_environment_inputs(&root, &resolved)
            .unwrap_err()
            .contains("does not match"));
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(unix)]
fn terminate_validation_process_group(child: &mut std::process::Child) {
    let process_group = child.id() as libc::pid_t;
    let _ = unsafe { libc::kill(-process_group, libc::SIGKILL) };
}

#[cfg(not(unix))]
fn terminate_validation_process_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

fn count_validation_output(mut output: impl Read) -> Result<usize, String> {
    let mut buffer = [0_u8; 8192];
    let mut bytes = 0_usize;
    loop {
        let read = output
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes = bytes.saturating_add(read);
    }
}
