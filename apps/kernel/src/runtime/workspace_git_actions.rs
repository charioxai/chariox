use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::DaemonError;
use crate::local::{WorkspaceGitActionResult, WorkspacePullRequestRecord};
use crate::runtime::workspace_git_changes::workspace_git_status_by_path;
use crate::runtime::workspace_git_common::{
    detect_git_branch, git_command_output, resolve_repo_root, run_workspace_git_command,
    workspace_default_compare_ref,
};

pub(crate) fn commit_workspace_changes(
    workspace_id: &str,
    worktree_id: &str,
    message: &str,
) -> Result<WorkspaceGitActionResult, DaemonError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace git commit",
            message: "commit message is required".to_string(),
        });
    }
    let worktree_path = worktree_id.trim();
    if worktree_path.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace git commit",
            message: "worktree_id is required".to_string(),
        });
    }
    let _repo_root = resolve_repo_root(worktree_path)?;
    let changes = workspace_git_status_by_path(worktree_path)?;
    if changes.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace git commit",
            message: "no workspace changes to commit".to_string(),
        });
    }
    run_workspace_git_command(worktree_path, &["add", "-A"], "workspace git add")?;
    run_workspace_git_command(
        worktree_path,
        &["commit", "-m", message],
        "workspace git commit",
    )?;
    let commit_sha = git_command_output(worktree_path, &["rev-parse", "--verify", "HEAD"]);
    Ok(WorkspaceGitActionResult {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        action: "commit".to_string(),
        message: "committed workspace changes".to_string(),
        commit_sha,
        branch: detect_git_branch(worktree_path).ok(),
        generated_at_ms: current_unix_ms(),
    })
}

pub(crate) fn push_workspace_branch(
    workspace_id: &str,
    worktree_id: &str,
    force_with_lease: bool,
) -> Result<WorkspaceGitActionResult, DaemonError> {
    let worktree_path = worktree_id.trim();
    if worktree_path.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace git push",
            message: "worktree_id is required".to_string(),
        });
    }
    let _repo_root = resolve_repo_root(worktree_path)?;
    let branch = detect_git_branch(worktree_path).ok();
    let upstream = git_command_output(
        worktree_path,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );
    if upstream.is_none() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace git push",
            message: "current branch has no upstream; push target is ambiguous".to_string(),
        });
    }
    let args = if force_with_lease {
        vec!["push", "--force-with-lease"]
    } else {
        vec!["push"]
    };
    run_workspace_git_command(worktree_path, &args, "workspace git push")?;
    Ok(WorkspaceGitActionResult {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        action: if force_with_lease {
            "force_push".to_string()
        } else {
            "push".to_string()
        },
        message: "pushed workspace branch".to_string(),
        commit_sha: git_command_output(worktree_path, &["rev-parse", "--verify", "HEAD"]),
        branch,
        generated_at_ms: current_unix_ms(),
    })
}

pub(crate) fn commit_and_push_workspace_changes(
    workspace_id: &str,
    worktree_id: &str,
    message: &str,
) -> Result<WorkspaceGitActionResult, DaemonError> {
    let commit_result = commit_workspace_changes(workspace_id, worktree_id, message)?;
    let push_result = push_workspace_branch(workspace_id, worktree_id, false)?;
    Ok(workspace_commit_and_push_result(
        workspace_id,
        worktree_id,
        commit_result,
        push_result,
    ))
}

pub(crate) fn create_workspace_pull_request(
    workspace_id: &str,
    worktree_id: &str,
    title: Option<&str>,
    body: Option<&str>,
    base_ref: Option<&str>,
    draft: bool,
) -> Result<WorkspacePullRequestRecord, DaemonError> {
    let worktree_path = worktree_id.trim();
    if worktree_path.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace pull request create",
            message: "worktree_id is required".to_string(),
        });
    }
    let repo_root = resolve_repo_root(worktree_path)?;
    let branch = detect_git_branch(worktree_path)?;
    if branch == "HEAD" || branch.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace pull request create",
            message: "cannot create a pull request from a detached HEAD".to_string(),
        });
    }
    ensure_workspace_branch_pushed(worktree_path, &branch)?;
    let base_ref = base_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_pull_request_base_ref)
        .unwrap_or_else(|| {
            normalize_pull_request_base_ref(&workspace_default_compare_ref(
                repo_root.to_string_lossy().as_ref(),
                Some(&branch),
            ))
        });
    let title = title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| pull_request_title_from_branch(&branch));
    let mut args = vec![
        "pr".to_string(),
        "create".to_string(),
        "--head".to_string(),
        branch.clone(),
        "--base".to_string(),
        base_ref.clone(),
        "--title".to_string(),
        title.clone(),
    ];
    if let Some(body) = body.map(str::trim).filter(|value| !value.is_empty()) {
        args.push("--body".to_string());
        args.push(body.to_string());
    } else {
        args.push("--fill".to_string());
    }
    if draft {
        args.push("--draft".to_string());
    }
    let url = run_gh_output(worktree_path, &args, "workspace pull request create")?;
    Ok(WorkspacePullRequestRecord {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        branch,
        base_ref,
        url,
        title: Some(title),
        draft,
        generated_at_ms: current_unix_ms(),
    })
}

fn workspace_commit_and_push_result(
    workspace_id: &str,
    worktree_id: &str,
    commit_result: WorkspaceGitActionResult,
    push_result: WorkspaceGitActionResult,
) -> WorkspaceGitActionResult {
    WorkspaceGitActionResult {
        action: "commit_and_push".to_string(),
        message: format!("{}; {}", commit_result.message, push_result.message),
        commit_sha: commit_result.commit_sha,
        branch: push_result.branch.or(commit_result.branch),
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        generated_at_ms: current_unix_ms(),
    }
}

fn ensure_workspace_branch_pushed(worktree_path: &str, branch: &str) -> Result<(), DaemonError> {
    let upstream = git_command_output(
        worktree_path,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );
    if upstream.is_some() {
        return Ok(());
    }
    run_workspace_git_command(
        worktree_path,
        &["push", "-u", "origin", branch],
        "workspace pull request push",
    )
}

fn normalize_pull_request_base_ref(reference: &str) -> String {
    let mut normalized = reference.trim();
    if let Some(stripped) = normalized.strip_prefix("refs/remotes/origin/") {
        normalized = stripped;
    }
    if let Some(stripped) = normalized.strip_prefix("refs/heads/") {
        normalized = stripped;
    }
    if let Some(stripped) = normalized.strip_prefix("origin/") {
        normalized = stripped;
    }
    normalized.to_string()
}

fn pull_request_title_from_branch(branch: &str) -> String {
    let title = branch
        .rsplit('/')
        .next()
        .unwrap_or(branch)
        .replace(['-', '_'], " ");
    let title = title.trim();
    if title.is_empty() {
        branch.to_string()
    } else {
        title.to_string()
    }
}

fn run_gh_output(
    worktree_path: &str,
    args: &[String],
    operation: &'static str,
) -> Result<String, DaemonError> {
    let output = std::process::Command::new("gh")
        .args(args)
        .current_dir(worktree_path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: error.to_string(),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("gh {} failed with status {}", args.join(" "), output.status)
        };
        return Err(DaemonError::LocalTransport { operation, message });
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation,
            message: "gh did not return a pull request URL".to_string(),
        });
    }
    Ok(stdout
        .lines()
        .last()
        .unwrap_or(stdout.as_str())
        .trim()
        .to_string())
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_pull_request_base_ref, pull_request_title_from_branch,
        workspace_commit_and_push_result,
    };
    use crate::local::WorkspaceGitActionResult;

    #[test]
    fn pull_request_base_refs_normalize_hosted_git_refs() {
        assert_eq!(
            normalize_pull_request_base_ref("refs/remotes/origin/main"),
            "main"
        );
        assert_eq!(normalize_pull_request_base_ref("refs/heads/dev"), "dev");
        assert_eq!(normalize_pull_request_base_ref("origin/release"), "release");
        assert_eq!(normalize_pull_request_base_ref("feature"), "feature");
    }

    #[test]
    fn pull_request_titles_come_from_branch_leaf() {
        assert_eq!(
            pull_request_title_from_branch("miguel/add-worktree-picker"),
            "add worktree picker"
        );
        assert_eq!(pull_request_title_from_branch("bug_fix"), "bug fix");
        assert_eq!(pull_request_title_from_branch("/"), "/");
    }

    #[test]
    fn commit_and_push_result_preserves_commit_sha_and_push_branch() {
        let result = workspace_commit_and_push_result(
            "/repo",
            "/repo/worktree",
            WorkspaceGitActionResult {
                workspace_id: "/repo".to_string(),
                worktree_id: "/repo/worktree".to_string(),
                action: "commit".to_string(),
                message: "committed workspace changes".to_string(),
                commit_sha: Some("abc123".to_string()),
                branch: Some("feature".to_string()),
                generated_at_ms: 1,
            },
            WorkspaceGitActionResult {
                workspace_id: "/repo".to_string(),
                worktree_id: "/repo/worktree".to_string(),
                action: "push".to_string(),
                message: "pushed workspace branch".to_string(),
                commit_sha: Some("def456".to_string()),
                branch: Some("origin/feature".to_string()),
                generated_at_ms: 2,
            },
        );

        assert_eq!(result.workspace_id, "/repo");
        assert_eq!(result.worktree_id, "/repo/worktree");
        assert_eq!(result.action, "commit_and_push");
        assert_eq!(
            result.message,
            "committed workspace changes; pushed workspace branch"
        );
        assert_eq!(result.commit_sha.as_deref(), Some("abc123"));
        assert_eq!(result.branch.as_deref(), Some("origin/feature"));
    }
}
