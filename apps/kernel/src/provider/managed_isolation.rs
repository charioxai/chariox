use std::collections::BTreeMap;
#[cfg(any(target_os = "linux", test))]
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use crate::error::DaemonError;

use super::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

pub(crate) const MANAGED_PROVIDER_ISOLATION_ENV: &str = "CHARIOX_MANAGED_PROVIDER_ISOLATION";
#[cfg(any(target_os = "linux", test))]
const CLAUDE_SANDBOX_ENV: &str = "IS_SANDBOX";
#[cfg(target_os = "linux")]
pub(crate) const MANAGED_PROVIDER_HOME_ENV: &str = "CHARIOX_MANAGED_PROVIDER_HOME";
pub(crate) const MANAGED_PROVIDER_ISOLATION_MARKER_ENV: &str =
    "CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE";

#[cfg(target_os = "linux")]
const BWRAP_PATH: &str = "/usr/bin/bwrap";
#[cfg(any(target_os = "linux", test))]
const SANDBOX_HOME: &str = "/home/chariox";
#[cfg(target_os = "linux")]
const SANDBOX_ACCOUNT_ROOT: &str = "/home/chariox/.provider-account";

const CONTROL_ENVIRONMENT_NAMES: &[&str] = &[
    "CHARIOX_RELAY_TOKEN",
    "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
    "CHARIOX_CLOUD_RELAY_CONFIG_PATH",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
    "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
    "CHARIOX_SLICE_DOCKER_BROKER_FD",
    "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED",
    "CHARIOX_MANAGED_BOOTSTRAP_PATH",
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

pub(crate) fn managed_provider_control_env_remove() -> &'static [&'static str] {
    CONTROL_ENVIRONMENT_NAMES
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
        return Err(isolation_error(
            "managed provider isolation is only supported on Linux",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        validate_bubblewrap_binary()?;
        let provider_home = managed_provider_home()?;
        let workspace_roots = managed_workspace_roots(request)?;
        let working_directory = managed_working_directory(request, &workspace_roots)?;
        let runtime_roots = managed_runtime_roots(&launch)?;
        let account_bindings = managed_account_bindings(&mut launch.pty_env)?;
        let program = rewrite_managed_program_path(&program, &provider_home, &account_bindings);

        let resolver = managed_resolver_binding()?;
        let (mut args, mut created_directories) =
            managed_namespace_args(resolver.as_deref(), Path::exists);
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
        for (source, destination) in account_bindings {
            append_bind(&mut args, &source, &destination, &mut created_directories);
        }
        for root in runtime_roots {
            append_bind(&mut args, &root, &root, &mut created_directories);
        }
        for root in workspace_roots {
            append_bind(&mut args, &root, &root, &mut created_directories);
        }

        append_managed_namespace_environment(&mut args, request);
        for name in CONTROL_ENVIRONMENT_NAMES {
            args.extend(["--unsetenv".to_string(), (*name).to_string()]);
            if !launch.pty_env_remove.iter().any(|value| value == name) {
                launch.pty_env_remove.push((*name).to_string());
            }
        }
        args.extend([
            "--chdir".to_string(),
            working_directory.display().to_string(),
            "--".to_string(),
            program,
        ]);
        args.append(&mut launch.pty_args);

        launch.pty_program = Some(BWRAP_PATH.to_string());
        launch.pty_args = args;
        // The sandbox path does not exist until bubblewrap assembles the
        // namespace. Spawn bubblewrap from the real provider home and let its
        // own --chdir select the approved in-sandbox directory.
        launch.working_directory = Some(provider_home);
        launch.process_label = format!("{}:managed-isolated", launch.process_label);
        Ok(launch)
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
        pty_env_remove: CONTROL_ENVIRONMENT_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
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
fn validate_bubblewrap_binary() -> Result<(), DaemonError> {
    use std::os::unix::fs::MetadataExt;

    let path = Path::new(BWRAP_PATH);
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        isolation_error(format!(
            "managed provider isolation needs {BWRAP_PATH}: {error}"
        ))
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
    {
        return Err(isolation_error(
            "managed provider isolation needs a root-owned, non-writable executable /usr/bin/bwrap",
        ));
    }
    Ok(())
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
) -> (Vec<String>, BTreeSet<PathBuf>) {
    let mut args = vec![
        "--die-with-parent".to_string(),
        "--new-session".to_string(),
        "--unshare-user".to_string(),
        "--unshare-pid".to_string(),
        "--unshare-ipc".to_string(),
        "--unshare-uts".to_string(),
        "--unshare-cgroup-try".to_string(),
        "--disable-userns".to_string(),
        "--uid".to_string(),
        "0".to_string(),
        "--gid".to_string(),
        "0".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
        "--ro-bind".to_string(),
        "/".to_string(),
        "/".to_string(),
    ];

    for path in ["/home", "/tmp", "/run", "/var/lib", "/var/tmp"] {
        if path_exists(Path::new(path)) {
            args.extend(["--tmpfs".to_string(), path.to_string()]);
        }
    }
    for path in ["/workspace", "/root", "/mnt", "/media"] {
        if path_exists(Path::new(path)) {
            args.extend(["--tmpfs".to_string(), path.to_string()]);
        }
    }
    let slice_logs = Path::new("/opt/chariox-slice/logs");
    if path_exists(slice_logs) {
        args.extend(["--tmpfs".to_string(), slice_logs.display().to_string()]);
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
    (args, created_directories)
}

#[cfg(target_os = "linux")]
fn managed_workspace_roots(request: &LaunchProviderRequest) -> Result<Vec<PathBuf>, DaemonError> {
    if request.session_id == "provider-account" {
        return Ok(Vec::new());
    }
    let mut roots = request.workspace_live_sync_roots.clone();
    if roots.is_empty() {
        let working_directory = request
            .working_directory
            .as_ref()
            .ok_or_else(|| isolation_error("managed provider launch has no workspace"))?;
        roots
            .push(resolve_git_root(working_directory).unwrap_or_else(|| working_directory.clone()));
    }
    let mut canonical = Vec::new();
    for root in roots {
        let root = canonical_directory(&root, "managed provider workspace")?;
        if canonical.iter().all(|existing| existing != &root) {
            canonical.push(root);
        }
    }
    for (index, root) in canonical.iter().enumerate() {
        if canonical.iter().enumerate().any(|(other_index, other)| {
            index != other_index && (root.starts_with(other) || other.starts_with(root))
        }) {
            return Err(isolation_error(
                "managed provider workspace roots must be separate repository mounts",
            ));
        }
    }
    if canonical.is_empty() {
        return Err(isolation_error(
            "managed provider launch has no approved repository roots",
        ));
    }
    Ok(canonical)
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
    if !roots.iter().any(|root| directory.starts_with(root)) {
        return Err(isolation_error(
            "managed provider working directory is outside the approved repositories",
        ));
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
    fn namespace_args_restore_the_resolver_after_masking_run() {
        let resolver = Path::new("/run/systemd/resolve/stub-resolv.conf");
        let (args, _) = managed_namespace_args(Some(resolver), |path| path == Path::new("/run"));

        let mask = args
            .windows(2)
            .position(|args| args == ["--tmpfs", "/run"])
            .expect("run should be masked");
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
        assert!(binding > mask);
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

    #[test]
    fn namespace_args_do_not_bind_a_resolver_outside_masked_run() {
        let (args, _) = managed_namespace_args(None, |path| path == Path::new("/run"));

        assert_eq!(args.iter().filter(|arg| *arg == "--ro-bind").count(), 1);
        assert!(!args.iter().any(|arg| arg == "/etc/resolv.conf"));
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
        let directory = Path::new("/tmp/chariox-claude-runtime-test");

        expose_runtime_directory_in_managed_namespace(&mut args, directory)
            .expect("managed runtime directory should be exposed");

        let separator = args
            .iter()
            .position(|arg| arg == "--")
            .expect("command separator should remain present");
        assert_eq!(
            &args[separator - 5..separator],
            [
                "--dir",
                "/tmp/chariox-claude-runtime-test",
                "--ro-bind",
                "/tmp/chariox-claude-runtime-test",
                "/tmp/chariox-claude-runtime-test",
            ]
        );
    }
}
