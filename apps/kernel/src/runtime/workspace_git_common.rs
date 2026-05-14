use std::path::{Path, PathBuf};

use crate::error::DaemonError;

pub(crate) fn git_command_output(path: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

pub(crate) fn run_workspace_git_command(
    worktree_path: &str,
    args: &[&str],
    operation: &'static str,
) -> Result<(), DaemonError> {
    let output = std::process::Command::new("git")
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
            format!(
                "git {} failed with status {}",
                args.join(" "),
                output.status
            )
        };
        return Err(DaemonError::LocalTransport { operation, message });
    }
    Ok(())
}

pub(crate) fn run_git(repo_root: &Path, args: &[&str]) -> Result<(), DaemonError> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "run git command",
            message: error.to_string(),
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation: "run git command",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

pub(crate) fn resolve_repo_root(workspace_path: &str) -> Result<PathBuf, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(workspace_path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "resolve repo root",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "resolve repo root",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

pub(crate) fn git_ref_exists(repo_root: &Path, reference: &str) -> Result<bool, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", reference])
        .current_dir(repo_root)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "check git ref",
            message: error.to_string(),
        })?;
    Ok(output.status.success())
}

pub(crate) fn git_reference_resolves(repo_root: &str, reference: &str) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", reference])
        .current_dir(repo_root)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn workspace_default_compare_ref(repo_root: &str, branch: Option<&str>) -> String {
    if git_reference_resolves(repo_root, "origin/main") {
        return "origin/main".to_string();
    }
    if git_reference_resolves(repo_root, "main") {
        return "main".to_string();
    }
    if git_reference_resolves(repo_root, "master") {
        return "master".to_string();
    }
    branch
        .filter(|value| !value.is_empty() && *value != "HEAD")
        .unwrap_or("HEAD")
        .to_string()
}

pub(crate) fn workspace_display_label(workspace_path: &str) -> Option<String> {
    git_command_output(workspace_path, &["remote", "get-url", "origin"])
        .as_deref()
        .and_then(repo_label_from_remote_url)
        .or_else(|| {
            Path::new(workspace_path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
}

pub(crate) fn worktree_display_label(
    path: &str,
    workspace_path: &str,
    branch: Option<&str>,
) -> Option<String> {
    let branch = branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| *value != "HEAD")
        .unwrap_or("detached");
    if same_fs_path(path, workspace_path) {
        return Some(branch.to_string());
    }
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|value| !value.is_empty())?;
    Some(format!("{name} / {branch}"))
}

pub(crate) fn same_fs_path(left: &str, right: &str) -> bool {
    std::fs::canonicalize(left).ok() == std::fs::canonicalize(right).ok()
        || Path::new(left) == Path::new(right)
}

pub(crate) fn detect_git_branch(path: &str) -> Result<String, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "detect git branch",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "detect git branch",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn repo_label_from_remote_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches(".git");
    let candidate = if let Some(rest) = trimmed.strip_prefix("git@") {
        rest.split_once(':').map(|(_, path)| path.to_string())
    } else if let Some((_, path)) = trimmed.split_once("://") {
        let mut parts = path.split('/').collect::<Vec<_>>();
        if parts.len() >= 3 {
            Some(parts.split_off(parts.len() - 2).join("/"))
        } else {
            None
        }
    } else {
        None
    }?;
    let parts = candidate
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    Some(format!(
        "{}/{}",
        parts[parts.len() - 2],
        parts[parts.len() - 1]
    ))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        repo_label_from_remote_url, same_fs_path, workspace_default_compare_ref,
        worktree_display_label,
    };

    #[test]
    fn repo_labels_parse_common_remote_urls() {
        assert_eq!(
            repo_label_from_remote_url("git@github.com:owner/repo.git").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            repo_label_from_remote_url("https://github.com/owner/repo.git").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(repo_label_from_remote_url("not-a-git-url"), None);
    }

    #[test]
    fn worktree_labels_prefer_branch_for_main_worktree() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("arroba-worktree-label-{nonce}"));
        let workspace = root.join("workspace");
        let feature = root.join("feature");
        std::fs::create_dir_all(&workspace).expect("workspace test directory should be created");
        std::fs::create_dir_all(&feature).expect("feature test directory should be created");
        let workspace = workspace.to_string_lossy();
        let feature = feature.to_string_lossy();

        assert_eq!(
            worktree_display_label(&workspace, &workspace, Some("main")).as_deref(),
            Some("main")
        );
        assert_eq!(
            worktree_display_label(&feature, &workspace, Some("feat")).as_deref(),
            Some("feature / feat")
        );
        assert_eq!(
            worktree_display_label(&feature, &workspace, Some("HEAD")).as_deref(),
            Some("feature / detached")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn same_fs_path_accepts_literal_matches() {
        assert!(same_fs_path("/repo/main", "/repo/main"));
    }

    #[test]
    fn default_compare_ref_falls_back_to_branch_then_head() {
        assert_eq!(
            workspace_default_compare_ref("/path/that/does/not/exist", Some("feature/test")),
            "feature/test"
        );
        assert_eq!(
            workspace_default_compare_ref("/path/that/does/not/exist", Some("HEAD")),
            "HEAD"
        );
        assert_eq!(
            workspace_default_compare_ref("/path/that/does/not/exist", None),
            "HEAD"
        );
    }
}
