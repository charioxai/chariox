use std::collections::HashSet;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::local::WaitingRoomLaunchTarget;

pub(crate) fn search_workspace_directories(
    query: &str,
    limit: usize,
    launch_target: Option<WaitingRoomLaunchTarget>,
) -> Result<Vec<String>, DaemonError> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let trimmed_query = query.trim();
    let normalized_query = trimmed_query.to_lowercase();

    if let Some(target) = launch_target {
        push_matching_path(
            &mut results,
            &mut seen,
            target.workspace_id,
            &normalized_query,
            limit,
        );
        push_matching_path(
            &mut results,
            &mut seen,
            target.worktree_id,
            &normalized_query,
            limit,
        );
    }

    if normalized_query.is_empty() {
        let roots = workspace_search_roots()?;
        for root in &roots {
            if crate::git_worktree_placement::preflight_working_directory(
                root,
                "search workspace directories",
                false,
                &[],
            )
            .is_err()
            {
                continue;
            }
            push_unique_path(&mut results, &mut seen, root.display().to_string());
            if results.len() >= limit {
                break;
            }
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if crate::git_worktree_placement::preflight_working_directory(
                        &path,
                        "search workspace directories",
                        false,
                        &[],
                    )
                    .is_ok()
                    {
                        push_unique_path(&mut results, &mut seen, path.display().to_string());
                        if results.len() >= limit {
                            break;
                        }
                    }
                }
            }
            if results.len() >= limit {
                break;
            }
        }
        results.truncate(limit);
        return Ok(results);
    }

    if looks_like_path_query(trimmed_query) {
        append_directory_completion(&mut results, &mut seen, trimmed_query, limit)?;
        results.truncate(limit);
        return Ok(results);
    }

    let roots = workspace_search_roots()?;
    for root in roots {
        append_matching_directory_children(
            &mut results,
            &mut seen,
            &root,
            &normalized_query,
            limit,
        )?;
    }
    results.truncate(limit);
    Ok(results)
}

pub(crate) fn create_workspace_directory(path: &str) -> Result<String, DaemonError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "create workspace directory",
            message: "workspace path is required".to_string(),
        });
    }
    let expanded = expand_workspace_query_path(trimmed);
    let directory = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "create workspace directory",
                message: format!("cannot create workspace directory {trimmed}: cannot resolve current directory: {error}"),
            })?
            .join(expanded)
    };
    for component in directory.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match std::fs::metadata(component) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err(workspace_creation_error(
                    &directory,
                    component,
                    io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "path exists and is not a directory",
                    ),
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(workspace_creation_error(&directory, component, error)),
        }
    }
    crate::git_worktree_placement::preflight_working_directory(
        &directory,
        "create workspace directory",
        true,
        &[],
    )?;
    create_workspace_components(&directory, &directory)?;
    crate::git_worktree_placement::preflight_working_directory(
        &directory,
        "create workspace directory",
        false,
        &[],
    )?;
    Ok(directory.display().to_string())
}

fn create_workspace_components(path: &Path, workspace: &Path) -> Result<(), DaemonError> {
    if path.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path.parent().filter(|parent| *parent != path) {
        create_workspace_components(parent, workspace)?;
    }
    std::fs::create_dir(path)
        .or_else(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() {
                Ok(())
            } else {
                Err(error)
            }
        })
        .map_err(|error| workspace_creation_error(workspace, path, error))
}

fn workspace_creation_error(workspace: &Path, component: &Path, error: io::Error) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "create workspace directory",
        message: format!(
            "cannot create workspace directory {}: {} creating {}",
            workspace.display(),
            error,
            component.display()
        ),
    }
}

fn workspace_search_roots() -> Result<Vec<PathBuf>, DaemonError> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for candidate in [
        std::env::current_dir().ok(),
        std::env::var_os("HOME").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    {
        let path = candidate;
        if seen.insert(path.clone()) {
            roots.push(path);
        }
    }

    // An explicit kernel repository-root setting is the primary search root.
    // Ordinary launches without it retain the cwd/HOME order.
    if std::env::var_os(crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV).is_some() {
        let configured = crate::managed_bootstrap::managed_repository_root_from_env()?;
        let resolved = crate::git_worktree_placement::preflight_working_directory(
            &configured,
            "search workspace directories",
            false,
            &[],
        )?;
        crate::git_worktree_placement::preflight_managed_repository_root(&resolved.canonical_path)?;
        if seen.insert(resolved.canonical_path.clone()) {
            roots.insert(0, resolved.canonical_path);
        }
    }

    Ok(roots)
}

fn push_unique_path(results: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    if value.trim().is_empty() {
        return;
    }
    if seen.insert(value.clone()) {
        results.push(value);
    }
}

fn push_matching_path(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    value: String,
    normalized_query: &str,
    limit: usize,
) {
    if results.len() >= limit {
        return;
    }
    if crate::git_worktree_placement::preflight_working_directory(
        Path::new(&value),
        "search workspace directories",
        false,
        &[],
    )
    .is_err()
    {
        return;
    }
    if normalized_query.is_empty() || value.to_lowercase().contains(normalized_query) {
        push_unique_path(results, seen, value);
    }
}

fn looks_like_path_query(query: &str) -> bool {
    query.starts_with('/') || query.starts_with("~/") || query == "~" || query.contains('/')
}

fn append_directory_completion(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    query: &str,
    limit: usize,
) -> Result<(), DaemonError> {
    let expanded = expand_workspace_query_path(query);
    if query == "~" {
        if crate::git_worktree_placement::preflight_working_directory(
            &expanded,
            "search workspace directories",
            false,
            &[],
        )
        .is_ok()
        {
            push_unique_path(results, seen, expanded.display().to_string());
            append_matching_directory_children(results, seen, &expanded, "", limit)?;
        }
        return Ok(());
    }
    if query.ends_with('/') {
        if crate::git_worktree_placement::preflight_working_directory(
            &expanded,
            "search workspace directories",
            false,
            &[],
        )
        .is_ok()
        {
            push_unique_path(results, seen, expanded.display().to_string());
        }
        return append_matching_directory_children(results, seen, &expanded, "", limit);
    }

    if crate::git_worktree_placement::preflight_working_directory(
        &expanded,
        "search workspace directories",
        false,
        &[],
    )
    .is_ok()
    {
        push_unique_path(results, seen, expanded.display().to_string());
    }
    let prefix = expanded
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_lowercase();
    let parent = expanded
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    append_matching_directory_children(results, seen, &parent, &prefix, limit)
}

fn append_matching_directory_children(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    parent: &Path,
    normalized_query: &str,
    limit: usize,
) -> Result<(), DaemonError> {
    if results.len() >= limit
        || crate::git_worktree_placement::preflight_working_directory(
            parent,
            "search workspace directories",
            false,
            &[],
        )
        .is_err()
    {
        return Ok(());
    }
    append_matching_directory_children_from_result(
        results,
        seen,
        normalized_query,
        limit,
        read_directory_children(parent),
    )
}

fn append_matching_directory_children_from_result(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    normalized_query: &str,
    limit: usize,
    entries: io::Result<Vec<io::Result<PathBuf>>>,
) -> Result<(), DaemonError> {
    let entries = match entries {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Ok(()),
        Err(error) => {
            return Err(DaemonError::LocalTransport {
                operation: "search workspace directories",
                message: error.to_string(),
            })
        }
    };
    let mut paths = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry {
            Ok(path) => paths.push(path),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => continue,
            Err(error) => {
                return Err(DaemonError::LocalTransport {
                    operation: "search workspace directories",
                    message: error.to_string(),
                })
            }
        }
    }
    append_matching_directory_paths(results, seen, paths, normalized_query, limit);
    Ok(())
}

fn read_directory_children(parent: &Path) -> io::Result<Vec<io::Result<PathBuf>>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(parent)? {
        entries.push(entry.map(|entry| entry.path()));
    }
    Ok(entries)
}

fn append_matching_directory_paths(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    entries: Vec<PathBuf>,
    normalized_query: &str,
    limit: usize,
) {
    let mut matches = Vec::new();
    for path in entries {
        if crate::git_worktree_placement::preflight_working_directory(
            &path,
            "search workspace directories",
            false,
            &[],
        )
        .is_err()
        {
            continue;
        }
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .to_lowercase();
        if normalized_query.is_empty() || name.contains(normalized_query) {
            matches.push(path);
        }
    }
    matches.sort_by(|left, right| {
        directory_match_rank(left, normalized_query)
            .cmp(&directory_match_rank(right, normalized_query))
            .then_with(|| directory_sort_name(left).cmp(&directory_sort_name(right)))
    });
    for path in matches {
        push_unique_path(results, seen, path.display().to_string());
        if results.len() >= limit {
            break;
        }
    }
}

fn directory_match_rank(path: &Path, normalized_query: &str) -> (u8, u8) {
    let name = directory_sort_name(path);
    let query = normalized_query.trim();
    let exact_rank = if !query.is_empty() && name == query {
        0
    } else if query.is_empty() || name.starts_with(query) {
        1
    } else {
        2
    };
    let hidden_rank = if query.starts_with('.') || !name.starts_with('.') {
        0
    } else {
        1
    };
    (exact_rank, hidden_rank)
}

fn directory_sort_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_lowercase()
}

pub(crate) fn expand_workspace_query_path(query: &str) -> PathBuf {
    if query == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(query));
    }
    if let Some(rest) = query.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(query)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::PathBuf;

    use super::{append_matching_directory_children_from_result, search_workspace_directories};

    #[test]
    fn workspace_creation_error_names_requested_path_and_failed_component() {
        let workspace = std::path::Path::new("/Users/miguel/x");
        let component = std::path::Path::new("/Users");
        let error = super::workspace_creation_error(
            workspace,
            component,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        );
        let message = error.to_string();
        assert!(
            message.contains("cannot create workspace directory /Users/miguel/x"),
            "{message}"
        );
        assert!(message.contains("creating /Users"), "{message}");
        assert!(
            message.to_lowercase().contains("permission denied"),
            "{message}"
        );
    }

    #[test]
    fn workspace_creation_reports_obstructing_component() {
        let root = unique_test_dir("workspace-create-obstructed");
        create_test_dir(root.clone());
        let obstruction = root.join("file");
        std::fs::write(&obstruction, "not a directory").unwrap();
        let target = obstruction.join("workspace");
        let message = super::create_workspace_directory(target.to_str().unwrap())
            .unwrap_err()
            .to_string();
        assert!(message.contains(&target.display().to_string()), "{message}");
        assert!(
            message.contains(&format!("creating {}", obstruction.display())),
            "{message}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_completion_keeps_sibling_prefix_matches_for_existing_path() {
        let root = unique_test_dir("workspace-directory-completion-siblings");
        create_test_dir(root.join("chariox"));
        create_test_dir(root.join("chariox-cloud"));
        create_test_dir(root.join("chariox-feature"));
        create_test_dir(root.join(".chariox"));
        create_test_dir(root.join("bar-chariox"));

        let results =
            search_workspace_directories(&root.join("chariox").display().to_string(), 20, None)
                .expect("workspace directory search should succeed");
        remove_test_dir(&root);

        let exact = root.join("chariox").display().to_string();
        let cloud = root.join("chariox-cloud").display().to_string();
        let feature = root.join("chariox-feature").display().to_string();
        let hidden = root.join(".chariox").display().to_string();
        let contains = root.join("bar-chariox").display().to_string();

        assert!(results.contains(&exact), "missing exact match: {results:?}");
        assert!(
            results.contains(&cloud),
            "missing prefix sibling: {results:?}"
        );
        assert!(
            results.contains(&feature),
            "missing prefix sibling: {results:?}"
        );
        assert!(
            results.contains(&hidden),
            "missing hidden contains match: {results:?}"
        );
        assert!(
            results.contains(&contains),
            "missing contains match: {results:?}"
        );

        let exact_index = result_index(&results, &exact);
        assert!(exact_index < result_index(&results, &cloud));
        assert!(exact_index < result_index(&results, &feature));
        assert!(result_index(&results, &cloud) < result_index(&results, &hidden));
        assert!(result_index(&results, &feature) < result_index(&results, &hidden));
        assert!(result_index(&results, &contains) < result_index(&results, &hidden));
    }

    #[test]
    fn directory_completion_lists_children_only_after_trailing_separator() {
        let root = unique_test_dir("workspace-directory-completion-children");
        create_test_dir(root.join("chariox").join("child"));
        create_test_dir(root.join("chariox-cloud"));

        let query = format!("{}/", root.join("chariox").display());
        let results = search_workspace_directories(&query, 20, None)
            .expect("workspace directory search should succeed");
        remove_test_dir(&root);

        assert!(
            results.contains(&root.join("chariox").join("child").display().to_string()),
            "missing child directory: {results:?}",
        );
        assert!(
            !results.contains(&root.join("chariox-cloud").display().to_string()),
            "trailing slash should not include siblings: {results:?}",
        );
    }

    #[test]
    fn exact_directory_completion_survives_permission_denied_children() {
        let root = PathBuf::from("/home");
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();
        super::push_unique_path(&mut results, &mut seen, root.display().to_string());

        let denied: io::Result<Vec<io::Result<PathBuf>>> = Ok(vec![Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "child directory entry denied",
        ))]);
        append_matching_directory_children_from_result(&mut results, &mut seen, "", 12, denied)
            .expect("permission denied child should be a best-effort completion result");
        assert_eq!(results, vec!["/home"]);

        let denied: io::Result<Vec<io::Result<PathBuf>>> = Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "directory enumeration denied",
        ));
        append_matching_directory_children_from_result(&mut results, &mut seen, "", 12, denied)
            .expect("permission denied parent should be a best-effort completion result");
        assert_eq!(results, vec!["/home"]);
    }

    #[cfg(unix)]
    #[test]
    fn exact_path_discovery_accepts_home_tmp_nested_and_post_enrollment_repo() {
        crate::test_support::isolated_env_test!();
        let _env = crate::env_lock::lock();
        let root = unique_test_dir("workspace-search-path1-exact-paths");
        let home = root.join("user-home");
        let chariox_home = home.join(".chariox");
        create_test_dir(chariox_home.clone());

        let previous_home = std::env::var_os("HOME");
        let previous_chariox_home = std::env::var_os("CHARIOX_HOME");
        let previous_managed_root =
            std::env::var_os(crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV);
        let previous_isolation = std::env::var_os("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        let previous_topology = std::env::var_os("CHARIOX_MANAGED_PROVIDER_TOPOLOGY");
        let protected_names = [
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            "CHARIOX_MANAGED_SLICE_SERVICE_ROOT",
            "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT",
            "CHARIOX_MANAGED_PROVIDER_HOME",
            "CHARIOX_MANAGED_VAULT_PATH",
            "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
            "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
            "CHARIOX_MANAGED_BOOTSTRAP_PATH",
            "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
            "CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH",
            "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
            "CHARIOX_DAEMON_SOCKET",
        ];
        let previous_protected = protected_names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();
        std::env::set_var("HOME", &home);
        std::env::set_var("CHARIOX_HOME", &chariox_home);
        std::env::remove_var(crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV);
        std::env::remove_var("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        std::env::remove_var("CHARIOX_MANAGED_PROVIDER_TOPOLOGY");
        for name in protected_names {
            std::env::remove_var(name);
        }

        // Treat the configured user/kernel homes as the enrolled state, then
        // create ordinary nested directories and a repository afterward.
        let nested = home.join("projects").join("nested").join("new-directory");
        create_test_dir(nested.clone());
        let repository = home.join("projects").join("post-enrollment-repository");
        create_test_dir(repository.clone());
        let output = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repository)
            .output()
            .expect("test Git repository should initialize");
        assert!(
            output.status.success(),
            "test Git initialization failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        for (mode, topology) in [("ordinary", None), ("path1", Some("path1"))] {
            if let Some(topology) = topology {
                std::env::set_var("CHARIOX_MANAGED_PROVIDER_TOPOLOGY", topology);
            } else {
                std::env::remove_var("CHARIOX_MANAGED_PROVIDER_TOPOLOGY");
            }

            for system_path in [PathBuf::from("/home"), PathBuf::from("/tmp")] {
                if !system_path.is_dir() {
                    continue;
                }
                let exact = system_path.display().to_string();
                let results = search_workspace_directories(&exact, 32, None)
                    .expect("system exact-path discovery should succeed");
                assert!(
                    results.contains(&exact),
                    "{mode} discovery should retain {exact}: {results:?}"
                );
            }

            for path in [&nested, &repository] {
                let exact = path.display().to_string();
                let results = search_workspace_directories(&exact, 32, None)
                    .expect("nested exact-path discovery should succeed");
                assert!(
                    results.contains(&exact),
                    "{mode} discovery should retain post-enrollment path {exact}: {results:?}"
                );
            }

            let created = home
                .join("projects")
                .join(format!("{mode}-created-after-enrollment"))
                .join("nested");
            let created_path = created.display().to_string();
            assert_eq!(
                super::create_workspace_directory(&created_path)
                    .expect("ordinary workspace directory creation should succeed"),
                created_path
            );
            let results = search_workspace_directories(&created_path, 32, None)
                .expect("created exact-path discovery should succeed");
            assert!(
                results.contains(&created_path),
                "{mode} discovery should retain a newly created path: {results:?}"
            );
        }

        restore_env("HOME", previous_home);
        restore_env("CHARIOX_HOME", previous_chariox_home);
        restore_env(
            crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV,
            previous_managed_root,
        );
        restore_env("CHARIOX_MANAGED_PROVIDER_ISOLATION", previous_isolation);
        restore_env("CHARIOX_MANAGED_PROVIDER_TOPOLOGY", previous_topology);
        for (name, value) in previous_protected {
            restore_env(name, value);
        }
        remove_test_dir(&root);
    }

    #[test]
    fn configured_managed_root_is_discovered_outside_home_and_current_directory() {
        let _env = crate::env_lock::lock();
        let root = unique_test_dir("workspace-search-configured-managed-root");
        let home = root.join("user-home");
        let kernel_home = root.join("kernel-state").join(".chariox");
        let managed_root = root.join("managed-repositories");
        let project = managed_root.join("configured-project");
        create_test_dir(home.clone());
        create_test_dir(kernel_home.clone());
        create_test_dir(project.clone());

        let previous_home = std::env::var_os("HOME");
        let previous_chariox_home = std::env::var_os("CHARIOX_HOME");
        let previous_managed_root =
            std::env::var_os(crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV);
        let protected_names = [
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            "CHARIOX_MANAGED_SLICE_SERVICE_ROOT",
            "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT",
            "CHARIOX_MANAGED_PROVIDER_HOME",
            "CHARIOX_MANAGED_VAULT_PATH",
            "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
            "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
            "CHARIOX_MANAGED_BOOTSTRAP_PATH",
            "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
            "CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH",
            "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
            "CHARIOX_DAEMON_SOCKET",
        ];
        let previous_protected = protected_names
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect::<Vec<_>>();
        for name in protected_names {
            std::env::remove_var(name);
        }
        std::env::set_var("HOME", &home);
        std::env::set_var("CHARIOX_HOME", &kernel_home);
        std::env::set_var(
            crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV,
            &managed_root,
        );

        let empty = search_workspace_directories("", usize::MAX, None);
        let named = search_workspace_directories("configured-project", 256, None);
        let configured_roots = super::workspace_search_roots();
        let canonical_managed_root = managed_root
            .canonicalize()
            .expect("configured managed root should canonicalize");
        let canonical_project = canonical_managed_root.join("configured-project");
        std::env::remove_var(crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV);
        let ordinary_roots = super::workspace_search_roots();

        restore_env("HOME", previous_home);
        restore_env("CHARIOX_HOME", previous_chariox_home);
        restore_env(
            crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV,
            previous_managed_root,
        );
        for (name, value) in previous_protected {
            restore_env(name, value);
        }
        remove_test_dir(&root);

        let managed_root_text = canonical_managed_root.display().to_string();
        let project_text = canonical_project.display().to_string();
        let empty = empty.expect("empty workspace discovery should include configured root");
        let named = named.expect("name discovery should include configured-root children");
        let configured_roots = configured_roots.expect("configured roots should resolve");
        assert_eq!(
            configured_roots.first(),
            Some(&canonical_managed_root),
            "the explicit root should be first so bounded empty discovery reaches it"
        );
        assert!(empty.contains(&managed_root_text), "{empty:?}");
        assert!(empty.contains(&project_text), "{empty:?}");
        assert!(named.contains(&project_text), "{named:?}");
        assert_eq!(
            ordinary_roots.expect("ordinary workspace roots should resolve"),
            vec![
                std::env::current_dir().expect("current directory should resolve"),
                home
            ],
            "an unset managed root must preserve the ordinary cwd/HOME roots"
        );
    }

    #[cfg(unix)]
    #[test]
    fn invalid_configured_managed_root_fails_workspace_discovery() {
        let _env = crate::env_lock::lock();
        let previous = std::env::var_os(crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV);
        std::env::set_var(
            crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV,
            "relative/managed-root",
        );

        let absolute = std::env::temp_dir().display().to_string();
        let absolute_result = search_workspace_directories(&absolute, 32, None);
        let result = search_workspace_directories("known", 32, None);

        restore_env(
            crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV,
            previous,
        );
        assert!(
            absolute_result
                .expect("absolute path discovery should not depend on search roots")
                .contains(&absolute),
            "absolute path discovery should remain independent of configured roots"
        );
        assert!(
            result.is_err(),
            "invalid trusted-root configuration must fail rather than silently fall back"
        );
    }

    #[test]
    fn empty_query_filters_protected_workspace_children() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let root = unique_test_dir("workspace-search-protected-children");
        let home = root.join("home");
        let protected = home.join(".chariox");
        let ordinary = home.join("workspace");
        create_test_dir(protected.join("state"));
        create_test_dir(ordinary.clone());

        let previous_home = std::env::var_os("HOME");
        let previous_chariox_home = std::env::var_os("CHARIOX_HOME");
        let previous_managed_root =
            std::env::var_os(crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV);
        std::env::set_var("HOME", &home);
        std::env::remove_var("CHARIOX_HOME");
        std::env::remove_var(crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV);

        let results = search_workspace_directories("", 100, None)
            .expect("empty workspace search should succeed");

        restore_env("HOME", previous_home);
        restore_env("CHARIOX_HOME", previous_chariox_home);
        restore_env(
            crate::managed_bootstrap::MANAGED_REPOSITORY_ROOT_ENV,
            previous_managed_root,
        );
        remove_test_dir(&root);

        assert!(
            results.contains(&home.display().to_string()),
            "search should retain the exact accessible root: {results:?}"
        );
        assert!(
            results.contains(&ordinary.display().to_string()),
            "search should retain an ordinary child: {results:?}"
        );
        assert!(
            !results.contains(&protected.display().to_string()),
            "search must filter protected children: {results:?}"
        );
    }

    #[test]
    fn directory_completion_prioritizes_hidden_dirs_when_query_starts_hidden() {
        crate::test_support::isolated_env_test!();
        let root = unique_test_dir("workspace-directory-completion-hidden");
        create_test_dir(root.join(".chariox"));
        create_test_dir(root.join(".chariox-cache"));
        create_test_dir(root.join("my-.chariox"));

        let results =
            search_workspace_directories(&root.join(".chariox").display().to_string(), 20, None)
                .expect("workspace directory search should succeed");
        remove_test_dir(&root);

        let exact = root.join(".chariox").display().to_string();
        let hidden_prefix = root.join(".chariox-cache").display().to_string();
        let contains = root.join("my-.chariox").display().to_string();
        assert!(result_index(&results, &exact) < result_index(&results, &hidden_prefix));
        assert!(result_index(&results, &hidden_prefix) < result_index(&results, &contains));
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("chariox-{label}-{}-{nanos}", std::process::id()))
    }

    fn create_test_dir(path: PathBuf) {
        fs::create_dir_all(path).expect("test directory should be created");
    }

    fn remove_test_dir(path: &PathBuf) {
        let _ = fs::remove_dir_all(path);
    }

    fn result_index(results: &[String], value: &str) -> usize {
        results
            .iter()
            .position(|result| result == value)
            .unwrap_or_else(|| panic!("missing {value} in {results:?}"))
    }

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}
