use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::config::{DaemonConfig, SliceImageBuildPolicy, DEFAULT_LINUX_SLICE_DOCKER_IMAGE};
use crate::error::DaemonError;

use super::model::{
    LocalDockerSliceAction, SliceBackendKind, SliceDisplayMode, SliceLogEntry,
    SliceProviderLoginStart, SliceRecord, SliceRelayEndpoint, SliceSavedStateRecord,
};
use super::ports::{busy_published_ports_for_slice, LocalDockerSlicePorts};

mod state;
#[cfg(test)]
mod tests;

pub use state::{
    create_local_docker_slice_backup, create_local_docker_slice_backup_live,
    default_local_docker_saved_state, remove_local_docker_saved_state,
    save_local_docker_slice_state, save_local_docker_slice_state_live,
    set_local_docker_default_saved_state,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDockerSliceRelay {
    pub relay_url: String,
    pub container_relay_url: Option<String>,
    pub relay_token: String,
    pub cloud_relay_config_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDockerSliceOptions {
    pub root: PathBuf,
    pub docker_image: String,
    pub build_image: SliceImageBuildPolicy,
    pub extension_dockerfile: Option<PathBuf>,
    pub saved_home_archive: Option<PathBuf>,
    pub allow_unconfined_seccomp: bool,
    pub memory_mb: Option<u32>,
    pub cpus: Option<String>,
    pub screen_width: u32,
    pub screen_height: u32,
}

const DOCKER_READY_ATTEMPTS: usize = 60;
const DOCKER_READY_RETRY_DELAY_MS: u64 = 1_000;
const SLICE_DOCKER_PROVISIONER_ENV: &str = "ARROBA_SLICE_DOCKER_PROVISIONER";

impl LocalDockerSliceOptions {
    pub fn from_config(config: &DaemonConfig) -> Self {
        let linux = &config.user_config.slices.linux;
        Self {
            root: config.slice_root(),
            docker_image: linux
                .docker_image
                .clone()
                .unwrap_or_else(|| DEFAULT_LINUX_SLICE_DOCKER_IMAGE.to_string()),
            build_image: linux.build_image.unwrap_or(SliceImageBuildPolicy::Auto),
            extension_dockerfile: linux
                .extension_dockerfile
                .as_deref()
                .map(expand_user_path_for_slice),
            saved_home_archive: None,
            allow_unconfined_seccomp: linux.allow_unconfined_seccomp.unwrap_or(false),
            memory_mb: linux.memory_mb,
            cpus: linux.cpus.clone(),
            screen_width: linux.screen_width.unwrap_or(1280),
            screen_height: linux.screen_height.unwrap_or(800),
        }
    }

    pub fn with_saved_state(mut self, state: &SliceSavedStateRecord) -> Self {
        self.docker_image = state.image_ref.clone();
        self.saved_home_archive = Some(PathBuf::from(&state.home_archive_path));
        self
    }

    fn screen_geometry(&self) -> String {
        format!("{}x{}x24", self.screen_width, self.screen_height)
    }
}

pub fn run_local_docker_slice_action(
    record: &SliceRecord,
    action: LocalDockerSliceAction,
    relay: Option<LocalDockerSliceRelay>,
    provider: Option<&str>,
    options: &LocalDockerSliceOptions,
) -> Result<(), DaemonError> {
    if record.backend != SliceBackendKind::LocalDocker {
        return Err(DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!("slice `{}` is not a local Docker slice", record.name),
        });
    }
    if record.os != "linux" {
        return Err(DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!(
                "local Docker slices only support linux, got `{}`",
                record.os
            ),
        });
    }
    if action == LocalDockerSliceAction::Provision {
        ensure_host_docker_ready()?;
        ensure_local_docker_slice_ports_available(record)?;
    }
    let script = linux_docker_slice_script()?;
    let mut command = Command::new(&script);
    command.arg(match action {
        LocalDockerSliceAction::Provision => "provision",
        LocalDockerSliceAction::ImportProviderAuth => "import-provider-auth",
        LocalDockerSliceAction::RemoveProviderAuth => "remove-provider-auth",
        LocalDockerSliceAction::Stop => "stop",
        LocalDockerSliceAction::Destroy => "destroy",
    });
    configure_local_docker_slice_command(&mut command, record, relay, options)?;
    if let Some(provider) = provider {
        command.env("ARROBA_SLICE_AUTH_PROVIDER", provider);
    }

    let log_path = local_docker_slice_action_log_path(&options.root, record, action);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!(
                "failed to create slice log dir {}: {error}",
                parent.display()
            ),
        })?;
    }
    let log_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&log_path)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!(
                "failed to open slice provisioner log {}: {error}",
                log_path.display()
            ),
        })?;
    let stderr_log = log_file
        .try_clone()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!(
                "failed to open slice provisioner stderr log {}: {error}",
                log_path.display()
            ),
        })?;
    let status = command
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(stderr_log))
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!(
                "failed to run {} {} (log: {}): {error}",
                script.display(),
                action.as_str(),
                log_path.display()
            ),
        })?;
    if status.success() {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "slice.local_docker",
        message: format!(
            "{} {} failed with status {} (log: {}): {}",
            script.display(),
            action.as_str(),
            status,
            log_path.display(),
            command_log_preview(&log_path)
        ),
    })
}

pub fn start_local_docker_slice_provider_login(
    record: &SliceRecord,
    provider: &str,
    options: &LocalDockerSliceOptions,
) -> Result<SliceProviderLoginStart, DaemonError> {
    if record.backend != SliceBackendKind::LocalDocker {
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message: format!("slice `{}` is not a local Docker slice", record.name),
        });
    }
    if record.os != "linux" {
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message: format!(
                "local Docker slices only support linux, got `{}`",
                record.os
            ),
        });
    }
    let script = linux_docker_slice_script()?;
    let mut command = Command::new(&script);
    command
        .arg("start-provider-login")
        .env("ARROBA_SLICE_LOGIN_PROVIDER", provider);
    configure_local_docker_slice_command(&mut command, record, None, options)?;
    let output = command
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message: format!(
                "failed to start provider login in slice `{}`: {error}",
                record.name
            ),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message: format!(
                "provider login in slice `{}` failed with status {}: {}",
                record.name,
                output.status,
                compact_login_message(&combined)
            ),
        });
    }
    let clean = compact_login_message(&combined);
    let verification_url = first_url(&clean);
    let user_code = first_device_code(&clean);
    Ok(SliceProviderLoginStart {
        provider: provider.to_string(),
        login_kind: if user_code.is_some() {
            "device".to_string()
        } else {
            "browser".to_string()
        },
        auth_url: verification_url.clone(),
        verification_url,
        user_code,
        status: "started".to_string(),
        message: clean,
    })
}

pub fn collect_local_docker_slice_logs(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    tail_lines: Option<u32>,
) -> Result<Vec<SliceLogEntry>, DaemonError> {
    if record.backend != SliceBackendKind::LocalDocker {
        return Err(DaemonError::LocalTransport {
            operation: "slice.logs",
            message: format!("slice `{}` is not a local Docker slice", record.name),
        });
    }
    if record.os != "linux" {
        return Err(DaemonError::LocalTransport {
            operation: "slice.logs",
            message: format!(
                "local Docker slices only support linux, got `{}`",
                record.os
            ),
        });
    }

    let tail_lines = tail_lines.unwrap_or(200).clamp(1, 2_000);
    let mut entries = Vec::new();
    for action in [
        LocalDockerSliceAction::Provision,
        LocalDockerSliceAction::ImportProviderAuth,
        LocalDockerSliceAction::RemoveProviderAuth,
        LocalDockerSliceAction::Stop,
        LocalDockerSliceAction::Destroy,
    ] {
        let path = local_docker_slice_action_log_path(&options.root, record, action);
        if path.is_file() {
            entries.push(read_slice_log_file_entry(
                action.as_str(),
                &path,
                tail_lines as usize,
            ));
        }
    }
    entries.push(local_docker_container_log_entry(record, tail_lines));
    Ok(entries)
}

pub fn inspect_local_docker_slice_host_runtime(
    record: &SliceRecord,
) -> super::SliceHostRuntimeState {
    if record.backend != SliceBackendKind::LocalDocker || record.os != "linux" {
        return super::SliceHostRuntimeState::Unknown;
    }
    let container = local_docker_container_name(record);
    let output = Command::new("docker")
        .args([
            "inspect",
            "--format",
            "{{.State.Running}} {{.State.Status}}",
            &container,
        ])
        .output();
    let Ok(output) = output else {
        return super::SliceHostRuntimeState::Unknown;
    };
    if !output.status.success() {
        return super::SliceHostRuntimeState::Missing;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.split_whitespace();
    match (fields.next(), fields.next()) {
        (Some("true"), _) => super::SliceHostRuntimeState::Running,
        (Some("false"), Some("exited" | "created" | "dead" | "paused")) => {
            super::SliceHostRuntimeState::Stopped
        }
        (Some("false"), _) => super::SliceHostRuntimeState::Stopped,
        _ => super::SliceHostRuntimeState::Unknown,
    }
}

fn read_slice_log_file_entry(source: &str, path: &Path, tail_lines: usize) -> SliceLogEntry {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (text, truncated) = tail_text_lines(&text, tail_lines);
            SliceLogEntry {
                source: source.to_string(),
                path: Some(path.display().to_string()),
                text,
                truncated,
            }
        }
        Err(error) => SliceLogEntry {
            source: source.to_string(),
            path: Some(path.display().to_string()),
            text: format!("failed to read log: {error}"),
            truncated: false,
        },
    }
}

fn local_docker_container_log_entry(record: &SliceRecord, tail_lines: u32) -> SliceLogEntry {
    let container = local_docker_container_name(record);
    let tail_lines_arg = tail_lines.to_string();
    let output = Command::new("docker")
        .args(["logs", "--tail", &tail_lines_arg, &container])
        .output();
    match output {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            if !output.status.success() && text.trim().is_empty() {
                text = format!("docker logs failed with status {}", output.status);
            }
            SliceLogEntry {
                source: "container".to_string(),
                path: None,
                text: text.trim().to_string(),
                truncated: false,
            }
        }
        Err(error) => SliceLogEntry {
            source: "container".to_string(),
            path: None,
            text: format!("docker logs unavailable: {error}"),
            truncated: false,
        },
    }
}

fn tail_text_lines(text: &str, tail_lines: usize) -> (String, bool) {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() <= tail_lines {
        return (text.trim().to_string(), false);
    }
    (
        lines[lines.len().saturating_sub(tail_lines)..]
            .join("\n")
            .trim()
            .to_string(),
        true,
    )
}

fn configure_local_docker_slice_command(
    command: &mut Command,
    record: &SliceRecord,
    relay: Option<LocalDockerSliceRelay>,
    options: &LocalDockerSliceOptions,
) -> Result<(), DaemonError> {
    let ports = LocalDockerSlicePorts::for_record(record);
    command
        .env("ARROBA_SLICE_NAME", local_docker_container_name(record))
        .env("ARROBA_SLICE_DOCKER_IMAGE", &options.docker_image)
        .env(
            "ARROBA_SLICE_BUILD_IMAGE",
            options.build_image.as_env_value(),
        )
        .env(
            "ARROBA_SLICE_HOME_VOLUME",
            format!("{}-home", local_docker_container_name(record)),
        )
        .env("ARROBA_SLICE_SCREEN_GEOMETRY", options.screen_geometry())
        .env("ARROBA_SLICE_CODEX_PORT", ports.codex.to_string())
        .env("ARROBA_SLICE_OPENCODE_PORT", ports.opencode.to_string())
        .env("ARROBA_SLICE_CODEX_PORT_RANGE", ports.codex_range())
        .env("ARROBA_SLICE_OPENCODE_PORT_RANGE", ports.opencode_range())
        .env("ARROBA_SLICE_KERNEL_PORT", ports.kernel.to_string())
        .env("ARROBA_SLICE_MCP_PORT", ports.mcp.to_string())
        .env("ARROBA_SLICE_RELAY_PORT", ports.relay.to_string())
        .env("ARROBA_SLICE_NOVNC_PORT", ports.novnc.to_string())
        .env(
            "ARROBA_SLICE_DISPLAY_MODE",
            match record.display_mode {
                SliceDisplayMode::Headed => "headed",
                SliceDisplayMode::Headless => "headless",
            },
        )
        .env("ARROBA_SLICE_START_DESKTOP", "1")
        .env("ARROBA_SLICE_START_PROVIDER_SERVERS", "0")
        .env("ARROBA_SLICE_START_RUNTIME", "1")
        .env("ARROBA_SLICE_IMPORT_PROVIDER_AUTH", "0")
        .env(
            "ARROBA_SLICE_ALLOW_UNCONFINED_SECCOMP",
            if options.allow_unconfined_seccomp {
                "1"
            } else {
                "0"
            },
        )
        .env("ARROBA_SLICE_PROVIDER_BIND_HOST", "127.0.0.1")
        .env(
            "ARROBA_SLICE_DAEMON_ALIAS",
            record.worker_kernel_ref.clone(),
        )
        .env("ARROBA_SLICE_MACHINE_ID", format!("slice:{}", record.id))
        .env("ARROBA_SLICE_MACHINE_ALIAS", record.name.clone());
    if let Some(memory_mb) = options.memory_mb {
        command.env("ARROBA_SLICE_DOCKER_MEMORY", format!("{memory_mb}m"));
    }
    if let Some(cpus) = options.cpus.as_deref() {
        command.env("ARROBA_SLICE_DOCKER_CPUS", cpus);
    }
    if let Some(extension_dockerfile) = options.extension_dockerfile.as_deref() {
        command.env("ARROBA_SLICE_EXTENSION_DOCKERFILE", extension_dockerfile);
    }
    if let Some(saved_home_archive) = options.saved_home_archive.as_deref() {
        command.env("ARROBA_SLICE_SAVED_HOME_ARCHIVE", saved_home_archive);
    }
    if let Some(relay) = relay {
        let LocalDockerSliceRelay {
            relay_token,
            container_relay_url,
            cloud_relay_config_json,
            ..
        } = relay;
        if let Some(cloud_relay_config_json) = cloud_relay_config_json {
            let host_config_path =
                write_cloud_relay_config_file(record, options, &cloud_relay_config_json)?;
            command.env(
                "ARROBA_SLICE_CLOUD_RELAY_CONFIG_HOST_PATH",
                host_config_path,
            );
            command.env(
                "ARROBA_SLICE_CLOUD_RELAY_CONFIG_JSON",
                cloud_relay_config_json,
            );
        }
        command.env("ARROBA_SLICE_RELAY_TOKEN", relay_token);
        if let Some(container_relay_url) = container_relay_url {
            command.env(
                "ARROBA_SLICE_RELAY_URL",
                relay_url_for_container(&container_relay_url),
            );
        }
    }
    if let Some(workspace_mount) = record.workspace_mount.as_deref() {
        command.env("ARROBA_SLICE_WORKSPACE", workspace_mount);
    }
    Ok(())
}

fn write_cloud_relay_config_file(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    config_json: &str,
) -> Result<PathBuf, DaemonError> {
    let dir = options.root.join("runtime").join(&record.id);
    std::fs::create_dir_all(&dir).map_err(|error| DaemonError::LocalTransport {
        operation: "slice.local_docker",
        message: format!(
            "failed to create slice runtime config dir {}: {error}",
            dir.display()
        ),
    })?;
    let path = dir.join("cloud-relay-config.json");
    std::fs::write(&path, config_json).map_err(|error| DaemonError::LocalTransport {
        operation: "slice.local_docker",
        message: format!(
            "failed to write slice cloud relay config {}: {error}",
            path.display()
        ),
    })?;
    Ok(path)
}

fn compact_login_message(output: &str) -> String {
    strip_ansi(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(24)
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

fn first_url(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| part.starts_with("https://") || part.starts_with("http://"))
        .map(|part| {
            part.trim_matches(|ch: char| ch == ',' || ch == '.' || ch == ')' || ch == ']')
                .to_string()
        })
}

fn first_device_code(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| {
            let trimmed = part.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-');
            trimmed.len() >= 8
                && trimmed.contains('-')
                && trimmed
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '-')
        })
        .map(|part| {
            part.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                .to_string()
        })
}

pub fn local_docker_private_relay(record: &SliceRecord) -> LocalDockerSliceRelay {
    let ports = LocalDockerSlicePorts::for_record(record);
    LocalDockerSliceRelay {
        relay_url: format!("ws://127.0.0.1:{}", ports.relay),
        container_relay_url: None,
        relay_token: local_docker_private_relay_token(record),
        cloud_relay_config_json: None,
    }
}

pub fn local_docker_private_relay_endpoint(record: &SliceRecord) -> SliceRelayEndpoint {
    SliceRelayEndpoint {
        url: local_docker_private_relay(record).relay_url,
        private: true,
    }
}

pub fn local_docker_private_relay_token(record: &SliceRecord) -> String {
    format!("slice-local-{}-{}", record.owner_kernel_id, record.id)
}

pub(super) fn relay_url_for_container(relay_url: &str) -> String {
    relay_url
        .strip_prefix("ws://127.0.0.1:")
        .map(|rest| format!("ws://host.docker.internal:{rest}"))
        .or_else(|| {
            relay_url
                .strip_prefix("ws://localhost:")
                .map(|rest| format!("ws://host.docker.internal:{rest}"))
        })
        .unwrap_or_else(|| relay_url.to_string())
}

impl LocalDockerSliceAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Provision => "provision",
            Self::ImportProviderAuth => "import-provider-auth",
            Self::RemoveProviderAuth => "remove-provider-auth",
            Self::Stop => "stop",
            Self::Destroy => "destroy",
        }
    }
}

pub(super) fn local_docker_container_name(record: &SliceRecord) -> String {
    format!("arroba-slice-{}", record.name)
}

pub(super) fn ensure_local_docker_slice_ports_available(
    record: &SliceRecord,
) -> Result<(), DaemonError> {
    if local_docker_container_is_running(record) {
        return Ok(());
    }
    let busy_ports = busy_published_ports_for_slice(record);
    if busy_ports.is_empty() {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "slice.local_docker.ports",
        message: format!(
            "slice `{}` cannot start because host port(s) {} are already in use",
            record.name,
            busy_ports
                .into_iter()
                .map(|port| port.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

pub(super) fn local_docker_container_is_running(record: &SliceRecord) -> bool {
    let output = Command::new("docker")
        .args(["ps", "--format", "{{.Names}}"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let container_name = local_docker_container_name(record);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim() == container_name)
}

pub(super) fn run_local_docker_slice_screen(
    record: &SliceRecord,
    action: &'static str,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let container = local_docker_container_name(record);
    let status = Command::new("docker")
        .args([
            "exec",
            "-u",
            "slice",
            &container,
            "/opt/arroba-slice/slice-screen.sh",
            action,
        ])
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!(
                "failed to run slice screen `{action}` in container `{container}`: {error}"
            ),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "slice screen `{action}` in container `{container}` failed with status {status}"
            ),
        })
    }
}

pub(super) fn local_docker_slice_action_log_path(
    root: &Path,
    record: &SliceRecord,
    action: LocalDockerSliceAction,
) -> PathBuf {
    root.join("logs").join(format!(
        "{}-{}.log",
        local_docker_container_name(record),
        action.as_str()
    ))
}

fn expand_user_path_for_slice(value: &str) -> PathBuf {
    let value = value.trim();
    if value == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(suffix) = value.strip_prefix("~/") {
        if let Some(home_dir) = std::env::var_os("HOME").map(PathBuf::from) {
            return home_dir.join(suffix);
        }
    }
    PathBuf::from(value)
}

fn linux_docker_slice_script() -> Result<PathBuf, DaemonError> {
    resolve_linux_docker_slice_script(
        std::env::var_os(SLICE_DOCKER_PROVISIONER_ENV).map(PathBuf::from),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

fn resolve_linux_docker_slice_script(
    configured: Option<PathBuf>,
    manifest_dir: &Path,
) -> Result<PathBuf, DaemonError> {
    let script = match configured {
        Some(script) => script,
        None => {
            let repo_root = manifest_dir
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.local_docker",
                    message: "failed to resolve repository root for slice scripts".to_string(),
                })?;
            repo_root
                .join("apps")
                .join("kernel")
                .join("slice-linux-docker")
                .join("provision-linux-docker-slice.sh")
        }
    };
    if script.is_file() {
        Ok(script)
    } else {
        Err(DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!("slice Docker provisioner not found at {}", script.display()),
        })
    }
}

pub(super) fn ensure_host_docker_ready() -> Result<(), DaemonError> {
    if !command_exists("docker") {
        return Err(DaemonError::LocalTransport {
            operation: "slice.local_docker.docker",
            message: "docker is required for local Docker slices".to_string(),
        });
    }
    if docker_is_ready() {
        return Ok(());
    }

    let mut start_attempts = Vec::new();
    if command_exists("colima") {
        start_attempts.push("colima start");
        let _ = Command::new("colima")
            .arg("start")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if wait_for_docker_ready() {
            return Ok(());
        }
    }

    #[cfg(target_os = "macos")]
    {
        if command_exists("open") {
            start_attempts.push("open -ga Docker");
            let _ = Command::new("open")
                .args(["-ga", "Docker"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if wait_for_docker_ready() {
                return Ok(());
            }
        }
    }

    let attempted = if start_attempts.is_empty() {
        "no supported Docker launcher found".to_string()
    } else {
        format!("attempted {}", start_attempts.join(" and "))
    };
    Err(DaemonError::LocalTransport {
        operation: "slice.local_docker.docker",
        message: format!("docker is not running and could not be started ({attempted})"),
    })
}

fn wait_for_docker_ready() -> bool {
    for _ in 0..DOCKER_READY_ATTEMPTS {
        if docker_is_ready() {
            return true;
        }
        thread::sleep(Duration::from_millis(DOCKER_READY_RETRY_DELAY_MS));
    }
    false
}

fn docker_is_ready() -> bool {
    Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .args(["-c", "command -v \"$1\" >/dev/null 2>&1", "sh", command])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn command_log_preview(path: &Path) -> String {
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    if text.len() > 4_000 {
        let start = text.len().saturating_sub(4_000);
        text = text[start..].to_string();
        text.push_str("...");
    }
    let text = text.trim();
    if text.is_empty() {
        "<no output>".to_string()
    } else {
        text.to_string()
    }
}
