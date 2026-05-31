use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod model;
mod ports;
mod store;

use crate::config::{DaemonConfig, SliceImageBuildPolicy};
use crate::error::DaemonError;
pub use model::{
    CreateSliceInput, LocalDockerSliceAction, SliceBackendKind, SliceDisplayEndpoint,
    SliceDisplayEndpointAccess, SliceDisplayEndpointKind, SliceDisplayMode, SliceLocalDockerPorts,
    SliceLogEntry, SliceProviderLoginStart, SliceRecord, SliceRelayEndpoint, SliceStatus,
};
use ports::{busy_published_ports_for_slice, LocalDockerSlicePorts};
pub use store::{SliceOperationGuard, SliceStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDockerSliceRelay {
    pub relay_url: String,
    pub container_relay_url: Option<String>,
    pub relay_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDockerSliceOptions {
    pub root: PathBuf,
    pub docker_image: String,
    pub build_image: SliceImageBuildPolicy,
    pub extension_dockerfile: Option<PathBuf>,
    pub allow_unconfined_seccomp: bool,
    pub memory_mb: Option<u32>,
    pub cpus: Option<String>,
    pub screen_width: u32,
    pub screen_height: u32,
}

impl LocalDockerSliceOptions {
    pub fn from_config(config: &DaemonConfig) -> Self {
        let linux = &config.user_config.slices.linux;
        Self {
            root: config.slice_root(),
            docker_image: linux
                .docker_image
                .clone()
                .unwrap_or_else(|| "arroba-slice-linux:local".to_string()),
            build_image: linux.build_image.unwrap_or(SliceImageBuildPolicy::Auto),
            extension_dockerfile: linux
                .extension_dockerfile
                .as_deref()
                .map(expand_user_path_for_slice),
            allow_unconfined_seccomp: linux.allow_unconfined_seccomp.unwrap_or(false),
            memory_mb: linux.memory_mb,
            cpus: linux.cpus.clone(),
            screen_width: linux.screen_width.unwrap_or(1280),
            screen_height: linux.screen_height.unwrap_or(800),
        }
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
    configure_local_docker_slice_command(&mut command, record, relay, options);
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
    configure_local_docker_slice_command(&mut command, record, None, options);
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
) {
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
            "ARROBA_SLICE_START_DESKTOP",
            if record.display_mode == SliceDisplayMode::Headed {
                "1"
            } else {
                "0"
            },
        )
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
        .env("ARROBA_SLICE_PROVIDER_BIND_HOST", "0.0.0.0")
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
    if let Some(relay) = relay {
        command.env("ARROBA_SLICE_RELAY_TOKEN", relay.relay_token);
        if let Some(container_relay_url) = relay.container_relay_url {
            command.env(
                "ARROBA_SLICE_RELAY_URL",
                relay_url_for_container(&container_relay_url),
            );
        }
    }
    if let Some(workspace_mount) = record.workspace_mount.as_deref() {
        command.env("ARROBA_SLICE_WORKSPACE", workspace_mount);
    }
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

fn relay_url_for_container(relay_url: &str) -> String {
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

fn local_docker_container_name(record: &SliceRecord) -> String {
    format!("arroba-slice-{}", record.name)
}

fn ensure_local_docker_slice_ports_available(record: &SliceRecord) -> Result<(), DaemonError> {
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

fn local_docker_container_is_running(record: &SliceRecord) -> bool {
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

fn local_docker_slice_action_log_path(
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
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: "failed to resolve repository root for slice scripts".to_string(),
        })?;
    let script = repo_root
        .join("apps")
        .join("kernel")
        .join("slice-linux-docker")
        .join("provision-linux-docker-slice.sh");
    if script.is_file() {
        Ok(script)
    } else {
        Err(DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!("slice Docker provisioner not found at {}", script.display()),
        })
    }
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::TcpListener;

    use super::*;
    use crate::slice_provider_auth::SliceProviderAuthSummary;

    fn create_input(name: &str) -> CreateSliceInput {
        CreateSliceInput {
            name: name.to_string(),
            backend: SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            display_mode: SliceDisplayMode::Headed,
            workspace_id: None,
            worktree_id: None,
            workspace_mount: Some("/repo".to_string()),
            worker_kernel_ref: None,
            display_url: Some("http://127.0.0.1:6080".to_string()),
            provider_auth: Vec::new(),
            now_ms: 42,
        }
    }

    #[test]
    fn slice_store_creates_resolves_and_exposes_display_endpoint() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        assert_eq!(slice.id, "slice-1");
        assert_eq!(slice.worker_kernel_ref, "slice:dev");
        assert_eq!(
            store.resolve("dev").expect("slice should resolve").id,
            slice.id
        );
        assert_eq!(
            store
                .display_endpoint("dev")
                .expect("display endpoint should resolve")
                .capabilities,
            vec!["view", "keyboard", "mouse"]
        );
    }

    #[test]
    fn slice_store_rejects_names_that_collide_with_existing_ids() {
        let store = SliceStore::default();
        store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("first slice should create");

        assert!(store
            .create("kernel-1", "machine-1", create_input("slice-1"))
            .is_err());
    }

    #[test]
    fn slice_store_restores_records_and_continues_numbering() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        let slice = store
            .set_relay_endpoint(
                &slice.id,
                Some(local_docker_private_relay_endpoint(&slice)),
                43,
            )
            .expect("relay endpoint should update");

        let restored = SliceStore::default();
        restored.restore_records(vec![slice.clone()]);
        assert_eq!(
            restored
                .resolve_by_worker_kernel_ref("slice:dev")
                .expect("worker ref should resolve")
                .relay_endpoint,
            slice.relay_endpoint
        );

        let next = restored
            .create("kernel-1", "machine-1", create_input("next"))
            .expect("new slice should create after restore");
        assert_eq!(next.id, "slice-2");
    }

    #[test]
    fn slice_store_reconciles_runtime_state_after_kernel_restart() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        let slice = store
            .set_relay_endpoint(
                &slice.id,
                Some(local_docker_private_relay_endpoint(&slice)),
                43,
            )
            .expect("relay endpoint should update");
        store
            .set_worker_presence(
                &slice.id,
                Some("worker-1".to_string()),
                Some("machine-2".to_string()),
                vec!["codex".to_string()],
                44,
            )
            .expect("worker presence should update");
        store
            .set_status(&slice.id, SliceStatus::Running, 45)
            .expect("slice should be running");

        let reconciled = store.reconcile_after_kernel_restart(46);

        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].status, SliceStatus::Unhealthy);
        assert_eq!(reconciled[0].worker_kernel_id, None);
        assert_eq!(reconciled[0].worker_machine_id, None);
        assert_eq!(reconciled[0].relay_endpoint, None);
        assert!(reconciled[0].providers.is_empty());
        assert_eq!(reconciled[0].updated_at_ms, 46);
    }

    #[test]
    fn slice_store_rejects_overlapping_operations_until_guard_drops() {
        let store = SliceStore::default();
        store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let guard = store
            .try_begin_operation("dev", "slice.start")
            .expect("first operation should start");
        let error = store
            .try_begin_operation("dev", "slice.stop")
            .expect_err("second operation should be rejected");
        assert!(error.to_string().contains("slice.start"));

        drop(guard);
        store
            .try_begin_operation("dev", "slice.stop")
            .expect("operation should start after first guard drops");
    }

    #[test]
    fn local_docker_slice_port_check_reports_busy_ports() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        let ports = LocalDockerSlicePorts::for_record(&slice);
        let _listener = TcpListener::bind(("127.0.0.1", ports.relay)).ok();

        let error = ensure_local_docker_slice_ports_available(&slice)
            .expect_err("busy port should be reported before provisioning");

        assert!(
            error.to_string().contains(&ports.relay.to_string()),
            "error should name the busy port: {error}"
        );
    }

    #[test]
    fn slice_store_assigns_distinct_local_docker_ports_per_slice() {
        let store = SliceStore::default();
        let mut first_input = create_input("one");
        first_input.display_url = None;
        let first = store
            .create("kernel-1", "machine-1", first_input)
            .expect("first slice should create");
        let second = store
            .create("kernel-1", "machine-1", create_input("two"))
            .expect("second slice should create");

        let first_ports = first
            .local_docker_ports
            .expect("local Docker slices should persist assigned ports");
        let second_ports = second
            .local_docker_ports
            .expect("local Docker slices should persist assigned ports");
        assert_ne!(first_ports, second_ports);
        assert_ne!(first_ports.relay, second_ports.relay);
        assert!(first
            .display_endpoint
            .as_ref()
            .expect("headed slice should expose display")
            .url
            .contains(&first_ports.novnc.to_string()));
    }

    #[test]
    fn local_docker_slice_logs_include_tailed_action_logs() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");
        let root =
            std::env::temp_dir().join(format!("arroba-slice-logs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let options = LocalDockerSliceOptions {
            root: root.clone(),
            docker_image: "arroba-slice-linux:test".to_string(),
            build_image: SliceImageBuildPolicy::Never,
            extension_dockerfile: None,
            allow_unconfined_seccomp: false,
            memory_mb: None,
            cpus: None,
            screen_width: 1280,
            screen_height: 800,
        };
        let log_path =
            local_docker_slice_action_log_path(&root, &slice, LocalDockerSliceAction::Provision);
        fs::create_dir_all(log_path.parent().expect("log should have parent"))
            .expect("log dir should create");
        fs::write(&log_path, "line-1\nline-2\nline-3\n").expect("log should write");

        let entries = collect_local_docker_slice_logs(&slice, &options, Some(2))
            .expect("logs should collect");

        let provision = entries
            .iter()
            .find(|entry| entry.source == "provision")
            .expect("provision log should be present");
        assert_eq!(provision.text, "line-2\nline-3");
        assert!(provision.truncated);
        assert!(entries.iter().any(|entry| entry.source == "container"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn slice_store_attaches_records_to_multiple_sessions_and_agents() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let attached = store
            .attach_session(&slice.id, "session-1", 44)
            .expect("slice should attach");

        assert_eq!(attached.session_id.as_deref(), Some("session-1"));
        assert_eq!(attached.session_ids, vec!["session-1"]);
        assert_eq!(attached.updated_at_ms, 44);
        assert_eq!(store.list_by_session("session-1").len(), 1);
        let attached = store
            .attach_agent(&slice.id, "session-2", "agent-2", 45)
            .expect("slice should support another session in same worktree");
        assert_eq!(attached.session_id.as_deref(), Some("session-2"));
        assert_eq!(attached.session_ids, vec!["session-1", "session-2"]);
        assert_eq!(attached.agent_ids, vec!["agent-2"]);
        assert_eq!(store.list_by_session("session-2").len(), 1);
    }

    #[test]
    fn slice_store_sets_and_clears_provider_auth_aliases() {
        let store = SliceStore::default();
        let mut input = create_input("dev");
        input.provider_auth = vec![SliceProviderAuthSummary {
            provider: "codex".to_string(),
            state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
            auth_type: Some("chatgpt".to_string()),
            account_id: Some("acct-1".to_string()),
            email: None,
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            alias: None,
            source: "test".to_string(),
        }];
        let slice = store
            .create("kernel-1", "machine-1", input)
            .expect("slice should create");

        let aliased = store
            .set_provider_auth_alias(&slice.id, "codex", Some("Work"), 44)
            .expect("alias should update");
        assert_eq!(aliased.provider_auth[0].alias.as_deref(), Some("Work"));
        assert_eq!(aliased.updated_at_ms, 44);

        let cleared = store
            .set_provider_auth_alias(&slice.id, "codex", Some("  "), 45)
            .expect("empty alias should clear");
        assert_eq!(cleared.provider_auth[0].alias, None);
        assert!(store
            .set_provider_auth_alias(&slice.id, "claude", Some("Personal"), 46)
            .is_err());
    }

    #[test]
    fn slice_store_keeps_provider_auth_summaries_per_slice() {
        let store = SliceStore::default();
        let first = store
            .create("kernel-1", "machine-1", create_input("first"))
            .expect("first slice should create");
        let second = store
            .create("kernel-1", "machine-1", create_input("second"))
            .expect("second slice should create");

        let first = store
            .set_provider_auth(
                &first.id,
                vec![SliceProviderAuthSummary {
                    provider: "codex".to_string(),
                    state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
                    auth_type: Some("api-key".to_string()),
                    account_id: Some("acct-1".to_string()),
                    email: None,
                    organization_id: None,
                    organization_name: None,
                    subscription_type: None,
                    alias: Some("work".to_string()),
                    source: "test".to_string(),
                }],
                44,
            )
            .expect("first auth should update");
        let second = store
            .set_provider_auth(
                &second.id,
                vec![SliceProviderAuthSummary {
                    provider: "codex".to_string(),
                    state: crate::slice_provider_auth::SliceProviderAuthState::Configured,
                    auth_type: Some("api-key".to_string()),
                    account_id: Some("acct-2".to_string()),
                    email: None,
                    organization_id: None,
                    organization_name: None,
                    subscription_type: None,
                    alias: Some("personal".to_string()),
                    source: "test".to_string(),
                }],
                45,
            )
            .expect("second auth should update");

        assert_eq!(first.provider_auth[0].account_id.as_deref(), Some("acct-1"));
        assert_eq!(
            second.provider_auth[0].account_id.as_deref(),
            Some("acct-2")
        );
        assert_eq!(
            store
                .resolve(&first.id)
                .expect("first should resolve")
                .provider_auth[0]
                .alias
                .as_deref(),
            Some("work")
        );
        assert_eq!(
            store
                .resolve(&second.id)
                .expect("second should resolve")
                .provider_auth[0]
                .alias
                .as_deref(),
            Some("personal")
        );
    }

    #[test]
    fn local_docker_private_relay_uses_host_endpoint_without_container_override() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let relay = local_docker_private_relay(&slice);
        let ports = LocalDockerSlicePorts::for_record(&slice);

        assert_eq!(relay.relay_url, format!("ws://127.0.0.1:{}", ports.relay));
        assert_eq!(relay.container_relay_url, None);
        assert_eq!(relay.relay_token, "slice-local-kernel-1-slice-1");
    }

    #[test]
    fn local_docker_relay_url_rewrites_host_loopback_for_container() {
        assert_eq!(
            relay_url_for_container("ws://127.0.0.1:43130"),
            "ws://host.docker.internal:43130"
        );
        assert_eq!(
            relay_url_for_container("ws://localhost:43130"),
            "ws://host.docker.internal:43130"
        );
        assert_eq!(
            relay_url_for_container("wss://relay.example/ws"),
            "wss://relay.example/ws"
        );
    }
}
