use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

const DEFAULT_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunShellCommandRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub worktree_root: PathBuf,
    pub working_directory: Option<PathBuf>,
    pub timeout_ms: u64,
    #[serde(skip)]
    pub(crate) environment: crate::provider::ProviderCredentialEnvironment,
}

impl RunShellCommandRequest {
    pub fn new(
        session_id: impl Into<String>,
        attachment_id: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        worktree_root: PathBuf,
        working_directory: Option<PathBuf>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            attachment_id: attachment_id.into(),
            command: command.into(),
            args,
            worktree_root,
            working_directory,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            environment: Default::default(),
        }
    }

    pub(crate) fn with_environment(
        mut self,
        environment: crate::provider::ProviderCredentialEnvironment,
    ) -> Self {
        self.environment = environment;
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunShellCommandResult {
    pub session_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Default)]
pub struct ShellCommandService;

impl ShellCommandService {
    pub fn new() -> Self {
        Self
    }

    pub fn run(
        &self,
        request: RunShellCommandRequest,
    ) -> Result<RunShellCommandResult, DaemonError> {
        let working_directory = resolve_working_directory(&request)?;
        let mut command = Command::new(&request.command);
        command.args(&request.args);
        command.current_dir(&working_directory);
        for name in crate::provider::managed_provider_control_env_remove() {
            command.env_remove(name);
        }
        for (name, value) in request.environment.iter() {
            command.env(name, value);
        }
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let output = capture_output(command, Duration::from_millis(request.timeout_ms)).map_err(
            |error| match error {
                CaptureError::TimedOut => DaemonError::ShellCommandTimedOut {
                    session_id: request.session_id.clone(),
                    command: request.command.clone(),
                    timeout_ms: request.timeout_ms,
                },
                CaptureError::Io(error) => DaemonError::ShellCommandFailed {
                    session_id: request.session_id.clone(),
                    command: request.command.clone(),
                    message: error.to_string(),
                },
            },
        )?;

        Ok(RunShellCommandResult {
            session_id: request.session_id,
            command: request.command,
            args: request.args,
            working_directory: Some(working_directory),
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[derive(Debug)]
enum CaptureError {
    Io(io::Error),
    TimedOut,
}

impl From<io::Error> for CaptureError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(windows)]
use crate::io::windows_pipe_process as platform;

#[cfg(unix)]
mod platform {
    use std::io;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    pub(super) trait Pipe: AsRawFd {}
    impl<T: AsRawFd> Pipe for T {}

    pub(super) fn available(_pipe: &impl Pipe) -> io::Result<usize> {
        Ok(8192)
    }

    pub(super) struct Process {
        pub(super) child: Child,
    }

    impl Process {
        pub(super) fn spawn(command: &mut Command) -> io::Result<Self> {
            command.process_group(0);
            let process = Self {
                child: command.spawn()?,
            };
            for fd in [
                process.child.stdout.as_ref().unwrap().as_raw_fd(),
                process.child.stderr.as_ref().unwrap().as_raw_fd(),
            ] {
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags < 0
                    || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(process)
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            // MP-08/MP-10/MP-11: the child group and both pipe readers have
            // one lifetime on success, timeout, read error and wait error.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn capture_output(mut command: Command, timeout: Duration) -> Result<Output, CaptureError> {
    let mut process = platform::Process::spawn(&mut command)?;
    let started = Instant::now();
    let mut status = None;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        // Nonblocking, bounded read batches drain both pipes without detached
        // threads. A flooding stream cannot starve the other or the deadline.
        let stdout_closed = drain_output(process.child.stdout.as_mut().unwrap(), &mut stdout)?;
        let stderr_closed = drain_output(process.child.stderr.as_mut().unwrap(), &mut stderr)?;
        if status.is_none() {
            status = process.child.try_wait()?;
        }
        if let Some(status) = status.filter(|_| stdout_closed && stderr_closed) {
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        // The deadline also covers pipes retained after the launcher exits.
        if started.elapsed() >= timeout {
            return Err(CaptureError::TimedOut);
        }
        thread::sleep(
            timeout
                .saturating_sub(started.elapsed())
                .min(Duration::from_millis(5)),
        );
    }
}

fn drain_output(pipe: &mut (impl Read + platform::Pipe), bytes: &mut Vec<u8>) -> io::Result<bool> {
    let mut chunk = [0; 8192];
    for _ in 0..8 {
        let available = platform::available(pipe)?;
        if available == 0 {
            #[cfg(windows)]
            return Ok(platform::readiness(pipe)?.is_none());
            #[cfg(unix)]
            return Ok(false);
        }
        let len = available.min(chunk.len());
        match pipe.read(&mut chunk[..len]) {
            Ok(0) => return Ok(true),
            Ok(len) => bytes.extend_from_slice(&chunk[..len]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

fn resolve_working_directory(request: &RunShellCommandRequest) -> Result<PathBuf, DaemonError> {
    let worktree_root = canonicalize_existing_path(&request.worktree_root).map_err(|error| {
        DaemonError::ShellCommandFailed {
            session_id: request.session_id.clone(),
            command: request.command.clone(),
            message: error.to_string(),
        }
    })?;

    let requested = request
        .working_directory
        .clone()
        .unwrap_or_else(|| worktree_root.clone());
    let resolved = canonicalize_existing_path(&requested).map_err(|error| {
        DaemonError::ShellCommandFailed {
            session_id: request.session_id.clone(),
            command: request.command.clone(),
            message: error.to_string(),
        }
    })?;

    if !resolved.starts_with(&worktree_root) {
        return Err(DaemonError::ShellCommandOutsideWorktree {
            session_id: request.session_id.clone(),
            working_directory: resolved.display().to_string(),
            worktree_root: worktree_root.display().to_string(),
        });
    }

    Ok(resolved)
}

fn canonicalize_existing_path(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::DaemonError;

    use super::{RunShellCommandRequest, ShellCommandService};

    #[test]
    fn runs_shell_command_and_captures_output() {
        let service = ShellCommandService::new();
        let result = service
            .run(RunShellCommandRequest::new(
                "session-1",
                "attachment-1",
                "/bin/sh",
                vec!["-lc".to_string(), "printf hello".to_string()],
                std::env::current_dir().expect("cwd should exist"),
                None,
            ))
            .expect("shell command should run");

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hello");
        assert!(result.stderr.is_empty());
    }

    #[test]
    fn drains_large_stdout_and_stderr_before_waiting_for_exit() {
        let result = ShellCommandService::new()
            .run(
                RunShellCommandRequest::new(
                    "session-large-output",
                    "attachment-large-output",
                    "/bin/sh",
                    vec![
                        "-c".into(),
                        "head -c 262144 /dev/zero; head -c 262144 /dev/zero >&2".into(),
                    ],
                    std::env::current_dir().expect("cwd should exist"),
                    None,
                )
                .with_timeout_ms(2000),
            )
            .expect("output larger than pipe capacity must not time out");
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.as_bytes(), vec![0; 262144]);
        assert_eq!(result.stderr.as_bytes(), vec![0; 262144]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn timed_out_descendants_and_output_readers_settle() {
        const PROBE: &str = "CHARIOX_SHELL_LIFETIME_TEST";
        if std::env::var_os(PROBE).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "capability::shell::tests::timed_out_descendants_and_output_readers_settle",
                    "--nocapture",
                ])
                .env(PROBE, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        // Isolate /proc counts from other tests and always remove the fixture.
        let root =
            std::env::temp_dir().join(format!("chariox-shell-lifetime-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                if let Ok(pid) = fs::read_to_string(self.0.join("producer.pid")) {
                    if let Ok(pid) = pid.trim().parse::<i32>() {
                        unsafe {
                            libc::kill(pid, libc::SIGKILL);
                        }
                    }
                }
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let fixture = Fixture(root.clone());
        let count = |path| fs::read_dir(path).unwrap().count();
        let threads = count("/proc/self/task");
        let descriptors = count("/proc/self/fd");
        for (producer, wait) in [
            ("while :; do printf output; sleep 0.01; done", "wait"),
            ("while :; do printf error >&2; sleep 0.01; done", "wait"),
            ("exec sleep 30", "wait"),
            ("exec sleep 30", "exit 0"),
        ] {
            let command = format!("sh -c 'echo $$ > producer.pid; {producer}' & {wait}");
            let started = std::time::Instant::now();
            let error = ShellCommandService::new()
                .run(
                    RunShellCommandRequest::new(
                        "timeout-descendant",
                        "attachment",
                        "/bin/sh",
                        vec!["-c".into(), command],
                        root.clone(),
                        None,
                    )
                    .with_timeout_ms(100),
                )
                .expect_err("descendant should time out");
            assert!(matches!(error, DaemonError::ShellCommandTimedOut { .. }));
            assert!(started.elapsed() < std::time::Duration::from_secs(2));
            let pid: i32 = fs::read_to_string(root.join("producer.pid"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let running = || {
                fs::read_to_string(format!("/proc/{pid}/stat"))
                    .is_ok_and(|stat| !stat.split_once(") ").unwrap().1.starts_with('Z'))
            };
            for _ in 0..50 {
                if !running() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let readers_settled =
                count("/proc/self/task") == threads && count("/proc/self/fd") == descriptors;
            assert!(
                !running(),
                "timed-out descendant is still running: {producer}"
            );
            assert!(
                readers_settled,
                "timed-out readers retained threads or pipe descriptors"
            );
            let next = ShellCommandService::new()
                .run(RunShellCommandRequest::new(
                    "after-timeout",
                    "attachment",
                    "/bin/sh",
                    vec!["-c".into(), "printf usable".into()],
                    root.clone(),
                    None,
                ))
                .unwrap();
            assert_eq!(next.stdout, "usable");
            fs::remove_file(root.join("producer.pid")).unwrap();
        }
        drop(fixture);
    }

    #[test]
    fn shell_command_does_not_inherit_managed_control_environment() {
        let _guard = crate::env_lock::lock();
        std::env::set_var(
            "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
            "/var/lib/chariox/receipt",
        );
        let result = ShellCommandService::new()
            .run(RunShellCommandRequest::new(
                "session-1",
                "attachment-1",
                "/bin/sh",
                vec![
                    "-c".to_string(),
                    "printf %s \"${CHARIOX_DISPOSABLE_WORKER_RECEIPT-unset}\"".to_string(),
                ],
                std::env::current_dir().expect("cwd should exist"),
                None,
            ))
            .expect("shell command should run");
        std::env::remove_var("CHARIOX_DISPOSABLE_WORKER_RECEIPT");

        assert_eq!(result.stdout, "unset");
    }

    #[test]
    fn runs_shell_command_in_requested_directory() {
        let service = ShellCommandService::new();
        let temp_dir = std::env::temp_dir().join("chariox-shell-capability-test");
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");

        let result = service
            .run(RunShellCommandRequest::new(
                "session-1",
                "attachment-1",
                "/bin/sh",
                vec!["-lc".to_string(), "pwd".to_string()],
                temp_dir.clone(),
                Some(temp_dir.clone()),
            ))
            .expect("shell command should run in requested directory");

        assert_eq!(result.exit_code, 0);
        assert!(result
            .stdout
            .trim_end()
            .ends_with("chariox-shell-capability-test"));
    }

    #[test]
    fn captures_non_zero_exit_status() {
        let service = ShellCommandService::new();
        let result = service
            .run(RunShellCommandRequest::new(
                "session-1",
                "attachment-1",
                "/bin/sh",
                vec!["-lc".to_string(), "printf error >&2; exit 7".to_string()],
                std::env::current_dir().expect("cwd should exist"),
                None,
            ))
            .expect("shell command should still return structured result");

        assert_eq!(result.exit_code, 7);
        assert_eq!(result.stderr, "error");
    }

    #[test]
    fn rejects_working_directory_outside_worktree() {
        let service = ShellCommandService::new();
        let worktree_root = std::env::temp_dir().join("chariox-shell-worktree-root");
        let outside_dir = std::env::temp_dir();
        fs::create_dir_all(&worktree_root).expect("worktree dir should exist");

        let error = service
            .run(RunShellCommandRequest::new(
                "session-1",
                "attachment-1",
                "/bin/sh",
                vec!["-lc".to_string(), "pwd".to_string()],
                worktree_root,
                Some(outside_dir),
            ))
            .expect_err("outside working directory should be rejected");

        match error {
            DaemonError::ShellCommandOutsideWorktree { .. } => {}
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn times_out_long_running_command() {
        let service = ShellCommandService::new();

        let error = service
            .run(
                RunShellCommandRequest::new(
                    "session-1",
                    "attachment-1",
                    "/bin/sh",
                    vec!["-lc".to_string(), "sleep 1".to_string()],
                    std::env::current_dir().expect("cwd should exist"),
                    None,
                )
                .with_timeout_ms(10),
            )
            .expect_err("long command should time out");

        match error {
            DaemonError::ShellCommandTimedOut { .. } => {}
            other => panic!("unexpected error: {other}"),
        }
    }
}
