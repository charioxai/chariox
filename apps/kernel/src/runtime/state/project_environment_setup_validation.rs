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

pub(super) fn ensure_worker_validation_boundary(config: &DaemonConfig) -> Result<(), DaemonError> {
    // General kernels are the ordinary local-worker placement. They still
    // pass through the canonical workspace and sanitized-environment checks
    // below, but they do not have (and must not require) a disposable-worker
    // receipt or a Cloud relay binding. Only the leased placement needs the
    // authenticated activity allocation.
    if config.kernel_runtime_role == KernelRuntimeRole::General {
        return Ok(());
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
    let mut removed = BTreeSet::new();
    removed.extend(
        crate::provider::managed_provider_isolation_env_remove()
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
    environment
}

const MAX_PROJECT_ENVIRONMENT_INPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PROJECT_ENVIRONMENT_INPUT_BYTES_TOTAL: u64 = 128 * 1024 * 1024;

/// Verify the content-only inputs in a definition against the worker's
/// already-materialized project worktree. This deliberately reads files but
/// never executes them or returns their contents across the setup seam.
pub(super) fn verify_project_environment_inputs(
    workspace_root: &Path,
    definition: &ProjectEnvironmentDefinition,
) -> Result<(), String> {
    let mut total_bytes_read = 0_u64;
    for input in &definition.inputs {
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
        if actual != input.sha256 {
            return Err(format!(
                "worker project input does not match its attested content: {}",
                input.path
            ));
        }
    }
    Ok(())
}

fn worker_validation_environment_allowed(name: &str, removed: &BTreeSet<String>) -> bool {
    !removed.contains(name)
        && !crate::secret::secret_like_env_name(name)
        && !WORKER_KERNEL_ENV_NAMES.contains(&name)
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
        ProjectEnvironmentInput, ProjectEnvironmentInputKind,
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
