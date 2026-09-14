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
    environment
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
    let deadline = Instant::now() + Duration::from_millis(VALIDATION_COMMAND_TIMEOUT_MS);
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
