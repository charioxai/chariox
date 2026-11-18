//! Commit and push of a Room's worktree. Both stay inside the worktree's own
//! repository: a commit holds exactly the files named, and a push goes to the
//! `origin` GitHub repository on a branch that is not the default one, with the
//! owner's account credential given to git through its environment (never its
//! arguments). Opening the pull request is the GitHub App's action.
use crate::runtime::bounded_process::{BoundedCommandExt as _, Work};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use zeroize::Zeroizing;

use crate::error::DaemonError;
use crate::local::WorkspaceGitActionResult;
use crate::runtime::workspace_git_changes::workspace_git_status_by_path;
use crate::runtime::workspace_git_common::{
    detect_git_branch, git_command_output, git_ref_exists, resolve_repo_root,
    run_workspace_git_command,
};

const MAX_FILES: usize = 500;
const MAX_BRANCH_CHARS: usize = 200;

/// The branch a commit landed on and whether the commit made it.
pub(crate) struct CommitOutcome {
    pub(crate) result: WorkspaceGitActionResult,
    pub(crate) created_branch: bool,
}

/// What git needs to push to GitHub as an account: kept out of argument lists
/// and out of every message.
pub(crate) struct PushCredential(Zeroizing<String>);

impl PushCredential {
    /// GitHub accepts a token over HTTPS as Basic with any user name; the
    /// `Authorization: Bearer` form that suits its REST API is refused there.
    pub(crate) fn from_token(token: &str) -> Self {
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("x-access-token:{token}").as_bytes());
        Self(Zeroizing::new(basic))
    }

    fn environment(&self) -> Vec<(String, String)> {
        let pairs = [
            ("GIT_TERMINAL_PROMPT", "0".to_string()),
            ("GIT_CONFIG_COUNT", "2".to_string()),
            ("GIT_CONFIG_KEY_0", "credential.helper".to_string()),
            ("GIT_CONFIG_VALUE_0", String::new()),
            (
                "GIT_CONFIG_KEY_1",
                "http.https://github.com/.extraheader".to_string(),
            ),
            (
                "GIT_CONFIG_VALUE_1",
                format!("Authorization: Basic {}", self.0.as_str()),
            ),
        ];
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect()
    }
}

/// Commits exactly `files` (paths relative to the worktree, each one changed).
/// Work on the default branch or a detached HEAD moves to a new
/// `chariox/<slug>` branch first; a named `branch` is created or reused.
pub(crate) fn commit_workspace_changes(
    workspace_id: &str,
    worktree_id: &str,
    files: &[String],
    message: &str,
    branch: Option<&str>,
) -> Result<CommitOutcome, DaemonError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(fail("workspace git commit", "commit message is required"));
    }
    let worktree_path = worktree_id.trim();
    if worktree_path.is_empty() {
        return Err(fail("workspace git commit", "worktree_id is required"));
    }
    resolve_repo_root(worktree_path)?;
    let changes = workspace_git_status_by_path(worktree_path)?;
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(fail(
            "workspace git commit",
            "name between 1 and 500 changed files to commit",
        ));
    }
    for file in files {
        if !changes.contains_key(file) {
            return Err(fail(
                "workspace git commit",
                &format!("`{file}` is not a changed file of this worktree"),
            ));
        }
    }
    let target = target_branch(worktree_path, message, branch)?;
    if let Some(name) = &target {
        run_workspace_git_command(
            worktree_path,
            &["switch", "--create", name],
            "workspace git branch",
        )?;
    }
    let mut add = vec!["add", "-A", "--"];
    add.extend(files.iter().map(String::as_str));
    run_workspace_git_command(worktree_path, &add, "workspace git add")?;
    let mut commit = vec!["commit", "--only", "-m", message, "--"];
    commit.extend(files.iter().map(String::as_str));
    run_workspace_git_command(worktree_path, &commit, "workspace git commit")?;
    Ok(CommitOutcome {
        result: WorkspaceGitActionResult {
            workspace_id: workspace_id.to_string(),
            worktree_id: worktree_id.to_string(),
            action: "commit".to_string(),
            message: "committed workspace changes".to_string(),
            commit_sha: git_command_output(worktree_path, &["rev-parse", "--verify", "HEAD"]),
            branch: detect_git_branch(worktree_path).ok(),
            generated_at_ms: current_unix_ms(),
        },
        created_branch: target.is_some(),
    })
}

/// The branch to switch to before committing, or none when the current one
/// stays: a named branch other than the current one, or a fresh feature branch
/// when the worktree is on the default branch or detached.
fn target_branch(
    worktree_path: &str,
    message: &str,
    requested: Option<&str>,
) -> Result<Option<String>, DaemonError> {
    let current = detect_git_branch(worktree_path)?;
    let default = default_branch(worktree_path);
    if let Some(name) = requested.map(str::trim).filter(|name| !name.is_empty()) {
        if name == default
            || name.len() > MAX_BRANCH_CHARS
            || !valid_branch_name(worktree_path, name)
        {
            return Err(fail(
                "workspace git branch",
                &format!("`{name}` is not a usable feature branch name"),
            ));
        }
        return Ok((name != current).then(|| name.to_string()));
    }
    if current != "HEAD" && current != default {
        return Ok(None);
    }
    let slug = branch_slug(message);
    let repo = std::path::Path::new(worktree_path);
    (1..)
        .map(|n| match n {
            1 => format!("chariox/{slug}"),
            n => format!("chariox/{slug}-{n}"),
        })
        .find(|name| !git_ref_exists(repo, &format!("refs/heads/{name}")).unwrap_or(true))
        .map(Some)
        .ok_or_else(|| fail("workspace git branch", "no free branch name"))
}

fn valid_branch_name(worktree_path: &str, name: &str) -> bool {
    !name.starts_with('-')
        && git_command_output(worktree_path, &["check-ref-format", "--branch", name]).is_some()
}

fn branch_slug(message: &str) -> String {
    let subject = message.lines().next().unwrap_or_default().to_lowercase();
    let mut slug = String::new();
    for character in subject.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(40).collect();
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() { "changes" } else { slug }.to_string()
}

/// The repository's default branch: what `origin/HEAD` points at, else `main`
/// or `master` when one exists.
pub(crate) fn default_branch(worktree_path: &str) -> String {
    if let Some(head) = git_command_output(
        worktree_path,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    ) {
        return head.strip_prefix("origin/").unwrap_or(&head).to_string();
    }
    let repo = std::path::Path::new(worktree_path);
    ["main", "master"]
        .into_iter()
        .find(|name| {
            ["refs/heads/", "refs/remotes/origin/"]
                .iter()
                .any(|prefix| git_ref_exists(repo, &format!("{prefix}{name}")).unwrap_or(false))
        })
        .unwrap_or("main")
        .to_string()
}

/// `owner/repo` of a GitHub HTTPS remote; none for any other remote.
pub(crate) fn github_https_repository(url: &str) -> Option<String> {
    let path = url.trim().strip_prefix("https://github.com/")?;
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    let (owner, repo) = path.split_once('/')?;
    let part = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    (part(owner) && part(repo)).then(|| format!("{owner}/{repo}"))
}

/// Pushes the current branch to `origin`, which must be a GitHub HTTPS
/// remote. The default branch and a detached HEAD are never pushed, and the
/// push never forces.
pub(crate) fn push_workspace_branch(
    workspace_id: &str,
    worktree_id: &str,
    credential: &PushCredential,
) -> Result<WorkspaceGitActionResult, DaemonError> {
    let worktree_path = worktree_id.trim();
    if worktree_path.is_empty() {
        return Err(fail("workspace git push", "worktree_id is required"));
    }
    resolve_repo_root(worktree_path)?;
    let remote = git_command_output(worktree_path, &["remote", "get-url", "origin"])
        .ok_or_else(|| fail("workspace git push", "the worktree has no `origin` remote"))?;
    if github_https_repository(&remote).is_none() {
        return Err(fail(
            "workspace git push",
            "`origin` must be an https://github.com/<owner>/<repo> remote; set it with `git remote set-url origin`",
        ));
    }
    push_branch(
        workspace_id,
        worktree_id,
        &credential.environment(),
        &[credential.0.as_str()],
    )
}

fn push_branch(
    workspace_id: &str,
    worktree_id: &str,
    environment: &[(String, String)],
    secrets: &[&str],
) -> Result<WorkspaceGitActionResult, DaemonError> {
    let worktree_path = worktree_id.trim();
    let branch = detect_git_branch(worktree_path)?;
    if branch == "HEAD" {
        return Err(fail(
            "workspace git push",
            "a detached HEAD has no branch to push; commit to a branch first",
        ));
    }
    if branch == default_branch(worktree_path) {
        return Err(fail(
            "workspace git push",
            &format!("`{branch}` is the default branch; commit to a feature branch and push that"),
        ));
    }
    let output = crate::process_spawn::command("git")
        .args(["push", "--set-upstream", "origin", &branch])
        .envs(environment.iter().map(|(name, value)| (name, value)))
        .current_dir(worktree_path)
        // push runs hooks and talks to the network.
        .output_for(Work::Mutation)
        .map_err(|error| fail("workspace git push", &error.to_string()))?;
    if !output.status.success() {
        let mut message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        for secret in secrets {
            message = message.replace(secret, "[redacted]");
        }
        if message.is_empty() {
            message = format!("git push failed with status {}", output.status);
        }
        return Err(fail("workspace git push", &message));
    }
    Ok(WorkspaceGitActionResult {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        action: "push".to_string(),
        message: "pushed workspace branch".to_string(),
        commit_sha: git_command_output(worktree_path, &["rev-parse", "--verify", "HEAD"]),
        branch: Some(branch),
        generated_at_ms: current_unix_ms(),
    })
}

fn fail(operation: &'static str, message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: message.to_string(),
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
