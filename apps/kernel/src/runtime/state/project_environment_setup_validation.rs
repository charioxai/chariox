use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use super::project_environment_setup_storage::{
    cleanup_after_settled_group, kill_live_process_group, open_pidfd, wait_child_or_cancel,
    wait_for_process_group_absence, ValidationProcessIdentity,
};

pub(super) const VALIDATION_COMMAND_TIMEOUT_MS: u64 = 120_000;
pub(super) const VALIDATION_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);
const VALIDATION_OUTPUT_SETTLE_TIMEOUT: Duration = Duration::from_secs(1);
const VALIDATION_OUTPUT_UNSETTLED_PREFIX: &str =
    "validation cleanup incomplete: output pipes remained open after process settlement";
const VALIDATION_OUTPUT_POLL_INTERVAL: Duration = Duration::from_millis(5);

struct ValidationOutputReader {
    task: std::thread::JoinHandle<Result<usize, String>>,
    stop: Arc<AtomicBool>,
}

#[cfg(unix)]
fn spawn_validation_output_reader<R>(output: R) -> Result<ValidationOutputReader, String>
where
    R: Read + Send + std::os::fd::AsRawFd + 'static,
{
    let fd = output.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(format!(
            "validation output pipe could not be made nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let reader_stop = Arc::clone(&stop);
    let task = std::thread::spawn(move || count_validation_output_nonblocking(output, reader_stop));
    Ok(ValidationOutputReader { task, stop })
}

#[cfg(not(unix))]
fn spawn_validation_output_reader<R>(mut output: R) -> Result<ValidationOutputReader, String>
where
    R: Read + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let task = std::thread::spawn(move || count_validation_output_blocking(&mut output));
    Ok(ValidationOutputReader { task, stop })
}

fn join_validation_output_reader(reader: ValidationOutputReader) -> Result<usize, String> {
    reader
        .task
        .join()
        .map_err(|_| "worker validation output reader panicked".to_string())?
}

type SettledValidationOutput = Option<(Option<usize>, Option<usize>)>;

fn settle_validation_output_readers(
    mut stdout: Option<ValidationOutputReader>,
    mut stderr: Option<ValidationOutputReader>,
    timeout: Duration,
) -> Result<SettledValidationOutput, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if stdout
            .as_ref()
            .is_none_or(|reader| reader.task.is_finished())
            && stderr
                .as_ref()
                .is_none_or(|reader| reader.task.is_finished())
        {
            let stdout_bytes = stdout
                .take()
                .map(join_validation_output_reader)
                .transpose()?;
            let stderr_bytes = stderr
                .take()
                .map(join_validation_output_reader)
                .transpose()?;
            return Ok(Some((stdout_bytes, stderr_bytes)));
        }
        let now = Instant::now();
        if now >= deadline {
            if let Some(reader) = stdout.as_ref() {
                reader.stop.store(true, Ordering::Release);
            }
            if let Some(reader) = stderr.as_ref() {
                reader.stop.store(true, Ordering::Release);
            }
            let stop_deadline = Instant::now() + Duration::from_millis(100);
            while Instant::now() < stop_deadline
                && !(stdout
                    .as_ref()
                    .is_none_or(|reader| reader.task.is_finished())
                    && stderr
                        .as_ref()
                        .is_none_or(|reader| reader.task.is_finished()))
            {
                std::thread::sleep(
                    VALIDATION_OUTPUT_POLL_INTERVAL
                        .min(stop_deadline.saturating_duration_since(Instant::now())),
                );
            }
            // A timed-out pipe is never reported as settled, even if the stop
            // request closes our readers during the bounded cancellation grace.
            if stdout
                .as_ref()
                .is_some_and(|reader| reader.task.is_finished())
            {
                if let Some(reader) = stdout.take() {
                    let _ = join_validation_output_reader(reader);
                }
            }
            if stderr
                .as_ref()
                .is_some_and(|reader| reader.task.is_finished())
            {
                if let Some(reader) = stderr.take() {
                    let _ = join_validation_output_reader(reader);
                }
            }
            return Ok(None);
        }
        std::thread::sleep(VALIDATION_OUTPUT_POLL_INTERVAL.min(deadline - now));
    }
}

fn mark_validation_output_unsettled(
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
) -> String {
    match store.mark_validation_output_unsettled(operation_id, attempt, command_index) {
        Ok(()) => format!(
            "{VALIDATION_OUTPUT_UNSETTLED_PREFIX}; owned scratch was retained for explicit recovery"
        ),
        Err(error) => {
            store.mark_validation_cleanup_incomplete(
                operation_id,
                attempt,
                "validation output remained open and its incomplete cleanup state could not be fully recorded",
            );
            format!(
                "{VALIDATION_OUTPUT_UNSETTLED_PREFIX}; cleanup state persistence failed: {error}"
            )
        }
    }
}

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
/// state below HOME. It lives under the kernel's resolved CHARIOX_HOME state
/// root; an existing workspace-adjacent HOME is preserved and blocks setup
/// until explicitly migrated.
pub(super) struct WorkerPreparationHome {
    path: PathBuf,
}

impl WorkerPreparationHome {
    /// Create a stable project/worker HOME beneath the kernel's resolved
    /// CHARIOX_HOME, never from provider or project-supplied environment.
    pub(super) fn for_project_worker(
        workspace_root: &Path,
        kernel_home: &Path,
        project_id: &str,
        worker_id: &str,
    ) -> Result<Self, DaemonError> {
        let workspace_root =
            canonical_preparation_directory(workspace_root, "worker project worktree")?;
        let kernel_home = canonical_preparation_directory(kernel_home, "worker kernel home")?;
        verify_preparation_directory_owner(&kernel_home, "worker kernel home")?;
        if workspace_root.starts_with(&kernel_home) {
            return Err(setup_error(
                "worker project worktree must remain outside kernel-owned home state",
            ));
        }

        let home_key = worker_preparation_home_key(&workspace_root, project_id, worker_id);
        reject_legacy_preparation_home(&workspace_root, &home_key)?;
        reject_preparation_home_inside_git_worktree(&kernel_home)?;

        let state_root = kernel_home.join("state");
        ensure_preparation_directory(&state_root, "worker kernel state root", false)?;
        let preparation_root = state_root.join("project-environment-preparation");
        ensure_preparation_directory(&preparation_root, "worker preparation state", true)?;
        let path = preparation_root.join(home_key);
        ensure_preparation_directory(&path, "worker preparation HOME", true)?;
        Ok(Self { path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

pub(super) fn resolved_worker_kernel_home(config: &DaemonConfig) -> Result<PathBuf, DaemonError> {
    let config_home = config
        .user_config_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| setup_error("worker kernel config has no home directory"))?;
    canonical_preparation_directory(config_home, "worker kernel home")
}

fn worker_preparation_home_key(workspace_root: &Path, project_id: &str, worker_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(workspace_root.as_os_str().to_string_lossy().as_bytes());
    digest.update([0]);
    digest.update(project_id.as_bytes());
    digest.update([0]);
    digest.update(worker_id.as_bytes());
    format!("{:x}", digest.finalize())
}

fn canonical_preparation_directory(path: &Path, label: &str) -> Result<PathBuf, DaemonError> {
    let canonical = path
        .canonicalize()
        .map_err(|error| setup_error(&format!("{label} could not be resolved: {error}")))?;
    let metadata = std::fs::symlink_metadata(&canonical)
        .map_err(|error| setup_error(&format!("{label} could not be inspected: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(setup_error(&format!("{label} must be a real directory")));
    }
    Ok(canonical)
}

fn reject_legacy_preparation_home(
    workspace_root: &Path,
    home_key: &str,
) -> Result<(), DaemonError> {
    let legacy_root = workspace_root
        .parent()
        .ok_or_else(|| setup_error("worker worktree has no legacy state parent"))?
        .join(".chariox-project-environment");
    let legacy_root_metadata = match std::fs::symlink_metadata(&legacy_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(setup_error(&format!(
                "legacy worker preparation state could not be inspected: {error}"
            )))
        }
    };
    if legacy_root_metadata.file_type().is_symlink() || !legacy_root_metadata.is_dir() {
        return Err(setup_error(
            "legacy workspace-adjacent preparation state is not a real directory; existing state was left untouched",
        ));
    }

    let legacy_home = legacy_root.join(home_key);
    match std::fs::symlink_metadata(&legacy_home) {
        Ok(_) => Err(setup_error(
            "legacy workspace-adjacent preparation HOME exists; setup is blocked until its installed tools are explicitly migrated; existing state was left untouched",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(setup_error(&format!(
            "legacy worker preparation HOME could not be inspected: {error}"
        ))),
    }
}

fn reject_preparation_home_inside_git_worktree(kernel_home: &Path) -> Result<(), DaemonError> {
    for ancestor in kernel_home.ancestors() {
        let marker = ancestor.join(".git");
        match std::fs::symlink_metadata(&marker) {
            Ok(_) => {
                return Err(setup_error(
                    "worker preparation HOME would be created inside a Git worktree",
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(setup_error(&format!(
                    "kernel state repository boundary could not be inspected: {error}"
                )))
            }
        }
    }
    Ok(())
}

fn ensure_preparation_directory(
    path: &Path,
    label: &str,
    private: bool,
) -> Result<(), DaemonError> {
    #[cfg(not(unix))]
    let _ = private;

    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(setup_error(&format!(
                        "{label} could not be created: {error}"
                    )))
                }
            }
        }
        Err(error) => {
            return Err(setup_error(&format!(
                "{label} could not be inspected: {error}"
            )))
        }
    }

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| setup_error(&format!("{label} could not be inspected: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(setup_error(&format!("{label} must be a real directory")));
    }
    verify_preparation_directory_owner(path, label)?;

    #[cfg(unix)]
    if private {
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .map_err(|error| {
                setup_error(&format!(
                    "{label} permissions could not be secured: {error}"
                ))
            })?;
    }
    Ok(())
}

fn verify_preparation_directory_owner(path: &Path, label: &str) -> Result<(), DaemonError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| setup_error(&format!("{label} could not be inspected: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(setup_error(&format!("{label} must be a real directory")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(setup_error(&format!(
                "{label} must be owned by the worker kernel user"
            )));
        }
    }
    Ok(())
}

pub(super) fn ensure_worker_validation_boundary(config: &DaemonConfig) -> Result<(), DaemonError> {
    // General kernels are the ordinary local-worker placement. They still
    // pass through the canonical workspace and sanitized-environment checks
    // below, but they do not have (and must not require) a disposable-worker
    // receipt or a Cloud relay binding. General-role leased setup additionally
    // requires explicit remote-lease opt-in and the authenticated lease target.
    // Only RemoteLeaseWorker requires a confirmed activity allocation.
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

#[cfg(test)]
pub(super) fn worker_validation_environment(
    provider_run: &RuntimeProviderRun,
) -> BTreeMap<String, String> {
    worker_validation_environment_with_home(provider_run, None)
}

#[cfg(test)]
pub(super) fn worker_validation_environment_with_home(
    provider_run: &RuntimeProviderRun,
    preparation_home: Option<&Path>,
) -> BTreeMap<String, String> {
    worker_validation_environment_with_home_and_definition(
        provider_run,
        preparation_home,
        None,
        None,
    )
}

pub(super) fn worker_validation_environment_with_home_and_definition(
    provider_run: &RuntimeProviderRun,
    preparation_home: Option<&Path>,
    workspace_root: Option<&Path>,
    definition: Option<&ProjectEnvironmentDefinition>,
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
    if let Some(preparation_home) = preparation_home {
        environment.insert("HOME".to_string(), preparation_home.display().to_string());
        let current_path = environment
            .get("PATH")
            .map(String::as_str)
            .unwrap_or_default();
        let base_path = provider_run.preparation_base_path().unwrap_or(current_path);
        environment.insert(
            "PATH".to_string(),
            worker_preparation_path(preparation_home, workspace_root, base_path, definition),
        );
    }
    environment
}

fn worker_preparation_path(
    preparation_home: &Path,
    workspace_root: Option<&Path>,
    current_path: &str,
    definition: Option<&ProjectEnvironmentDefinition>,
) -> String {
    let local_bin = preparation_home.join(".local").join("bin");
    let cargo_bin = preparation_home.join(".cargo").join("bin");
    let mut entries = Vec::new();
    if let (Some(workspace_root), Some(definition)) = (workspace_root, definition) {
        for path_entry in &definition.path_entries {
            let root = match path_entry.base {
                crate::local::ProjectEnvironmentPathBase::PreparationHome => preparation_home,
                crate::local::ProjectEnvironmentPathBase::Workspace => workspace_root,
            };
            let entry = root.join(&path_entry.path);
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
    }
    for entry in [local_bin, cargo_bin] {
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    for entry in std::env::split_paths(std::ffi::OsStr::new(current_path)) {
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    match std::env::join_paths(entries.iter().map(|entry| entry.as_os_str())) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => {
            let separator = if cfg!(windows) { ";" } else { ":" };
            entries
                .iter()
                .map(|entry| entry.display().to_string())
                .collect::<Vec<_>>()
                .join(separator)
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
    resolve_project_environment_input_attestations_before_open(workspace_root, definition, |_| {})
}

fn resolve_project_environment_input_attestations_before_open(
    workspace_root: &Path,
    definition: &ProjectEnvironmentDefinition,
    mut before_open: impl FnMut(&Path),
) -> Result<ProjectEnvironmentDefinition, String> {
    definition.validate_for_utility_output()?;
    let mut resolved = definition.clone();
    let root = crate::managed_context::development::directory::open_root(workspace_root)
        .map_err(|_| "worker project root is unavailable".to_string())?;
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
        before_open(&candidate);
        let mut file = crate::managed_context::development::directory::open_relative(
            &root,
            Path::new(&input.path),
        )
        .map_err(|_| {
            format!(
                "worker project input could not be opened safely: {}",
                input.path
            )
        })?;
        if !file.metadata().is_ok_and(|metadata| {
            metadata.is_file() && metadata.len() <= MAX_PROJECT_ENVIRONMENT_INPUT_BYTES
        }) {
            return Err(format!(
                "worker project input is not a bounded regular file: {}",
                input.path
            ));
        }
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

#[expect(
    clippy::too_many_arguments,
    reason = "Keeps subprocess handles, authority, cancellation and recovery hooks explicit at this boundary."
)]
pub(super) fn run_worker_validation_command_with_recovery(
    command_text: &str,
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    scratch: &WorkerValidationScratch,
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
    should_cancel: impl Fn() -> bool,
    overall_deadline: Option<Instant>,
) -> Result<(i32, usize, usize), String> {
    run_worker_validation_command_with_hook(
        command_text,
        workspace_root,
        environment,
        scratch,
        store,
        operation_id,
        attempt,
        command_index,
        should_cancel,
        overall_deadline,
        || Ok(()),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keeps subprocess handles, authority, cancellation and recovery hooks explicit at this boundary."
)]
pub(super) fn run_worker_validation_command_with_hook(
    command_text: &str,
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    scratch: &WorkerValidationScratch,
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
    should_cancel: impl Fn() -> bool,
    overall_deadline: Option<Instant>,
    after_process_lease_persisted: impl FnOnce() -> Result<(), String>,
) -> Result<(i32, usize, usize), String> {
    run_worker_validation_command_with_persistence_hook(
        command_text,
        workspace_root,
        environment,
        scratch,
        store,
        operation_id,
        attempt,
        command_index,
        should_cancel,
        overall_deadline,
        |store, operation_id, attempt, command_index, pid| {
            store.persist_validation_process_identity(operation_id, attempt, command_index, pid)
        },
        after_process_lease_persisted,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keeps subprocess handles, authority, cancellation and recovery hooks explicit at this boundary."
)]
pub(super) fn run_worker_validation_command_with_persistence_hook(
    command_text: &str,
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    scratch: &WorkerValidationScratch,
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
    should_cancel: impl Fn() -> bool,
    overall_deadline: Option<Instant>,
    persist_process_identity: impl FnOnce(
        &ProjectEnvironmentSetupStore,
        &str,
        u32,
        usize,
        u32,
    ) -> Result<ValidationProcessIdentity, String>,
    after_process_lease_persisted: impl FnOnce() -> Result<(), String>,
) -> Result<(i32, usize, usize), String> {
    run_worker_validation_command_with_output_timeout(
        command_text,
        workspace_root,
        environment,
        scratch,
        store,
        operation_id,
        attempt,
        command_index,
        should_cancel,
        overall_deadline,
        persist_process_identity,
        after_process_lease_persisted,
        VALIDATION_OUTPUT_SETTLE_TIMEOUT,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keeps subprocess handles, authority, cancellation and recovery hooks explicit at this boundary."
)]
pub(super) fn run_worker_validation_command_with_output_timeout(
    command_text: &str,
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    scratch: &WorkerValidationScratch,
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
    should_cancel: impl Fn() -> bool,
    overall_deadline: Option<Instant>,
    persist_process_identity: impl FnOnce(
        &ProjectEnvironmentSetupStore,
        &str,
        u32,
        usize,
        u32,
    ) -> Result<ValidationProcessIdentity, String>,
    after_process_lease_persisted: impl FnOnce() -> Result<(), String>,
    output_settle_timeout: Duration,
) -> Result<(i32, usize, usize), String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            command_index,
            persist_process_identity,
            after_process_lease_persisted,
            output_settle_timeout,
        );
        let result = run_worker_validation_command_with_scratch(
            command_text,
            workspace_root,
            environment,
            scratch,
            should_cancel,
            overall_deadline,
        );
        if let Err(error) = &result {
            if error.starts_with(VALIDATION_OUTPUT_UNSETTLED_PREFIX) {
                store.mark_validation_cleanup_incomplete(
                    operation_id,
                    attempt,
                    "non-Linux validation output remained open; owned scratch was retained and restart process recovery is unsupported",
                );
            }
        }
        return result;
    }

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;

        let command_deadline =
            Instant::now() + Duration::from_millis(VALIDATION_COMMAND_TIMEOUT_MS);
        let deadline =
            overall_deadline.map_or(command_deadline, |value| value.min(command_deadline));
        if let Err(error) = store.persist_validation_scratch_intent(
            operation_id,
            attempt,
            command_index,
            scratch.path(),
        ) {
            if let Err(cleanup_error) = scratch.cleanup() {
                store.mark_validation_cleanup_incomplete(
                    operation_id,
                    attempt,
                    "validation scratch could not be removed after durable intent failure",
                );
                return Err(format!(
                    "{error}; validation cleanup incomplete: {cleanup_error}"
                ));
            }
            return Err(error);
        }
        if deadline.saturating_duration_since(Instant::now()).is_zero() {
            return Err(finish_unstarted_lease(
                scratch,
                store,
                operation_id,
                attempt,
                command_index,
                "worker validation command exceeded the overall setup deadline".to_string(),
            ));
        }

        // The shell has executed but remains blocked on a private stdin pipe.
        // Its durable process identity is committed before this pipe releases
        // the requested project command.
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("IFS= read -r _ || exit 125; exec 0</dev/null; exec /bin/sh -c \"$1\"")
            .arg("chariox-validation-gate")
            .arg(command_text)
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .envs(environment)
            .env(VALIDATION_SCRATCH_DIR_ENV, scratch.path().as_os_str());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return Err(finish_unstarted_lease(
                    scratch,
                    store,
                    operation_id,
                    attempt,
                    command_index,
                    error.to_string(),
                ));
            }
        };
        let pid = child.id();
        let gate_stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        if gate_stdin.is_none() || stdout.is_none() || stderr.is_none() {
            return Err(abort_gated_child(
                &mut child,
                gate_stdin,
                stdout,
                stderr,
                scratch,
                store,
                operation_id,
                attempt,
                command_index,
                "worker validation command could not establish its start gate and output pipes",
            ));
        }
        let mut gate_stdin = gate_stdin.expect("the validation gate pipe was checked");
        let stdout_pipe = stdout.expect("checked stdout");
        let stderr_pipe = stderr.expect("checked stderr");
        let stdout_reader = match spawn_validation_output_reader(stdout_pipe) {
            Ok(reader) => reader,
            Err(error) => {
                return Err(abort_gated_child(
                    &mut child,
                    Some(gate_stdin),
                    None,
                    Some(stderr_pipe),
                    scratch,
                    store,
                    operation_id,
                    attempt,
                    command_index,
                    &error,
                ));
            }
        };
        let stderr_reader = match spawn_validation_output_reader(stderr_pipe) {
            Ok(reader) => reader,
            Err(error) => {
                return Err(abort_gated_child_with_readers(
                    &mut child,
                    Some(gate_stdin),
                    Some(stdout_reader),
                    None,
                    scratch,
                    store,
                    operation_id,
                    attempt,
                    command_index,
                    &error,
                ));
            }
        };
        let pidfd = match open_pidfd(pid) {
            Ok(pidfd) => pidfd,
            Err(error) => {
                return Err(abort_gated_child_with_readers(
                    &mut child,
                    Some(gate_stdin),
                    Some(stdout_reader),
                    Some(stderr_reader),
                    scratch,
                    store,
                    operation_id,
                    attempt,
                    command_index,
                    &error,
                ));
            }
        };
        let identity =
            match persist_process_identity(store, operation_id, attempt, command_index, pid) {
                Ok(identity) => identity,
                Err(error) => {
                    return Err(abort_gated_child_with_readers(
                        &mut child,
                        Some(gate_stdin),
                        Some(stdout_reader),
                        Some(stderr_reader),
                        scratch,
                        store,
                        operation_id,
                        attempt,
                        command_index,
                        &error,
                    ));
                }
            };
        if let Err(error) = after_process_lease_persisted() {
            return Err(terminate_gated_child(
                &mut child,
                &identity,
                Some(gate_stdin),
                Some(stdout_reader),
                Some(stderr_reader),
                scratch,
                store,
                operation_id,
                attempt,
                command_index,
                &error,
            ));
        }
        if let Err(error) =
            store.release_validation_start_gate(operation_id, attempt, command_index, || {
                gate_stdin
                    .write_all(b"\n")
                    .map_err(|error| error.to_string())
            })
        {
            return Err(terminate_gated_child(
                &mut child,
                &identity,
                Some(gate_stdin),
                Some(stdout_reader),
                Some(stderr_reader),
                scratch,
                store,
                operation_id,
                attempt,
                command_index,
                &error,
            ));
        }
        drop(gate_stdin);

        match wait_child_or_cancel(&pidfd, &mut child, &identity, deadline, &should_cancel) {
            Ok(status) => {
                let output = match settle_validation_output_readers(
                    Some(stdout_reader),
                    Some(stderr_reader),
                    output_settle_timeout,
                ) {
                    Ok(Some((Some(stdout_bytes), Some(stderr_bytes)))) => {
                        Ok((status.code().unwrap_or(-1), stdout_bytes, stderr_bytes))
                    }
                    Ok(None) => {
                        return Err(mark_validation_output_unsettled(
                            store,
                            operation_id,
                            attempt,
                            command_index,
                        ));
                    }
                    Ok(Some(_)) => {
                        Err("worker validation output reader was unavailable".to_string())
                    }
                    Err(error) => Err(error),
                };
                cleanup_after_settled_group(scratch, store, operation_id, attempt, command_index)?;
                output
            }
            Err(error) if error.process_group_settled => {
                if let Ok(None) = settle_validation_output_readers(
                    Some(stdout_reader),
                    Some(stderr_reader),
                    output_settle_timeout,
                ) {
                    return Err(mark_validation_output_unsettled(
                        store,
                        operation_id,
                        attempt,
                        command_index,
                    ));
                }
                cleanup_after_settled_group(scratch, store, operation_id, attempt, command_index)?;
                Err(error.message)
            }
            Err(error) => {
                store.mark_validation_cleanup_incomplete(
                    operation_id,
                    attempt,
                    "kernel could not prove the validation process group was absent",
                );
                drop(stdout_reader);
                drop(stderr_reader);
                Err(format!("validation cleanup incomplete: {}", error.message))
            }
        }
    }
}

fn finish_unstarted_lease(
    scratch: &WorkerValidationScratch,
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
    reason: String,
) -> String {
    match cleanup_after_settled_group(scratch, store, operation_id, attempt, command_index) {
        Ok(()) => reason,
        Err(cleanup_error) => {
            store.mark_validation_cleanup_incomplete(
                operation_id,
                attempt,
                "validation scratch or intent could not be settled after a spawn failure",
            );
            format!("{reason}; validation cleanup incomplete: {cleanup_error}")
        }
    }
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::too_many_arguments,
    reason = "Keeps subprocess handles, authority, cancellation and recovery hooks explicit at this boundary."
)]
fn abort_gated_child(
    child: &mut std::process::Child,
    gate_stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
    scratch: &WorkerValidationScratch,
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
    reason: &str,
) -> String {
    let stdout_reader = stdout.and_then(|pipe| spawn_validation_output_reader(pipe).ok());
    let stderr_reader = stderr.and_then(|pipe| spawn_validation_output_reader(pipe).ok());
    abort_gated_child_with_readers(
        child,
        gate_stdin,
        stdout_reader,
        stderr_reader,
        scratch,
        store,
        operation_id,
        attempt,
        command_index,
        reason,
    )
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::too_many_arguments,
    reason = "Keeps subprocess handles, authority, cancellation and recovery hooks explicit at this boundary."
)]
fn abort_gated_child_with_readers(
    child: &mut std::process::Child,
    gate_stdin: Option<std::process::ChildStdin>,
    stdout_reader: Option<ValidationOutputReader>,
    stderr_reader: Option<ValidationOutputReader>,
    scratch: &WorkerValidationScratch,
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
    reason: &str,
) -> String {
    drop(gate_stdin);
    let _ = child.kill();
    match child.wait_timeout(Duration::from_secs(1)) {
        Ok(Some(_)) => {
            if matches!(
                settle_validation_output_readers(
                    stdout_reader,
                    stderr_reader,
                    VALIDATION_OUTPUT_SETTLE_TIMEOUT,
                ),
                Ok(None)
            ) {
                store.mark_validation_cleanup_incomplete(
                    operation_id,
                    attempt,
                    "gated validation output pipes did not close after child reaping",
                );
                return format!(
                    "{VALIDATION_OUTPUT_UNSETTLED_PREFIX}; gated command was not released"
                );
            }
            finish_unstarted_lease(
                scratch,
                store,
                operation_id,
                attempt,
                command_index,
                reason.to_string(),
            )
        }
        Ok(None) => {
            store.mark_validation_cleanup_incomplete(
                operation_id,
                attempt,
                "gated validation child did not exit after a start failure",
            );
            "validation cleanup incomplete: gated child could not be reaped".to_string()
        }
        Err(error) => {
            store.mark_validation_cleanup_incomplete(
                operation_id,
                attempt,
                "gated validation child could not be reaped after a start failure",
            );
            format!("validation cleanup incomplete: gated child could not be reaped: {error}")
        }
    }
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::too_many_arguments,
    reason = "Keeps subprocess handles, authority, cancellation and recovery hooks explicit at this boundary."
)]
fn terminate_gated_child(
    child: &mut std::process::Child,
    identity: &ValidationProcessIdentity,
    gate_stdin: Option<std::process::ChildStdin>,
    stdout_reader: Option<ValidationOutputReader>,
    stderr_reader: Option<ValidationOutputReader>,
    scratch: &WorkerValidationScratch,
    store: &ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
    reason: &str,
) -> String {
    drop(gate_stdin);
    if let Err(error) = kill_live_process_group(child, identity) {
        store.mark_validation_cleanup_incomplete(
            operation_id,
            attempt,
            "gated validation process group could not be verified for termination",
        );
        return format!("validation cleanup incomplete: {error}");
    }
    match child.wait_timeout(Duration::from_secs(1)) {
        Ok(Some(_)) => {
            if let Err(error) = wait_for_process_group_absence(identity.process_group_id()) {
                store.mark_validation_cleanup_incomplete(
                    operation_id,
                    attempt,
                    "gated validation process group did not settle after termination",
                );
                return format!("validation cleanup incomplete: {error}");
            }
            if matches!(
                settle_validation_output_readers(
                    stdout_reader,
                    stderr_reader,
                    VALIDATION_OUTPUT_SETTLE_TIMEOUT,
                ),
                Ok(None)
            ) {
                store.mark_validation_cleanup_incomplete(
                    operation_id,
                    attempt,
                    "gated validation output pipes did not close after process-group settlement",
                );
                return format!(
                    "{VALIDATION_OUTPUT_UNSETTLED_PREFIX}; gated command was not released"
                );
            }
            finish_unstarted_lease(
                scratch,
                store,
                operation_id,
                attempt,
                command_index,
                reason.to_string(),
            )
        }
        Ok(None) => {
            store.mark_validation_cleanup_incomplete(
                operation_id,
                attempt,
                "gated validation process group did not exit after termination",
            );
            "validation cleanup incomplete: gated process could not be reaped".to_string()
        }
        Err(error) => {
            store.mark_validation_cleanup_incomplete(
                operation_id,
                attempt,
                "gated validation process group could not be reaped after termination",
            );
            format!("validation cleanup incomplete: gated process could not be reaped: {error}")
        }
    }
}

pub(super) fn run_worker_validation_command(
    command_text: &str,
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    should_cancel: impl Fn() -> bool,
    overall_deadline: Option<Instant>,
) -> Result<(i32, usize, usize), String> {
    // MP-08/MP-11: General kernels use their ordinary provider/workspace
    // authority; leased setup also requires explicit remote-lease opt-in and
    // an authenticated lease. RemoteLeaseWorker additionally requires a
    // confirmed allocation receipt. Both fence the provider context and
    // canonical workspace before this child runs. Keep the shell local to
    // that execution boundary and do not route through the home
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
    let stdout_reader = match spawn_validation_output_reader(stdout) {
        Ok(reader) => reader,
        Err(error) => {
            terminate_validation_process_group(&mut child);
            let _ = child.wait();
            return Err(error);
        }
    };
    let stderr_reader = match spawn_validation_output_reader(stderr) {
        Ok(reader) => reader,
        Err(error) => {
            terminate_validation_process_group(&mut child);
            let _ = child.wait();
            return match settle_validation_output_readers(
                Some(stdout_reader),
                None,
                VALIDATION_OUTPUT_SETTLE_TIMEOUT,
            ) {
                Ok(None) => Err(format!("{VALIDATION_OUTPUT_UNSETTLED_PREFIX}; {error}")),
                Ok(Some(_)) | Err(_) => Err(error),
            };
        }
    };
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
            return match settle_validation_output_readers(
                Some(stdout_reader),
                Some(stderr_reader),
                VALIDATION_OUTPUT_SETTLE_TIMEOUT,
            ) {
                Ok(None) => Err(format!("{VALIDATION_OUTPUT_UNSETTLED_PREFIX}; {error}")),
                Ok(Some(_)) | Err(_) => Err(error),
            };
        }
    };
    terminate_validation_process_group(&mut child);
    let Some((Some(stdout_bytes), Some(stderr_bytes))) = settle_validation_output_readers(
        Some(stdout_reader),
        Some(stderr_reader),
        VALIDATION_OUTPUT_SETTLE_TIMEOUT,
    )?
    else {
        return Err(format!(
            "{VALIDATION_OUTPUT_UNSETTLED_PREFIX}; scratch must be retained"
        ));
    };
    Ok((status.code().unwrap_or(-1), stdout_bytes, stderr_bytes))
}

#[cfg(any(test, not(target_os = "linux")))]
pub(super) fn run_worker_validation_command_with_scratch(
    command_text: &str,
    workspace_root: &Path,
    environment: &BTreeMap<String, String>,
    scratch: &WorkerValidationScratch,
    should_cancel: impl Fn() -> bool,
    overall_deadline: Option<Instant>,
) -> Result<(i32, usize, usize), String> {
    let mut command_environment = environment.clone();
    command_environment.insert(
        VALIDATION_SCRATCH_DIR_ENV.to_string(),
        scratch.path().display().to_string(),
    );
    let command_result = run_worker_validation_command(
        command_text,
        workspace_root,
        &command_environment,
        should_cancel,
        overall_deadline,
    );
    if let Err(error) = &command_result {
        if error.starts_with(VALIDATION_OUTPUT_UNSETTLED_PREFIX) {
            return command_result;
        }
    }
    let cleanup_result = scratch.cleanup();
    match (command_result, cleanup_result) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(format!(
            "worker validation scratch cleanup failed: {cleanup_error}"
        )),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; worker validation scratch cleanup failed: {cleanup_error}"
        )),
    }
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
            &should_cancel,
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

#[cfg(all(test, not(target_os = "linux"), unix))]
mod non_linux_recovery_fallback_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture {
        root: PathBuf,
        workspace: PathBuf,
        operation_id: String,
        scratch: WorkerValidationScratch,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let suffix = NEXT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "chariox-validation-nonlinux-{}-{label}-{suffix}",
                std::process::id(),
            ));
            let workspace = root.join("workspace");
            let durable_home = root.join("home");
            std::fs::create_dir_all(&workspace).expect("fixture workspace should be created");
            std::fs::create_dir_all(&durable_home).expect("fixture HOME should be created");
            let operation_id = format!("nonlinux-{label}-{suffix}");
            let scratch = WorkerValidationScratch::create_for_worker(
                &workspace,
                &durable_home,
                &operation_id,
                1,
                0,
            )
            .expect("the ordinary validation scratch should be created");
            Self {
                root,
                workspace,
                operation_id,
                scratch,
            }
        }

        fn run(
            &self,
            command: &str,
            environment: &BTreeMap<String, String>,
            should_cancel: impl Fn() -> bool,
        ) -> Result<(i32, usize, usize), String> {
            let store = super::super::ProjectEnvironmentSetupStore::default();
            run_worker_validation_command_with_recovery(
                command,
                &self.workspace,
                environment,
                &self.scratch,
                &store,
                &self.operation_id,
                1,
                0,
                should_cancel,
                None,
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.scratch.cleanup();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn ordinary_non_linux_recovery_fallback_runs_success_and_cleans_owned_scratch() {
        let fixture = Fixture::new("success");
        let result = fixture
            .run("printf ready", &BTreeMap::new(), || false)
            .expect("ordinary validation should use the shared runner");
        assert_eq!(result, (0, 5, 0));
        assert!(!fixture.scratch.path().exists());
    }

    #[test]
    fn ordinary_non_linux_recovery_fallback_records_failure_and_cleans_owned_scratch() {
        let fixture = Fixture::new("failure");
        let result = fixture
            .run("printf nope >&2; exit 9", &BTreeMap::new(), || false)
            .expect("a nonzero command exit should remain an ordinary validation result");
        assert_eq!(result, (9, 0, 4));
        assert!(!fixture.scratch.path().exists());
    }

    #[test]
    fn ordinary_non_linux_recovery_fallback_cancels_and_cleans_owned_scratch() {
        let fixture = Fixture::new("cancel");
        let started = fixture.root.join("command-started");
        let environment = BTreeMap::from([(
            "NONLINUX_VALIDATION_STARTED".to_string(),
            started.display().to_string(),
        )]);
        let result = fixture.run(
            "printf started > \"$NONLINUX_VALIDATION_STARTED\"; sleep 2",
            &environment,
            || started.exists(),
        );
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(
            started.exists(),
            "the real validation command must start before cancel"
        );
        assert!(!fixture.scratch.path().exists());
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

#[cfg(unix)]
fn count_validation_output_nonblocking<R>(
    mut output: R,
    stop: Arc<AtomicBool>,
) -> Result<usize, String>
where
    R: Read + std::os::fd::AsRawFd,
{
    let mut buffer = [0_u8; 8192];
    let mut bytes = 0_usize;
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(bytes);
        }
        let mut descriptor = libc::pollfd {
            fd: output.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 20) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.to_string());
        }
        if ready == 0 {
            continue;
        }
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(bytes);
            }
            match output.read(&mut buffer) {
                Ok(0) => return Ok(bytes),
                Ok(read) => bytes = bytes.saturating_add(read),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
    }
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
        let kernel_home = root.join("kernel-home");
        std::fs::create_dir_all(&workspace).expect("durable-home workspace should exist");
        std::fs::create_dir_all(&kernel_home).expect("worker kernel home should exist");
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

        let preparation_home = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-1",
            "worker-1",
        )
        .expect("durable preparation HOME should be created");
        let expected_preparation_root = kernel_home
            .canonicalize()
            .unwrap()
            .join("state/project-environment-preparation");
        assert!(preparation_home
            .path()
            .starts_with(&expected_preparation_root));
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

        let validation_home = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-1",
            "worker-1",
        )
        .expect("validation should reuse durable preparation HOME");
        assert_eq!(validation_home.path(), preparation_home.path());
        let other_project = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-2",
            "worker-1",
        )
        .expect("another project should receive its own preparation HOME");
        let other_worker = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-1",
            "worker-2",
        )
        .expect("another worker should receive its own preparation HOME");
        let other_workspace = root.join("other-workspace");
        std::fs::create_dir_all(&other_workspace).expect("other worktree should exist");
        let other_worktree = WorkerPreparationHome::for_project_worker(
            &other_workspace,
            &kernel_home,
            "project-1",
            "worker-1",
        )
        .expect("another worktree should receive its own preparation HOME");
        assert_ne!(preparation_home.path(), other_project.path());
        assert_ne!(preparation_home.path(), other_worker.path());
        assert_ne!(preparation_home.path(), other_worktree.path());
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
    fn preparation_home_uses_kernel_state_when_accessible_worktree_parent_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "chariox-project-preparation-readonly-parent-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace_parent = root.join("readonly-workspace-parent");
        let workspace = workspace_parent.join("project");
        let kernel_home = root.join("kernel-home");
        std::fs::create_dir_all(&workspace).expect("accessible worktree should exist");
        std::fs::create_dir_all(&kernel_home).expect("trusted kernel home should exist");
        std::fs::set_permissions(&workspace_parent, std::fs::Permissions::from_mode(0o555))
            .expect("fixture worktree parent should become non-writable");

        let result = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-readonly-parent",
            "worker-1",
        );
        std::fs::set_permissions(&workspace_parent, std::fs::Permissions::from_mode(0o700))
            .expect("fixture worktree parent should be writable for cleanup");

        let preparation_home =
            result.expect("kernel state should not depend on worktree-parent write access");
        let expected_preparation_root = kernel_home
            .canonicalize()
            .unwrap()
            .join("state/project-environment-preparation");
        assert!(preparation_home
            .path()
            .starts_with(&expected_preparation_root));
        assert!(
            !workspace_parent
                .join(".chariox-project-environment")
                .exists(),
            "preparation must not write beside the project worktree",
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preparation_home_for_modeled_tmp_workspace_stays_under_kernel_state() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-preparation-modeled-tmp-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("tmp");
        let kernel_home = root.join("kernel-home");
        std::fs::create_dir_all(&workspace).expect("modeled /tmp worktree should exist");
        std::fs::create_dir_all(&kernel_home).expect("trusted kernel home should exist");

        let preparation_home = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-modeled-tmp",
            "worker-1",
        )
        .expect("modeled /tmp placement should use kernel-owned state");

        let expected_preparation_root = kernel_home
            .canonicalize()
            .unwrap()
            .join("state/project-environment-preparation");
        assert!(preparation_home
            .path()
            .starts_with(&expected_preparation_root));
        assert!(
            !root.join(".chariox-project-environment").exists(),
            "no workspace-adjacent preparation root should be created",
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preparation_home_allows_arbitrary_home_ancestor_of_kernel_home() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-preparation-home-ancestor-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("home");
        let kernel_home = workspace.join("user/.chariox");
        std::fs::create_dir_all(&kernel_home).expect("home workspace and kernel home should exist");

        let preparation_home = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-home-ancestor",
            "worker-1",
        )
        .expect("an arbitrary /home-like workspace may contain the kernel home as a descendant");

        assert!(preparation_home
            .path()
            .starts_with(kernel_home.join("state")));
        assert!(
            !workspace.join(".git").exists(),
            "the fixture models an arbitrary directory, not a tracked repository",
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preparation_home_rejects_kernel_state_inside_a_git_worktree() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-preparation-kernel-home-in-worktree-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("workspace");
        let kernel_home = workspace.join("kernel-home");
        std::fs::create_dir_all(&kernel_home).expect("worktree and kernel home should exist");
        std::fs::create_dir(workspace.join(".git")).expect("git worktree marker should exist");

        let result = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-kernel-state-inside-git-worktree",
            "worker-1",
        );

        let error = match result {
            Ok(_) => panic!("kernel state inside a Git worktree must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("inside a Git worktree"));
        assert!(
            !kernel_home.join("state").exists(),
            "preparation must not create Chariox state inside the Git worktree",
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preparation_home_resolves_default_kernel_home_without_chariox_home() {
        let _environment_lock = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-project-preparation-default-home-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let ordinary_home = root.join("ordinary-home");
        let config_home = ordinary_home.join(".chariox");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&config_home).expect("default kernel config home should exist");
        std::fs::create_dir_all(&workspace).expect("project workspace should exist");

        let old_chariox_home = std::env::var_os("CHARIOX_HOME");
        let old_home = std::env::var_os("HOME");
        let old_xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
        unsafe {
            std::env::remove_var("CHARIOX_HOME");
            std::env::set_var("HOME", &ordinary_home);
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        let config = DaemonConfig::new("preparation-default-home", "worker-1", "worker");
        let resolved_kernel_home = resolved_worker_kernel_home(&config);
        unsafe {
            match old_chariox_home {
                Some(value) => std::env::set_var("CHARIOX_HOME", value),
                None => std::env::remove_var("CHARIOX_HOME"),
            }
            match old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match old_xdg_config_home {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }

        assert_eq!(
            config.user_config_path,
            config_home.join("config.toml"),
            "ordinary config resolution should use HOME when CHARIOX_HOME is unset",
        );
        let kernel_home = resolved_kernel_home.expect("default kernel home should resolve");
        assert_eq!(kernel_home, config_home.canonicalize().unwrap());
        let preparation_home = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-default-home",
            "worker-1",
        )
        .expect("preparation HOME should use the standard kernel config home");
        assert!(preparation_home
            .path()
            .starts_with(kernel_home.join("state/project-environment-preparation")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn existing_sibling_preparation_home_fails_closed_without_moving_or_discarding_tools() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-preparation-legacy-conflict-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("workspace");
        let kernel_home = root.join("kernel-home");
        std::fs::create_dir_all(&workspace).expect("legacy workspace should exist");
        std::fs::create_dir_all(&kernel_home).expect("trusted kernel home should exist");
        let home_key = worker_preparation_home_key(
            &workspace.canonicalize().unwrap(),
            "project-legacy",
            "worker-1",
        );
        let legacy_home = root.join(".chariox-project-environment").join(home_key);
        std::fs::create_dir_all(legacy_home.join(".local/bin"))
            .expect("legacy preparation tools should exist");
        let legacy_tool = legacy_home.join(".local/bin/installed-tool");
        std::fs::write(&legacy_tool, b"preserve-installed-tool")
            .expect("legacy tool should be written");

        let result = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-legacy",
            "worker-1",
        );
        let error = match result {
            Ok(_) => panic!("legacy state requires explicit migration"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("explicitly migrated"));
        assert_eq!(
            std::fs::read(&legacy_tool).unwrap(),
            b"preserve-installed-tool",
        );
        assert!(
            !kernel_home.join("state").exists(),
            "conflict must fail before creating a competing HOME",
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn preparation_home_rejects_symlinked_kernel_state_without_following_it() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "chariox-project-preparation-state-symlink-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("workspace");
        let kernel_home = root.join("kernel-home");
        let foreign_state = root.join("foreign-state");
        std::fs::create_dir_all(&workspace).expect("worktree should exist");
        std::fs::create_dir_all(&kernel_home).expect("trusted kernel home should exist");
        std::fs::create_dir_all(&foreign_state).expect("foreign state should exist");
        std::fs::write(foreign_state.join("keep"), b"preserve")
            .expect("foreign sentinel should be written");
        symlink(&foreign_state, kernel_home.join("state"))
            .expect("kernel state replacement symlink should be created");

        let result = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-symlink-state",
            "worker-1",
        );

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(foreign_state.join("keep")).unwrap(),
            b"preserve"
        );
        assert!(!foreign_state
            .join("project-environment-preparation")
            .exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn worker_path_includes_definition_derived_home_and_workspace_toolchain_dirs() {
        let preparation_home = Path::new("/tmp/chariox-preparation-home");
        let workspace_root = Path::new("/tmp/project-workspace");
        let definition = ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
            source: ProjectEnvironmentDefinitionSource::Commands,
            target_platform: "linux-x86_64".to_string(),
            source_path: None,
            inputs: Vec::new(),
            path_entries: vec![
                crate::local::ProjectEnvironmentPathEntry {
                    base: crate::local::ProjectEnvironmentPathBase::PreparationHome,
                    path: "go/bin".to_string(),
                },
                crate::local::ProjectEnvironmentPathEntry {
                    base: crate::local::ProjectEnvironmentPathBase::PreparationHome,
                    path: ".local/share/pnpm".to_string(),
                },
                crate::local::ProjectEnvironmentPathEntry {
                    base: crate::local::ProjectEnvironmentPathBase::Workspace,
                    path: ".venv/bin".to_string(),
                },
            ],
            setup_steps: Vec::new(),
            validation_commands: vec!["true".to_string()],
        };
        let request = crate::provider::LaunchProviderRequest::new(
            "session-path-entry",
            "codex",
            "codex",
            "default",
            "default",
        );
        let run = crate::provider::RuntimeProviderRun::new(
            "provider-run-path-entry",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "path-entry-test".to_string(),
                pty_target: None,
                pty_program: Some("/bin/sh".to_string()),
                pty_args: Vec::new(),
                // Provider launches inherit PATH from the kernel process;
                // definition-derived entries must still be prepended to that
                // effective base for preparation and validation.
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: Some(workspace_root.to_path_buf()),
                structured_endpoint: None,
            },
        );
        let environment = worker_validation_environment_with_home_and_definition(
            &run,
            Some(preparation_home),
            Some(workspace_root),
            Some(&definition),
        );
        let path = environment
            .get("PATH")
            .expect("derived worker environment should have PATH");
        let entries = std::env::split_paths(std::ffi::OsStr::new(path)).collect::<Vec<_>>();
        let inherited_path = std::env::var("PATH").expect("test process should have PATH");
        assert_eq!(
            run.preparation_base_path(),
            Some(inherited_path.as_str()),
            "provider construction must snapshot inherited PATH before preparation"
        );
        let mut expected_entries = vec![
            preparation_home.join("go/bin"),
            preparation_home.join(".local/share/pnpm"),
            workspace_root.join(".venv/bin"),
            preparation_home.join(".local/bin"),
            preparation_home.join(".cargo/bin"),
        ];
        for entry in std::env::split_paths(std::ffi::OsStr::new(&inherited_path)) {
            if !expected_entries.contains(&entry) {
                expected_entries.push(entry);
            }
        }
        assert_eq!(entries, expected_entries);
    }

    #[cfg(unix)]
    #[test]
    fn worker_path_rebuild_drops_removed_definition_entry_before_validation() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-path-rebuild-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("workspace");
        let preparation_home = root.join("preparation-home");
        let old_bin = preparation_home.join("old/bin");
        let fresh_bin = workspace.join(".venv/bin");
        std::fs::create_dir_all(&old_bin).expect("old toolchain directory should exist");
        std::fs::create_dir_all(&fresh_bin).expect("fresh toolchain directory should exist");

        let old_tool = old_bin.join("project-tool");
        let fresh_tool = fresh_bin.join("project-tool");
        std::fs::write(&old_tool, b"#!/bin/sh\nprintf '%s' old\n").expect("old tool should exist");
        std::fs::set_permissions(&old_tool, std::fs::Permissions::from_mode(0o755))
            .expect("old tool should be executable");
        std::fs::write(&fresh_tool, b"#!/bin/sh\nprintf '%s' fresh\n")
            .expect("fresh tool should exist");
        std::fs::set_permissions(&fresh_tool, std::fs::Permissions::from_mode(0o755))
            .expect("fresh tool should be executable");

        let request = crate::provider::LaunchProviderRequest::new(
            "session-path-rebuild",
            "codex",
            "codex",
            "default",
            "default",
        );
        let run = crate::provider::RuntimeProviderRun::new(
            "provider-run-path-rebuild",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "path-rebuild-test".to_string(),
                pty_target: None,
                pty_program: Some("/bin/sh".to_string()),
                pty_args: Vec::new(),
                // Production provider launches inherit PATH from the kernel
                // process instead of repeating it in pty_env.
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: Some(workspace.clone()),
                structured_endpoint: None,
            },
        );
        let inherited_path = std::env::var("PATH").expect("test process should have PATH");
        assert_eq!(
            run.preparation_base_path(),
            Some(inherited_path.as_str()),
            "provider construction must snapshot inherited PATH before preparation"
        );
        let old_definition = ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
            source: ProjectEnvironmentDefinitionSource::Commands,
            target_platform: "linux-x86_64".to_string(),
            source_path: None,
            inputs: Vec::new(),
            path_entries: vec![crate::local::ProjectEnvironmentPathEntry {
                base: crate::local::ProjectEnvironmentPathBase::PreparationHome,
                path: "old/bin".to_string(),
            }],
            setup_steps: Vec::new(),
            validation_commands: vec!["true".to_string()],
        };
        let old_environment = worker_validation_environment_with_home_and_definition(
            &run,
            Some(&preparation_home),
            Some(&workspace),
            Some(&old_definition),
        );
        let old_path = old_environment
            .get("PATH")
            .expect("old definition environment should have PATH")
            .clone();
        assert!(
            std::env::split_paths(std::ffi::OsStr::new(&old_path)).any(|entry| entry == old_bin),
            "old definition should initially project its toolchain directory"
        );

        let mut prepared_run = run.clone();
        prepared_run
            .set_preparation_environment(
                old_environment
                    .get("HOME")
                    .expect("old definition environment should have HOME")
                    .clone(),
                old_path,
            )
            .expect("old prepared environment should bind");
        let fresh_definition = ProjectEnvironmentDefinition {
            path_entries: vec![crate::local::ProjectEnvironmentPathEntry {
                base: crate::local::ProjectEnvironmentPathBase::Workspace,
                path: ".venv/bin".to_string(),
            }],
            ..old_definition
        };
        let fresh_environment = worker_validation_environment_with_home_and_definition(
            &prepared_run,
            Some(&preparation_home),
            Some(&workspace),
            Some(&fresh_definition),
        );
        let fresh_path = fresh_environment
            .get("PATH")
            .expect("fresh definition environment should have PATH");
        let fresh_entries =
            std::env::split_paths(std::ffi::OsStr::new(fresh_path)).collect::<Vec<_>>();
        assert!(
            !fresh_entries.contains(&old_bin),
            "a removed definition path must not survive a provider rebind: {fresh_entries:?}"
        );
        assert!(
            fresh_entries.contains(&fresh_bin),
            "the replacement definition path must reach the fresh validation"
        );
        let expected_tool = fresh_tool.display();
        let (exit_code, _, _) = run_worker_validation_command(
            &format!(
                "test \"$(command -v project-tool)\" = \"{expected_tool}\" && test \"$(project-tool)\" = fresh"
            ),
            &workspace,
            &fresh_environment,
            || false,
            None,
        )
        .expect("fresh worker validation should execute with the replacement PATH");
        assert_eq!(exit_code, 0);

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
        let kernel_home = root.join("kernel-home");
        std::fs::create_dir_all(&workspace).expect("provider environment workspace should exist");
        std::fs::create_dir_all(&kernel_home).expect("worker kernel home should exist");
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

        let preparation_home = WorkerPreparationHome::for_project_worker(
            &workspace,
            &kernel_home,
            "project-1",
            "worker-1",
        )
        .expect("durable provider preparation HOME should be created");
        let provider_home = root.join("old-home");
        assert_ne!(preparation_home.path(), provider_home.as_path());
        let definition = ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
            source: ProjectEnvironmentDefinitionSource::Commands,
            target_platform: "linux-x86_64".to_string(),
            source_path: None,
            inputs: Vec::new(),
            path_entries: vec![crate::local::ProjectEnvironmentPathEntry {
                base: crate::local::ProjectEnvironmentPathBase::PreparationHome,
                path: "go/bin".to_string(),
            }],
            setup_steps: Vec::new(),
            validation_commands: vec!["project-tool".to_string()],
        };
        let prepared_environment = worker_validation_environment_with_home_and_definition(
            &run,
            Some(preparation_home.path()),
            Some(&workspace),
            Some(&definition),
        );
        let expected_preparation_home = preparation_home.path().display().to_string();
        assert_eq!(
            prepared_environment.get("HOME"),
            Some(&expected_preparation_home),
            "provider-supplied HOME must not replace kernel-owned preparation state",
        );
        let prepared_path = prepared_environment
            .get("PATH")
            .expect("prepared worker environment should have PATH");
        assert!(prepared_path
            .starts_with(&preparation_home.path().join("go/bin").display().to_string()));
        run_worker_setup_steps(
            &[r#"set -eu; mkdir -p "$HOME/go/bin"; printf '%s\n' '#!/bin/sh' 'printf "%s\n" installed-from-post-ready-provider' > "$HOME/go/bin/inner-tool"; chmod 700 "$HOME/go/bin/inner-tool"; printf '%s\n' '#!/bin/sh' 'command -v inner-tool >/dev/null' 'exec inner-tool' > "$HOME/go/bin/project-tool"; chmod 700 "$HOME/go/bin/project-tool""#.to_string()],
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
        let mut binding = crate::provider::initialize_claude_runtime(&prepared_run)
            .expect("real provider child should start with the prepared context");
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
                .join("go")
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
        std::fs::create_dir_all(&root).expect("project fixture root should exist");
        let root = root
            .canonicalize()
            .expect("project fixture root should be canonicalized");
        let scripts = root.join("scripts");
        let testdata = root.join("testdata");
        std::fs::create_dir_all(&scripts).expect("script directory should exist");
        std::fs::create_dir_all(&testdata).expect("application fixture directory should exist");
        // Production passes the canonical worker workspace; on macOS temp_dir()
        // is behind a /var symlink.
        let root = root
            .canonicalize()
            .expect("script fixture root should resolve");
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
            path_entries: Vec::new(),
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

    #[cfg(unix)]
    #[test]
    fn mp11_attestation_ancestor_and_fifo_swaps_fail_at_the_actual_open_seam() {
        use std::os::unix::fs::symlink;
        crate::test_support::assert_fifo_rejected(|fifo| {
            let root = fifo.parent().unwrap().join("workspace");
            let outside = fifo.parent().unwrap().join("outside");
            std::fs::create_dir_all(root.join("nested")).unwrap();
            std::fs::create_dir(&outside).unwrap();
            std::fs::write(root.join("nested/package.json"), b"synthetic inside").unwrap();
            std::fs::write(outside.join("package.json"), b"synthetic outside").unwrap();
            let definition = ProjectEnvironmentDefinition {
                schema_version: 1,
                origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
                source: ProjectEnvironmentDefinitionSource::Devcontainer,
                target_platform: "linux-x86_64".into(),
                source_path: Some("nested/package.json".into()),
                inputs: vec![ProjectEnvironmentInput {
                    kind: ProjectEnvironmentInputKind::Recipe,
                    path: "nested/package.json".into(),
                    sha256: crate::session::KERNEL_COMPUTED_INPUT_ATTESTATION.into(),
                }],
                path_entries: Vec::new(),
                setup_steps: Vec::new(),
                validation_commands: vec!["true".into()],
            };
            let ancestor = resolve_project_environment_input_attestations_before_open(
                &root,
                &definition,
                |_| {
                    std::fs::rename(root.join("nested"), root.join("original")).unwrap();
                    symlink(&outside, root.join("nested")).unwrap();
                },
            )
            .is_err();
            std::fs::remove_file(root.join("nested")).unwrap();
            std::fs::rename(root.join("original"), root.join("nested")).unwrap();
            let leaf = resolve_project_environment_input_attestations_before_open(
                &root,
                &definition,
                |candidate| {
                    std::fs::remove_file(candidate).unwrap();
                    symlink(&fifo, candidate).unwrap();
                },
            )
            .is_err();
            ancestor && leaf
        });
    }
    #[test]
    fn worker_input_attestation_is_content_bound_and_fails_closed() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-input-attestation-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("input fixture workspace should exist");
        let root = root
            .canonicalize()
            .expect("input fixture workspace should be canonicalized");
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
            path_entries: Vec::new(),
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

    #[cfg(unix)]
    #[test]
    fn worker_input_attestation_rejects_materialized_symlink_parent_escape() {
        let root = std::env::temp_dir().join(format!(
            "chariox-project-environment-input-escape-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        std::fs::create_dir_all(&workspace).expect("input fixture workspace should exist");
        std::fs::create_dir_all(&outside).expect("outside fixture should exist");
        let workspace = workspace
            .canonicalize()
            .expect("input fixture workspace should be canonicalized");
        let outside = outside
            .canonicalize()
            .expect("outside fixture should be canonicalized");
        let contents = br#"{"name":"outside"}
"#;
        std::fs::write(outside.join("package.json"), contents)
            .expect("outside input fixture should be written");
        std::os::unix::fs::symlink(&outside, workspace.join("linked"))
            .expect("workspace directory alias should be created");
        let definition = ProjectEnvironmentDefinition {
            schema_version: 1,
            origin: ProjectEnvironmentDefinitionOrigin::UserAuthored,
            source: ProjectEnvironmentDefinitionSource::Devcontainer,
            target_platform: "linux-x86_64".to_string(),
            source_path: Some("linked/package.json".to_string()),
            inputs: vec![ProjectEnvironmentInput {
                kind: ProjectEnvironmentInputKind::Recipe,
                path: "linked/package.json".to_string(),
                sha256: format!("sha256:{:x}", Sha256::digest(contents)),
            }],
            path_entries: Vec::new(),
            setup_steps: Vec::new(),
            validation_commands: vec!["true".to_string()],
        };

        let error = verify_project_environment_inputs(&workspace, &definition)
            .expect_err("an input resolved outside the canonical worktree must be rejected");
        assert!(error.contains("could not be opened safely"));

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
        let root = root
            .canonicalize()
            .expect("input fixture workspace should be canonicalized");
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
            path_entries: Vec::new(),
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

#[cfg(not(unix))]
fn count_validation_output_blocking(output: &mut impl Read) -> Result<usize, String> {
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
