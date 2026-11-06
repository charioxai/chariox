use crate::runtime::bounded_process::{BoundedCommandExt as _, Work};
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
    let output = crate::process_spawn::command("gh")
        .args(args)
        .current_dir(worktree_path)
        .output_for(Work::Mutation)
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

    #[cfg(unix)]
    mod slow_hooks {
        use super::super::{commit_workspace_changes, push_workspace_branch};
        use crate::runtime::bounded_process::{with_test_command_limit, with_test_mutation_limit};
        use std::os::unix::fs::PermissionsExt as _;
        use std::path::{Path, PathBuf};
        use std::time::{Duration, Instant};

        const QUERY_CAP: Duration = Duration::from_secs(1);

        fn git(cwd: &Path, args: &[&str]) {
            let output = crate::process_spawn::command("git")
                .args(args)
                .current_dir(cwd)
                .output()
                .expect("git should run");
            assert!(
                output.status.success(),
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn install_hook(repo_git_dir: &Path, name: &str, body: &str) {
            let hook = repo_git_dir.join("hooks").join(name);
            std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
            std::fs::write(&hook, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        struct Fixture {
            root: PathBuf,
            clone: PathBuf,
            remote: PathBuf,
        }

        impl Fixture {
            fn new(label: &str) -> Self {
                let root = std::env::temp_dir().join(format!(
                    "chariox-git-{label}-{}-{}",
                    std::process::id(),
                    crate::session::unix_epoch_ms()
                ));
                let remote = root.join("remote.git");
                let clone = root.join("clone");
                std::fs::create_dir_all(&root).unwrap();
                git(&root, &["init", "--bare", "-b", "main", "remote.git"]);
                git(&root, &["clone", "remote.git", "clone"]);
                git(&clone, &["config", "user.email", "tests@example.invalid"]);
                git(&clone, &["config", "user.name", "Chariox Tests"]);
                std::fs::write(clone.join("README.md"), "seed\n").unwrap();
                git(&clone, &["add", "."]);
                git(&clone, &["commit", "-m", "seed"]);
                git(&clone, &["push", "-u", "origin", "HEAD:main"]);
                git(&clone, &["branch", "--set-upstream-to=origin/main"]);
                std::fs::write(clone.join("change.txt"), "change\n").unwrap();
                Self {
                    root,
                    clone,
                    remote,
                }
            }

            fn path(&self) -> &str {
                self.clone.to_str().unwrap()
            }
        }

        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }

        #[test]
        fn commit_with_a_hook_slower_than_the_query_cap_succeeds() {
            let fixture = Fixture::new("slow-commit");
            install_hook(&fixture.clone.join(".git"), "pre-commit", "sleep 3");
            let started = Instant::now();
            let result = with_test_command_limit(QUERY_CAP, || {
                commit_workspace_changes("w", fixture.path(), "slow hook")
            });
            assert!(result.is_ok(), "commit failed: {:?}", result.err());
            assert!(started.elapsed() >= Duration::from_secs(3));
        }

        #[test]
        fn push_with_a_remote_hook_slower_than_the_query_cap_succeeds() {
            let fixture = Fixture::new("slow-push");
            git(&fixture.clone, &["add", "."]);
            git(&fixture.clone, &["commit", "-m", "to push"]);
            install_hook(&fixture.remote, "pre-receive", "sleep 3");
            let result = with_test_command_limit(QUERY_CAP, || {
                push_workspace_branch("w", fixture.path(), false)
            });
            assert!(result.is_ok(), "push failed: {:?}", result.err());
        }

        fn process_is_alive(pid: i32) -> bool {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|stat| !stat.contains(") Z "))
                .unwrap_or(false)
        }

        #[test]
        fn hung_commit_hook_ends_at_the_mutation_deadline_with_its_tree_reaped() {
            let fixture = Fixture::new("hung-commit");
            let pid_file = fixture.root.join("hook.pid");
            install_hook(
                &fixture.clone.join(".git"),
                "pre-commit",
                &format!("sleep 300 &\necho $! > '{}'\nwait", pid_file.display()),
            );
            let started = Instant::now();
            let error = with_test_mutation_limit(Duration::from_secs(2), || {
                commit_workspace_changes("w", fixture.path(), "hung hook")
            })
            .expect_err("a hung hook must end at the mutation deadline");
            assert!(started.elapsed() < Duration::from_secs(20), "{error}");
            let pid: i32 = std::fs::read_to_string(&pid_file)
                .expect("hook should have started its child")
                .trim()
                .parse()
                .unwrap();
            let reaped_by = Instant::now() + Duration::from_secs(5);
            while process_is_alive(pid) && Instant::now() < reaped_by {
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(
                !process_is_alive(pid),
                "hook child {pid} outlived the deadline"
            );
        }
    }
}
