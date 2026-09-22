use std::path::{Path, PathBuf};
use std::process::Command;

use crate::agent::GitWorktreePlacement;
use crate::error::DaemonError;

const CHARIOX_STATE_DIRECTORY_NAMES: &[&str] = &[
    ".chariox",
    "kernels",
    "state",
    "managed-context",
    "managed-runtime-auth",
    "managed",
    "sessions",
    "daemon",
    "machine",
];
const PROTECTED_DIRECTORY_ENV_NAMES: &[&str] = &[
    "CHARIOX_CAPABILITY_ISOLATION_ROOT",
    "CHARIOX_MANAGED_SLICE_SERVICE_ROOT",
    "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT",
];
const MANAGED_ISOLATION_PROTECTED_DIRECTORY_ENV_NAMES: &[&str] = &["CHARIOX_MANAGED_PROVIDER_HOME"];
const PROTECTED_FILE_ENV_NAMES: &[&str] = &[
    "CHARIOX_MANAGED_VAULT_PATH",
    "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
    "CHARIOX_MANAGED_BOOTSTRAP_PATH",
    "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
    "CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH",
    "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
    "CHARIOX_DAEMON_SOCKET",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkingDirectoryPreflight {
    pub(crate) requested_path: PathBuf,
    pub(crate) canonical_path: PathBuf,
    pub(crate) exists: bool,
}

/// Apply the ordinary-kernel working-directory contract at every entry point.
///
/// The check deliberately follows symlinks, requires only the same metadata and
/// ancestor-execute access needed by a Linux `current_dir`, and never enumerates
/// children. Managed isolation can pass selected re-exposed roots when it must
/// mask a dedicated service namespace; Path 1 and ordinary launches pass none.
pub(crate) fn preflight_working_directory(
    path: &Path,
    operation: &'static str,
    allow_missing: bool,
    reexposed_roots: &[PathBuf],
) -> Result<WorkingDirectoryPreflight, DaemonError> {
    let (canonical_path, exists) = match std::fs::metadata(path) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(working_directory_error(
                operation,
                format!("working directory `{}` is not a directory", path.display()),
            ));
        }
        Ok(_) => (
            path.canonicalize().map_err(|error| {
                working_directory_error(
                    operation,
                    format!(
                        "working directory `{}` cannot be accessed: {error}",
                        path.display()
                    ),
                )
            })?,
            true,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_missing => {
            (canonical_or_lexical_path(path, operation)?, false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(working_directory_error(
                operation,
                format!("working directory `{}` does not exist", path.display()),
            ));
        }
        Err(error) => {
            return Err(working_directory_error(
                operation,
                format!(
                    "working directory `{}` cannot be accessed: {error}",
                    path.display()
                ),
            ));
        }
    };

    let protection = ordinary_working_directory_protection(operation)?;
    let mut reexposed_roots = reexposed_roots
        .iter()
        .map(|root| canonical_or_lexical_path(root, operation))
        .collect::<Result<Vec<_>, _>>()?;
    // Workflow runtime instances are kernel-owned artifacts, not caller-provided
    // live-sync roots. Derive this one narrow authorization from the configured
    // Chariox state namespace so every launch boundary shares the same rule.
    if let Some(root) = kernel_owned_workflow_runtime_instance_root(&canonical_path, operation)? {
        reexposed_roots.push(root);
    }
    if reexposed_roots.iter().any(|root| {
        protection
            .directories
            .iter()
            .any(|protected| root == protected)
    }) {
        return Err(working_directory_error(
            operation,
            format!(
                "working directory `{}` cannot re-expose protected Chariox service state",
                path.display()
            ),
        ));
    }
    if protection
        .directories
        .iter()
        .any(|root| canonical_path_is_within(&canonical_path, root))
        && reexposed_roots.iter().all(|root| {
            !protection
                .directories
                .iter()
                .any(|protected| root != protected && root.starts_with(protected))
                || !canonical_path_is_within(&canonical_path, root)
        })
    {
        return Err(working_directory_error(
            operation,
            format!(
                "working directory `{}` is inside protected Chariox service state",
                path.display()
            ),
        ));
    }
    if protection
        .files
        .iter()
        .any(|protected| protected == &canonical_path)
    {
        return Err(working_directory_error(
            operation,
            format!(
                "working directory `{}` is a protected Chariox control path",
                path.display()
            ),
        ));
    }

    Ok(WorkingDirectoryPreflight {
        requested_path: path.to_path_buf(),
        canonical_path,
        exists,
    })
}

fn working_directory_error(operation: &'static str, message: String) -> DaemonError {
    DaemonError::LocalTransport { operation, message }
}

#[derive(Debug, Default)]
struct WorkingDirectoryProtection {
    directories: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

fn ordinary_working_directory_protection(
    operation: &'static str,
) -> Result<WorkingDirectoryProtection, DaemonError> {
    let mut protection = WorkingDirectoryProtection::default();
    if let Some(raw_home) = std::env::var_os("CHARIOX_HOME") {
        let home = configured_protected_path(&raw_home, "CHARIOX_HOME", operation)?;
        if home.file_name() == Some(std::ffi::OsStr::new(".chariox")) {
            protection.directories.push(home);
        } else {
            for name in CHARIOX_STATE_DIRECTORY_NAMES {
                protection.directories.push(home.join(name));
            }
        }
    } else if let Some(raw_home) = std::env::var_os("HOME") {
        let home = configured_protected_path(&raw_home, "HOME", operation)?;
        protection.directories.push(canonical_or_lexical_path(
            &home.join(".chariox"),
            operation,
        )?);
    }
    for name in PROTECTED_DIRECTORY_ENV_NAMES {
        if let Some(raw) = std::env::var_os(name) {
            protection
                .directories
                .push(configured_protected_path(&raw, name, operation)?);
        }
    }
    if crate::provider::managed_provider_isolation_required() {
        for name in MANAGED_ISOLATION_PROTECTED_DIRECTORY_ENV_NAMES {
            if let Some(raw) = std::env::var_os(name) {
                protection
                    .directories
                    .push(configured_protected_path(&raw, name, operation)?);
            }
        }
    }
    for name in PROTECTED_FILE_ENV_NAMES {
        if let Some(raw) = std::env::var_os(name) {
            protection
                .files
                .push(configured_protected_path(&raw, name, operation)?);
        }
    }
    protection.directories.sort();
    protection.directories.dedup();
    protection.files.sort();
    protection.files.dedup();
    Ok(protection)
}

fn configured_protected_path(
    raw: &std::ffi::OsStr,
    name: &str,
    operation: &'static str,
) -> Result<PathBuf, DaemonError> {
    let path = PathBuf::from(raw);
    if !path.is_absolute()
        || path == Path::new("/")
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(working_directory_error(
            operation,
            format!("protected Chariox path `{name}` must be absolute and path-safe"),
        ));
    }
    canonical_or_lexical_path(&path, operation)
}

fn canonical_or_lexical_path(path: &Path, operation: &'static str) -> Result<PathBuf, DaemonError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| {
                working_directory_error(
                    operation,
                    format!(
                        "cannot resolve working directory `{}`: {error}",
                        path.display()
                    ),
                )
            })?
            .join(path)
    };
    if let Ok(canonical) = absolute.canonicalize() {
        return Ok(canonical);
    }

    // Preserve canonical symlink resolution for a planned child below an
    // existing directory. This keeps protection checks aligned with Linux
    // path traversal even when the final directory is created later.
    let mut unresolved: Vec<std::ffi::OsString> = Vec::new();
    let mut ancestor = absolute.clone();
    loop {
        if let Ok(canonical) = ancestor.canonicalize() {
            let mut result = canonical;
            for component in unresolved.iter().rev() {
                result.push(component);
            }
            return Ok(result);
        }
        let Some(name) = ancestor.file_name() else {
            break;
        };
        unresolved.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            break;
        };
        if parent == ancestor {
            break;
        }
        ancestor = parent.to_path_buf();
    }
    lexically_normalized_absolute_path(&absolute, operation)
}

fn lexically_normalized_absolute_path(
    path: &Path,
    operation: &'static str,
) -> Result<PathBuf, DaemonError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| {
                working_directory_error(
                    operation,
                    format!(
                        "cannot resolve working directory `{}`: {error}",
                        path.display()
                    ),
                )
            })?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

fn canonical_path_is_within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn kernel_owned_workflow_runtime_instance_root(
    canonical_path: &Path,
    operation: &'static str,
) -> Result<Option<PathBuf>, DaemonError> {
    let instances_roots = if let Some(raw_home) = std::env::var_os("CHARIOX_HOME") {
        vec![PathBuf::from(raw_home)
            .join("state")
            .join("workflow-runtime")
            .join("instances")]
    } else if let Some(raw_home) = std::env::var_os("HOME") {
        vec![PathBuf::from(raw_home)
            .join(".chariox")
            .join("state")
            .join("workflow-runtime")
            .join("instances")]
    } else {
        Vec::new()
    };

    for instances_root in instances_roots {
        let instances_root = canonical_or_lexical_path(&instances_root, operation)?;
        let Ok(relative) = canonical_path.strip_prefix(&instances_root) else {
            continue;
        };
        let mut components = relative.components();
        let Some(std::path::Component::Normal(session_id)) = components.next() else {
            continue;
        };
        let Some(std::path::Component::Normal(instance_id)) = components.next() else {
            continue;
        };
        let instance_root = instances_root.join(session_id).join(instance_id);
        if canonical_path_is_within(canonical_path, &instance_root) {
            return Ok(Some(instance_root));
        }
    }
    Ok(None)
}

pub(crate) fn resolve_existing_worktree(
    directory: &str,
    base_directory: impl AsRef<Path>,
    operation: &'static str,
) -> Result<String, DaemonError> {
    let resolved = resolve_target_directory(base_directory.as_ref(), directory);
    preflight_working_directory(&resolved, operation, false, &[])?;
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
    preflight_working_directory(base_directory, operation, false, &[])?;
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
    preflight_working_directory(worktree_directory, operation, true, &[])?;
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
    preflight_working_directory(base_directory, operation, false, &[])?;
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
    preflight_working_directory(&target_directory, operation, true, &[])?;
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
    preflight_working_directory(worktree_directory, operation, true, &[])?;
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
        default_worktree_directory_base, is_git_worktree, preflight_working_directory,
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

    #[test]
    fn ordinary_and_path1_inputs_share_the_same_access_contract() {
        let root = plain_temp_directory("common-preflight");
        let nested = root.join("nested").join("new");
        std::fs::create_dir_all(&nested).expect("nested directory should exist");
        let post_enrollment_repository = root.join("repository-created-after-enrollment");
        std::fs::create_dir_all(&post_enrollment_repository)
            .expect("post-enrollment repository should exist");
        let existing = [
            PathBuf::from("/"),
            PathBuf::from("/home"),
            PathBuf::from("/var"),
            PathBuf::from("/usr/lib"),
            PathBuf::from("/tmp"),
            nested,
            post_enrollment_repository,
        ];

        for path in existing {
            if !path.is_dir() {
                continue;
            }
            let ordinary = preflight_working_directory(&path, "ordinary.cwd", false, &[])
                .expect("ordinary path should pass its access contract");
            let path1 = preflight_working_directory(&path, "path1.cwd", false, &[])
                .expect("Path-1 path should pass the same access contract");
            assert_eq!(ordinary.canonical_path, path1.canonical_path);
            assert!(ordinary.exists && path1.exists);
        }

        let missing = root.join("created-after-preflight");
        let planned = preflight_working_directory(&missing, "ordinary.create", true, &[])
            .expect("newly-created directory should be admissible before creation");
        assert!(!planned.exists);
        std::fs::create_dir_all(&missing).expect("planned directory should be created");
        let path1 = preflight_working_directory(&missing, "path1.cwd", false, &[])
            .expect("Path-1 should accept the newly-created directory");
        assert_eq!(path1.canonical_path, missing.canonicalize().unwrap());

        let missing_error =
            preflight_working_directory(&root.join("does-not-exist"), "path1.cwd", false, &[])
                .expect_err("missing working directories must fail closed");
        assert!(missing_error.to_string().contains("does not exist"));
        std::fs::remove_dir_all(root).expect("preflight fixture should be removable");
    }

    #[test]
    fn path1_provider_home_child_uses_ordinary_cwd_contract() {
        let _env = crate::env_lock::lock();
        let root = plain_temp_directory("path1-provider-home");
        let provider_home = root.join("provider-home");
        let selected = provider_home.join("selected-repository");
        let sibling = root.join("sibling-repository");
        let vault = provider_home.join("vault.json");
        std::fs::create_dir_all(&selected).expect("selected repository should exist");
        std::fs::create_dir_all(&sibling).expect("sibling repository should exist");

        let prior_topology = std::env::var_os("CHARIOX_MANAGED_PROVIDER_TOPOLOGY");
        let prior_isolation = std::env::var_os("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        let prior_provider_home = std::env::var_os("CHARIOX_MANAGED_PROVIDER_HOME");
        let prior_vault = std::env::var_os("CHARIOX_MANAGED_VAULT_PATH");
        std::env::remove_var("CHARIOX_MANAGED_PROVIDER_TOPOLOGY");
        std::env::remove_var("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        std::env::remove_var("CHARIOX_MANAGED_PROVIDER_HOME");
        std::env::set_var("CHARIOX_MANAGED_VAULT_PATH", &vault);

        let ordinary = preflight_working_directory(&selected, "ordinary.cwd", false, &[])
            .expect("ordinary provider-home child should be usable");

        std::env::set_var("CHARIOX_MANAGED_PROVIDER_TOPOLOGY", "path1");
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_HOME", &provider_home);
        let path1 = preflight_working_directory(&selected, "path1.cwd", false, &[])
            .expect("Path-1 must preserve the ordinary provider-home child cwd contract");
        assert_eq!(path1.canonical_path, ordinary.canonical_path);
        preflight_working_directory(&sibling, "path1.sibling.cwd", false, &[])
            .expect("an unrelated sibling must remain usable");
        let vault_error = preflight_working_directory(&vault, "path1.vault.cwd", true, &[])
            .expect_err("the exact managed vault path must remain protected");
        assert!(vault_error
            .to_string()
            .contains("protected Chariox control path"));

        std::env::set_var("CHARIOX_MANAGED_PROVIDER_TOPOLOGY", "shared_host");
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_ISOLATION", "1");
        let shared_host_error =
            preflight_working_directory(&selected, "shared-host.cwd", false, &[])
                .expect_err("shared-host provider isolation must keep its provider-home boundary");
        assert!(shared_host_error
            .to_string()
            .contains("protected Chariox service state"));

        restore_env("CHARIOX_MANAGED_PROVIDER_TOPOLOGY", prior_topology);
        restore_env("CHARIOX_MANAGED_PROVIDER_ISOLATION", prior_isolation);
        restore_env("CHARIOX_MANAGED_PROVIDER_HOME", prior_provider_home);
        restore_env("CHARIOX_MANAGED_VAULT_PATH", prior_vault);
        std::fs::remove_dir_all(root).expect("provider-home fixture should be removable");
    }

    #[test]
    fn chariox_home_authorizes_only_kernel_workflow_runtime_instance_children() {
        let _env = crate::env_lock::lock();
        let root = plain_temp_directory("workflow-runtime-auth");
        let instance = root
            .join("state")
            .join("workflow-runtime")
            .join("instances")
            .join("session-1")
            .join("instance-1");
        let nested = instance.join("src");
        let unrelated = root.join("state").join("unrelated");
        std::fs::create_dir_all(&nested).expect("workflow instance should exist");
        std::fs::create_dir_all(&unrelated).expect("unrelated state should exist");

        let previous_home = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", &root);

        preflight_working_directory(&instance, "workflow.runtime.cwd", false, &[])
            .expect("the exact kernel-owned workflow instance should be authorized");
        preflight_working_directory(&nested, "workflow.runtime.nested.cwd", false, &[])
            .expect("children of the kernel-owned workflow instance should be authorized");
        preflight_working_directory(&unrelated, "workflow.unrelated.cwd", false, &[])
            .expect_err("unrelated Chariox state must remain protected");

        restore_env("CHARIOX_HOME", previous_home);
        std::fs::remove_dir_all(root).expect("workflow auth fixture should be removable");
    }

    #[test]
    fn workflow_runtime_git_provisioning_reaches_kernel_owned_instance_root() {
        let _env = crate::env_lock::lock();
        let root = plain_temp_directory("workflow-runtime-git");
        let chariox_home = root.join("chariox-home");
        let source = root.join("source");
        let target = chariox_home
            .join("state")
            .join("workflow-runtime")
            .join("instances")
            .join("session-1")
            .join("instance-1");
        std::fs::create_dir_all(&source).expect("source repository should exist");
        std::fs::create_dir_all(target.parent().expect("target parent should exist"))
            .expect("workflow instance parent should exist");
        run_test_git(&source, &["init", "-b", "main"]);
        run_test_git(&source, &["config", "user.email", "test@chariox.local"]);
        run_test_git(&source, &["config", "user.name", "Chariox Test"]);
        std::fs::write(source.join("README"), "workflow runtime\n")
            .expect("source fixture should be writable");
        run_test_git(&source, &["add", "README"]);
        run_test_git(&source, &["commit", "-m", "initial"]);

        let previous_home = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", &chariox_home);
        let placement = GitWorktreePlacement {
            target_directory: Some(target.display().to_string()),
            branch: None,
            from_ref: Some("HEAD".to_string()),
        };
        let selected = prepare_workflow_runtime_worktree_or_reuse_directory(
            &placement,
            &source,
            None,
            "provision workflow runtime test instance",
        )
        .expect("kernel-owned workflow runtime worktree should provision");
        assert_eq!(PathBuf::from(&selected), target);
        assert!(is_git_worktree(&target));

        remove_workflow_runtime_worktree(
            &source,
            &selected,
            "cleanup workflow runtime test instance",
        )
        .expect("workflow runtime worktree should clean up");
        assert!(!target.exists());

        restore_env("CHARIOX_HOME", previous_home);
        std::fs::remove_dir_all(root).expect("workflow git fixture should be removable");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_follow_ordinary_kernel_resolution() {
        let root = plain_temp_directory("common-preflight-symlink");
        let target = root.join("target");
        let alias = root.join("alias");
        std::fs::create_dir_all(&target).expect("symlink target should exist");
        std::os::unix::fs::symlink(&target, &alias).expect("directory alias should exist");

        for operation in ["ordinary.cwd", "path1.cwd"] {
            let result = preflight_working_directory(&alias, operation, false, &[])
                .expect("ordinary-accepted symlinks should remain accepted");
            assert_eq!(result.requested_path, alias);
            assert_eq!(result.canonical_path, target.canonicalize().unwrap());
        }
        let planned = preflight_working_directory(
            &alias.join("created-after-enrollment").join("deeper"),
            "path1.create",
            true,
            &[],
        )
        .expect("missing children below symlinked directories should be plan-able");
        assert_eq!(
            planned.canonical_path,
            target.join("created-after-enrollment").join("deeper")
        );
        std::fs::remove_file(alias).expect("directory alias should be removable");
        std::fs::remove_dir_all(root).expect("symlink fixture should be removable");
    }

    #[test]
    fn home_fallback_protects_only_home_chariox_state() {
        let _env = crate::env_lock::lock();
        let root = plain_temp_directory("common-preflight-home-fallback");
        let home = root.join("home");
        let chariox_state = home.join(".chariox");
        std::fs::create_dir_all(chariox_state.join("child"))
            .expect("fallback Chariox state should exist");
        std::fs::create_dir_all(home.join("state")).expect("ordinary state workspace should exist");
        std::fs::create_dir_all(home.join("sessions"))
            .expect("ordinary sessions workspace should exist");

        let prior_home = std::env::var_os("HOME");
        let prior_chariox_home = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("HOME", &home);
        std::env::remove_var("CHARIOX_HOME");

        preflight_working_directory(&home.join("state"), "path1.cwd", false, &[])
            .expect("HOME/state should remain an ordinary workspace");
        preflight_working_directory(&home.join("sessions"), "path1.cwd", false, &[])
            .expect("HOME/sessions should remain an ordinary workspace");
        preflight_working_directory(&chariox_state, "path1.cwd", false, &[])
            .expect_err("HOME/.chariox must remain protected");
        preflight_working_directory(&chariox_state.join("child"), "path1.cwd", false, &[])
            .expect_err("HOME/.chariox descendants must remain protected");

        restore_env("HOME", prior_home);
        restore_env("CHARIOX_HOME", prior_chariox_home);
        std::fs::remove_dir_all(root).expect("fallback fixture should be removable");
    }

    #[test]
    fn only_exact_control_state_and_service_descendants_are_rejected() {
        let _env = crate::env_lock::lock();
        let root = plain_temp_directory("common-preflight-control-state");
        let service = root.join("service-state");
        let selected = service.join("selected-repository");
        let nested_selected = selected.join("nested-repository");
        let sibling = root.join("sibling");
        let control_file = root.join("bootstrap-receipt.json");
        std::fs::create_dir_all(&nested_selected).expect("selected repository should exist");
        std::fs::create_dir_all(&sibling).expect("sibling should exist");
        std::fs::write(&control_file, "control\n").expect("control file should exist");

        let prior_service = std::env::var_os("CHARIOX_MANAGED_SLICE_SERVICE_ROOT");
        let prior_publication = std::env::var_os("CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT");
        let prior_receipt = std::env::var_os("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT");
        std::env::set_var("CHARIOX_MANAGED_SLICE_SERVICE_ROOT", &service);
        std::env::set_var("CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT", &selected);
        std::env::set_var("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT", &control_file);

        let service_error = preflight_working_directory(&service, "ordinary.cwd", false, &[])
            .expect_err("dedicated service state must be rejected");
        assert!(service_error
            .to_string()
            .contains("protected Chariox service state"));
        let child_error =
            preflight_working_directory(&service.join("nested"), "path1.cwd", true, &[])
                .expect_err("service-state descendants must be rejected");
        assert!(child_error
            .to_string()
            .contains("protected Chariox service state"));
        preflight_working_directory(&root, "ordinary.cwd", false, &[])
            .expect("the service parent must remain usable");
        preflight_working_directory(&sibling, "path1.cwd", false, &[])
            .expect("an unrelated sibling must remain usable");
        preflight_working_directory(
            &nested_selected,
            "managed.selected.cwd",
            false,
            std::slice::from_ref(&nested_selected),
        )
        .expect("a selected managed child may be explicitly re-exposed");
        preflight_working_directory(
            &selected,
            "managed.publication.cwd",
            false,
            std::slice::from_ref(&selected),
        )
        .expect_err("a re-exposed root equal to any protected root is forbidden");
        preflight_working_directory(
            &service,
            "managed.service.cwd",
            false,
            std::slice::from_ref(&service),
        )
        .expect_err("re-exposing an entire service root must remain forbidden");
        let file_error = preflight_working_directory(&control_file, "path1.cwd", false, &[])
            .expect_err("exact control files must not become working directories");
        assert!(file_error.to_string().contains("not a directory"));

        restore_env("CHARIOX_MANAGED_SLICE_SERVICE_ROOT", prior_service);
        restore_env("CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT", prior_publication);
        restore_env("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT", prior_receipt);
        std::fs::remove_dir_all(root).expect("control-state fixture should be removable");
    }

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    fn run_test_git(directory: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(directory)
            .output()
            .expect("git test command should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
