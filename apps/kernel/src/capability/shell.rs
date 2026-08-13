use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;

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
        }
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
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|error| DaemonError::ShellCommandFailed {
                session_id: request.session_id.clone(),
                command: request.command.clone(),
                message: error.to_string(),
            })?;

        let timeout = Duration::from_millis(request.timeout_ms);
        let status =
            child
                .wait_timeout(timeout)
                .map_err(|error| DaemonError::ShellCommandFailed {
                    session_id: request.session_id.clone(),
                    command: request.command.clone(),
                    message: error.to_string(),
                })?;

        let status = match status {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(DaemonError::ShellCommandTimedOut {
                    session_id: request.session_id,
                    command: request.command,
                    timeout_ms: request.timeout_ms,
                });
            }
        };

        let output = child
            .wait_with_output()
            .map_err(|error| DaemonError::ShellCommandFailed {
                session_id: request.session_id.clone(),
                command: request.command.clone(),
                message: error.to_string(),
            })?;

        Ok(RunShellCommandResult {
            session_id: request.session_id,
            command: request.command,
            args: request.args,
            working_directory: Some(working_directory),
            exit_code: status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
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
