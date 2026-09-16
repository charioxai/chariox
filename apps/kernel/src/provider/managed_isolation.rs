use std::collections::BTreeMap;
#[cfg(any(target_os = "linux", test))]
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use crate::error::DaemonError;

use super::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun};

pub(crate) const MANAGED_PROVIDER_ISOLATION_ENV: &str = "CHARIOX_MANAGED_PROVIDER_ISOLATION";
pub(crate) const MANAGED_SLICE_SERVICE_ROOT_ENV: &str = "CHARIOX_MANAGED_SLICE_SERVICE_ROOT";
pub(crate) const MANAGED_SLICE_PUBLICATION_ROOT_ENV: &str =
    "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT";
#[cfg(any(target_os = "linux", test))]
const CLAUDE_SANDBOX_ENV: &str = "IS_SANDBOX";
#[cfg(target_os = "linux")]
pub(crate) const MANAGED_PROVIDER_HOME_ENV: &str = "CHARIOX_MANAGED_PROVIDER_HOME";
pub(crate) const MANAGED_PROVIDER_ISOLATION_MARKER_ENV: &str =
    "CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE";
const MANAGED_WORKSPACE_ROOT_COUNT_ENV: &str = "CHARIOX_MANAGED_WORKSPACE_ROOT_COUNT";
const MANAGED_WORKSPACE_ROOT_ENV_PREFIX: &str = "CHARIOX_MANAGED_WORKSPACE_ROOT_";
#[cfg(any(target_os = "linux", test))]
const MAX_MANAGED_WORKSPACE_ROOTS: usize = 128;

#[cfg(target_os = "linux")]
const BWRAP_PATH: &str = "/usr/bin/bwrap";
#[cfg(target_os = "linux")]
const MANAGED_PROVIDER_BWRAP_ENV: &str = "CHARIOX_MANAGED_PROVIDER_BWRAP";
#[cfg(any(target_os = "linux", test))]
const SANDBOX_HOME: &str = "/home/chariox";
const SANDBOX_ACCOUNT_ROOT: &str = "/home/chariox/.provider-account";

const CONTROL_ENVIRONMENT_NAMES: &[&str] = &[
    "CHARIOX_HOME",
    "CHARIOX_CAPABILITY_ISOLATION_ROOT",
    "CHARIOX_RELAY_TOKEN",
    "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
    "CHARIOX_CLOUD_RELAY_CONFIG_PATH",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
    "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
    "CHARIOX_SLICE_DOCKER_BROKER_FD",
    "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED",
    "CHARIOX_MANAGED_BOOTSTRAP_PATH",
    "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
    "CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH",
    "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
    "CHARIOX_MANAGED_PROVIDER_BWRAP",
    "CHARIOX_MANAGED_PROVIDER_HOME",
    "CHARIOX_MANAGED_PROVIDER_ISOLATION",
    "CHARIOX_MANAGED_VAULT_PATH",
    "CHARIOX_SLICE_ROOT",
    MANAGED_SLICE_SERVICE_ROOT_ENV,
    MANAGED_SLICE_PUBLICATION_ROOT_ENV,
    "CHARIOX_MANAGED_RELEASE_SIGNATURE",
    "CHARIOX_MANAGED_RELEASE_PUBLIC_KEY",
];

#[cfg(target_os = "linux")]
const PROVIDER_ACCOUNT_PATH_ENVIRONMENT: &[&str] = &[
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "OPENCODE_CONFIG_DIR",
];

pub(crate) fn managed_provider_isolation_required() -> bool {
    std::env::var(MANAGED_PROVIDER_ISOLATION_ENV)
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

pub(crate) fn managed_provider_control_env_remove() -> Vec<String> {
    let mut names = CONTROL_ENVIRONMENT_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    names.extend([
        MANAGED_WORKSPACE_ROOT_COUNT_ENV.to_string(),
        "GIT_CONFIG_COUNT".to_string(),
        "GIT_CONFIG_PARAMETERS".to_string(),
    ]);
    names.extend(std::env::vars_os().filter_map(|(name, _)| {
        let name = name.into_string().ok()?;
        (name.starts_with(MANAGED_WORKSPACE_ROOT_ENV_PREFIX)
            || name.starts_with("GIT_CONFIG_KEY_")
            || name.starts_with("GIT_CONFIG_VALUE_"))
        .then_some(name)
    }));
    names.sort();
    names.dedup();
    names
}

pub(crate) fn provider_reported_path_on_kernel(
    run: &RuntimeProviderRun,
    reported_path: &str,
) -> Option<PathBuf> {
    let reported_path = Path::new(reported_path);
    let managed = run
        .pty_args()
        .windows(3)
        .any(|args| args == ["--setenv", MANAGED_PROVIDER_ISOLATION_MARKER_ENV, "1"]);
    if !managed {
        return Some(reported_path.to_path_buf());
    }

    let args = run.pty_args();
    let separator = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    let (source, destination) = args[..separator]
        .windows(3)
        .filter(|args| matches!(args[0].as_str(), "--bind" | "--ro-bind"))
        .filter_map(|args| {
            let source = PathBuf::from(&args[1]);
            let destination = PathBuf::from(&args[2]);
            if !destination.starts_with(SANDBOX_ACCOUNT_ROOT) {
                return None;
            }
            if reported_path.starts_with(&destination) {
                Some((source, destination))
            } else if reported_path.starts_with(&source) {
                Some((source.clone(), source))
            } else {
                None
            }
        })
        .max_by_key(|(_, destination)| destination.components().count())?;
    let relative = reported_path.strip_prefix(&destination).ok()?;
    if relative.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return None;
    }
    let source = source.canonicalize().ok()?;
    let resolved = source.join(relative).canonicalize().ok()?;
    resolved.starts_with(&source).then_some(resolved)
}

#[cfg(any(target_os = "linux", test))]
fn managed_slice_workspace_roots() -> Result<Vec<PathBuf>, DaemonError> {
    // These are trusted mounts for workspace data that lives below a masked
    // service-state tree. They are not the provider's filesystem allowlist:
    // the managed namespace keeps the ordinary host filesystem mounted and
    // only uses these roots to re-expose transferred data after masking.
    let Some(raw_count) = std::env::var_os(MANAGED_WORKSPACE_ROOT_COUNT_ENV) else {
        return Ok(Vec::new());
    };
    let raw_count = raw_count
        .into_string()
        .map_err(|_| isolation_error("managed workspace root count must be valid UTF-8"))?;
    let count = raw_count
        .parse::<usize>()
        .map_err(|_| isolation_error("managed workspace root count must be an unsigned integer"))?;
    if count > MAX_MANAGED_WORKSPACE_ROOTS {
        return Err(isolation_error(format!(
            "managed workspace root count exceeds {MAX_MANAGED_WORKSPACE_ROOTS}"
        )));
    }
    let mut roots = Vec::with_capacity(count);
    for index in 0..count {
        let name = format!("{MANAGED_WORKSPACE_ROOT_ENV_PREFIX}{index}");
        let value = std::env::var_os(&name)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| isolation_error(format!("managed workspace root {index} is missing")))?;
        let root = PathBuf::from(value);
        if !root.is_absolute() {
            return Err(isolation_error(format!(
                "managed workspace root {index} must be absolute"
            )));
        }
        if roots.iter().all(|existing| existing != &root) {
            roots.push(root);
        }
    }
    Ok(roots)
}

#[cfg(any(target_os = "linux", test))]
fn managed_private_temp_roots(process_temp_root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/tmp"),
        PathBuf::from("/var/tmp"),
        process_temp_root.to_path_buf(),
    ];
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(any(target_os = "linux", test))]
fn managed_private_namespace_roots(process_temp_root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/run")];
    roots.extend(managed_private_temp_roots(process_temp_root));
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(any(target_os = "linux", test))]
fn managed_configured_slice_service_root() -> Option<PathBuf> {
    let configured = std::env::var_os(MANAGED_SLICE_SERVICE_ROOT_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)?;
    (configured.is_absolute() && configured != Path::new("/")).then_some(configured)
}

#[cfg(any(target_os = "linux", test))]
fn managed_configured_slice_publication_root() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os(MANAGED_SLICE_PUBLICATION_ROOT_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return (configured.is_absolute() && configured != Path::new("/")).then_some(configured);
    }

    // Keep the known host default safe if a pre-contract managed supervisor
    // starts this kernel without the explicit publication invariant. New
    // relocated deployments must set MANAGED_SLICE_PUBLICATION_ROOT_ENV;
    // transport socket topology is deliberately not policy here.
    let slice_root = std::env::var_os("CHARIOX_SLICE_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)?;
    (slice_root == Path::new("/var/lib/chariox-slice-share/slices")).then_some(slice_root)
}

#[cfg(target_os = "linux")]
fn managed_protected_namespace_directories(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("/run")];
    let chariox_home = std::env::var_os("CHARIOX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".chariox"))
        });

    if let Some(home) = chariox_home {
        if home.is_absolute() {
            if home.file_name() == Some(std::ffi::OsStr::new(".chariox")) {
                paths.push(home);
            } else {
                for relative in [
                    ".chariox",
                    "kernels",
                    "state",
                    "managed-context",
                    "managed-runtime-auth",
                    "managed",
                    "sessions",
                    "daemon",
                    "machine",
                ] {
                    paths.push(home.join(relative));
                }
            }
        }
    }

    for boundary in [
        managed_configured_slice_service_root(),
        managed_configured_slice_publication_root(),
    ]
    .into_iter()
    .flatten()
    {
        paths.push(boundary);
    }

    // Configured support roots are not a provider filesystem allowlist. The
    // ordinary root remains mounted, while trusted helper code is mounted
    // read-only below. The managed slice service boundary is handled above as
    // service state so only selected publication children are re-exposed.
    for name in [
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        "CHARIOX_MANAGED_PROVIDER_HOME",
    ] {
        if let Some(path) = std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            paths.push(path);
        }
    }
    for name in [
        "CHARIOX_MANAGED_VAULT_PATH",
        "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
        "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
        "CHARIOX_MANAGED_BOOTSTRAP_PATH",
        "CHARIOX_MANAGED_BOOTSTRAP_RECEIPT",
        "CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH",
        "CHARIOX_DISPOSABLE_WORKER_RECEIPT",
        "CHARIOX_DAEMON_SOCKET",
    ] {
        let Some(path) = std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        else {
            continue;
        };
        if let Some(parent) = path.parent().map(Path::to_path_buf) {
            paths.push(parent.clone());
            if name == "CHARIOX_SLICE_DOCKER_BROKER_SOCKET"
                && parent.file_name() == Some(std::ffi::OsStr::new("control"))
            {
                if let Some(private_root) = parent.parent() {
                    paths.push(private_root.to_path_buf());
                }
            }
        }
    }
    paths.extend(extra.iter().filter(|path| path.is_absolute()).cloned());
    paths.sort_by_key(|path| path.components().count());
    paths.dedup();
    let mut compact = Vec::with_capacity(paths.len());
    for path in paths {
        if path == Path::new("/") || compact.iter().any(|parent| path.starts_with(parent)) {
            continue;
        }
        compact.push(path);
    }
    compact
}

#[cfg(target_os = "linux")]
fn managed_trusted_read_only_paths() -> Result<Vec<PathBuf>, DaemonError> {
    let mut candidates = Vec::new();
    let default_slice_root = Path::new("/opt/chariox-slice");
    if default_slice_root.exists() {
        candidates.push(default_slice_root.to_path_buf());
    }
    if let Some(path) = std::env::var_os("CHARIOX_SLICE_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        if !path.is_absolute() {
            return Err(isolation_error("CHARIOX_SLICE_ROOT must be absolute"));
        }
        if managed_configured_slice_publication_root().as_deref() != Some(path.as_path()) {
            candidates.push(path);
        }
    }
    if let Some(path) = std::env::var_os(MANAGED_PROVIDER_BWRAP_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        if !path.is_absolute() {
            return Err(isolation_error(
                "CHARIOX_MANAGED_PROVIDER_BWRAP must be absolute",
            ));
        }
        candidates.push(path);
    }

    let mut paths = candidates
        .into_iter()
        .map(|path| {
            path.canonicalize().map_err(|error| {
                isolation_error(format!(
                    "managed trusted runtime path {} could not be resolved: {error}",
                    path.display()
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort_by_key(|path| path.components().count());
    paths.dedup();
    let mut compact = Vec::with_capacity(paths.len());
    for path in paths {
        if path == Path::new("/") || compact.iter().any(|parent| path.starts_with(parent)) {
            continue;
        }
        compact.push(path);
    }
    Ok(compact)
}

#[cfg(target_os = "linux")]
fn append_managed_protected_namespace_directories(
    args: &mut Vec<String>,
    directories: &[PathBuf],
    created: &mut BTreeSet<PathBuf>,
) {
    for directory in directories {
        // /run is masked while the root namespace is assembled, before
        // resolver and current-run runtime paths are re-exposed. Reapplying
        // a tmpfs here would hide those legitimate later bindings.
        if directory == Path::new("/run") {
            continue;
        }
        append_directory(args, directory, created);
        args.extend(["--tmpfs".to_string(), directory.display().to_string()]);
    }
}

#[cfg(target_os = "linux")]
fn append_managed_trusted_read_only_paths(
    args: &mut Vec<String>,
    paths: &[PathBuf],
    created: &mut BTreeSet<PathBuf>,
) {
    for path in paths {
        append_read_only_bind(args, path, path, created);
    }
}

pub(crate) fn apply_managed_provider_isolation(
    mut launch: ProviderLaunchResult,
    request: &LaunchProviderRequest,
) -> Result<ProviderLaunchResult, DaemonError> {
    if !managed_provider_isolation_required() {
        return Ok(launch);
    }

    let Some(program) = launch.pty_program.take() else {
        if launch.endpoint_mode == AgentEndpointMode::External {
            return Err(isolation_error(
                "managed kernels reject provider endpoints that are not launched inside the managed isolation boundary",
            ));
        }
        return Err(isolation_error(
            "managed provider launch did not expose an executable",
        ));
    };

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (program, request);
        Err(isolation_error(
            "managed provider isolation is only supported on Linux",
        ))
    }

    #[cfg(target_os = "linux")]
    {
        let bubblewrap = managed_bubblewrap_binary()?;
        let provider_home = managed_provider_home()?;
        let workspace_roots = managed_workspace_roots(request)?;
        let working_directory = managed_working_directory(request, &workspace_roots)?;
        let runtime_roots = managed_runtime_roots(&launch)?;
        let account_bindings = managed_account_bindings(&mut launch.pty_env)?;
        let program = rewrite_managed_program_path(&program, &provider_home, &account_bindings);
        let prompt_attachment_root = managed_prompt_attachment_root(request)?;
        let protected_namespace_roots = managed_protected_namespace_directories(&[]);
        let trusted_read_only_paths = managed_trusted_read_only_paths()?;

        let resolver = managed_resolver_binding()?;
        let (mut args, mut created_directories) = managed_namespace_args(
            resolver.as_deref(),
            Path::exists,
            prompt_attachment_root.as_deref(),
        );
        append_directory(&mut args, Path::new(SANDBOX_HOME), &mut created_directories);
        append_bind(
            &mut args,
            &provider_home,
            Path::new(SANDBOX_HOME),
            &mut created_directories,
        );
        append_directory(
            &mut args,
            Path::new(SANDBOX_ACCOUNT_ROOT),
            &mut created_directories,
        );
        for (source, destination) in &account_bindings {
            append_bind(&mut args, source, destination, &mut created_directories);
        }
        let mut protected_directories = protected_namespace_roots.clone();
        protected_directories.push(provider_home.clone());
        protected_directories.extend(account_bindings.iter().map(|(source, _)| source.clone()));
        append_managed_protected_namespace_directories(
            &mut args,
            &protected_directories,
            &mut created_directories,
        );
        append_managed_trusted_read_only_paths(
            &mut args,
            &trusted_read_only_paths,
            &mut created_directories,
        );
        for root in runtime_roots {
            append_bind(&mut args, &root, &root, &mut created_directories);
        }
        for root in &workspace_roots {
            if managed_workspace_root_requires_rebind(
                root,
                &protected_namespace_roots,
                &trusted_read_only_paths,
            ) {
                append_bind(&mut args, root, root, &mut created_directories);
            }
        }

        append_managed_namespace_environment(&mut args, request);
        let mut environment_remove = managed_provider_control_env_remove();
        environment_remove.extend(launch.pty_env.keys().filter_map(|name| {
            (name.starts_with("GIT_CONFIG_KEY_") || name.starts_with("GIT_CONFIG_VALUE_"))
                .then_some(name.clone())
        }));
        environment_remove.sort();
        environment_remove.dedup();
        for name in environment_remove {
            args.extend(["--unsetenv".to_string(), name.clone()]);
            if !launch.pty_env_remove.iter().any(|value| value == &name) {
                launch.pty_env_remove.push(name);
            }
        }
        append_managed_git_safe_directory_environment(&mut args, &workspace_roots);
        args.extend([
            "--chdir".to_string(),
            working_directory.display().to_string(),
            "--".to_string(),
            program,
        ]);
        args.append(&mut launch.pty_args);

        launch.pty_program = Some(bubblewrap.display().to_string());
        launch.pty_args = args;
        // Provider-account utilities use the synthetic sandbox home, which
        // does not exist until bubblewrap assembles the namespace. Ordinary
        // runs keep the real workspace as both the process cwd and the
        // provider protocol cwd so turns do not fall back to /tmp.
        launch.working_directory = Some(managed_launch_working_directory(
            request,
            &provider_home,
            &working_directory,
        ));
        launch.process_label = format!("{}:managed-isolated", launch.process_label);
        Ok(launch)
    }
}

#[cfg(any(target_os = "linux", test))]
fn managed_launch_working_directory(
    request: &LaunchProviderRequest,
    provider_home: &Path,
    sandbox_working_directory: &Path,
) -> PathBuf {
    if request.session_id == "provider-account" {
        provider_home.to_path_buf()
    } else {
        sandbox_working_directory.to_path_buf()
    }
}

#[cfg(any(target_os = "linux", test))]
fn append_managed_namespace_environment(args: &mut Vec<String>, request: &LaunchProviderRequest) {
    args.extend([
        "--setenv".to_string(),
        "HOME".to_string(),
        SANDBOX_HOME.to_string(),
        "--setenv".to_string(),
        "USER".to_string(),
        "chariox".to_string(),
        "--setenv".to_string(),
        "LOGNAME".to_string(),
        "chariox".to_string(),
        "--setenv".to_string(),
        "SHELL".to_string(),
        "/bin/sh".to_string(),
        "--setenv".to_string(),
        MANAGED_PROVIDER_ISOLATION_MARKER_ENV.to_string(),
        "1".to_string(),
    ]);
    if request.adapter_key == "claude" {
        args.extend([
            "--setenv".to_string(),
            CLAUDE_SANDBOX_ENV.to_string(),
            "1".to_string(),
        ]);
    } else {
        args.extend(["--unsetenv".to_string(), CLAUDE_SANDBOX_ENV.to_string()]);
    }
}

#[cfg(any(target_os = "linux", test))]
fn append_managed_git_safe_directory_environment(
    args: &mut Vec<String>,
    workspace_roots: &[PathBuf],
) {
    if workspace_roots.is_empty() {
        return;
    }
    let safe_directory_count = workspace_roots.len().saturating_mul(2);
    args.extend([
        "--setenv".to_string(),
        "GIT_CONFIG_COUNT".to_string(),
        safe_directory_count.saturating_add(1).to_string(),
        "--setenv".to_string(),
        "GIT_CONFIG_KEY_0".to_string(),
        "safe.directory".to_string(),
        "--setenv".to_string(),
        "GIT_CONFIG_VALUE_0".to_string(),
        String::new(),
    ]);
    for (index, root) in workspace_roots.iter().enumerate() {
        for (offset, value) in [root.display().to_string(), format!("{}/*", root.display())]
            .into_iter()
            .enumerate()
        {
            let index = index
                .saturating_mul(2)
                .saturating_add(offset)
                .saturating_add(1);
            args.extend([
                "--setenv".to_string(),
                format!("GIT_CONFIG_KEY_{index}"),
                "safe.directory".to_string(),
                "--setenv".to_string(),
                format!("GIT_CONFIG_VALUE_{index}"),
                value,
            ]);
        }
    }
}

pub(crate) fn expose_runtime_directory_in_managed_namespace(
    args: &mut Vec<String>,
    directory: &Path,
) -> Result<(), DaemonError> {
    let managed = args
        .windows(3)
        .any(|window| window == ["--setenv", MANAGED_PROVIDER_ISOLATION_MARKER_ENV, "1"]);
    if !managed {
        return Ok(());
    }
    if !directory.is_absolute() || directory.parent() != Some(std::env::temp_dir().as_path()) {
        return Err(isolation_error(
            "managed provider runtime directory must be a direct child of the process temp directory",
        ));
    }
    let separator = args.iter().position(|arg| arg == "--").ok_or_else(|| {
        isolation_error("managed provider launch is missing its command separator")
    })?;
    let path = directory.display().to_string();
    args.splice(
        separator..separator,
        [
            "--dir".to_string(),
            path.clone(),
            "--ro-bind".to_string(),
            path.clone(),
            path,
        ],
    );
    Ok(())
}

pub(crate) fn managed_isolated_utility_launch(
    program: impl Into<String>,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    working_directory: Option<PathBuf>,
    label: &str,
) -> Result<ProviderLaunchResult, DaemonError> {
    let mut request = LaunchProviderRequest::new(
        if working_directory.is_some() {
            "managed-utility"
        } else {
            "provider-account"
        },
        "managed-utility",
        "managed-utility",
        "default",
        "default",
    );
    if let Some(directory) = working_directory.clone() {
        request = request.with_working_directory(directory);
    }
    let launch = ProviderLaunchResult {
        endpoint_mode: AgentEndpointMode::Managed,
        process_label: label.to_string(),
        pty_target: None,
        pty_program: Some(program.into()),
        pty_args: args,
        pty_env: environment,
        pty_env_remove: managed_provider_control_env_remove(),
        working_directory,
        structured_endpoint: None,
    };
    apply_managed_provider_isolation(launch, &request)
}

pub(crate) fn managed_isolated_utility_command(
    program: impl Into<String>,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    working_directory: Option<PathBuf>,
    label: &str,
) -> Result<Command, DaemonError> {
    let launch =
        managed_isolated_utility_launch(program, args, environment, working_directory, label)?;
    command_from_provider_launch(launch)
}

pub(crate) fn command_from_provider_launch(
    launch: ProviderLaunchResult,
) -> Result<Command, DaemonError> {
    let program = launch
        .pty_program
        .ok_or_else(|| isolation_error("managed utility launch did not expose an executable"))?;
    let mut command = Command::new(program);
    command.args(launch.pty_args);
    for (name, _) in std::env::vars() {
        if crate::secret::secret_like_env_name(&name) {
            command.env_remove(name);
        }
    }
    for name in launch.pty_env_remove {
        command.env_remove(name);
    }
    command.envs(launch.pty_env);
    if let Some(directory) = launch.working_directory {
        command.current_dir(directory);
    }
    Ok(command)
}

#[cfg(target_os = "linux")]
fn managed_bubblewrap_binary() -> Result<PathBuf, DaemonError> {
    use std::os::unix::fs::MetadataExt;

    let path = std::env::var_os(MANAGED_PROVIDER_BWRAP_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(BWRAP_PATH));
    if !path.is_absolute() {
        return Err(isolation_error(
            "managed provider isolation needs an absolute Bubblewrap launcher path",
        ));
    }
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
        isolation_error(format!(
            "managed provider isolation needs {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
    {
        return Err(isolation_error(
            "managed provider isolation needs a root-owned, non-writable Bubblewrap launcher",
        ));
    }
    Ok(path)
}

#[cfg(target_os = "linux")]
fn managed_provider_home() -> Result<PathBuf, DaemonError> {
    let home = std::env::var_os(MANAGED_PROVIDER_HOME_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| isolation_error("managed provider HOME is not configured"))?;
    canonical_directory(&home, "managed provider HOME")
}

#[cfg(target_os = "linux")]
fn managed_resolver_binding() -> Result<Option<PathBuf>, DaemonError> {
    let resolver = Path::new("/etc/resolv.conf");
    let target = resolver.canonicalize().map_err(|error| {
        isolation_error(format!(
            "managed provider isolation cannot resolve /etc/resolv.conf: {error}"
        ))
    })?;
    let metadata = std::fs::metadata(&target).map_err(|error| {
        isolation_error(format!(
            "managed provider isolation cannot inspect resolver target: {error}"
        ))
    })?;
    if !metadata.is_file() {
        return Err(isolation_error(
            "managed provider resolver target must be a regular file",
        ));
    }
    Ok(target.starts_with("/run").then_some(target))
}

#[cfg(any(target_os = "linux", test))]
fn managed_namespace_args(
    resolver: Option<&Path>,
    path_exists: impl Fn(&Path) -> bool,
    prompt_attachment_root: Option<&Path>,
) -> (Vec<String>, BTreeSet<PathBuf>) {
    let process_temp_root = std::env::temp_dir();
    managed_namespace_args_with_process_temp_root(
        resolver,
        path_exists,
        prompt_attachment_root,
        &process_temp_root,
    )
}

#[cfg(any(target_os = "linux", test))]
fn managed_namespace_args_with_process_temp_root(
    resolver: Option<&Path>,
    path_exists: impl Fn(&Path) -> bool,
    prompt_attachment_root: Option<&Path>,
    process_temp_root: &Path,
) -> (Vec<String>, BTreeSet<PathBuf>) {
    let mut args = vec![
        "--die-with-parent".to_string(),
        "--new-session".to_string(),
        "--unshare-user".to_string(),
        "--unshare-pid".to_string(),
        "--unshare-ipc".to_string(),
        "--unshare-uts".to_string(),
        "--unshare-cgroup-try".to_string(),
        "--uid".to_string(),
        "0".to_string(),
        "--gid".to_string(),
        "0".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
        // Managed isolation must preserve ordinary filesystem permission
        // checks. Protected service paths are masked separately below; the
        // rest of the host/container root remains available to the provider.
        "--bind".to_string(),
        "/".to_string(),
        "/".to_string(),
    ];
    // The ordinary root remains available so providers retain the host's
    // normal filesystem-permission behavior. The kernel's process temp roots
    // (including the configured std::env::temp_dir()) are the exception:
    // every managed provider runs with the same outer service uid, so shared
    // temp roots would let sibling runs read or mutate private runtime files
    // despite their 0700/0600 modes. Current-run runtime and prompt-
    // attachment directories are rebound below.
    // GNU Screen and other service-owned control sockets live below /run. It
    // is re-exposed only through explicit resolver/runtime bindings added
    // after this mask.
    let private_namespace_roots = managed_private_namespace_roots(process_temp_root);
    for path in private_namespace_roots {
        if path.is_absolute() && path != Path::new("/") && path_exists(&path) {
            args.extend(["--tmpfs".to_string(), path.display().to_string()]);
        }
    }
    args.extend([
        "--proc".to_string(),
        "/proc".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
    ]);

    let mut created_directories = BTreeSet::new();
    if let Some(resolver) = resolver {
        append_read_only_bind(&mut args, resolver, resolver, &mut created_directories);
    }
    if let Some(root) = prompt_attachment_root {
        append_read_only_bind(&mut args, root, root, &mut created_directories);
    }
    (args, created_directories)
}

#[cfg(target_os = "linux")]
fn managed_prompt_attachment_root(
    request: &LaunchProviderRequest,
) -> Result<Option<PathBuf>, DaemonError> {
    let Some(agent_id) = request.agent_id.as_deref() else {
        return Ok(None);
    };
    let root =
        crate::runtime::agent_actor::prompt_attachment_materialization::inline_prompt_attachment_root(
            &request.session_id,
            agent_id,
        );
    std::fs::create_dir_all(&root).map_err(|error| {
        isolation_error(format!(
            "managed provider prompt attachment directory could not be created: {error}"
        ))
    })?;
    let metadata = std::fs::symlink_metadata(&root).map_err(|error| {
        isolation_error(format!(
            "managed provider prompt attachment directory could not be inspected: {error}"
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(isolation_error(
            "managed provider prompt attachment path must be a nonsymlink directory",
        ));
    }
    let temp_root = std::env::temp_dir();
    let relative = root.strip_prefix(&temp_root).map_err(|_| {
        isolation_error(
            "managed provider prompt attachment path must stay under the process temp directory",
        )
    })?;
    let expected = temp_root
        .canonicalize()
        .map_err(|error| {
            isolation_error(format!(
                "managed provider temp directory could not be resolved: {error}"
            ))
        })?
        .join(relative);
    let canonical = root.canonicalize().map_err(|error| {
        isolation_error(format!(
            "managed provider prompt attachment directory could not be resolved: {error}"
        ))
    })?;
    if canonical != expected {
        return Err(isolation_error(
            "managed provider prompt attachment path must not traverse symlinks",
        ));
    }
    Ok(Some(root))
}

#[cfg(target_os = "linux")]
fn managed_workspace_roots(request: &LaunchProviderRequest) -> Result<Vec<PathBuf>, DaemonError> {
    let process_temp_root = std::env::temp_dir();
    managed_workspace_roots_with_private_temp_roots(
        request,
        &managed_private_temp_roots(&process_temp_root),
    )
}

#[cfg(target_os = "linux")]
fn managed_workspace_roots_with_private_temp_roots(
    request: &LaunchProviderRequest,
    private_temp_roots: &[PathBuf],
) -> Result<Vec<PathBuf>, DaemonError> {
    if request.session_id == "provider-account" {
        return Ok(Vec::new());
    }
    let protected = managed_protected_namespace_directories(&[]);
    let host_publication_root = managed_configured_slice_publication_root();
    let configured = managed_slice_workspace_roots()?;
    let mut roots = configured.clone();
    let mut requested = request.workspace_live_sync_roots.clone();
    let working_root = request.working_directory.as_ref().map(|working_directory| {
        resolve_git_root(working_directory).unwrap_or_else(|| working_directory.clone())
    });
    if requested.is_empty() {
        requested.push(
            working_root
                .clone()
                .ok_or_else(|| isolation_error("managed provider launch has no workspace"))?,
        );
    } else if let Some(working_root) = working_root {
        // A caller may provide live-sync roots separately from the selected
        // cwd. Keep the cwd as a candidate when it is below a masked private
        // temp root; otherwise the provider would chdir into the empty mask.
        requested.push(working_root);
    }

    // A protected service tree may contain a selected repository. Rebind only
    // that exact repository after masking the service tree. A private temp
    // root is the other exception: rebind the selected child, while leaving
    // sibling temp paths hidden. Paths outside these actual masked roots stay
    // on the ordinary root mount with normal filesystem permissions.
    for root in requested {
        let Ok(root) = canonical_directory(&root, "managed provider workspace") else {
            continue;
        };
        let below_private_temp_root = private_temp_roots
            .iter()
            .any(|private| root != *private && root.starts_with(private));
        let below_host_publication_root = host_publication_root
            .as_ref()
            .is_some_and(|publication| root != *publication && root.starts_with(publication));
        let below_protected_root = protected
            .iter()
            .any(|protected| root.starts_with(protected));
        if !below_private_temp_root
            && !(below_protected_root
                && (is_managed_transfer_path(&root) || below_host_publication_root))
        {
            continue;
        }
        roots.push(root);
    }

    let mut canonical = Vec::new();
    for root in roots {
        let Ok(root) = canonical_directory(&root, "managed provider workspace") else {
            continue;
        };
        if canonical.iter().all(|existing| existing != &root) {
            canonical.push(root);
        }
    }
    Ok(canonical)
}

#[cfg(target_os = "linux")]
fn is_managed_transfer_path(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            std::path::Component::Normal(name)
                if name == std::ffi::OsStr::new("managed-context-workspaces")
                    || name == std::ffi::OsStr::new("managed-context-empty-workspaces")
        )
    })
}

#[cfg(target_os = "linux")]
fn managed_workspace_root_requires_rebind(
    root: &Path,
    protected: &[PathBuf],
    trusted_read_only: &[PathBuf],
) -> bool {
    let process_temp_root = std::env::temp_dir();
    managed_workspace_root_requires_rebind_with_private_temp_roots(
        root,
        protected,
        trusted_read_only,
        &managed_private_temp_roots(&process_temp_root),
    )
}

#[cfg(target_os = "linux")]
fn managed_workspace_root_requires_rebind_with_private_temp_roots(
    root: &Path,
    protected: &[PathBuf],
    trusted_read_only: &[PathBuf],
    private_temp_roots: &[PathBuf],
) -> bool {
    if protected
        .iter()
        .chain(trusted_read_only)
        .any(|path| root == path)
    {
        return false;
    }
    if protected
        .iter()
        .chain(trusted_read_only)
        .any(|path| root.starts_with(path))
    {
        return true;
    }
    private_temp_roots
        .iter()
        .any(|path| root != path && root.starts_with(path))
}

#[cfg(target_os = "linux")]
fn managed_working_directory(
    request: &LaunchProviderRequest,
    roots: &[PathBuf],
) -> Result<PathBuf, DaemonError> {
    if request.session_id == "provider-account" {
        return Ok(PathBuf::from(SANDBOX_HOME));
    }
    let directory = request
        .working_directory
        .as_ref()
        .ok_or_else(|| isolation_error("managed provider launch has no working directory"))?;
    let directory = canonical_directory(directory, "managed provider working directory")?;
    let protected = managed_protected_namespace_directories(&[]);
    if let Some(protected_root) = protected.iter().find(|path| directory.starts_with(path)) {
        let reexposed_child = roots.iter().any(|root| {
            root != protected_root
                && root.starts_with(protected_root)
                && directory.starts_with(root)
        });
        if !reexposed_child {
            return Err(isolation_error(
                "managed provider working directory is inside protected managed service state",
            ));
        }
    }
    Ok(directory)
}

#[cfg(target_os = "linux")]
fn managed_runtime_roots(launch: &ProviderLaunchResult) -> Result<Vec<PathBuf>, DaemonError> {
    let mut roots = Vec::new();
    for (name, value) in &launch.pty_env {
        if !name.starts_with("CHARIOX_CLAUDE_NATIVE_") {
            continue;
        }
        let path = PathBuf::from(value);
        let candidate = if path.is_dir() {
            path
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| isolation_error("managed Claude runtime path has no parent"))?
        };
        let root = canonical_directory(&candidate, "managed provider runtime files")?;
        if roots.iter().all(|existing| existing != &root) {
            roots.push(root);
        }
    }
    roots.sort();
    roots.dedup_by(|next, previous| next.starts_with(previous));
    Ok(roots)
}

#[cfg(target_os = "linux")]
fn managed_account_bindings(
    environment: &mut BTreeMap<String, String>,
) -> Result<Vec<(PathBuf, PathBuf)>, DaemonError> {
    let mut bindings: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (ordinal, name) in PROVIDER_ACCOUNT_PATH_ENVIRONMENT.iter().enumerate() {
        let Some(value) = environment.get(*name).cloned() else {
            continue;
        };
        let source = canonical_directory(Path::new(&value), "provider account directory")?;
        if let Some((parent_source, parent_destination)) = bindings
            .iter()
            .find(|(existing, _)| source.starts_with(existing))
        {
            let relative = source.strip_prefix(parent_source).map_err(|_| {
                isolation_error("provider account directory binding is inconsistent")
            })?;
            environment.insert(
                (*name).to_string(),
                parent_destination.join(relative).display().to_string(),
            );
            continue;
        }
        let destination = Path::new(SANDBOX_ACCOUNT_ROOT).join(format!("root-{ordinal}"));
        environment.insert((*name).to_string(), destination.display().to_string());
        bindings.push((source, destination));
    }
    Ok(bindings)
}

#[cfg(target_os = "linux")]
fn rewrite_managed_program_path(
    program: &str,
    provider_home: &Path,
    account_bindings: &[(PathBuf, PathBuf)],
) -> String {
    let path = Path::new(program);
    if !path.is_absolute() {
        return program.to_string();
    }
    if let Ok(relative) = path.strip_prefix(provider_home) {
        return Path::new(SANDBOX_HOME).join(relative).display().to_string();
    }
    for (source, destination) in account_bindings {
        if let Ok(relative) = path.strip_prefix(source) {
            return destination.join(relative).display().to_string();
        }
    }
    program.to_string()
}

#[cfg(target_os = "linux")]
fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, DaemonError> {
    if !path.is_absolute() {
        return Err(isolation_error(format!("{label} must be absolute")));
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| isolation_error(format!("failed to inspect {label}: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(isolation_error(format!("{label} must be a real directory")));
    }
    path.canonicalize()
        .map_err(|error| isolation_error(format!("failed to canonicalize {label}: {error}")))
}

#[cfg(target_os = "linux")]
fn resolve_git_root(path: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let root = PathBuf::from(root.trim());
    (!root.as_os_str().is_empty()).then_some(root)
}

#[cfg(target_os = "linux")]
fn append_bind(
    args: &mut Vec<String>,
    source: &Path,
    destination: &Path,
    created: &mut BTreeSet<PathBuf>,
) {
    append_directory(args, destination, created);
    args.extend([
        "--bind".to_string(),
        source.display().to_string(),
        destination.display().to_string(),
    ]);
}

#[cfg(any(target_os = "linux", test))]
fn append_read_only_bind(
    args: &mut Vec<String>,
    source: &Path,
    destination: &Path,
    created: &mut BTreeSet<PathBuf>,
) {
    append_directory(
        args,
        destination.parent().unwrap_or(Path::new("/")),
        created,
    );
    args.extend([
        "--ro-bind".to_string(),
        source.display().to_string(),
        destination.display().to_string(),
    ]);
}

#[cfg(any(target_os = "linux", test))]
fn append_directory(args: &mut Vec<String>, destination: &Path, created: &mut BTreeSet<PathBuf>) {
    let mut ancestors = destination.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        if ancestor == Path::new("/") || !created.insert(ancestor.to_path_buf()) {
            continue;
        }
        args.extend(["--dir".to_string(), ancestor.display().to_string()]);
    }
}

fn isolation_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed_provider_isolation",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_provider_reported_path_resolves_through_its_account_bind() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-reported-path-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms(),
        ));
        let host_account = root.join("account");
        let host_transcript = host_account.join("projects/session.jsonl");
        std::fs::create_dir_all(
            host_transcript
                .parent()
                .expect("transcript should have a parent"),
        )
        .expect("account transcript root should exist");
        std::fs::write(&host_transcript, "{}\n").expect("transcript should exist");
        let sandbox_account = Path::new("/home/chariox/.provider-account").join("root-1");
        let sandbox_transcript = sandbox_account.join("projects/session.jsonl");
        let request = LaunchProviderRequest::new(
            "session-managed-path",
            "claude",
            "claude-headless",
            "default",
            "claude-opus",
        );
        let run = RuntimeProviderRun::new(
            "provider-run-managed-path",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "claude:headless:managed-isolated".to_string(),
                pty_target: None,
                pty_program: Some("/usr/bin/bwrap".to_string()),
                pty_args: vec![
                    "--ro-bind".to_string(),
                    "/".to_string(),
                    "/".to_string(),
                    "--bind".to_string(),
                    host_account.display().to_string(),
                    sandbox_account.display().to_string(),
                    "--setenv".to_string(),
                    MANAGED_PROVIDER_ISOLATION_MARKER_ENV.to_string(),
                    "1".to_string(),
                    "--".to_string(),
                    "/usr/local/bin/claude".to_string(),
                ],
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );

        assert_eq!(
            provider_reported_path_on_kernel(
                &run,
                sandbox_transcript
                    .to_str()
                    .expect("sandbox transcript should be utf8"),
            ),
            Some(
                host_transcript
                    .canonicalize()
                    .expect("host path should resolve")
            ),
        );
        assert_eq!(
            provider_reported_path_on_kernel(
                &run,
                host_transcript
                    .to_str()
                    .expect("host transcript should be utf8"),
            ),
            Some(
                host_transcript
                    .canonicalize()
                    .expect("host path should resolve")
            ),
            "persisted transcript cursors must remain resolvable",
        );
        assert_eq!(
            provider_reported_path_on_kernel(
                &run,
                "/home/chariox/.provider-account/root-1/../root-2/session.jsonl",
            ),
            None,
            "provider paths must not escape their kernel-owned bind",
        );
        let outside_bind = root.join("outside-bind.jsonl");
        std::fs::write(&outside_bind, "secret\n").expect("outside fixture should exist");
        assert_eq!(
            provider_reported_path_on_kernel(
                &run,
                outside_bind
                    .to_str()
                    .expect("outside fixture should be utf8"),
            ),
            None,
            "the Bubblewrap root binding must not authorize arbitrary host paths",
        );
        #[cfg(unix)]
        {
            let outside = root.join("outside.jsonl");
            let escape = host_account.join("projects/escape.jsonl");
            std::fs::write(&outside, "secret\n").expect("outside fixture should exist");
            std::os::unix::fs::symlink(&outside, &escape)
                .expect("escape symlink should be created");
            assert_eq!(
                provider_reported_path_on_kernel(
                    &run,
                    "/home/chariox/.provider-account/root-1/projects/escape.jsonl",
                ),
                None,
                "provider transcript symlinks must not escape their account bind",
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn namespace_args_keep_the_ordinary_root_and_private_temp_roots() {
        let resolver = Path::new("/run/systemd/resolve/stub-resolv.conf");
        let (args, _) = managed_namespace_args(
            Some(resolver),
            |path| {
                path == Path::new("/run")
                    || path == Path::new("/tmp")
                    || path == Path::new("/var/tmp")
            },
            None,
        );

        let binding = args
            .windows(3)
            .position(|args| {
                args == [
                    "--ro-bind",
                    "/run/systemd/resolve/stub-resolv.conf",
                    "/run/systemd/resolve/stub-resolv.conf",
                ]
            })
            .expect("resolver target should be restored read-only");
        assert!(args.windows(3).any(|args| args == ["--bind", "/", "/"]));
        assert!(args.windows(2).any(|args| args == ["--tmpfs", "/tmp"]));
        assert!(args.windows(2).any(|args| args == ["--tmpfs", "/var/tmp"]));
        let run_mask = args
            .windows(2)
            .position(|args| args == ["--tmpfs", "/run"])
            .expect("the service runtime tree must be masked");
        assert!(
            run_mask < binding,
            "resolver must be re-exposed after /run is masked"
        );
        assert_eq!(
            &args[binding - 6..binding],
            [
                "--dir",
                "/run",
                "--dir",
                "/run/systemd",
                "--dir",
                "/run/systemd/resolve",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn namespace_args_mask_the_actual_process_temp_root() {
        let _env = crate::env_lock::lock();
        let previous_tmpdir = std::env::var_os("TMPDIR");
        let temp_root = std::env::temp_dir().join(format!(
            "chariox-managed-isolation-process-temp-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&temp_root).expect("custom process temp root should exist");
        std::env::set_var("TMPDIR", &temp_root);
        let process_temp_root = std::env::temp_dir();
        let (args, _) = managed_namespace_args(
            None,
            |path| {
                path == Path::new("/tmp")
                    || path == Path::new("/var/tmp")
                    || path == process_temp_root
            },
            None,
        );

        restore_env("TMPDIR", previous_tmpdir);
        let _ = std::fs::remove_dir_all(&temp_root);

        assert!(args.windows(2).any(|args| {
            args == [
                "--tmpfs",
                process_temp_root
                    .to_str()
                    .expect("temp root should be utf8"),
            ]
        }));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_launch_rebinds_selected_workspace_below_custom_tmpdir() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-selected-temp-workspace-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let custom_tmpdir = root.join("custom-tmp");
        let workspace = custom_tmpdir.join("selected-repository");
        std::fs::create_dir_all(&workspace).expect("selected temp workspace should exist");

        let request = LaunchProviderRequest::new(
            "selected-temp-workspace-session",
            "codex",
            "codex",
            "default",
            "gpt-5.6-luna",
        )
        .with_working_directory(workspace.clone())
        .with_workspace_live_sync_roots(vec![workspace.clone()]);
        let private_temp_roots = managed_private_temp_roots(&custom_tmpdir);
        let roots = managed_workspace_roots_with_private_temp_roots(&request, &private_temp_roots)
            .expect("selected workspace below custom TMPDIR should resolve");
        let protected = managed_protected_namespace_directories(&[]);
        let trusted = Vec::new();
        let (mut args, mut created) =
            managed_namespace_args_with_process_temp_root(None, Path::exists, None, &custom_tmpdir);
        append_managed_protected_namespace_directories(&mut args, &protected, &mut created);
        append_managed_trusted_read_only_paths(&mut args, &trusted, &mut created);
        for root in &roots {
            if managed_workspace_root_requires_rebind_with_private_temp_roots(
                root,
                &protected,
                &trusted,
                &private_temp_roots,
            ) {
                append_bind(&mut args, root, root, &mut created);
            }
        }

        let workspace = workspace
            .canonicalize()
            .expect("selected workspace should canonicalize");
        let custom_tmpdir = custom_tmpdir
            .canonicalize()
            .expect("custom TMPDIR should canonicalize");
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(roots, vec![workspace.clone()]);
        let temp_mask = args
            .windows(2)
            .position(|window| {
                window
                    == [
                        "--tmpfs",
                        custom_tmpdir
                            .to_str()
                            .expect("custom TMPDIR should be utf8"),
                    ]
            })
            .expect("custom TMPDIR should be masked");
        let workspace_bind = args
            .windows(3)
            .position(|window| {
                window
                    == [
                        "--bind",
                        workspace.to_str().expect("workspace should be utf8"),
                        workspace.to_str().expect("workspace should be utf8"),
                    ]
            })
            .expect("selected workspace should be rebound after its temp mask");
        assert!(temp_mask < workspace_bind);
        assert!(!args.windows(3).any(|window| {
            window
                == [
                    "--bind",
                    custom_tmpdir
                        .to_str()
                        .expect("custom TMPDIR should be utf8"),
                    custom_tmpdir
                        .to_str()
                        .expect("custom TMPDIR should be utf8"),
                ]
        }));
    }

    #[test]
    fn namespace_args_do_not_bind_a_resolver_without_a_runtime_target() {
        let (args, _) = managed_namespace_args(None, |path| path == Path::new("/run"), None);

        assert_eq!(args.iter().filter(|arg| *arg == "--ro-bind").count(), 0);
        assert!(!args.iter().any(|arg| arg == "/etc/resolv.conf"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_namespace_rebinds_current_runtime_after_trusted_paths_and_run_mask() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-trusted-runtime-order-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let slice_root = root.join("slice-code");
        let helper = root.join("helper/bin/bwrap");
        std::fs::create_dir_all(&slice_root).expect("trusted slice root should exist");
        std::fs::create_dir_all(helper.parent().expect("helper parent should exist"))
            .expect("helper parent should exist");
        std::fs::write(&helper, "trusted helper\n").expect("helper should exist");

        let _env = crate::env_lock::lock();
        let previous_slice_root = std::env::var_os("CHARIOX_SLICE_ROOT");
        let previous_bwrap = std::env::var_os(MANAGED_PROVIDER_BWRAP_ENV);
        std::env::set_var("CHARIOX_SLICE_ROOT", &slice_root);
        std::env::set_var(MANAGED_PROVIDER_BWRAP_ENV, &helper);

        let (mut args, mut created) = managed_namespace_args(None, Path::exists, None);
        let trusted =
            managed_trusted_read_only_paths().expect("configured trusted paths should resolve");
        append_managed_trusted_read_only_paths(&mut args, &trusted, &mut created);
        let current_runtime = Path::new("/run/chariox-current-runtime");
        append_bind(&mut args, current_runtime, current_runtime, &mut created);

        restore_env("CHARIOX_SLICE_ROOT", previous_slice_root);
        restore_env(MANAGED_PROVIDER_BWRAP_ENV, previous_bwrap);

        let slice_root = slice_root
            .canonicalize()
            .expect("slice root should canonicalize");
        let trusted_bind = args
            .windows(3)
            .position(|window| {
                window
                    == [
                        "--ro-bind",
                        slice_root.to_str().expect("slice path should be utf8"),
                        slice_root.to_str().expect("slice path should be utf8"),
                    ]
            })
            .expect("configured slice code should be read-only mounted");
        let run_mask = args
            .windows(2)
            .position(|window| window == ["--tmpfs", "/run"])
            .expect("service runtime tree should be masked");
        let runtime_bind = args
            .windows(3)
            .position(|window| {
                window
                    == [
                        "--bind",
                        "/run/chariox-current-runtime",
                        "/run/chariox-current-runtime",
                    ]
            })
            .expect("current-run runtime should be re-exposed read-write");
        assert!(
            run_mask < trusted_bind,
            "trusted paths must be mounted after /run is masked"
        );
        assert!(
            trusted_bind < runtime_bind,
            "current-run runtime must win after trusted mounts"
        );
        assert!(!args.windows(2).any(|window| {
            window
                == [
                    "--tmpfs",
                    slice_root.to_str().expect("slice path should be utf8"),
                ]
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_custom_claude_runtime_root_remains_rebindable_after_masks() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-custom-runtime-root-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("custom runtime root should exist");
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "managed-runtime-root-test".to_string(),
            pty_target: None,
            pty_program: Some("/bin/sh".to_string()),
            pty_args: Vec::new(),
            pty_env: BTreeMap::from([(
                "CHARIOX_CLAUDE_NATIVE_RUNTIME".to_string(),
                root.display().to_string(),
            )]),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        };
        let runtime_roots =
            managed_runtime_roots(&launch).expect("custom Claude runtime root should be accepted");
        let (mut args, mut created) = managed_namespace_args(None, Path::exists, None);
        for runtime_root in &runtime_roots {
            append_bind(&mut args, runtime_root, runtime_root, &mut created);
        }

        let runtime_root = root
            .canonicalize()
            .expect("runtime root should canonicalize");
        let run_mask = args
            .windows(2)
            .position(|window| window == ["--tmpfs", "/run"])
            .expect("service runtime tree should be masked");
        let temp_mask = args
            .windows(2)
            .position(|window| window == ["--tmpfs", "/tmp"])
            .expect("shared temp tree should be masked");
        let runtime_bind = args
            .windows(3)
            .position(|window| {
                window
                    == [
                        "--bind",
                        runtime_root.to_str().expect("runtime path should be utf8"),
                        runtime_root.to_str().expect("runtime path should be utf8"),
                    ]
            })
            .expect("current configured runtime should be rebound read-write");
        assert!(run_mask < runtime_bind);
        assert!(temp_mask < runtime_bind);
        let protected_run = vec![PathBuf::from("/run")];
        assert!(!managed_workspace_root_requires_rebind(
            Path::new("/run"),
            &protected_run,
            &[],
        ));
        assert!(managed_workspace_root_requires_rebind(
            Path::new("/run/chariox-current-runtime"),
            &protected_run,
            &[],
        ));

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_configured_slice_publication_is_protected_not_trusted_wholesale() {
        let _env = crate::env_lock::lock();
        let previous_slice_root = std::env::var_os("CHARIOX_SLICE_ROOT");
        let previous_broker = std::env::var_os("CHARIOX_SLICE_DOCKER_BROKER_SOCKET");
        let previous_service_root = std::env::var_os(MANAGED_SLICE_SERVICE_ROOT_ENV);
        let previous_publication_root = std::env::var_os(MANAGED_SLICE_PUBLICATION_ROOT_ENV);
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-relocated-slice-boundary-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let share_root = root.join("relocated-share");
        let publication_root = share_root.join("configured-publications");
        std::fs::create_dir_all(&publication_root).expect("configured publication root");
        std::env::set_var("CHARIOX_SLICE_ROOT", &publication_root);
        std::env::set_var(MANAGED_SLICE_SERVICE_ROOT_ENV, &share_root);
        std::env::set_var(MANAGED_SLICE_PUBLICATION_ROOT_ENV, &publication_root);
        // This is the environment after the supervisor has handed off the
        // broker FD. Publication policy must not depend on the transport.
        std::env::remove_var("CHARIOX_SLICE_DOCKER_BROKER_SOCKET");

        let protected = managed_protected_namespace_directories(&[]);
        let trusted = managed_trusted_read_only_paths()
            .expect("configured publication path should not need a trusted bind");
        let detected = managed_configured_slice_publication_root();

        restore_env("CHARIOX_SLICE_ROOT", previous_slice_root);
        restore_env("CHARIOX_SLICE_DOCKER_BROKER_SOCKET", previous_broker);
        restore_env(MANAGED_SLICE_SERVICE_ROOT_ENV, previous_service_root);
        restore_env(
            MANAGED_SLICE_PUBLICATION_ROOT_ENV,
            previous_publication_root,
        );

        let service = share_root
            .canonicalize()
            .expect("configured service root should canonicalize");
        let publication = publication_root
            .canonicalize()
            .expect("configured publication root should canonicalize");
        assert!(protected.contains(&service));
        assert!(publication.starts_with(&service));
        assert!(!trusted.contains(&publication));
        assert_eq!(detected, Some(publication.clone()));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_bwrap_probe_blocks_trusted_helper_writes_and_run_screen_paths() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-trusted-bwrap-probe-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let slice_root = root.join("slice-code");
        let helper = slice_root.join("bin/chariox-kernel");
        std::fs::create_dir_all(helper.parent().expect("helper parent should exist"))
            .expect("helper parent should exist");
        std::fs::write(&helper, "trusted helper\n").expect("helper should exist");

        let bwrap = Path::new(BWRAP_PATH);
        if !bwrap.is_file() {
            eprintln!(
                "skipped managed bwrap trusted-helper probe: {} is unavailable",
                bwrap.display()
            );
            let _ = std::fs::remove_dir_all(root);
            return;
        }

        let (mut args, mut created) = managed_namespace_args(None, Path::exists, None);
        append_managed_trusted_read_only_paths(
            &mut args,
            &[slice_root
                .canonicalize()
                .expect("slice root should canonicalize")],
            &mut created,
        );
        args.extend([
            "--".to_string(),
            "/bin/sh".to_string(),
            "-eu".to_string(),
            "-c".to_string(),
            "test \"$(cat \"$1\")\" = 'trusted helper'\nif printf attacker > \"$1\"; then exit 12; fi\ntest \"$(cat \"$1\")\" = 'trusted helper'\ntest ! -e /run/screen/S-slice".to_string(),
            "managed-trusted-bwrap-probe".to_string(),
            helper.display().to_string(),
        ]);
        let output = match Command::new(bwrap).args(args).output() {
            Ok(output) => output,
            Err(error) => {
                eprintln!("skipped managed bwrap trusted-helper probe: {error}");
                let _ = std::fs::remove_dir_all(root);
                return;
            }
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No permissions to create a new namespace")
            || stderr.contains("Operation not permitted")
        {
            eprintln!(
                "skipped managed bwrap trusted-helper probe: user namespaces are unavailable"
            );
            let _ = std::fs::remove_dir_all(root);
            return;
        }
        assert!(
            output.status.success(),
            "managed bwrap trusted-helper probe failed: {stderr}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn managed_namespace_scopes_claude_sandbox_state_to_the_claude_adapter() {
        for provider in ["claude", "claude-headless", "claude-p"] {
            let request =
                LaunchProviderRequest::new("session-1", "claude", provider, "default", "sonnet");
            let mut args = Vec::new();
            append_managed_namespace_environment(&mut args, &request);
            assert!(args
                .windows(3)
                .any(|args| { args == ["--setenv", CLAUDE_SANDBOX_ENV, "1"] }));
            assert!(!args
                .windows(2)
                .any(|args| args == ["--unsetenv", CLAUDE_SANDBOX_ENV]));
        }

        let request =
            LaunchProviderRequest::new("session-1", "codex", "claude", "default", "sonnet");
        let mut args = Vec::new();
        append_managed_namespace_environment(&mut args, &request);
        assert!(args
            .windows(2)
            .any(|args| args == ["--unsetenv", CLAUDE_SANDBOX_ENV]));
        assert!(!args
            .windows(3)
            .any(|args| args == ["--setenv", CLAUDE_SANDBOX_ENV, "1"]));
    }

    #[test]
    fn managed_namespace_replaces_the_service_accounts_nologin_shell() {
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-5.6-luna");
        let mut args = Vec::new();

        append_managed_namespace_environment(&mut args, &request);

        assert!(args
            .windows(3)
            .any(|args| args == ["--setenv", "SHELL", "/bin/sh"]));
    }

    #[test]
    fn managed_git_trusts_bootstrap_workspace_roots_without_global_trust() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-git-safe-directory-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let approved_primary = root.join("primary");
        let approved_supporting = root.join("supporting");
        let unapproved = root.join("unapproved");
        let global_config = root.join("hostile-global.gitconfig");
        for repository in [&approved_primary, &approved_supporting, &unapproved] {
            std::fs::create_dir_all(repository).expect("repository root should create");
            let status = Command::new("git")
                .arg("-C")
                .arg(repository)
                .args(["init", "--quiet"])
                .status()
                .expect("Git fixture should start");
            assert!(status.success(), "Git fixture should initialize");
        }
        std::fs::write(&global_config, "[safe]\n\tdirectory = *\n")
            .expect("hostile global Git config should write");
        let approved_primary = approved_primary
            .canonicalize()
            .expect("primary repository should canonicalize");
        let approved_supporting = approved_supporting
            .canonicalize()
            .expect("supporting repository should canonicalize");
        let unapproved = unapproved
            .canonicalize()
            .expect("unapproved repository should canonicalize");
        let mut args = Vec::new();
        append_managed_git_safe_directory_environment(
            &mut args,
            &[approved_primary.clone(), approved_supporting.clone()],
        );

        let git_status = |repository: &Path| {
            let mut command = Command::new("git");
            command
                .arg("-C")
                .arg(repository)
                .args(["status", "--porcelain"])
                .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", &global_config);
            for environment in args.windows(3) {
                if environment[0] == "--setenv" && environment[1].starts_with("GIT_CONFIG_") {
                    command.env(&environment[1], &environment[2]);
                }
            }
            command.output().expect("Git status should run")
        };

        for approved in [&approved_primary, &approved_supporting] {
            let output = git_status(approved);
            assert!(
                output.status.success(),
                "approved repository should be trusted: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let denied = git_status(&unapproved);
        assert!(
            !denied.status.success(),
            "unapproved repository must stay untrusted"
        );
        assert!(String::from_utf8_lossy(&denied.stderr).contains("dubious ownership"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_bwrap_probe_hides_unselected_slice_publication_siblings() {
        let _env = crate::env_lock::lock();
        let previous_slice_root = std::env::var_os("CHARIOX_SLICE_ROOT");
        let previous_broker = std::env::var_os("CHARIOX_SLICE_DOCKER_BROKER_SOCKET");
        let previous_service_root = std::env::var_os(MANAGED_SLICE_SERVICE_ROOT_ENV);
        let previous_publication_root = std::env::var_os(MANAGED_SLICE_PUBLICATION_ROOT_ENV);
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-slice-publication-bwrap-probe-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let share_root = root.join("relocated-share");
        let publication = share_root.join("configured-publications");
        let selected = publication.join("selected");
        let sibling = publication.join("sibling");
        std::fs::create_dir_all(&selected).expect("selected publication should exist");
        std::fs::create_dir_all(&sibling).expect("sibling publication should exist");
        std::fs::write(selected.join("selected.txt"), "selected\n")
            .expect("selected marker should exist");
        std::fs::write(sibling.join("sibling.txt"), "sibling\n")
            .expect("sibling marker should exist");
        std::env::set_var("CHARIOX_SLICE_ROOT", &publication);
        std::env::set_var(MANAGED_SLICE_SERVICE_ROOT_ENV, &share_root);
        std::env::set_var(MANAGED_SLICE_PUBLICATION_ROOT_ENV, &publication);
        std::env::remove_var("CHARIOX_SLICE_DOCKER_BROKER_SOCKET");

        let request = LaunchProviderRequest::new(
            "managed-slice-publication-probe",
            "codex",
            "codex",
            "default",
            "gpt-5.6-luna",
        )
        .with_working_directory(selected.clone())
        .with_workspace_live_sync_roots(vec![selected.clone()]);
        let workspace_roots = managed_workspace_roots(&request)
            .expect("selected configured publication should resolve");
        let protected = managed_protected_namespace_directories(&[]);
        let trusted = managed_trusted_read_only_paths()
            .expect("configured publication should not be trusted wholesale");
        assert_eq!(
            managed_configured_slice_publication_root(),
            Some(publication.clone())
        );
        assert_eq!(workspace_roots, vec![selected.canonicalize().unwrap()]);

        let bwrap = Path::new(BWRAP_PATH);
        if !bwrap.is_file() {
            eprintln!(
                "skipped managed bwrap slice-publication probe: {} is unavailable",
                bwrap.display()
            );
            restore_env("CHARIOX_SLICE_ROOT", previous_slice_root);
            restore_env("CHARIOX_SLICE_DOCKER_BROKER_SOCKET", previous_broker);
            restore_env(MANAGED_SLICE_SERVICE_ROOT_ENV, previous_service_root);
            restore_env(
                MANAGED_SLICE_PUBLICATION_ROOT_ENV,
                previous_publication_root,
            );
            let _ = std::fs::remove_dir_all(root);
            return;
        }

        let (mut args, mut created) = managed_namespace_args(None, Path::exists, None);
        append_managed_protected_namespace_directories(&mut args, &protected, &mut created);
        append_managed_trusted_read_only_paths(&mut args, &trusted, &mut created);
        for root in &workspace_roots {
            if managed_workspace_root_requires_rebind(root, &protected, &trusted) {
                append_bind(&mut args, root, root, &mut created);
            }
        }
        assert!(args.windows(2).any(|window| {
            window
                == [
                    "--tmpfs",
                    share_root.to_str().expect("service root should be utf8"),
                ]
        }));
        assert!(args.windows(3).any(|window| {
            window
                == [
                    "--bind",
                    selected.to_str().expect("selected path should be utf8"),
                    selected.to_str().expect("selected path should be utf8"),
                ]
        }));
        assert!(!args.windows(3).any(|window| {
            window
                == [
                    "--ro-bind",
                    publication.to_str().expect("publication should be utf8"),
                    publication.to_str().expect("publication should be utf8"),
                ]
        }));
        args.extend([
            "--".to_string(),
            "/bin/sh".to_string(),
            "-eu".to_string(),
            "-c".to_string(),
            "test -r \"$1/selected.txt\"\ntest ! -e \"$2/sibling.txt\"\nprintf provider > \"$1/provider-write\""
                .to_string(),
            "managed-slice-publication-probe".to_string(),
            selected.display().to_string(),
            sibling.display().to_string(),
        ]);
        restore_env("CHARIOX_SLICE_ROOT", previous_slice_root);
        restore_env("CHARIOX_SLICE_DOCKER_BROKER_SOCKET", previous_broker);
        restore_env(MANAGED_SLICE_SERVICE_ROOT_ENV, previous_service_root);
        restore_env(
            MANAGED_SLICE_PUBLICATION_ROOT_ENV,
            previous_publication_root,
        );
        let output = match Command::new(bwrap).args(args).output() {
            Ok(output) => output,
            Err(error) => {
                eprintln!("skipped managed bwrap slice-publication probe: {error}");
                let _ = std::fs::remove_dir_all(root);
                return;
            }
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No permissions to create a new namespace")
            || stderr.contains("Operation not permitted")
        {
            eprintln!(
                "skipped managed bwrap slice-publication probe: user namespaces are unavailable"
            );
            let _ = std::fs::remove_dir_all(root);
            return;
        }
        assert!(
            output.status.success(),
            "managed bwrap slice-publication probe failed: {stderr}"
        );
        assert!(selected.join("provider-write").is_file());
        assert!(!sibling.join("provider-write").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_isolation_keeps_ordinary_repositories_outside_transfer_roots() {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-isolation-ordinary-filesystem-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let chariox_home = root.join("chariox-home");
        let transferred_repository =
            chariox_home.join(".chariox/state/managed-context-workspaces/publication/selected");
        let outside_repository = root.join("ordinary/new-repository");
        let cloned_repository = root.join("ordinary/cloned-repository");
        let slice_code = root.join("slice-code");
        let custom_helper = root.join("custom-helper/bin/bwrap");
        let broker_socket = root.join("slice-share/.broker-private/control/control.sock");
        std::fs::create_dir_all(&transferred_repository).expect("transferred repository");
        std::fs::create_dir_all(&outside_repository).expect("outside repository");
        std::fs::create_dir_all(&slice_code).expect("trusted slice code directory");
        std::fs::create_dir_all(custom_helper.parent().expect("helper parent"))
            .expect("custom helper directory");
        std::fs::write(&custom_helper, "trusted helper\n").expect("custom helper should exist");
        std::fs::create_dir_all(broker_socket.parent().expect("broker socket parent"))
            .expect("broker control directory");

        let _env = crate::env_lock::lock();
        let previous_home = std::env::var_os("CHARIOX_HOME");
        let previous_capability_root = std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT");
        let previous_vault = std::env::var_os("CHARIOX_MANAGED_VAULT_PATH");
        let previous_broker = std::env::var_os("CHARIOX_SLICE_DOCKER_BROKER_SOCKET");
        let previous_slice_root = std::env::var_os("CHARIOX_SLICE_ROOT");
        let previous_bwrap = std::env::var_os(MANAGED_PROVIDER_BWRAP_ENV);
        let previous_workspace_count = std::env::var_os(MANAGED_WORKSPACE_ROOT_COUNT_ENV);
        std::env::set_var("CHARIOX_HOME", &chariox_home);
        std::env::set_var(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            chariox_home.join("managed-context/kernel"),
        );
        std::env::set_var(
            "CHARIOX_MANAGED_VAULT_PATH",
            chariox_home.join(".chariox/vault/vault.json"),
        );
        std::env::set_var("CHARIOX_SLICE_DOCKER_BROKER_SOCKET", &broker_socket);
        std::env::set_var("CHARIOX_SLICE_ROOT", &slice_code);
        std::env::set_var(MANAGED_PROVIDER_BWRAP_ENV, &custom_helper);
        std::env::remove_var(MANAGED_WORKSPACE_ROOT_COUNT_ENV);

        let request = LaunchProviderRequest::new(
            "ordinary-filesystem-session",
            "codex",
            "codex",
            "default",
            "gpt-5.6-luna",
        )
        .with_working_directory(outside_repository.clone())
        .with_workspace_live_sync_roots(vec![transferred_repository.clone()]);
        let roots = managed_workspace_roots(&request).expect("managed roots should resolve");
        let working_directory = managed_working_directory(&request, &roots)
            .expect("ordinary paths outside service state should remain valid cwd");
        let protected = managed_protected_namespace_directories(&[]);
        let trusted_read_only =
            managed_trusted_read_only_paths().expect("configured trusted paths should resolve");
        let mut args = managed_namespace_args(None, |_| true, None).0;
        let mut created = BTreeSet::new();
        append_managed_protected_namespace_directories(&mut args, &protected, &mut created);
        append_managed_trusted_read_only_paths(&mut args, &trusted_read_only, &mut created);
        for root in &roots {
            if managed_workspace_root_requires_rebind(root, &protected, &trusted_read_only) {
                append_bind(&mut args, root, root, &mut created);
            }
        }

        restore_env("CHARIOX_HOME", previous_home);
        restore_env(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            previous_capability_root,
        );
        restore_env("CHARIOX_MANAGED_VAULT_PATH", previous_vault);
        restore_env("CHARIOX_SLICE_DOCKER_BROKER_SOCKET", previous_broker);
        restore_env("CHARIOX_SLICE_ROOT", previous_slice_root);
        restore_env(MANAGED_PROVIDER_BWRAP_ENV, previous_bwrap);
        restore_env(MANAGED_WORKSPACE_ROOT_COUNT_ENV, previous_workspace_count);

        assert_eq!(
            roots,
            vec![
                transferred_repository.canonicalize().unwrap(),
                outside_repository.canonicalize().unwrap(),
            ]
        );
        assert_eq!(
            working_directory,
            outside_repository.canonicalize().unwrap()
        );
        assert!(roots.contains(&outside_repository.canonicalize().unwrap()));
        assert!(protected.contains(&chariox_home.join(".chariox")));
        assert!(protected.contains(&root.join("slice-share/.broker-private")));
        assert!(!protected.contains(&slice_code));
        assert!(trusted_read_only.contains(&slice_code.canonicalize().unwrap()));
        assert!(trusted_read_only.contains(&custom_helper.canonicalize().unwrap()));
        assert!(args.windows(3).any(|window| window == ["--bind", "/", "/"]));
        assert!(args.windows(3).any(|window| {
            window
                == [
                    "--ro-bind",
                    slice_code.to_str().expect("slice path utf8"),
                    slice_code.to_str().expect("slice path utf8"),
                ]
        }));
        assert!(args.windows(3).any(|window| {
            window
                == [
                    "--ro-bind",
                    custom_helper.to_str().expect("helper path utf8"),
                    custom_helper.to_str().expect("helper path utf8"),
                ]
        }));
        assert!(args.windows(3).any(|window| {
            window
                == [
                    "--bind",
                    transferred_repository
                        .to_str()
                        .expect("transferred repository should be utf8"),
                    transferred_repository
                        .to_str()
                        .expect("transferred repository should be utf8"),
                ]
        }));
        assert!(args.windows(3).any(|window| {
            window
                == [
                    "--bind",
                    outside_repository.to_str().expect("outside path utf8"),
                    outside_repository.to_str().expect("outside path utf8"),
                ]
        }));
        assert!(!args.windows(3).any(|window| {
            window
                == [
                    "--bind",
                    chariox_home
                        .join(".chariox/state/managed-context-workspaces/publication")
                        .to_str()
                        .expect("publication root should be utf8"),
                    chariox_home
                        .join(".chariox/state/managed-context-workspaces/publication")
                        .to_str()
                        .expect("publication root should be utf8"),
                ]
        }));
        assert!(!args.windows(2).any(|window| {
            window
                == [
                    "--tmpfs",
                    outside_repository.to_str().expect("outside path utf8"),
                ]
        }));
        assert!(!args.windows(2).any(|window| {
            window == ["--tmpfs", slice_code.to_str().expect("slice path utf8")]
        }));

        let status = Command::new("git")
            .arg("-C")
            .arg(&outside_repository)
            .args(["init", "--quiet"])
            .status()
            .expect("Git should start outside the transfer root");
        assert!(status.success());
        std::fs::write(
            outside_repository.join("README.md"),
            "ordinary filesystem\n",
        )
        .expect("ordinary repository should be writable");
        let status = Command::new("git")
            .arg("-C")
            .arg(&outside_repository)
            .args(["add", "README.md"])
            .status()
            .expect("Git add should start outside the transfer root");
        assert!(status.success());
        let status = Command::new("git")
            .arg("-C")
            .arg(&outside_repository)
            .args([
                "-c",
                "user.name=probe",
                "-c",
                "user.email=probe@example.invalid",
            ])
            .args(["commit", "--quiet", "-m", "probe"])
            .status()
            .expect("Git commit should start outside the transfer root");
        assert!(status.success());
        let status = Command::new("git")
            .args(["clone", "--quiet"])
            .arg(&outside_repository)
            .arg(&cloned_repository)
            .status()
            .expect("Git clone should start outside the transfer root");
        assert!(status.success());
        let status = Command::new("git")
            .arg("-C")
            .arg(&cloned_repository)
            .args(["status", "--porcelain"])
            .status()
            .expect("Git status should start outside the transfer root");
        assert!(status.success());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ordinary_kernel_keeps_provider_launch_unwrapped() {
        let _env = crate::env_lock::lock();
        let previous_isolation = std::env::var_os(MANAGED_PROVIDER_ISOLATION_ENV);
        std::env::remove_var(MANAGED_PROVIDER_ISOLATION_ENV);
        let launch = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "ordinary-provider".to_string(),
            pty_target: None,
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec!["-c".to_string(), "printf ordinary".to_string()],
            pty_env: BTreeMap::from([(String::from("ORDINARY_FS"), String::from("1"))]),
            pty_env_remove: Vec::new(),
            working_directory: Some(PathBuf::from("/tmp")),
            structured_endpoint: None,
        };
        let request = LaunchProviderRequest::new(
            "ordinary-kernel-session",
            "codex",
            "codex",
            "default",
            "gpt-5.6-luna",
        );
        let prepared = apply_managed_provider_isolation(launch, &request)
            .expect("ordinary kernel launch should not require managed isolation");
        restore_env(MANAGED_PROVIDER_ISOLATION_ENV, previous_isolation);

        assert_eq!(prepared.pty_program.as_deref(), Some("/bin/sh"));
        assert_eq!(prepared.pty_args, ["-c", "printf ordinary"]);
        assert_eq!(prepared.working_directory, Some(PathBuf::from("/tmp")));
        assert!(!prepared
            .pty_args
            .windows(3)
            .any(|window| window == ["--setenv", MANAGED_PROVIDER_ISOLATION_MARKER_ENV, "1"]));
    }

    #[test]
    fn managed_launch_keeps_the_workspace_as_the_provider_protocol_cwd() {
        let request =
            LaunchProviderRequest::new("session-1", "codex", "codex", "default", "gpt-5.6-luna");

        assert_eq!(
            managed_launch_working_directory(
                &request,
                Path::new("/var/lib/chariox/home/provider-accounts/codex/default"),
                Path::new("/var/lib/chariox/home/workspaces/repo"),
            ),
            PathBuf::from("/var/lib/chariox/home/workspaces/repo"),
        );
    }

    #[test]
    fn managed_provider_account_launch_starts_from_the_real_profile_home() {
        let request = LaunchProviderRequest::new(
            "provider-account",
            "managed-utility",
            "managed-utility",
            "default",
            "default",
        );

        assert_eq!(
            managed_launch_working_directory(
                &request,
                Path::new("/var/lib/chariox/home/provider-accounts/codex/default"),
                Path::new(SANDBOX_HOME),
            ),
            PathBuf::from("/var/lib/chariox/home/provider-accounts/codex/default"),
        );
    }

    #[test]
    fn managed_slice_workspace_roots_are_loaded_and_removed_from_provider_environment() {
        let primary = std::env::temp_dir().join(format!(
            "chariox-managed-slice-primary-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let supporting = std::env::temp_dir().join(format!(
            "chariox-managed-slice-supporting-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&primary).expect("primary workspace should exist");
        std::fs::create_dir_all(&supporting).expect("supporting workspace should exist");

        let _env = crate::env_lock::lock();
        let previous_count = std::env::var_os(MANAGED_WORKSPACE_ROOT_COUNT_ENV);
        let previous_primary = std::env::var_os("CHARIOX_MANAGED_WORKSPACE_ROOT_0");
        let previous_supporting = std::env::var_os("CHARIOX_MANAGED_WORKSPACE_ROOT_1");
        let previous_git_key = std::env::var_os("GIT_CONFIG_KEY_17");
        let previous_git_value = std::env::var_os("GIT_CONFIG_VALUE_17");
        std::env::set_var(MANAGED_WORKSPACE_ROOT_COUNT_ENV, "2");
        std::env::set_var("CHARIOX_MANAGED_WORKSPACE_ROOT_0", &primary);
        std::env::set_var("CHARIOX_MANAGED_WORKSPACE_ROOT_1", &supporting);
        std::env::set_var("GIT_CONFIG_KEY_17", "unsafe.fixture");
        std::env::set_var("GIT_CONFIG_VALUE_17", "unsafe-fixture");

        let roots =
            managed_slice_workspace_roots().expect("managed slice workspace roots should load");
        #[cfg(target_os = "linux")]
        let isolated_roots = managed_workspace_roots(
            &LaunchProviderRequest::new(
                "hidden-session",
                "codex",
                "codex",
                "default",
                "gpt-5.6-luna",
            )
            .with_working_directory(primary.clone())
            .with_workspace_live_sync_roots(vec![primary.clone()]),
        )
        .expect("managed live-sync isolation should retain every slice workspace root");
        let removed = managed_provider_control_env_remove();

        restore_env(MANAGED_WORKSPACE_ROOT_COUNT_ENV, previous_count);
        restore_env("CHARIOX_MANAGED_WORKSPACE_ROOT_0", previous_primary);
        restore_env("CHARIOX_MANAGED_WORKSPACE_ROOT_1", previous_supporting);
        restore_env("GIT_CONFIG_KEY_17", previous_git_key);
        restore_env("GIT_CONFIG_VALUE_17", previous_git_value);

        assert_eq!(roots, vec![primary.clone(), supporting.clone()]);
        #[cfg(target_os = "linux")]
        assert_eq!(isolated_roots, vec![primary.clone(), supporting.clone()]);
        for name in [
            MANAGED_WORKSPACE_ROOT_COUNT_ENV,
            "CHARIOX_MANAGED_WORKSPACE_ROOT_0",
            "CHARIOX_MANAGED_WORKSPACE_ROOT_1",
            MANAGED_SLICE_SERVICE_ROOT_ENV,
            MANAGED_SLICE_PUBLICATION_ROOT_ENV,
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_KEY_17",
            "GIT_CONFIG_VALUE_17",
        ] {
            assert!(removed.iter().any(|removed| removed == name));
        }
        let _ = std::fs::remove_dir_all(primary);
        let _ = std::fs::remove_dir_all(supporting);
    }

    #[test]
    fn managed_namespace_exposes_runtime_directory_read_only_before_the_command() {
        let mut args = vec![
            "--tmpfs".to_string(),
            "/tmp".to_string(),
            "--setenv".to_string(),
            MANAGED_PROVIDER_ISOLATION_MARKER_ENV.to_string(),
            "1".to_string(),
            "--".to_string(),
            "/usr/local/bin/claude".to_string(),
        ];
        let directory = std::env::temp_dir().join("chariox-claude-runtime-test");
        let directory_text = directory.display().to_string();

        expose_runtime_directory_in_managed_namespace(&mut args, &directory)
            .expect("managed runtime directory should be exposed");

        let separator = args
            .iter()
            .position(|arg| arg == "--")
            .expect("command separator should remain present");
        assert_eq!(
            &args[separator - 5..separator],
            [
                "--dir",
                directory_text.as_str(),
                "--ro-bind",
                directory_text.as_str(),
                directory_text.as_str(),
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_namespace_concurrent_runs_keep_sibling_runtime_tokens_invisible() {
        let temp_root = std::env::temp_dir();
        let probe_id = format!(
            "chariox-managed-runtime-sibling-probe-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        );
        let runtime_a = temp_root.join(format!("{probe_id}-a"));
        let runtime_b = temp_root.join(format!("{probe_id}-b"));
        let cleanup = || {
            let _ = std::fs::remove_dir_all(&runtime_a);
            let _ = std::fs::remove_dir_all(&runtime_b);
        };
        std::fs::create_dir_all(&runtime_a).expect("first runtime directory should exist");
        std::fs::create_dir_all(&runtime_b).expect("second runtime directory should exist");
        let token_a = runtime_a.join("mcp-config.json");
        let token_b = runtime_b.join("mcp-config.json");
        std::fs::write(&token_a, "Bearer sibling-a").expect("first token should exist");
        std::fs::write(&token_b, "Bearer sibling-b").expect("second token should exist");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            for runtime in [&runtime_a, &runtime_b] {
                std::fs::set_permissions(runtime, std::fs::Permissions::from_mode(0o700))
                    .expect("runtime directory should be private");
            }
            for token in [&token_a, &token_b] {
                std::fs::set_permissions(token, std::fs::Permissions::from_mode(0o600))
                    .expect("runtime token should be private");
            }
        }

        let bwrap = Path::new(BWRAP_PATH);
        if !bwrap.is_file() {
            eprintln!(
                "skipped managed bwrap sibling probe: {} is unavailable",
                bwrap.display()
            );
            cleanup();
            return;
        }

        let make_args = |own_runtime: &Path, sibling_token: &Path, expected_token: &str| {
            let (mut args, _) = managed_namespace_args(None, Path::exists, None);
            args.extend([
                "--setenv".to_string(),
                MANAGED_PROVIDER_ISOLATION_MARKER_ENV.to_string(),
                "1".to_string(),
                "--".to_string(),
                "/bin/sh".to_string(),
            ]);
            expose_runtime_directory_in_managed_namespace(&mut args, own_runtime)
                .expect("current runtime should be re-exposed");
            args.extend([
                "-eu".to_string(),
                "-c".to_string(),
                concat!("test \"$(cat \"$1\")\" = \"$3\"\n", "test ! -e \"$2\"\n",).to_string(),
                "managed-runtime-sibling-probe".to_string(),
                own_runtime.join("mcp-config.json").display().to_string(),
                sibling_token.display().to_string(),
                expected_token.to_string(),
            ]);
            args
        };

        let mut first = Command::new(bwrap);
        first
            .args(make_args(&runtime_a, &token_b, "Bearer sibling-a"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut second = Command::new(bwrap);
        second
            .args(make_args(&runtime_b, &token_a, "Bearer sibling-b"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut first = match first.spawn() {
            Ok(child) => child,
            Err(error) => {
                eprintln!("skipped managed bwrap sibling probe: {error}");
                cleanup();
                return;
            }
        };
        let second = match second.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = first.kill();
                let _ = first.wait();
                eprintln!("skipped managed bwrap sibling probe: {error}");
                cleanup();
                return;
            }
        };
        let first = first
            .wait_with_output()
            .expect("first bwrap sibling probe should finish");
        let second = second
            .wait_with_output()
            .expect("second bwrap sibling probe should finish");
        let namespace_unavailable = |output: &std::process::Output| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            stderr.contains("No permissions to create a new namespace")
                || stderr.contains("Operation not permitted")
        };
        if namespace_unavailable(&first) || namespace_unavailable(&second) {
            eprintln!("skipped managed bwrap sibling probe: user namespaces are unavailable");
            cleanup();
            return;
        }
        assert!(
            first.status.success(),
            "first managed bwrap sibling probe failed: {}",
            String::from_utf8_lossy(&first.stderr)
        );
        assert!(
            second.status.success(),
            "second managed bwrap sibling probe failed: {}",
            String::from_utf8_lossy(&second.stderr)
        );
        cleanup();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_namespace_exposes_materialized_prompt_attachment_to_its_provider() {
        use crate::runtime::agent_actor::prompt_attachment_materialization::INLINE_PROMPT_ATTACHMENT_DIR;
        use base64::Engine;

        let session_id = format!(
            "managed-attachment-session-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        );
        let request =
            LaunchProviderRequest::new(&session_id, "codex", "codex", "default", "gpt-5.6-luna")
                .with_agent_id("agent:two");
        let attachment_root = managed_prompt_attachment_root(&request)
            .expect("managed attachment root should prepare")
            .expect("agent-bound launches should have an attachment root");
        let attachment = crate::session::PromptAttachment::new(
            "chariox-cloud://artifact/art-1",
            "text/plain",
            Some("remote-attachment-probe.txt".to_string()),
        )
        .with_contents_base64(base64::engine::general_purpose::STANDARD.encode("attachment probe"));
        let materialized = crate::runtime::agent_actor::prompt_attachment_materialization::materialize_inline_prompt_attachments(
            &session_id,
            "agent:two",
            vec![attachment],
        )
        .expect("inline attachment should materialize before provider input");
        let attachment_path = Path::new(
            materialized[0]
                .url()
                .strip_prefix("file://")
                .expect("materialized attachment should use a file URL"),
        );

        let (args, _) = managed_namespace_args(
            None,
            |path| path == Path::new("/tmp"),
            Some(&attachment_root),
        );

        assert!(attachment_path.starts_with(&attachment_root));
        assert!(args.windows(3).any(|window| {
            window[0] == "--ro-bind"
                && window[1] == attachment_root.to_string_lossy()
                && window[2] == attachment_root.to_string_lossy()
        }));
        let global_root = std::env::temp_dir().join(INLINE_PROMPT_ATTACHMENT_DIR);
        assert!(!args.windows(3).any(|window| {
            window[0] == "--ro-bind"
                && window[1] == global_root.to_string_lossy()
                && window[2] == global_root.to_string_lossy()
        }));
        let _ = std::fs::remove_dir_all(
            attachment_root
                .parent()
                .expect("attachment root should have a session parent"),
        );
    }

    fn restore_env(name: &str, previous: Option<std::ffi::OsString>) {
        match previous {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}
