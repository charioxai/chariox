use std::path::{Path, PathBuf};
use std::process::Command;

use crate::agent::GitWorktreePlacement;
use crate::error::DaemonError;

pub(crate) fn resolve_existing_worktree(
    directory: &str,
    base_directory: impl AsRef<Path>,
    operation: &'static str,
) -> Result<String, DaemonError> {
    let resolved = resolve_target_directory(base_directory.as_ref(), directory);
    if !resolved.exists() {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("working directory `{}` does not exist", resolved.display()),
        });
    }
    if !resolved.is_dir() {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "working directory `{}` is not a directory",
                resolved.display()
            ),
        });
    }
    Ok(resolved.display().to_string())
}

pub(crate) fn is_git_worktree(base_directory: impl AsRef<Path>) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(base_directory.as_ref())
        .output()
        .is_ok_and(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        })
}

pub(crate) fn prepare_workflow_runtime_worktree_or_reuse_directory(
    placement: &GitWorktreePlacement,
    base_directory: impl AsRef<Path>,
    target_hint: Option<&str>,
    operation: &'static str,
) -> Result<String, DaemonError> {
    let base_directory = base_directory.as_ref();
    if is_git_worktree(base_directory) {
        return prepare_git_worktree(placement, base_directory, target_hint, operation);
    }
    resolve_existing_worktree(
        &base_directory.display().to_string(),
        Path::new("."),
        operation,
    )
}

pub(crate) fn remove_workflow_runtime_worktree(
    base_directory: impl AsRef<Path>,
    worktree_directory: impl AsRef<Path>,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let base_directory = base_directory.as_ref();
    let worktree_directory = worktree_directory.as_ref();
    if base_directory == worktree_directory
        || std::fs::canonicalize(base_directory)
            .ok()
            .zip(std::fs::canonicalize(worktree_directory).ok())
            .is_some_and(|(base, worktree)| base == worktree)
    {
        return Ok(());
    }
    remove_git_worktree(base_directory, worktree_directory, operation)
}

pub(crate) fn prepare_git_worktree(
    placement: &GitWorktreePlacement,
    base_directory: impl AsRef<Path>,
    target_hint: Option<&str>,
    operation: &'static str,
) -> Result<String, DaemonError> {
    let base_directory = base_directory.as_ref();
    let repo_root = run_git(base_directory, &["rev-parse", "--show-toplevel"], operation)?;
    let repo_root = PathBuf::from(repo_root.trim());
    if repo_root.as_os_str().is_empty() {
        return Err(DaemonError::LocalTransport {
            operation,
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
        .map(|target| resolve_target_directory(base_directory, target))
        .unwrap_or_else(|| {
            let repo_name = repo_root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("worktree");
            repo_root.parent().unwrap_or_else(|| Path::new(".")).join(
                default_worktree_directory_base(
                    repo_name,
                    placement.branch.as_deref().unwrap_or(from_ref),
                ),
            )
        });

    let target = target_directory.display().to_string();
    let args = if let Some(branch) = placement.branch.as_deref() {
        if git_branch_exists(&repo_root, branch, operation)? {
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
    run_git(&repo_root, &arg_refs, operation)?;
    resolve_existing_worktree(&target, Path::new("."), operation)
}

pub(crate) fn remove_git_worktree(
    base_directory: impl AsRef<Path>,
    worktree_directory: impl AsRef<Path>,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let base_directory = base_directory.as_ref();
    let worktree_directory = worktree_directory.as_ref();
    let repo_root = run_git(base_directory, &["rev-parse", "--show-toplevel"], operation)?;
    let repo_root = PathBuf::from(repo_root.trim());
    if worktree_directory == repo_root || worktree_directory.parent().is_none() {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "refusing to remove unsafe worktree path `{}`",
                worktree_directory.display()
            ),
        });
    }
    if !worktree_directory.exists() {
        run_git(&repo_root, &["worktree", "prune"], operation)?;
        return Ok(());
    }
    run_git(
        &repo_root,
        &[
            "worktree",
            "remove",
            "--force",
            &worktree_directory.display().to_string(),
        ],
        operation,
    )?;
    run_git(&repo_root, &["worktree", "prune"], operation)?;
    Ok(())
}

fn resolve_target_directory(base_directory: &Path, target: &str) -> PathBuf {
    let path = PathBuf::from(target);
    if path.is_absolute() {
        path
    } else {
        base_directory.join(path)
    }
}

fn git_branch_exists(
    repo_root: &Path,
    branch: &str,
    operation: &'static str,
) -> Result<bool, DaemonError> {
    match run_git(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
        operation,
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

fn run_git(cwd: &Path, args: &[&str], operation: &'static str) -> Result<String, DaemonError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!(
                "git {} failed in `{}`: {error}",
                args.join(" "),
                cwd.display()
            ),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation,
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

fn default_worktree_directory_base(repo_name: &str, branch_or_ref: &str) -> String {
    let repo_slug = slugify_git_branch(repo_name);
    let branch_leaf = branch_or_ref
        .rsplit('/')
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or(branch_or_ref);
    let branch_slug = slugify_git_branch(branch_leaf);
    let repo_prefix = format!("{}-", repo_slug.to_ascii_lowercase());
    let branch_slug_lower = branch_slug.to_ascii_lowercase();
    if branch_slug_lower == repo_slug.to_ascii_lowercase()
        || branch_slug_lower.starts_with(&repo_prefix)
    {
        branch_slug
    } else {
        format!("{repo_slug}-{branch_slug}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        default_worktree_directory_base, is_git_worktree,
        prepare_workflow_runtime_worktree_or_reuse_directory, remove_workflow_runtime_worktree,
    };
    use crate::agent::GitWorktreePlacement;
    use std::path::PathBuf;

    fn plain_temp_directory(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "chariox-git-worktree-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temporary directory should exist");
        directory
    }

    #[test]
    fn default_worktree_directory_base_uses_branch_leaf_without_duplicate_repo_prefix() {
        assert_eq!(
            default_worktree_directory_base(
                "chariox-cloud",
                "chariox/chariox-cloud-session-1783622367"
            ),
            "chariox-cloud-session-1783622367"
        );
        assert_eq!(
            default_worktree_directory_base("chariox", "chariox/chariox-session-1779647319"),
            "chariox-session-1779647319"
        );
        assert_eq!(
            default_worktree_directory_base("chariox-cloud", "feature/worktree-name"),
            "chariox-cloud-worktree-name"
        );
    }

    #[test]
    fn git_worktree_detection_rejects_plain_directories() {
        let directory = plain_temp_directory("detection");
        assert!(!is_git_worktree(&directory));
        std::fs::remove_dir(directory).expect("temporary directory should be removable");
    }

    #[test]
    fn runtime_instance_reuses_and_never_removes_a_plain_shared_directory() {
        let directory = plain_temp_directory("shared");
        let unused_target = directory.join("unused-instance");
        let placement = GitWorktreePlacement {
            target_directory: Some(unused_target.display().to_string()),
            branch: None,
            from_ref: Some("HEAD".to_string()),
        };
        let selected = prepare_workflow_runtime_worktree_or_reuse_directory(
            &placement,
            &directory,
            None,
            "prepare test runtime instance",
        )
        .expect("plain workspace should be reusable");

        assert_eq!(selected, directory.display().to_string());
        assert!(!unused_target.exists());
        remove_workflow_runtime_worktree(&directory, &selected, "cleanup test runtime instance")
            .expect("shared workspace cleanup should be a no-op");
        assert!(directory.exists());
        std::fs::remove_dir(directory).expect("temporary directory should be removable");
    }

    #[cfg(unix)]
    #[test]
    fn runtime_instance_cleanup_preserves_a_symlinked_plain_shared_directory() {
        let directory = plain_temp_directory("shared-symlink");
        let alias = directory.with_extension("alias");
        std::os::unix::fs::symlink(&directory, &alias)
            .expect("temporary directory alias should be created");

        remove_workflow_runtime_worktree(&directory, &alias, "cleanup aliased runtime instance")
            .expect("shared workspace cleanup should be a no-op through an alias");

        assert!(directory.exists());
        assert!(alias.exists());
        std::fs::remove_file(alias).expect("temporary alias should be removable");
        std::fs::remove_dir(directory).expect("temporary directory should be removable");
    }
}
