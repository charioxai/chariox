use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::error::DaemonError;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivePolicy {
    schema_version: u32,
    minimum_free_bytes: u64,
    progress_timeout_ms: u64,
}

fn policy() -> &'static ArchivePolicy {
    static POLICY: OnceLock<ArchivePolicy> = OnceLock::new();
    POLICY.get_or_init(|| {
        let policy: ArchivePolicy = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/slice-linux-docker/home-archive-policy.json"
        )))
        .expect("packaged home archive policy must be valid");
        assert_eq!(policy.schema_version, 1);
        assert!(policy.minimum_free_bytes <= 9_007_199_254_740_991);
        assert!(policy.progress_timeout_ms > 0 && policy.progress_timeout_ms <= 2_147_483_647);
        policy
    })
}

pub(super) fn minimum_free_bytes() -> u64 {
    policy().minimum_free_bytes
}

fn progress_timeout() -> Duration {
    #[cfg(test)]
    if let Some(timeout) = TEST_PROGRESS_TIMEOUT.with(std::cell::Cell::get) {
        return timeout;
    }
    Duration::from_millis(policy().progress_timeout_ms)
}

pub(super) fn capture(
    helper: &str,
    archive_path: &Path,
    operation: &'static str,
) -> Result<(PathBuf, u64, String), DaemonError> {
    let parent = archive_path.parent().unwrap_or_else(|| Path::new("."));
    capture_with_available_space(helper, archive_path, operation, || {
        fs2::available_space(parent)
    })
}

pub(super) fn capture_with_available_space(
    helper: &str,
    archive_path: &Path,
    operation: &'static str,
    available: impl FnMut() -> std::io::Result<u64>,
) -> Result<(PathBuf, u64, String), DaemonError> {
    capture_with_progress_timeout(
        helper,
        archive_path,
        operation,
        available,
        progress_timeout(),
    )
}

fn capture_with_progress_timeout(
    helper: &str,
    archive_path: &Path,
    operation: &'static str,
    mut available: impl FnMut() -> std::io::Result<u64>,
    progress_timeout: Duration,
) -> Result<(PathBuf, u64, String), DaemonError> {
    let error = |message: String| DaemonError::LocalTransport { operation, message };
    if super::broker::configured() {
        return Err(error(
            "managed home archives must use broker capture".into(),
        ));
    }
    let reserve = minimum_free_bytes();
    let space_error = || error("insufficient space for slice home archive".into());
    if available().map_err(|e| error(format!("failed to inspect archive storage: {e}")))? <= reserve
    {
        return Err(space_error());
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(archive_path)
        .map_err(|e| error(format!("failed to create private slice home archive: {e}")))?;
    let result = (|| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| error(format!("failed to protect slice home archive: {e}")))?;
        }
        // Direct tar output is streamed through a fixed buffer to private product
        // state. No login shell, helper-layer archive, or diagnostic byte capture.
        let mut command = Command::new("docker");
        command
            .args([
                "exec",
                "-u",
                "root",
                helper,
                "tar",
                "--zstd",
                "-C",
                "/home-src",
                "-cf",
                "-",
                ".",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        let mut process = crate::io::windows_pipe_process::Process::spawn(&mut command)
            .map_err(|e| error(format!("failed to own archive producer: {e}")))?;
        #[cfg(windows)]
        let child = &mut process.child;
        #[cfg(unix)]
        let mut child = command
            .spawn()
            .map_err(|e| error(format!("failed to stream slice home archive: {e}")))?;
        let mut producer_settled = false;
        let captured = (|| {
            let mut output = child
                .stdout
                .take()
                .ok_or_else(|| error("archive output pipe is unavailable".into()))?;
            #[cfg(unix)]
            nonblocking_pipe(&output)
                .map_err(|e| error(format!("failed to guard archive output: {e}")))?;
            let mut progress = Instant::now();
            let mut eof = false;
            let mut buffer = [0_u8; 64 * 1024];
            let mut hash = Sha256::new();
            let mut size = 0_u64;
            let status = loop {
                if progress.elapsed() >= progress_timeout {
                    return Err(error("slice home archive capture made no progress".into()));
                }
                let count = if eof {
                    0
                } else {
                    #[cfg(unix)]
                    let ready = buffer.len();
                    #[cfg(windows)]
                    let ready = match crate::io::windows_pipe_process::readiness(&output)
                        .map_err(|e| error(format!("failed to inspect archive output: {e}")))?
                    {
                        Some(ready) => ready.min(buffer.len()),
                        None => {
                            eof = true;
                            0
                        }
                    };
                    if ready == 0 {
                        0
                    } else {
                        match output.read(&mut buffer[..ready]) {
                            Ok(0) => {
                                eof = true;
                                0
                            }
                            Ok(count) => count,
                            Err(e)
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::WouldBlock
                                        | std::io::ErrorKind::Interrupted
                                ) =>
                            {
                                0
                            }
                            Err(e) => {
                                return Err(error(format!(
                                    "failed to read slice home archive: {e}"
                                )))
                            }
                        }
                    }
                };
                if count == 0 {
                    if eof {
                        if let Some(status) = child.try_wait().map_err(|e| {
                            error(format!("failed to settle slice archive process: {e}"))
                        })? {
                            producer_settled = true;
                            break status;
                        }
                    }
                    std::thread::sleep(
                        Duration::from_millis(10)
                            .min(progress_timeout.saturating_sub(progress.elapsed())),
                    );
                    continue;
                }
                if available()
                    .map_err(|e| error(format!("failed to inspect archive storage: {e}")))?
                    < reserve.saturating_add(count as u64)
                {
                    return Err(space_error());
                }
                size = size
                    .checked_add(count as u64)
                    .filter(|n| *n <= 9_007_199_254_740_991)
                    .ok_or_else(|| {
                        error("slice home archive size cannot be represented safely".into())
                    })?;
                file.write_all(&buffer[..count])
                    .map_err(|e| error(format!("failed to write slice home archive: {e}")))?;
                hash.update(&buffer[..count]);
                progress = Instant::now();
            };
            if !status.success() {
                return Err(error(format!("slice home archive failed with {status}")));
            }
            if size == 0 {
                return Err(error("slice home archive is empty".into()));
            }
            if available().map_err(|e| error(format!("failed to inspect archive storage: {e}")))?
                < reserve
            {
                return Err(space_error());
            }
            file.sync_all()
                .map_err(|e| error(format!("failed to sync slice home archive: {e}")))?;
            Ok((
                archive_path.to_path_buf(),
                size,
                format!("{:x}", hash.finalize()),
            ))
        })();
        if !producer_settled {
            #[cfg(unix)]
            {
                stop_producer(&mut child);
                let _ = child.wait();
            }
            #[cfg(windows)]
            {
                let _ = process.stop();
            }
        }
        captured
    })();
    if result.is_err() {
        remove_created_archive(&file, archive_path);
    }
    result
}

fn remove_created_archive(file: &File, path: &Path) {
    // Never remove an existing generation or a path replaced while capturing.
    let Ok(created) = file.metadata() else { return };
    let Ok(current) = std::fs::symlink_metadata(path) else {
        return;
    };
    if !current.is_file() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if created.dev() != current.dev() || created.ino() != current.ino() {
            return;
        }
    }
    #[cfg(not(unix))]
    if created.created().ok() != current.created().ok() || created.len() != current.len() {
        return;
    }
    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
fn nonblocking_pipe(output: &std::process::ChildStdout) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = output.as_raw_fd();
    // The child stdout descriptor remains owned/open for both fcntl calls.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn stop_producer(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // process_group(0) gives this child a group owned by this capture only.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = child.kill();
}

#[cfg(all(test, unix))]
mod progress_tests;

#[cfg(test)]
std::thread_local! {
    static TEST_PROGRESS_TIMEOUT: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn with_test_progress_timeout<T>(timeout: Duration, operation: impl FnOnce() -> T) -> T {
    struct Reset(Option<Duration>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PROGRESS_TIMEOUT.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_PROGRESS_TIMEOUT.with(|value| value.replace(Some(timeout))));
    operation()
}
