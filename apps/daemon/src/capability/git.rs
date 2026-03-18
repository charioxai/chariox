use std::path::PathBuf;
use std::process::Command;

use crate::error::DaemonError;

use super::common::resolve_worktree_scoped_path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectGitRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub worktree_root: PathBuf,
    pub working_directory: Option<PathBuf>,
}

impl InspectGitRequest {
    pub fn new(
        session_id: impl Into<String>,
        attachment_id: impl Into<String>,
        worktree_root: PathBuf,
        working_directory: Option<PathBuf>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            attachment_id: attachment_id.into(),
            worktree_root,
            working_directory,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectGitResult {
    pub session_id: String,
    pub working_directory: PathBuf,
    pub branch: String,
    pub status: String,
}

#[derive(Debug, Clone, Default)]
pub struct GitCapabilityService;

impl GitCapabilityService {
    pub fn new() -> Self {
        Self
    }

    pub fn inspect(&self, request: InspectGitRequest) -> Result<InspectGitResult, DaemonError> {
        let working_directory = resolve_worktree_scoped_path(
            &request.session_id,
            &request.worktree_root,
            request.working_directory.as_deref(),
        )?;

        let branch = run_git(
            &request.session_id,
            &working_directory,
            &["symbolic-ref", "--short", "HEAD"],
        )?;
        let status = run_git(
            &request.session_id,
            &working_directory,
            &["status", "--short", "--branch"],
        )?;

        Ok(InspectGitResult {
            session_id: request.session_id,
            working_directory,
            branch: branch.trim().to_string(),
            status,
        })
    }
}

fn run_git(
    session_id: &str,
    working_directory: &PathBuf,
    args: &[&str],
) -> Result<String, DaemonError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(working_directory)
        .output()
        .map_err(|error| DaemonError::GitCapabilityFailed {
            session_id: session_id.to_string(),
            working_directory: working_directory.display().to_string(),
            message: error.to_string(),
        })?;

    if !output.status.success() {
        return Err(DaemonError::GitCapabilityFailed {
            session_id: session_id.to_string(),
            working_directory: working_directory.display().to_string(),
            message: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use super::{GitCapabilityService, InspectGitRequest};

    #[test]
    fn inspects_git_status_inside_repo() {
        let root = std::env::temp_dir().join("arroba-git-capability-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root should exist");
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&root)
            .output()
            .expect("git init should work");
        fs::write(root.join("README.md"), "hello").expect("file should exist");

        let result = GitCapabilityService::new()
            .inspect(InspectGitRequest::new(
                "session-1",
                "attachment-1",
                root,
                None,
            ))
            .expect("git inspection should succeed");

        assert_eq!(result.branch, "main");
        assert!(result.status.contains("main"));
    }
}
