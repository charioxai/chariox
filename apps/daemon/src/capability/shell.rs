use std::path::PathBuf;
use std::process::Command;

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunShellCommandRequest {
    pub session_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
}

impl RunShellCommandRequest {
    pub fn new(
        session_id: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        working_directory: Option<PathBuf>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            command: command.into(),
            args,
            working_directory,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
        let mut command = Command::new(&request.command);
        command.args(&request.args);
        if let Some(working_directory) = request.working_directory.as_ref() {
            command.current_dir(working_directory);
        }

        let output = command
            .output()
            .map_err(|error| DaemonError::ShellCommandFailed {
                session_id: request.session_id.clone(),
                command: request.command.clone(),
                message: error.to_string(),
            })?;

        Ok(RunShellCommandResult {
            session_id: request.session_id,
            command: request.command,
            args: request.args,
            working_directory: request.working_directory,
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{RunShellCommandRequest, ShellCommandService};

    #[test]
    fn runs_shell_command_and_captures_output() {
        let service = ShellCommandService::new();
        let result = service
            .run(RunShellCommandRequest::new(
                "session-1",
                "/bin/sh",
                vec!["-lc".to_string(), "printf hello".to_string()],
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
        let temp_dir = std::env::temp_dir().join("arroba-shell-capability-test");
        fs::create_dir_all(&temp_dir).expect("temp dir should exist");

        let result = service
            .run(RunShellCommandRequest::new(
                "session-1",
                "/bin/sh",
                vec!["-lc".to_string(), "pwd".to_string()],
                Some(temp_dir.clone()),
            ))
            .expect("shell command should run in requested directory");

        assert_eq!(result.exit_code, 0);
        assert!(result
            .stdout
            .trim_end()
            .ends_with("arroba-shell-capability-test"));
    }

    #[test]
    fn captures_non_zero_exit_status() {
        let service = ShellCommandService::new();
        let result = service
            .run(RunShellCommandRequest::new(
                "session-1",
                "/bin/sh",
                vec!["-lc".to_string(), "printf error >&2; exit 7".to_string()],
                None,
            ))
            .expect("shell command should still return structured result");

        assert_eq!(result.exit_code, 7);
        assert_eq!(result.stderr, "error");
    }
}
