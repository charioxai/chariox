use std::path::{Path, PathBuf};
use std::process::Command;

use crate::agent::GitWorktreePlacement;
use crate::error::DaemonError;

pub(super) fn prepare_remote_git_worktree(
    placement: &GitWorktreePlacement,
    target_hint: Option<&str>,
) -> Result<String, DaemonError> {
    let base_directory = std::env::current_dir().map_err(|error| DaemonError::LocalTransport {
        operation: "resolve remote git worktree base",
        message: error.to_string(),
    })?;
    let repo_root = run_remote_git(&base_directory, &["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(repo_root.trim());
    if repo_root.as_os_str().is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "create remote git worktree",
            message: format!(
                "git did not report a repository root for `{}`",
                base_directory.display()
            ),
        });
    }

    let from_ref = placement.from_ref.as_deref().unwrap_or("HEAD");
    let target_directory = placement
        .target_directory
        .as_deref()
        .or(target_hint)
        .map(|target| {
            let path = PathBuf::from(target);
            if path.is_absolute() {
                path
            } else {
                base_directory.join(path)
            }
        })
        .unwrap_or_else(|| {
            let slug = slugify_git_branch(placement.branch.as_deref().unwrap_or(from_ref));
            let repo_name = repo_root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("worktree");
            repo_root
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!("{repo_name}-{slug}"))
        });

    let target = target_directory.display().to_string();
    let args = if let Some(branch) = placement.branch.as_deref() {
        if remote_git_branch_exists(&repo_root, branch)? {
            vec![
                "worktree".to_string(),
                "add".to_string(),
                target.clone(),
                branch.to_string(),
            ]
        } else {
            vec![
                "worktree".to_string(),
                "add".to_string(),
                "-b".to_string(),
                branch.to_string(),
                target.clone(),
                from_ref.to_string(),
            ]
        }
    } else {
        vec![
            "worktree".to_string(),
            "add".to_string(),
            target.clone(),
            from_ref.to_string(),
        ]
    };
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_remote_git(&repo_root, &arg_refs)?;
    Ok(target)
}

fn remote_git_branch_exists(repo_root: &Path, branch: &str) -> Result<bool, DaemonError> {
    match run_remote_git(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    ) {
        Ok(_) => Ok(true),
        Err(error) => {
            if error.to_string().contains("git rev-parse") {
                Ok(false)
            } else {
                Err(error)
            }
        }
    }
}

fn run_remote_git(cwd: &Path, args: &[&str]) -> Result<String, DaemonError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "run remote git",
            message: format!(
                "git {} failed in `{}`: {error}",
                args.join(" "),
                cwd.display()
            ),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "run remote git",
            message: format!(
                "git {} failed in `{}`: {}",
                args.join(" "),
                cwd.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn slugify_git_branch(value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        "worktree".to_string()
    } else {
        slug
    }
}
