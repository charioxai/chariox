use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wait_timeout::ChildExt;

const RECOVERY_VERSION: u8 = 1;
const SCRATCH_PREFIX: &str = "chariox-project-environment-setup-";
const OWNER_MARKER: &str = ".chariox-project-setup-owner";
const GROUP_SETTLE_TIMEOUT: Duration = Duration::from_secs(3);
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ValidationCommandLease {
    version: u8,
    operation_id: String,
    attempt: u32,
    command_index: usize,
    scratch_path: PathBuf,
    owner_token: String,
    boot_id: String,
    process: Option<ValidationProcessIdentity>,
    #[serde(default)]
    output_pipe_unsettled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::runtime::state::project_environment_setup) struct ValidationProcessIdentity {
    boot_id: String,
    pid: u32,
    process_group_id: u32,
    session_id: u32,
    start_time_ticks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcIdentity {
    pid: u32,
    process_group_id: u32,
    session_id: u32,
    start_time_ticks: u64,
    state: char,
}

#[derive(Debug)]
pub(in crate::runtime::state::project_environment_setup) struct ProcessRunError {
    pub(in crate::runtime::state::project_environment_setup) message: String,
    pub(in crate::runtime::state::project_environment_setup) process_group_settled: bool,
}

impl ValidationCommandLease {
    pub(super) fn scratch_intent(
        operation_id: &str,
        attempt: u32,
        command_index: usize,
        scratch_path: &Path,
    ) -> Result<Self, String> {
        let boot_id = current_boot_id()?;
        Self::scratch_intent_for_boot(operation_id, attempt, command_index, scratch_path, &boot_id)
    }

    fn scratch_intent_for_boot(
        operation_id: &str,
        attempt: u32,
        command_index: usize,
        scratch_path: &Path,
        boot_id: &str,
    ) -> Result<Self, String> {
        if operation_id.is_empty() || attempt == 0 || !valid_boot_id(boot_id) {
            return Err("validation recovery identity is incomplete".to_string());
        }
        Ok(Self {
            version: RECOVERY_VERSION,
            operation_id: operation_id.to_string(),
            attempt,
            command_index,
            scratch_path: scratch_path.to_path_buf(),
            owner_token: owner_token(operation_id, attempt, command_index),
            boot_id: boot_id.to_string(),
            process: None,
            output_pipe_unsettled: false,
        })
    }

    pub(super) fn bind_process(
        &mut self,
        process: ValidationProcessIdentity,
    ) -> Result<(), String> {
        if process.boot_id != self.boot_id
            || process.pid <= 1
            || process.process_group_id != process.pid
            || process.session_id != process.pid
            || process.start_time_ticks == 0
        {
            return Err(
                "validation process identity does not match its scratch intent".to_string(),
            );
        }
        self.process = Some(process);
        Ok(())
    }

    pub(super) fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub(super) fn matches_operation(&self, operation_id: &str, attempt: u32) -> bool {
        self.operation_id == operation_id && self.attempt == attempt
    }

    pub(super) fn attempt(&self) -> u32 {
        self.attempt
    }

    pub(super) fn command_index(&self) -> usize {
        self.command_index
    }

    pub(super) fn boot_id(&self) -> &str {
        &self.boot_id
    }

    pub(super) fn process(&self) -> Option<&ValidationProcessIdentity> {
        self.process.as_ref()
    }

    pub(super) fn mark_output_pipe_unsettled(&mut self) {
        self.output_pipe_unsettled = true;
    }
}

impl ValidationProcessIdentity {
    pub(in crate::runtime::state::project_environment_setup) fn process_group_id(&self) -> u32 {
        self.process_group_id
    }

    pub(super) fn from_child(pid: u32, expected_boot_id: &str) -> Result<Self, String> {
        let boot_id = current_boot_id()?;
        if boot_id != expected_boot_id {
            return Err(
                "worker boot identity changed while validation child was starting".to_string(),
            );
        }
        let actual = read_proc_identity(pid)?;
        if actual.pid != pid || actual.process_group_id != pid || actual.session_id != pid {
            return Err("validation child did not establish its owned process group".to_string());
        }
        Ok(Self {
            boot_id,
            pid,
            process_group_id: actual.process_group_id,
            session_id: actual.session_id,
            start_time_ticks: actual.start_time_ticks,
        })
    }
}

pub(super) fn recover_validation_command(lease: &ValidationCommandLease) -> Result<(), String> {
    let boot_id = current_boot_id()?;
    recover_validation_command_for_boot(lease, &boot_id)
}

fn recover_validation_command_for_boot(
    lease: &ValidationCommandLease,
    current_boot_id: &str,
) -> Result<(), String> {
    validate_lease(lease)?;
    if !valid_boot_id(current_boot_id) {
        return Err("current worker boot identity is invalid".to_string());
    }
    if current_boot_id != lease.boot_id {
        // A verified boot transition proves that none of the prior boot's
        // command processes or output-pipe holders can still be running. Keep
        // the filesystem owner check as the independent authority for removal.
        return remove_owned_validation_scratch(lease);
    }
    if lease.output_pipe_unsettled {
        return Err(
            "validation output remained open after process-group settlement; scratch ownership recovery is incomplete"
                .to_string(),
        );
    }
    match lease.process.as_ref() {
        Some(process) => settle_process_group(process)?,
        None => {
            // Intent is committed before spawn and the wrapper cannot run the
            // user command until its identity is committed. With no process
            // identity there is no safe numeric group to signal or probe.
            return Err(
                "validation child identity was not durably recorded; cleanup is incomplete"
                    .to_string(),
            );
        }
    }
    remove_owned_validation_scratch(lease)
}

pub(in crate::runtime::state::project_environment_setup) fn cleanup_after_settled_group(
    scratch: &super::super::project_environment_setup_scratch::WorkerValidationScratch,
    store: &super::ProjectEnvironmentSetupStore,
    operation_id: &str,
    attempt: u32,
    command_index: usize,
) -> Result<(), String> {
    if let Err(error) = scratch.cleanup() {
        store.mark_validation_cleanup_incomplete(
            operation_id,
            attempt,
            "owned validation scratch could not be removed after process settlement",
        );
        return Err(format!(
            "owned validation scratch could not be removed: {error}"
        ));
    }
    if let Err(error) = store.clear_validation_command_lease(operation_id, attempt, command_index) {
        store.mark_validation_cleanup_incomplete(
            operation_id,
            attempt,
            "settled validation scratch lease could not be durably cleared",
        );
        return Err(error);
    }
    Ok(())
}

fn remove_owned_validation_scratch(lease: &ValidationCommandLease) -> Result<(), String> {
    validate_lease(lease)?;
    let expected_name = format!("{SCRATCH_PREFIX}{}", &lease.owner_token[7..]);
    if lease
        .scratch_path
        .file_name()
        .and_then(|name| name.to_str())
        != Some(expected_name.as_str())
    {
        return Err("validation scratch path does not match its operation identity".to_string());
    }
    let parent = lease
        .scratch_path
        .parent()
        .ok_or_else(|| "validation scratch path has no parent".to_string())?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        format!("validation scratch parent could not be canonicalized: {error}")
    })?;
    if canonical_parent != parent || !allowed_scratch_roots().iter().any(|root| root == parent) {
        return Err(
            "validation scratch is outside the canonical worker temporary roots".to_string(),
        );
    }
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("validation scratch parent could not be inspected: {error}"))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err("validation scratch parent is not a real directory".to_string());
    }

    let directory = match fs::symlink_metadata(&lease.scratch_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "validation scratch could not be inspected: {error}"
            ))
        }
    };
    if directory.file_type().is_symlink() || !directory.is_dir() {
        return Err("validation scratch is not an owned real directory".to_string());
    }
    let marker_path = lease.scratch_path.join(OWNER_MARKER);
    let marker_metadata = fs::symlink_metadata(&marker_path).map_err(|error| {
        format!("validation scratch owner marker could not be inspected: {error}")
    })?;
    if marker_metadata.file_type().is_symlink() || !marker_metadata.is_file() {
        return Err("validation scratch owner marker is not a regular file".to_string());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let marker = options
        .open(&marker_path)
        .map_err(|error| format!("validation scratch owner marker could not be read: {error}"))?;
    let mut actual_owner = Vec::with_capacity(lease.owner_token.len() + 1);
    marker
        .take((lease.owner_token.len() + 1) as u64)
        .read_to_end(&mut actual_owner)
        .map_err(|error| format!("validation scratch owner marker could not be read: {error}"))?;
    if actual_owner.as_slice() != lease.owner_token.as_bytes() {
        return Err("validation scratch owner marker changed".to_string());
    }
    fs::remove_dir_all(&lease.scratch_path)
        .map_err(|error| format!("owned validation scratch could not be removed: {error}"))?;
    match fs::symlink_metadata(&lease.scratch_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err("owned validation scratch remains after cleanup".to_string()),
        Err(error) => Err(format!(
            "validation scratch cleanup could not be verified: {error}"
        )),
    }
}

fn validate_live_child_identity(identity: &ValidationProcessIdentity) -> Result<(), String> {
    if current_boot_id()? != identity.boot_id {
        return Err("validation child belongs to a different worker boot".to_string());
    }
    let current = read_proc_identity(identity.pid)?;
    if !matches_identity(&current, identity) {
        return Err("validation child process identity changed".to_string());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn process_group_is_absent(process_group_id: u32) -> Result<bool, String> {
    if process_group_id <= 1 || process_group_id > i32::MAX as u32 {
        return Err("validation process group identity is invalid".to_string());
    }
    let result = unsafe { libc::kill(-(process_group_id as libc::pid_t), 0) };
    if result == 0 {
        return Ok(false);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Ok(true),
        Some(libc::EPERM) => Ok(false),
        _ => Err("validation process group could not be inspected".to_string()),
    }
}

#[cfg(not(target_os = "linux"))]
fn process_group_is_absent(_process_group_id: u32) -> Result<bool, String> {
    Err("durable validation process recovery is unsupported on this platform".to_string())
}

#[cfg(target_os = "linux")]
pub(in crate::runtime::state::project_environment_setup) fn open_pidfd(
    pid: u32,
) -> Result<File, String> {
    use std::os::fd::FromRawFd;
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err("validation child PID is invalid".to_string());
    }
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) as i32 };
    if fd < 0 {
        return Err(format!(
            "validation child pidfd could not be opened: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(not(target_os = "linux"))]
pub(in crate::runtime::state::project_environment_setup) fn open_pidfd(
    _pid: u32,
) -> Result<File, String> {
    Err("durable validation process recovery is unsupported on this platform".to_string())
}

#[cfg(target_os = "linux")]
pub(in crate::runtime::state::project_environment_setup) fn wait_child_or_cancel(
    pidfd: &File,
    child: &mut std::process::Child,
    identity: &ValidationProcessIdentity,
    deadline: Instant,
    should_cancel: &impl Fn() -> bool,
) -> Result<std::process::ExitStatus, ProcessRunError> {
    use std::os::fd::AsRawFd;
    loop {
        if should_cancel() {
            if let Err(message) = kill_live_process_group(child, identity) {
                return Err(ProcessRunError {
                    message,
                    process_group_settled: false,
                });
            }
            if let Err(message) = reap_child_bounded(child) {
                return Err(ProcessRunError {
                    message,
                    process_group_settled: false,
                });
            }
            return match wait_for_process_group_absence(identity.process_group_id) {
                Ok(()) => Err(ProcessRunError {
                    message: "worker validation command cancelled".to_string(),
                    process_group_settled: true,
                }),
                Err(message) => Err(ProcessRunError {
                    message,
                    process_group_settled: false,
                }),
            };
        }
        if Instant::now() >= deadline {
            if let Err(message) = kill_live_process_group(child, identity) {
                return Err(ProcessRunError {
                    message,
                    process_group_settled: false,
                });
            }
            if let Err(message) = reap_child_bounded(child) {
                return Err(ProcessRunError {
                    message,
                    process_group_settled: false,
                });
            }
            return match wait_for_process_group_absence(identity.process_group_id) {
                Ok(()) => Err(ProcessRunError {
                    message: "worker validation command timed out".to_string(),
                    process_group_settled: true,
                }),
                Err(message) => Err(ProcessRunError {
                    message,
                    process_group_settled: false,
                }),
            };
        }
        let mut descriptor = libc::pollfd {
            fd: pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(50))
            .as_millis()
            .max(1) as libc::c_int;
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            let message = format!("validation child completion could not be observed: {error}");
            let cleanup = kill_live_process_group(child, identity)
                .and_then(|()| reap_child_bounded(child))
                .and_then(|()| wait_for_process_group_absence(identity.process_group_id));
            return match cleanup {
                Ok(()) => Err(ProcessRunError {
                    message,
                    process_group_settled: true,
                }),
                Err(cleanup_error) => Err(ProcessRunError {
                    message: cleanup_error,
                    process_group_settled: false,
                }),
            };
        }
        if result == 0 {
            continue;
        }
        if let Err(message) = validate_live_child_identity(identity) {
            return Err(ProcessRunError {
                message,
                process_group_settled: false,
            });
        }
        if let Err(message) = kill_live_process_group(child, identity) {
            return Err(ProcessRunError {
                message,
                process_group_settled: false,
            });
        }
        let status = child.wait().map_err(|error| ProcessRunError {
            message: format!("validation child could not be reaped: {error}"),
            process_group_settled: false,
        })?;
        wait_for_process_group_absence(identity.process_group_id).map_err(|message| {
            ProcessRunError {
                message,
                process_group_settled: false,
            }
        })?;
        return Ok(status);
    }
}

#[cfg(target_os = "linux")]
fn reap_child_bounded(child: &mut std::process::Child) -> Result<(), String> {
    match child.wait_timeout(Duration::from_secs(1)) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err("validation child did not exit before the reap deadline".to_string()),
        Err(error) => Err(format!("validation child could not be reaped: {error}")),
    }
}

#[cfg(not(target_os = "linux"))]
pub(in crate::runtime::state::project_environment_setup) fn wait_child_or_cancel(
    _pidfd: &File,
    _child: &mut std::process::Child,
    _identity: &ValidationProcessIdentity,
    _deadline: Instant,
    _should_cancel: &impl Fn() -> bool,
) -> Result<std::process::ExitStatus, ProcessRunError> {
    Err(ProcessRunError {
        message: "durable validation process recovery is unsupported on this platform".to_string(),
        process_group_settled: false,
    })
}

#[cfg(target_os = "linux")]
pub(in crate::runtime::state::project_environment_setup) fn kill_live_process_group(
    child: &std::process::Child,
    identity: &ValidationProcessIdentity,
) -> Result<(), String> {
    if child.id() != identity.pid {
        return Err("validation child handle no longer matches its process lease".to_string());
    }
    validate_live_child_identity(identity)?;
    let result = unsafe { libc::kill(-(identity.process_group_id as libc::pid_t), libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ESRCH) => Ok(()),
        _ => Err("owned validation process group could not be terminated".to_string()),
    }
}

#[cfg(not(target_os = "linux"))]
pub(in crate::runtime::state::project_environment_setup) fn kill_live_process_group(
    _child: &std::process::Child,
    _identity: &ValidationProcessIdentity,
) -> Result<(), String> {
    Err("durable validation process recovery is unsupported on this platform".to_string())
}

#[cfg(target_os = "linux")]
fn settle_process_group(identity: &ValidationProcessIdentity) -> Result<(), String> {
    if identity.pid <= 1
        || identity.process_group_id != identity.pid
        || identity.session_id != identity.pid
        || identity.start_time_ticks == 0
    {
        return Err("validation process lease is malformed".to_string());
    }
    let initial = match read_proc_identity(identity.pid) {
        Ok(current) if matches_identity(&current, identity) => current,
        Ok(_) | Err(_) => return wait_for_process_group_absence(identity.process_group_id),
    };
    if initial.state == 'Z' || initial.state == 'X' {
        return wait_for_process_group_absence(identity.process_group_id);
    }

    let pidfd = open_pidfd(identity.pid)?;
    let after_open = match read_proc_identity(identity.pid) {
        Ok(current) if matches_identity(&current, identity) => current,
        Ok(_) | Err(_) => return wait_for_process_group_absence(identity.process_group_id),
    };
    if after_open.state == 'Z' || after_open.state == 'X' {
        return wait_for_process_group_absence(identity.process_group_id);
    }
    if send_pidfd_signal(&pidfd, libc::SIGSTOP).is_err() {
        return wait_for_process_group_absence(identity.process_group_id);
    }
    let stop_deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        match read_proc_identity(identity.pid) {
            Ok(current)
                if matches_identity(&current, identity) && matches!(current.state, 'T' | 't') =>
            {
                break;
            }
            Ok(current)
                if matches_identity(&current, identity) && matches!(current.state, 'Z' | 'X') =>
            {
                return wait_for_process_group_absence(identity.process_group_id);
            }
            Ok(current) if matches_identity(&current, identity) => {}
            Ok(_) | Err(_) => return wait_for_process_group_absence(identity.process_group_id),
        }
        if Instant::now() >= stop_deadline {
            return Err(
                "owned validation child could not be stopped for safe group cleanup".to_string(),
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let stopped = read_proc_identity(identity.pid)?;
    if !matches_identity(&stopped, identity) || !matches!(stopped.state, 'T' | 't') {
        return Err("validation child identity changed before process-group cleanup".to_string());
    }
    kill_stopped_process_group(identity)?;
    wait_for_process_group_absence(identity.process_group_id)
}

#[cfg(target_os = "linux")]
fn kill_stopped_process_group(identity: &ValidationProcessIdentity) -> Result<(), String> {
    let stopped = read_proc_identity(identity.pid)?;
    if !matches_identity(&stopped, identity) || !matches!(stopped.state, 'T' | 't') {
        return Err(
            "validation child identity changed before process-group termination".to_string(),
        );
    }
    let result = unsafe { libc::kill(-(identity.process_group_id as libc::pid_t), libc::SIGKILL) };
    if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err("owned validation process group could not be terminated".to_string())
    }
}

#[cfg(not(target_os = "linux"))]
fn settle_process_group(_identity: &ValidationProcessIdentity) -> Result<(), String> {
    Err("durable validation process recovery is unsupported on this platform".to_string())
}

#[cfg(target_os = "linux")]
fn send_pidfd_signal(pidfd: &File, signal: libc::c_int) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        ) as libc::c_int
    };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "validation child could not be signalled by pidfd: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(not(target_os = "linux"))]
fn send_pidfd_signal(_pidfd: &File, _signal: libc::c_int) -> Result<(), String> {
    Err("durable validation process recovery is unsupported on this platform".to_string())
}

pub(in crate::runtime::state::project_environment_setup) fn wait_for_process_group_absence(
    process_group_id: u32,
) -> Result<(), String> {
    let deadline = Instant::now() + GROUP_SETTLE_TIMEOUT;
    loop {
        if process_group_is_absent(process_group_id)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(
                "validation process group did not become absent before cleanup deadline"
                    .to_string(),
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn matches_identity(current: &ProcIdentity, expected: &ValidationProcessIdentity) -> bool {
    current.pid == expected.pid
        && current.process_group_id == expected.process_group_id
        && current.session_id == expected.session_id
        && current.start_time_ticks == expected.start_time_ticks
}

fn validate_lease(lease: &ValidationCommandLease) -> Result<(), String> {
    if lease.version != RECOVERY_VERSION
        || lease.operation_id.is_empty()
        || lease.attempt == 0
        || !valid_boot_id(&lease.boot_id)
        || lease.owner_token != owner_token(&lease.operation_id, lease.attempt, lease.command_index)
    {
        return Err("validation command lease is malformed".to_string());
    }
    if let Some(process) = lease.process.as_ref() {
        if process.boot_id != lease.boot_id
            || process.pid <= 1
            || process.pid > i32::MAX as u32
            || process.process_group_id != process.pid
            || process.session_id != process.pid
            || process.start_time_ticks == 0
        {
            return Err("validation process lease is malformed".to_string());
        }
    }
    Ok(())
}

fn valid_boot_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
}

fn owner_token(operation_id: &str, attempt: u32, command_index: usize) -> String {
    let mut digest = Sha256::new();
    digest.update(b"chariox-project-environment-setup-validation-scratch-v1\0");
    digest.update(operation_id.as_bytes());
    digest.update([0]);
    digest.update(attempt.to_be_bytes());
    digest.update((command_index as u64).to_be_bytes());
    format!("sha256:{:x}", digest.finalize())
}

fn allowed_scratch_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    #[cfg(unix)]
    roots.extend([PathBuf::from("/var/tmp"), PathBuf::from("/tmp")]);
    roots
        .into_iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .filter(|root| {
            fs::symlink_metadata(root)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn current_boot_id() -> Result<String, String> {
    let value = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|error| format!("worker boot identity could not be read: {error}"))?;
    let value = value.trim();
    if !valid_boot_id(value) {
        return Err("worker boot identity is invalid".to_string());
    }
    Ok(value.to_string())
}

#[cfg(not(target_os = "linux"))]
fn current_boot_id() -> Result<String, String> {
    Err("durable validation process recovery is unsupported on this platform".to_string())
}

#[cfg(target_os = "linux")]
fn read_proc_identity(pid: u32) -> Result<ProcIdentity, String> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err("validation child PID is invalid".to_string());
    }
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|error| format!("validation child identity could not be read: {error}"))?;
    let close = stat
        .rfind(')')
        .ok_or_else(|| "validation child stat record is malformed".to_string())?;
    let fields = stat[close + 1..].split_whitespace().collect::<Vec<_>>();
    if fields.len() <= 19 {
        return Err("validation child stat record is truncated".to_string());
    }
    let state = fields[0]
        .chars()
        .next()
        .ok_or_else(|| "validation child state is missing".to_string())?;
    let parent_group = fields[2]
        .parse::<u32>()
        .map_err(|_| "validation child process group is malformed".to_string())?;
    let session_id = fields[3]
        .parse::<u32>()
        .map_err(|_| "validation child session is malformed".to_string())?;
    let start_time_ticks = fields[19]
        .parse::<u64>()
        .map_err(|_| "validation child start time is malformed".to_string())?;
    Ok(ProcIdentity {
        pid,
        process_group_id: parent_group,
        session_id,
        start_time_ticks,
        state,
    })
}

#[cfg(not(target_os = "linux"))]
fn read_proc_identity(_pid: u32) -> Result<ProcIdentity, String> {
    Err("durable validation process recovery is unsupported on this platform".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::process::{Command, Stdio};

    #[cfg(target_os = "linux")]
    #[test]
    fn scratch_recovery_requires_exact_marker_and_preserves_symlink_or_tampering() {
        let root = test_root("marker");
        let scratch = make_scratch(&root, "marker-op", 1, 0);
        let lease = ValidationCommandLease::scratch_intent("marker-op", 1, 0, scratch.path())
            .expect("the test lease should be created");

        fs::write(scratch.path().join(OWNER_MARKER), b"foreign owner")
            .expect("the marker should be tampered");
        assert!(remove_owned_validation_scratch(&lease).is_err());
        assert!(scratch.path().exists());

        fs::remove_file(scratch.path().join(OWNER_MARKER)).expect("the marker should be removed");
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("foreign"), scratch.path().join(OWNER_MARKER))
            .expect("the foreign marker symlink should be installed");
        #[cfg(not(unix))]
        fs::write(
            scratch.path().join(OWNER_MARKER),
            lease.owner_token.as_bytes(),
        )
        .expect("the expected marker should be restored");
        assert!(remove_owned_validation_scratch(&lease).is_err());
        assert!(scratch.path().exists());
        #[cfg(unix)]
        fs::remove_file(scratch.path().join(OWNER_MARKER))
            .expect("the test symlink should be removed");
        let _ = fs::remove_dir_all(scratch.path());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recovery_kills_only_the_exact_live_validation_group_and_removes_owned_scratch() {
        use std::os::unix::process::CommandExt;
        let root = test_root("live-group");
        let scratch = make_scratch(&root, "live-group-op", 1, 0);
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("30").stdin(std::process::Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().expect("the fixture process should start");
        let identity = ValidationProcessIdentity::from_child(
            child.id(),
            &current_boot_id().expect("the Linux boot id should be available"),
        )
        .expect("the fixture process should have an exact Linux identity");
        let mut lease =
            ValidationCommandLease::scratch_intent("live-group-op", 1, 0, scratch.path())
                .expect("the test lease should be created");
        lease
            .bind_process(identity.clone())
            .expect("the fixture process should bind to the lease");

        let reaper = std::thread::spawn(move || {
            let mut child = child;
            child
                .wait()
                .expect("the killed fixture process should be reaped")
        });
        recover_validation_command(&lease).expect("the owned group should be killed and settled");
        assert!(!scratch.path().exists());
        assert!(process_group_is_absent(identity.process_group_id)
            .expect("the group should be readable"));
        let _ = reaper.join().expect("the fixture reaper should complete");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mismatched_start_identity_does_not_signal_a_live_foreign_group_or_remove_scratch() {
        use std::os::unix::process::CommandExt;
        let root = test_root("mismatch");
        let scratch = make_scratch(&root, "mismatch-op", 1, 0);
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("30").stdin(std::process::Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().expect("the fixture process should start");
        let mut identity = ValidationProcessIdentity::from_child(
            child.id(),
            &current_boot_id().expect("the Linux boot id should be available"),
        )
        .expect("the fixture identity should be readable");
        identity.start_time_ticks = identity.start_time_ticks.saturating_add(1);
        let mut lease = ValidationCommandLease::scratch_intent("mismatch-op", 1, 0, scratch.path())
            .expect("the test lease should be created");
        lease
            .bind_process(identity.clone())
            .expect("the mismatched identity remains structurally valid");

        assert!(recover_validation_command(&lease).is_err());
        assert!(scratch.path().exists());
        assert!(!process_group_is_absent(identity.process_group_id)
            .expect("the group should be readable"));
        let _ = unsafe { libc::kill(-(identity.process_group_id as libc::pid_t), libc::SIGKILL) };
        let _ = child.wait();
        let _ = fs::remove_dir_all(scratch.path());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mismatched_pid_does_not_signal_a_live_foreign_group_or_remove_scratch() {
        use std::os::unix::process::CommandExt;

        let root = test_root("foreign-pid");
        let scratch = make_scratch(&root, "foreign-pid-op", 1, 0);
        let mut command = Command::new("/bin/sleep");
        command.arg("30").stdin(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .expect("the foreign fixture process should start");
        let identity = ValidationProcessIdentity::from_child(
            child.id(),
            &current_boot_id().expect("the Linux boot id should be available"),
        )
        .expect("the fixture process identity should be readable");
        let mut lease =
            ValidationCommandLease::scratch_intent("foreign-pid-op", 1, 0, scratch.path())
                .expect("the test lease should be created");
        lease
            .bind_process(identity.clone())
            .expect("the fixture identity should bind before corruption");
        lease
            .process
            .as_mut()
            .expect("the process identity should exist")
            .pid = identity.pid.saturating_add(1000);

        assert!(recover_validation_command(&lease).is_err());
        assert!(scratch.path().exists());
        assert!(!process_group_is_absent(identity.process_group_id)
            .expect("the fixture group should be readable"));
        let _ = unsafe { libc::kill(-(identity.process_group_id as libc::pid_t), libc::SIGKILL) };
        let _ = child.wait();
        let _ = fs::remove_dir_all(scratch.path());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_group_leader_with_surviving_descendant_preserves_group_and_scratch() {
        use std::os::unix::process::CommandExt;

        let root = test_root("missing-leader");
        let scratch = make_scratch(&root, "missing-leader-op", 1, 0);
        let descendant_pid_path = root.join("descendant-pid");
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("sleep 30 & echo $! > \"$1\"; IFS= read -r _")
            .arg("validation-group-fixture")
            .arg(&descendant_pid_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut leader = command.spawn().expect("the fixture leader should start");
        let leader_pid = leader.id();
        let _gate = leader
            .stdin
            .take()
            .expect("the fixture leader should wait on stdin");
        let deadline = Instant::now() + Duration::from_secs(1);
        while !descendant_pid_path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            descendant_pid_path.exists(),
            "the fixture descendant should start"
        );
        let identity = ValidationProcessIdentity::from_child(
            leader_pid,
            &current_boot_id().expect("the Linux boot ID should be available"),
        )
        .expect("the live group leader identity should be captured");
        let descendant_pid = fs::read_to_string(&descendant_pid_path)
            .expect("the fixture should record its descendant")
            .trim()
            .parse::<u32>()
            .expect("the fixture descendant PID should be numeric");
        let descendant =
            read_proc_identity(descendant_pid).expect("the fixture descendant should remain live");
        assert_eq!(descendant.process_group_id, leader_pid);
        assert_eq!(descendant.session_id, leader_pid);

        let mut lease =
            ValidationCommandLease::scratch_intent("missing-leader-op", 1, 0, scratch.path())
                .expect("the test lease should be created");
        lease
            .bind_process(identity.clone())
            .expect("the leader identity should bind to the lease");
        leader
            .kill()
            .expect("the exact group leader should be terminated");
        leader
            .wait()
            .expect("the exact group leader should be reaped");

        let error = recover_validation_command(&lease)
            .expect_err("a leaderless surviving group must remain explicitly incomplete");
        assert!(error.contains("did not become absent"));
        assert!(scratch.path().exists());
        assert!(!process_group_is_absent(identity.process_group_id)
            .expect("the surviving group should be inspectable"));

        let _ = unsafe { libc::kill(-(identity.process_group_id as libc::pid_t), libc::SIGKILL) };
        wait_for_process_group_absence(identity.process_group_id)
            .expect("the test-owned descendant group should settle after cleanup");
        let _ = fs::remove_dir_all(scratch.path());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn malformed_boot_identity_cannot_authorize_scratch_cleanup() {
        let root = test_root("malformed-boot");
        let scratch = make_scratch(&root, "malformed-boot-op", 1, 0);
        let mut lease =
            ValidationCommandLease::scratch_intent("malformed-boot-op", 1, 0, scratch.path())
                .expect("the test lease should be created");
        lease.boot_id = "not-a-kernel-boot-id".to_string();

        assert!(recover_validation_command(&lease).is_err());
        assert!(scratch.path().exists());
        let _ = fs::remove_dir_all(scratch.path());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn different_boot_identity_never_signals_a_reused_numeric_process_group() {
        use std::os::unix::process::CommandExt;

        let root = test_root("different-boot");
        let scratch = make_scratch(&root, "different-boot-op", 1, 0);
        let mut command = Command::new("/bin/sleep");
        command.arg("30").stdin(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .expect("the foreign fixture process should start");
        let proc_identity = read_proc_identity(child.id())
            .expect("the fixture process identity should be readable without boot-file access");
        let prior_boot = "00000000-0000-0000-0000-000000000001";
        let current_boot = "00000000-0000-0000-0000-000000000002";
        let identity = ValidationProcessIdentity {
            boot_id: prior_boot.to_string(),
            pid: proc_identity.pid,
            process_group_id: proc_identity.process_group_id,
            session_id: proc_identity.session_id,
            start_time_ticks: proc_identity.start_time_ticks,
        };
        let mut lease = ValidationCommandLease::scratch_intent_for_boot(
            "different-boot-op",
            1,
            0,
            scratch.path(),
            prior_boot,
        )
        .expect("the test lease should be created");
        lease
            .bind_process(identity.clone())
            .expect("the test process should initially bind");
        lease.mark_output_pipe_unsettled();

        let recovery = recover_validation_command_for_boot(&lease, current_boot);
        let group_absent = process_group_is_absent(identity.process_group_id);
        let scratch_remains = scratch.path().exists();
        let _ = unsafe { libc::kill(-(identity.process_group_id as libc::pid_t), libc::SIGKILL) };
        let _ = child.wait();
        recovery
            .expect("a verified prior boot permits owner-checked cleanup despite uncertain pipes");
        assert_eq!(
            group_absent,
            Ok(false),
            "the modeled old PID must not be signalled"
        );
        assert!(!scratch_remains);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn same_boot_unsettled_output_remains_fail_closed() {
        let root = test_root("same-boot-output-unsettled");
        let scratch = make_scratch(&root, "same-boot-output-unsettled-op", 1, 0);
        let boot_id = "00000000-0000-0000-0000-000000000001";
        let mut lease = ValidationCommandLease::scratch_intent_for_boot(
            "same-boot-output-unsettled-op",
            1,
            0,
            scratch.path(),
            boot_id,
        )
        .expect("the modeled lease should be valid");
        lease.mark_output_pipe_unsettled();

        let error = recover_validation_command_for_boot(&lease, boot_id)
            .expect_err("same-boot uncertainty must remain nonretryable");
        assert!(error.contains("output remained open"));
        assert!(scratch.path().exists());
        let _ = fs::remove_dir_all(scratch.path());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scratch_intent_without_process_stays_incomplete_same_boot_and_cleans_after_boot_change() {
        let root = test_root("intent-boot-change");
        let scratch = make_scratch(&root, "intent-boot-change-op", 1, 0);
        let prior_boot = "00000000-0000-0000-0000-000000000011";
        let current_boot = "00000000-0000-0000-0000-000000000012";
        let lease = ValidationCommandLease::scratch_intent_for_boot(
            "intent-boot-change-op",
            1,
            0,
            scratch.path(),
            prior_boot,
        )
        .expect("the modeled scratch intent should be valid");

        assert!(recover_validation_command_for_boot(&lease, prior_boot).is_err());
        assert!(
            scratch.path().exists(),
            "same-boot missing process identity is not absence proof"
        );
        recover_validation_command_for_boot(&lease, current_boot)
            .expect("a verified reboot proves the gated child cannot remain");
        assert!(!scratch.path().exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prior_boot_output_uncertainty_still_requires_exact_marker_and_allowed_root() {
        let root = test_root("prior-boot-owner-check");
        let scratch = make_scratch(&root, "prior-boot-owner-check-op", 1, 0);
        let prior_boot = "00000000-0000-0000-0000-000000000021";
        let current_boot = "00000000-0000-0000-0000-000000000022";
        let mut lease = ValidationCommandLease::scratch_intent_for_boot(
            "prior-boot-owner-check-op",
            1,
            0,
            scratch.path(),
            prior_boot,
        )
        .expect("the modeled lease should be valid");
        lease.mark_output_pipe_unsettled();

        fs::write(scratch.path().join(OWNER_MARKER), b"foreign owner")
            .expect("the owner marker should be tampered");
        assert!(recover_validation_command_for_boot(&lease, current_boot).is_err());
        assert!(
            scratch.path().exists(),
            "cross-boot cleanup must preserve a foreign marker"
        );
        fs::write(
            scratch.path().join(OWNER_MARKER),
            lease.owner_token.as_bytes(),
        )
        .expect("the exact owner marker should be restored for the path check");

        let foreign_parent = root.join("outside-allowed-scratch-roots");
        let foreign_path = foreign_parent.join(
            lease
                .scratch_path
                .file_name()
                .expect("the owned scratch should have an operation-derived name"),
        );
        fs::create_dir_all(&foreign_path).expect("the outside scratch fixture should be created");
        fs::write(
            foreign_path.join(OWNER_MARKER),
            lease.owner_token.as_bytes(),
        )
        .expect("the outside fixture should carry the matching owner marker");
        lease.scratch_path = foreign_path.clone();
        assert!(recover_validation_command_for_boot(&lease, current_boot).is_err());
        assert!(
            foreign_path.exists(),
            "a matching marker cannot authorize an outside root"
        );
        assert!(
            scratch.path().exists(),
            "failed outside-path validation leaves original scratch intact"
        );

        let _ = fs::remove_dir_all(foreign_parent);
        let _ = fs::remove_dir_all(scratch.path());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn production_gate_persists_exact_process_identity_before_releasing_project_command() {
        use std::sync::mpsc;

        let root = test_root("start-gate");
        let (store, durable) = durable_setup_store(&root, "start-gate-op");
        let scratch = make_scratch(&root, "start-gate-op", 1, 0);
        let scratch_path = scratch.path().to_path_buf();
        let sentinel = root.join("command-ran");
        let (persisted_tx, persisted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let runner_store = store.clone();
        let runner_scratch = scratch;
        let workspace = root.join("workspace");
        let runner_root = root.clone();
        let runner = std::thread::spawn(move || {
            let mut environment = BTreeMap::new();
            environment.insert(
                "LEASE_TEST_SENTINEL".to_string(),
                runner_root
                    .join("command-ran")
                    .to_string_lossy()
                    .into_owned(),
            );
            super::super::project_environment_setup_validation::run_worker_validation_command_with_hook(
                "printf passed > \"$LEASE_TEST_SENTINEL\"",
                &workspace,
                &environment,
                &runner_scratch,
                &runner_store,
                "start-gate-op",
                1,
                0,
                || false,
                None,
                || {
                    persisted_tx.send(()).expect("the lease should be observable");
                    release_rx
                        .recv_timeout(Duration::from_secs(3))
                        .map_err(|error| error.to_string())
                },
            )
        });

        persisted_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("the process identity should be durable before command start");
        assert!(
            !sentinel.exists(),
            "user code must remain behind the process lease gate"
        );
        let events = durable
            .load_events_by_kind("project.environment_setup.updated")
            .expect("the setup journal should be readable");
        let lease = &events
            .last()
            .expect("the process lease event should be present")
            .payload["entry"]["validation_command_lease"];
        assert_eq!(lease["command_index"], 0);
        assert!(lease["process"].is_object());
        assert!(lease["scratch_path"].is_string());

        release_tx
            .send(())
            .expect("the start gate should be released");
        assert_eq!(
            runner
                .join()
                .expect("the production runner should finish")
                .expect("the project command should complete"),
            (0, 0, 0)
        );
        assert!(sentinel.exists());
        assert!(!scratch_path.exists());
        let final_event = durable
            .load_events_by_kind("project.environment_setup.updated")
            .expect("the settled setup journal should be readable")
            .pop()
            .expect("the settled lease event should exist");
        assert!(final_event.payload["entry"]["validation_command_lease"].is_null());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_durable_writer_never_releases_the_project_command_gate() {
        let root = test_root("intent-failure");
        let store = super::super::ProjectEnvironmentSetupStore::default();
        begin_validation(&store, "intent-failure-op");
        let scratch = make_scratch(&root, "intent-failure-op", 1, 0);
        let sentinel = root.join("command-ran");
        let mut environment = BTreeMap::new();
        environment.insert(
            "LEASE_TEST_SENTINEL".to_string(),
            sentinel.to_string_lossy().into_owned(),
        );
        let result = super::super::project_environment_setup_validation::run_worker_validation_command_with_hook(
            "printf passed > \"$LEASE_TEST_SENTINEL\"",
            &root.join("workspace"),
            &environment,
            &scratch,
            &store,
            "intent-failure-op",
            1,
            0,
            || false,
            None,
            || Ok(()),
        );
        assert!(result.is_err());
        assert!(!sentinel.exists());
        assert!(!scratch.path().exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_child_identity_persistence_kills_gated_child_and_cleans_exact_scratch() {
        let root = test_root("identity-write-failure");
        let (store, durable) = durable_setup_store(&root, "identity-write-failure-op");
        let scratch = make_scratch(&root, "identity-write-failure-op", 1, 0);
        let scratch_path = scratch.path().to_path_buf();
        let sentinel = root.join("command-ran");
        let mut environment = BTreeMap::new();
        environment.insert(
            "LEASE_TEST_SENTINEL".to_string(),
            sentinel.to_string_lossy().into_owned(),
        );
        let result = super::super::project_environment_setup_validation::run_worker_validation_command_with_persistence_hook(
            "printf passed > \"$LEASE_TEST_SENTINEL\"",
            &root.join("workspace"),
            &environment,
            &scratch,
            &store,
            "identity-write-failure-op",
            1,
            0,
            || false,
            None,
            |_store, _operation_id, _attempt, _command_index, _pid| {
                Err("injected process identity journal failure".to_string())
            },
            || Ok(()),
        );
        assert!(result.is_err());
        assert!(!sentinel.exists());
        assert!(!scratch_path.exists());
        let final_event = durable
            .load_events_by_kind("project.environment_setup.updated")
            .expect("the cleanup event should be readable")
            .pop()
            .expect("the intent should be cleared after the gated child was reaped");
        assert!(final_event.payload["entry"]["validation_command_lease"].is_null());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn detached_output_writer_hits_absolute_bound_and_retains_durable_cleanup_lease() {
        let root = test_root("detached-output-writer");
        let (store, durable) = durable_setup_store(&root, "detached-output-writer-op");
        let scratch = make_scratch(&root, "detached-output-writer-op", 1, 0);
        let scratch_path = scratch.path().to_path_buf();
        let fixture = DetachedOutputWriterFixture::new(&root, &scratch);
        let environment = fixture.environment();
        let started = Instant::now();
        let result = super::super::project_environment_setup_validation::run_worker_validation_command_with_output_timeout(
            DetachedOutputWriterFixture::command_text(),
            &root.join("workspace"),
            &environment,
            &scratch,
            &store,
            "detached-output-writer-op",
            1,
            0,
            || false,
            None,
            |store, operation_id, attempt, command_index, pid| {
                store.persist_validation_process_identity(
                    operation_id,
                    attempt,
                    command_index,
                    pid,
                )
            },
            || Ok(()),
            Duration::from_millis(40),
        );
        let error = result.expect_err("a detached writer must not hold validation indefinitely");
        assert!(
            error.contains("output pipes remained open"),
            "unexpected error: {error}"
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(
            scratch_path.exists(),
            "scratch remains while detached output may be active"
        );
        let writer_identity = fixture.live_identity().expect(
            "the parent must wait until the detached writer has escaped and retained stdout",
        );
        assert_eq!(writer_identity.process_group_id, writer_identity.pid);
        assert_eq!(writer_identity.session_id, writer_identity.pid);

        let event = durable
            .load_events_by_kind("project.environment_setup.updated")
            .expect("the output cleanup state should be readable")
            .pop()
            .expect("the output uncertainty event should be persisted");
        assert_eq!(
            event.payload["entry"]["validation_command_lease"]["output_pipe_unsettled"],
            true
        );
        assert_eq!(
            event.payload["entry"]["status"]["failure_code"],
            "validation_cleanup_incomplete"
        );
        assert_eq!(event.payload["entry"]["status"]["retryable"], false);

        let _restored =
            super::super::ProjectEnvironmentSetupStore::restore_from_durable_state(&durable);
        assert!(
            scratch_path.exists(),
            "restart must preserve scratch with uncertain output ownership"
        );
        let restored_event = durable
            .load_events_by_kind("project.environment_setup.updated")
            .expect("the restored cleanup state should be readable")
            .pop()
            .expect("restore should persist its conservative result");
        assert_eq!(
            restored_event.payload["entry"]["status"]["failure_code"],
            "validation_cleanup_incomplete"
        );
        assert_eq!(
            restored_event.payload["entry"]["validation_command_lease"]["output_pipe_unsettled"],
            true
        );

        fixture
            .cleanup()
            .expect("the exact detached writer should be reaped before owned scratch cleanup");
        assert!(
            !scratch_path.exists(),
            "the test-owned scratch should be removed after writer cleanup"
        );
    }

    struct DetachedOutputWriterFixture<'a> {
        root: &'a Path,
        scratch: &'a super::super::project_environment_setup_scratch::WorkerValidationScratch,
        identity_path: PathBuf,
    }

    impl<'a> DetachedOutputWriterFixture<'a> {
        const IDENTITY_ENV: &'static str = "CHARIOX_TEST_DETACHED_OUTPUT_IDENTITY";

        fn new(
            root: &'a Path,
            scratch: &'a super::super::project_environment_setup_scratch::WorkerValidationScratch,
        ) -> Self {
            Self {
                root,
                scratch,
                identity_path: root.join("detached-output-writer.identity"),
            }
        }

        fn command_text() -> &'static str {
            r#"
/usr/bin/setsid /bin/sh -c '
writer_pid=$$
proc_stat=$(/bin/cat "/proc/$writer_pid/stat") || exit 91
proc_rest=${proc_stat##*) }
set -- $proc_rest
[ "$#" -ge 20 ] || exit 92
writer_pgid=$3
writer_sid=$4
writer_start_time=${20}
[ "$writer_pid" = "$writer_pgid" ] || exit 93
[ "$writer_pid" = "$writer_sid" ] || exit 94
case "$writer_start_time" in ""|*[!0-9]*) exit 95 ;; esac
[ "$writer_start_time" -gt 0 ] || exit 96
[ -p "/proc/$writer_pid/fd/1" ] || exit 97
printf "%s %s %s %s\n" "$writer_pid" "$writer_pgid" "$writer_sid" "$writer_start_time" > "${CHARIOX_TEST_DETACHED_OUTPUT_IDENTITY}.tmp" || exit 98
/bin/mv "${CHARIOX_TEST_DETACHED_OUTPUT_IDENTITY}.tmp" "$CHARIOX_TEST_DETACHED_OUTPUT_IDENTITY" || exit 99
exec /bin/sleep 5
' &
attempt=0
while [ "$attempt" -lt 40 ]; do
  [ -s "$CHARIOX_TEST_DETACHED_OUTPUT_IDENTITY" ] && exit 0
  /bin/sleep 0.005
  attempt=$((attempt + 1))
done
[ -s "$CHARIOX_TEST_DETACHED_OUTPUT_IDENTITY" ] || exit 100
exit 0
"#
        }

        fn environment(&self) -> BTreeMap<String, String> {
            BTreeMap::from([(
                Self::IDENTITY_ENV.to_string(),
                self.identity_path.to_string_lossy().into_owned(),
            )])
        }

        fn recorded_identity(&self) -> Result<(u32, u32, u32, u64), String> {
            let recorded = fs::read_to_string(&self.identity_path)
                .map_err(|error| format!("detached writer identity could not be read: {error}"))?;
            let fields = recorded.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err("detached writer identity is malformed".to_string());
            }
            let pid = fields[0]
                .parse::<u32>()
                .map_err(|_| "detached writer PID is malformed".to_string())?;
            let process_group_id = fields[1]
                .parse::<u32>()
                .map_err(|_| "detached writer process group is malformed".to_string())?;
            let session_id = fields[2]
                .parse::<u32>()
                .map_err(|_| "detached writer session is malformed".to_string())?;
            let start_time_ticks = fields[3]
                .parse::<u64>()
                .map_err(|_| "detached writer start time is malformed".to_string())?;
            if pid <= 1
                || pid > i32::MAX as u32
                || process_group_id != pid
                || session_id != pid
                || start_time_ticks == 0
            {
                return Err("detached writer identity is not a new session leader".to_string());
            }
            Ok((pid, process_group_id, session_id, start_time_ticks))
        }

        fn live_identity(&self) -> Result<ValidationProcessIdentity, String> {
            let (pid, process_group_id, session_id, start_time_ticks) = self.recorded_identity()?;
            let current = read_proc_identity(pid)?;
            if current.process_group_id != process_group_id
                || current.session_id != session_id
                || current.start_time_ticks != start_time_ticks
                || matches!(current.state, 'Z' | 'X')
            {
                return Err("detached writer process identity changed before cleanup".to_string());
            }
            let stdout_target =
                fs::read_link(PathBuf::from(format!("/proc/{pid}/fd/1"))).map_err(|error| {
                    format!("detached writer stdout could not be inspected: {error}")
                })?;
            if !stdout_target.to_string_lossy().starts_with("pipe:[") {
                return Err(
                    "detached writer no longer holds the validation stdout pipe".to_string()
                );
            }
            Ok(ValidationProcessIdentity {
                boot_id: current_boot_id()?,
                pid,
                process_group_id,
                session_id,
                start_time_ticks,
            })
        }

        fn cleanup(&self) -> Result<(), String> {
            if !self.root.exists() {
                return Ok(());
            }
            if self.identity_path.exists() {
                let (_, process_group_id, _, _) = self.recorded_identity()?;
                if !process_group_is_absent(process_group_id)? {
                    let identity = self.live_identity()?;
                    settle_process_group(&identity)?;
                }
                if !process_group_is_absent(process_group_id)? {
                    return Err("detached writer process group remained after reap".to_string());
                }
            }
            if self.scratch.path().exists() {
                self.scratch.cleanup()?;
            }
            fs::remove_dir_all(self.root).map_err(|error| {
                format!("test-owned detached writer root could not be removed: {error}")
            })
        }
    }

    impl Drop for DetachedOutputWriterFixture<'_> {
        fn drop(&mut self) {
            let _ = self.cleanup();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn durable_restore_settles_live_group_and_persists_retryable_failed_status() {
        use std::os::unix::process::CommandExt;

        let root = test_root("restore-live-group");
        let (store, durable) = durable_setup_store(&root, "restore-live-group-op");
        let scratch = make_scratch(&root, "restore-live-group-op", 1, 0);
        let home_cache = root.join("home/.cache/project-tool/cache-entry");
        fs::create_dir_all(
            home_cache
                .parent()
                .expect("cache entry should have a parent"),
        )
        .expect("durable HOME cache should be created");
        fs::write(&home_cache, b"preserve durable cache")
            .expect("durable HOME cache should be populated");
        store
            .persist_validation_scratch_intent("restore-live-group-op", 1, 0, scratch.path())
            .expect("scratch intent should be durably recorded");
        let mut command = Command::new("/bin/sleep");
        command.arg("30").stdin(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command
            .spawn()
            .expect("the fixture validation process should start");
        store
            .persist_validation_process_identity("restore-live-group-op", 1, 0, child.id())
            .expect("the exact fixture process identity should be recorded");
        let reaper = std::thread::spawn(move || {
            let mut child = child;
            child.wait().expect("the restored child should be reaped")
        });

        let restored =
            super::super::ProjectEnvironmentSetupStore::restore_from_durable_state(&durable);
        let entries = restored
            .entries
            .lock()
            .expect("the restored setup entries should be available");
        let entry = entries
            .get("restore-live-group-op")
            .expect("the setup operation should restore");
        assert_eq!(
            entry.status.phase,
            super::super::ProjectEnvironmentSetupPhase::Failed
        );
        assert_eq!(
            entry.status.failure_code.as_deref(),
            Some("kernel_restarted")
        );
        assert!(entry.status.retryable);
        assert!(entry.validation_command_lease.is_none());
        drop(entries);
        let _ = reaper.join().expect("the process reaper should finish");
        assert!(!scratch.path().exists());
        assert_eq!(
            fs::read(&home_cache).expect("durable cache should remain"),
            b"preserve durable cache"
        );

        let event = durable
            .load_events_by_kind("project.environment_setup.updated")
            .expect("the restored failure event should be durable")
            .pop()
            .expect("the restart failure should have an event");
        let persisted = serde_json::from_value::<super::super::PersistedSetupEntry>(event.payload)
            .expect("the durable setup event should decode");
        assert_eq!(
            persisted.entry.status.phase,
            super::super::ProjectEnvironmentSetupPhase::Failed
        );
        assert_eq!(
            persisted.entry.status.failure_code.as_deref(),
            Some("kernel_restarted")
        );
        assert!(persisted.entry.validation_command_lease.is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn restore_with_intent_but_no_process_identity_preserves_scratch_and_marks_incomplete() {
        let root = test_root("intent-only-restore");
        let (store, durable) = durable_setup_store(&root, "intent-only-op");
        let scratch = make_scratch(&root, "intent-only-op", 1, 0);
        store
            .persist_validation_scratch_intent("intent-only-op", 1, 0, scratch.path())
            .expect("pre-spawn scratch intent should be persisted");

        let restored =
            super::super::ProjectEnvironmentSetupStore::restore_from_durable_state(&durable);
        let entries = restored
            .entries
            .lock()
            .expect("restored state should be readable");
        let entry = entries
            .get("intent-only-op")
            .expect("the operation should restore");
        assert_eq!(
            entry.status.phase,
            super::super::ProjectEnvironmentSetupPhase::Failed
        );
        assert_eq!(
            entry.status.failure_code.as_deref(),
            Some("validation_cleanup_incomplete")
        );
        assert!(!entry.status.retryable);
        assert!(entry.validation_command_lease.is_some());
        drop(entries);
        assert!(
            scratch.path().exists(),
            "missing identity is not proof of absence"
        );

        let event = durable
            .load_events_by_kind("project.environment_setup.updated")
            .expect("cleanup-incomplete failure should be persisted")
            .pop()
            .expect("restore should append a failure event");
        let persisted = serde_json::from_value::<super::super::PersistedSetupEntry>(event.payload)
            .expect("the restored event should decode");
        assert_eq!(
            persisted.entry.status.failure_code.as_deref(),
            Some("validation_cleanup_incomplete")
        );
        assert!(persisted.entry.validation_command_lease.is_some());
        let _ = fs::remove_dir_all(scratch.path());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_inflight_validation_without_a_lease_is_not_assumed_clean() {
        let root = test_root("legacy-validation");
        let (store, durable) = durable_setup_store(&root, "legacy-validation-op");
        assert!(store.update("legacy-validation-op", 1, |entry| {
            entry.validation_recovery_version = 0;
            entry.validation_command_lease = None;
        }));

        let restored =
            super::super::ProjectEnvironmentSetupStore::restore_from_durable_state(&durable);
        let entries = restored
            .entries
            .lock()
            .expect("restored legacy entries should be readable");
        let entry = entries
            .get("legacy-validation-op")
            .expect("the legacy setup operation should restore");
        assert_eq!(
            entry.status.phase,
            super::super::ProjectEnvironmentSetupPhase::Failed
        );
        assert_eq!(
            entry.status.failure_code.as_deref(),
            Some("validation_cleanup_incomplete")
        );
        assert!(!entry.status.retryable);
        drop(entries);
        let event = durable
            .load_events_by_kind("project.environment_setup.updated")
            .expect("the legacy incomplete state should persist")
            .pop()
            .expect("restore should append the incomplete status");
        let persisted = serde_json::from_value::<super::super::PersistedSetupEntry>(event.payload)
            .expect("the status event should decode");
        assert_eq!(
            persisted.entry.status.failure_code.as_deref(),
            Some("validation_cleanup_incomplete")
        );
        assert!(!persisted.entry.status.retryable);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_restart_records_unsupported_recovery_without_blocking_command_execution_contract()
    {
        let root = test_root("nonlinux-restart-recovery");
        let (_store, durable) = durable_setup_store(&root, "nonlinux-restart-recovery-op");
        let _restored =
            super::super::ProjectEnvironmentSetupStore::restore_from_durable_state(&durable);
        let event = durable
            .load_events_by_kind("project.environment_setup.updated")
            .expect("the unsupported recovery status should be readable")
            .pop()
            .expect("restore should persist the conservative status");
        assert_eq!(
            event.payload["entry"]["status"]["failure_code"],
            "validation_cleanup_incomplete"
        );
        assert_eq!(event.payload["entry"]["status"]["retryable"], false);
        assert!(event.payload["entry"]["status"]["failure_message"]
            .as_str()
            .is_some_and(|message| message.contains("unsupported on its originating platform")));
        let _ = fs::remove_dir_all(&root);
    }

    fn test_root(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "chariox-validation-recovery-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).expect("the test root should be created");
        path
    }

    fn make_scratch(
        root: &Path,
        operation_id: &str,
        attempt: u32,
        command_index: usize,
    ) -> super::super::project_environment_setup_scratch::WorkerValidationScratch {
        let workspace = root.join("workspace");
        let home = root.join("home");
        fs::create_dir_all(&workspace).expect("the test workspace should be created");
        fs::create_dir_all(&home).expect("the test HOME should be created");
        super::super::project_environment_setup_scratch::WorkerValidationScratch::create_for_worker(
            &workspace,
            &home,
            operation_id,
            attempt,
            command_index,
        )
        .expect("the operation scratch should be created")
    }

    fn durable_setup_store(
        root: &Path,
        operation_id: &str,
    ) -> (
        super::super::ProjectEnvironmentSetupStore,
        super::super::DurableKernelStateStore,
    ) {
        let durable = super::super::DurableKernelStateStore::open(root.join("state.sqlite"))
            .expect("the durable state store should open");
        let store =
            super::super::ProjectEnvironmentSetupStore::restore_from_durable_state(&durable);
        begin_validation(&store, operation_id);
        (store, durable)
    }

    fn begin_validation(store: &super::super::ProjectEnvironmentSetupStore, operation_id: &str) {
        let execution = super::super::SetupExecution {
            owner_user_id: "owner".to_string(),
            operation_id: operation_id.to_string(),
            project_id: "project".to_string(),
            session_id: "session".to_string(),
            agent_id: "agent".to_string(),
            execution_session_id: "session".to_string(),
            execution_agent_id: "agent".to_string(),
            workspace_id: "worktree".to_string(),
            target_worker_id: "worker".to_string(),
            target_platform: "linux".to_string(),
            definition: None,
            validation_commands: vec!["true".to_string()],
            persist_project_definition: false,
            remote_leased_agent_id: None,
        };
        store.begin(execution).expect("the test setup should begin");
        assert!(store.update(operation_id, 1, |entry| {
            entry.status.phase = super::super::ProjectEnvironmentSetupPhase::Validating;
        }));
    }
}
