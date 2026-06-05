use crate::error::DaemonError;
use std::collections::BTreeMap;
#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::path::PathBuf;

use super::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun};

pub(crate) const WORKSPACE_WRITE_FENCE_ENV: &str = "ARROBA_WORKSPACE_WRITE_FENCE";
pub(crate) const MACOS_SEATBELT_BACKEND: &str = "macos-seatbelt";
pub(crate) const WORKSPACE_WRITE_FENCE_UNSUPPORTED_REASON: &str = "workspace live sync managed mode needs selective write fencing, which is only implemented on macOS; use tracked mode on this worker or run the managed provider on a supported host";

pub(crate) fn workspace_write_fence_supported() -> bool {
    cfg!(target_os = "macos")
}

pub(crate) fn workspace_write_fence_backend() -> Option<&'static str> {
    workspace_write_fence_supported().then_some(MACOS_SEATBELT_BACKEND)
}

pub(crate) fn workspace_write_fence_unavailable_reason() -> Option<&'static str> {
    (!workspace_write_fence_supported()).then_some(WORKSPACE_WRITE_FENCE_UNSUPPORTED_REASON)
}

pub(crate) fn apply_workspace_write_fence(
    mut launch: ProviderLaunchResult,
    request: &LaunchProviderRequest,
) -> Result<ProviderLaunchResult, DaemonError> {
    if !request.requires_workspace_live_sync() || launch.endpoint_mode != AgentEndpointMode::Managed
    {
        return Ok(launch);
    }

    let protected_roots = if request.workspace_live_sync_roots.is_empty() {
        let Some(workspace_root) = launch.working_directory.clone() else {
            return Err(DaemonError::LocalTransport {
                operation: "workspace_write_fence",
                message: "workspace live sync provider runs require a workspace working directory"
                    .to_string(),
            });
        };
        vec![workspace_root]
    } else {
        request.workspace_live_sync_roots.clone()
    };
    if protected_roots.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace_write_fence",
            message: "workspace live sync provider runs require at least one protected root"
                .to_string(),
        });
    };
    let Some(program) = launch.pty_program.clone() else {
        return Err(DaemonError::LocalTransport {
            operation: "workspace_write_fence",
            message: "workspace live sync provider runs require an Arroba-owned provider process"
                .to_string(),
        });
    };

    apply_platform_workspace_write_fence(&mut launch, request, &protected_roots, program)?;
    Ok(launch)
}

pub(crate) fn workspace_write_fence_active(run: &RuntimeProviderRun) -> bool {
    workspace_write_fence_active_env(run.pty_env())
}

pub(crate) fn workspace_write_fence_active_env(env: &BTreeMap<String, String>) -> bool {
    env.get(WORKSPACE_WRITE_FENCE_ENV)
        .is_some_and(|value| value == MACOS_SEATBELT_BACKEND)
}

#[cfg(target_os = "macos")]
fn apply_platform_workspace_write_fence(
    launch: &mut ProviderLaunchResult,
    request: &LaunchProviderRequest,
    protected_roots: &[PathBuf],
    program: String,
) -> Result<(), DaemonError> {
    let sandbox_exec = Path::new("/usr/bin/sandbox-exec");
    if !sandbox_exec.exists() {
        return Err(DaemonError::LocalTransport {
            operation: "workspace_write_fence",
            message: "macOS workspace write fence requires /usr/bin/sandbox-exec".to_string(),
        });
    }

    let canonical_roots = canonical_workspace_live_sync_roots(protected_roots)?;
    let exception_roots = nested_git_repository_exception_roots(&canonical_roots);
    let profile_path = write_macos_seatbelt_profile(request, &canonical_roots, &exception_roots)?;

    let mut wrapped_args = vec![
        "-f".to_string(),
        profile_path.display().to_string(),
        program,
    ];
    wrapped_args.append(&mut launch.pty_args);

    launch.pty_program = Some(sandbox_exec.display().to_string());
    launch.pty_args = wrapped_args;
    launch.pty_env.insert(
        WORKSPACE_WRITE_FENCE_ENV.to_string(),
        MACOS_SEATBELT_BACKEND.to_string(),
    );
    launch.process_label = format!("{}:fenced", launch.process_label);
    Ok(())
}

#[cfg(target_os = "macos")]
fn canonical_workspace_live_sync_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>, DaemonError> {
    let mut canonical_roots = Vec::new();
    for root in roots {
        let canonical = root
            .canonicalize()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "workspace_write_fence",
                message: format!(
                    "failed to canonicalize workspace `{}`: {error}",
                    root.display()
                ),
            })?;
        if !canonical_roots.contains(&canonical) {
            canonical_roots.push(canonical);
        }
    }
    Ok(canonical_roots)
}

#[cfg(not(target_os = "macos"))]
fn apply_platform_workspace_write_fence(
    launch: &mut ProviderLaunchResult,
    _request: &LaunchProviderRequest,
    _protected_roots: &[PathBuf],
    _program: String,
) -> Result<(), DaemonError> {
    let _ = launch;
    Err(DaemonError::LocalTransport {
        operation: "workspace_write_fence",
        message: WORKSPACE_WRITE_FENCE_UNSUPPORTED_REASON.to_string(),
    })
}

#[cfg(target_os = "macos")]
fn write_macos_seatbelt_profile(
    request: &LaunchProviderRequest,
    canonical_roots: &[PathBuf],
    exception_roots: &[PathBuf],
) -> Result<PathBuf, DaemonError> {
    let profile_dir = std::env::temp_dir().join("arroba-workspace-write-fences");
    fs::create_dir_all(&profile_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "workspace_write_fence",
        message: format!(
            "failed to create workspace write fence profile directory `{}`: {error}",
            profile_dir.display()
        ),
    })?;
    let profile_path = profile_dir.join(format!(
        "workspace-{:x}.sb",
        workspace_fence_hash(request, canonical_roots)
    ));
    let profile = macos_seatbelt_profile(canonical_roots, exception_roots);
    fs::write(&profile_path, profile).map_err(|error| DaemonError::LocalTransport {
        operation: "workspace_write_fence",
        message: format!(
            "failed to write workspace write fence profile `{}`: {error}",
            profile_path.display()
        ),
    })?;
    Ok(profile_path)
}

#[cfg(target_os = "macos")]
fn workspace_fence_hash(request: &LaunchProviderRequest, canonical_roots: &[PathBuf]) -> u64 {
    let mut hasher = DefaultHasher::new();
    canonical_roots.hash(&mut hasher);
    request.session_id.hash(&mut hasher);
    request.agent_id.hash(&mut hasher);
    request.provider.hash(&mut hasher);
    request.model.hash(&mut hasher);
    hasher.finish()
}

fn macos_seatbelt_profile(canonical_roots: &[PathBuf], exception_roots: &[PathBuf]) -> String {
    let mut profile = "(version 1)\n".to_string();
    for root in canonical_roots {
        profile.push_str(&format!(
            "(deny file-write* (subpath \"{}\"))\n",
            seatbelt_string(root)
        ));
    }
    for root in exception_roots {
        profile.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            seatbelt_string(root)
        ));
    }
    profile.push_str("(allow default)\n");
    profile
}

#[cfg(target_os = "macos")]
fn nested_git_repository_exception_roots(canonical_roots: &[PathBuf]) -> Vec<PathBuf> {
    const MAX_SCANNED_DIRECTORIES: usize = 20_000;

    let mut exceptions = Vec::new();
    let mut scanned = 0_usize;
    for root in canonical_roots {
        collect_nested_git_repository_exception_roots(
            root,
            root,
            canonical_roots,
            &mut exceptions,
            &mut scanned,
            MAX_SCANNED_DIRECTORIES,
        );
    }
    exceptions.sort();
    exceptions.dedup();
    exceptions
}

#[cfg(target_os = "macos")]
fn collect_nested_git_repository_exception_roots(
    scan_root: &Path,
    directory: &Path,
    protected_roots: &[PathBuf],
    exceptions: &mut Vec<PathBuf>,
    scanned: &mut usize,
    max_scanned: usize,
) {
    if *scanned >= max_scanned {
        return;
    }
    *scanned += 1;

    let git_marker = directory.join(".git");
    if git_marker.exists() && directory != scan_root {
        if let Ok(canonical) = directory.canonicalize() {
            if protected_roots.iter().all(|root| root != &canonical)
                && !exceptions.iter().any(|root| root == &canonical)
            {
                exceptions.push(canonical);
            }
        }
        return;
    }

    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == ".git"
            || name == ".arroba"
            || name == "node_modules"
            || name == "target"
            || name == ".next"
            || name == "dist"
            || name == "build"
        {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_nested_git_repository_exception_roots(
                scan_root,
                &entry.path(),
                protected_roots,
                exceptions,
                scanned,
                max_scanned,
            );
            if *scanned >= max_scanned {
                return;
            }
        }
    }
}

fn seatbelt_string(path: &Path) -> String {
    let mut escaped = String::new();
    for ch in path.display().to_string().chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

    use super::{
        apply_workspace_write_fence, workspace_write_fence_active_env,
        workspace_write_fence_supported, MACOS_SEATBELT_BACKEND, WORKSPACE_WRITE_FENCE_ENV,
    };

    #[test]
    fn detects_active_workspace_write_fence_env() {
        let mut env = BTreeMap::new();
        assert!(!workspace_write_fence_active_env(&env));

        env.insert(
            WORKSPACE_WRITE_FENCE_ENV.to_string(),
            MACOS_SEATBELT_BACKEND.to_string(),
        );
        assert!(workspace_write_fence_active_env(&env));
    }

    #[test]
    fn unrestricted_launch_is_not_wrapped() {
        let request =
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "model");
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode:serve".to_string(),
            pty_target: None,
            pty_program: Some("/bin/echo".to_string()),
            pty_args: vec!["hello".to_string()],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some(std::env::temp_dir()),
            structured_endpoint: Some("http://127.0.0.1:1".to_string()),
        };

        let wrapped =
            apply_workspace_write_fence(launch.clone(), &request).expect("launch should resolve");

        assert_eq!(wrapped, launch);
    }

    #[test]
    fn workspace_write_fence_support_is_platform_explicit() {
        assert_eq!(workspace_write_fence_supported(), cfg!(target_os = "macos"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn managed_workspace_live_sync_fails_without_selective_write_fence() {
        let request =
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "model")
                .with_workspace_live_sync_managed()
                .with_working_directory(std::env::temp_dir());
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode:serve".to_string(),
            pty_target: None,
            pty_program: Some("/bin/echo".to_string()),
            pty_args: vec!["hello".to_string()],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some(std::env::temp_dir()),
            structured_endpoint: Some("http://127.0.0.1:1".to_string()),
        };

        let error =
            apply_workspace_write_fence(launch, &request).expect_err("managed fence should fail");

        assert!(error.to_string().contains("selective write fencing"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn workspace_live_sync_launch_is_wrapped_with_macos_seatbelt() {
        let workspace = std::env::temp_dir().join(format!(
            "arroba-workspace-write-fence-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace fixture should exist");
        let request =
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "model")
                .with_workspace_live_sync_managed()
                .with_working_directory(workspace.clone());
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode:serve".to_string(),
            pty_target: None,
            pty_program: Some("/bin/echo".to_string()),
            pty_args: vec!["hello".to_string()],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some(workspace.clone()),
            structured_endpoint: Some("http://127.0.0.1:1".to_string()),
        };

        let wrapped =
            apply_workspace_write_fence(launch, &request).expect("launch should be wrapped");

        let _ = std::fs::remove_dir_all(&workspace);
        assert_eq!(
            wrapped.pty_program.as_deref(),
            Some("/usr/bin/sandbox-exec")
        );
        assert_eq!(wrapped.pty_args[0], "-f");
        assert!(wrapped.pty_args[1].contains("arroba-workspace-write-fences"));
        assert_eq!(wrapped.pty_args[2], "/bin/echo");
        assert_eq!(wrapped.pty_args[3], "hello");
        assert_eq!(
            wrapped
                .pty_env
                .get(WORKSPACE_WRITE_FENCE_ENV)
                .map(String::as_str),
            Some(MACOS_SEATBELT_BACKEND)
        );
        assert!(wrapped.process_label.ends_with(":fenced"));
    }

    #[test]
    fn macos_profile_denies_each_protected_root_only() {
        let profile = super::macos_seatbelt_profile(
            &[
                PathBuf::from("/repo/selected"),
                PathBuf::from("/repo/attached"),
            ],
            &[],
        );

        assert!(profile.contains("(deny file-write* (subpath \"/repo/selected\"))"));
        assert!(profile.contains("(deny file-write* (subpath \"/repo/attached\"))"));
        assert!(!profile.contains("(subpath \"/repo\")"));
        assert!(profile.ends_with("(allow default)\n"));
    }

    #[test]
    fn macos_profile_allows_nested_unsynced_git_repo_exceptions() {
        let profile = super::macos_seatbelt_profile(
            &[PathBuf::from("/repo/selected")],
            &[PathBuf::from("/repo/selected/other-repo")],
        );

        assert!(profile.contains("(deny file-write* (subpath \"/repo/selected\"))"));
        assert!(profile.contains("(allow file-write* (subpath \"/repo/selected/other-repo\"))"));
        assert!(profile.ends_with("(allow default)\n"));
    }

    #[test]
    fn macos_profile_escapes_protected_roots() {
        let profile = super::macos_seatbelt_profile(&[PathBuf::from("/repo/quoted \"root\"")], &[]);

        assert!(profile.contains("(deny file-write* (subpath \"/repo/quoted \\\"root\\\"\"))"));
        assert!(profile.ends_with("(allow default)\n"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_workspace_write_fence_blocks_only_protected_roots_at_runtime() {
        let base = std::env::temp_dir().join(format!(
            "arroba-workspace-write-fence-runtime-test-{}",
            std::process::id()
        ));
        let protected_root = base.join("selected");
        let outside_root = base.join("sibling");
        std::fs::create_dir_all(&protected_root).expect("protected fixture should exist");
        std::fs::create_dir_all(&outside_root).expect("outside fixture should exist");

        let outside_launch =
            fenced_shell_launch(&protected_root, &outside_root, "printf ok > outside.txt");
        let outside = std::process::Command::new(outside_launch.pty_program.as_deref().unwrap())
            .args(&outside_launch.pty_args)
            .current_dir(&outside_root)
            .envs(&outside_launch.pty_env)
            .output()
            .expect("outside write process should launch");

        let protected_launch = fenced_shell_launch(
            &protected_root,
            &outside_root,
            &format!(
                "printf denied > {}",
                protected_root.join("inside.txt").display()
            ),
        );
        let protected =
            std::process::Command::new(protected_launch.pty_program.as_deref().unwrap())
                .args(&protected_launch.pty_args)
                .current_dir(&outside_root)
                .envs(&protected_launch.pty_env)
                .output()
                .expect("protected write process should launch");

        let _ = std::fs::remove_dir_all(&base);

        assert!(
            outside.status.success(),
            "outside write should succeed: stderr={}",
            String::from_utf8_lossy(&outside.stderr)
        );
        assert!(
            !protected.status.success(),
            "protected write should fail inside selected root"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_workspace_write_fence_allows_nested_unsynced_git_repo_at_runtime() {
        let base = std::env::temp_dir().join(format!(
            "arroba-workspace-write-fence-nested-repo-test-{}",
            std::process::id()
        ));
        let protected_root = base.join("selected");
        let nested_repo = protected_root.join("other-repo");
        std::fs::create_dir_all(&nested_repo).expect("nested repo fixture should exist");
        run_git_init(&nested_repo);

        let outside_launch = fenced_shell_launch(
            &protected_root,
            &nested_repo,
            "printf ok > nested-write.txt",
        );
        let nested = std::process::Command::new(outside_launch.pty_program.as_deref().unwrap())
            .args(&outside_launch.pty_args)
            .current_dir(&nested_repo)
            .envs(&outside_launch.pty_env)
            .output()
            .expect("nested write process should launch");

        let protected_launch = fenced_shell_launch(
            &protected_root,
            &nested_repo,
            &format!(
                "printf denied > {}",
                protected_root.join("inside-selected.txt").display()
            ),
        );
        let protected =
            std::process::Command::new(protected_launch.pty_program.as_deref().unwrap())
                .args(&protected_launch.pty_args)
                .current_dir(&nested_repo)
                .envs(&protected_launch.pty_env)
                .output()
                .expect("protected write process should launch");

        let _ = std::fs::remove_dir_all(&base);

        assert!(
            nested.status.success(),
            "nested git repo write should succeed: stderr={}",
            String::from_utf8_lossy(&nested.stderr)
        );
        assert!(
            !protected.status.success(),
            "selected root write should stay blocked"
        );
    }

    #[cfg(target_os = "macos")]
    fn fenced_shell_launch(
        protected_root: &std::path::Path,
        working_directory: &std::path::Path,
        command: &str,
    ) -> ProviderLaunchResult {
        let request =
            LaunchProviderRequest::new("session-1", "opencode", "opencode", "default", "model")
                .with_workspace_live_sync_managed()
                .with_working_directory(working_directory.to_path_buf())
                .with_workspace_live_sync_roots(vec![protected_root.to_path_buf()]);
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "opencode:serve".to_string(),
            pty_target: None,
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec!["-c".to_string(), command.to_string()],
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: Some(working_directory.to_path_buf()),
            structured_endpoint: Some("http://127.0.0.1:1".to_string()),
        };
        apply_workspace_write_fence(launch, &request).expect("launch should be fenced")
    }

    #[cfg(target_os = "macos")]
    fn run_git_init(path: &std::path::Path) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("init")
            .arg("-b")
            .arg("main")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git init should run");
        assert!(
            status.success(),
            "git init should succeed in {}",
            path.display()
        );
    }
}
