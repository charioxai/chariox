use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceBackendKind {
    LocalDocker,
    SshDocker,
}

impl Default for SliceBackendKind {
    fn default() -> Self {
        Self::LocalDocker
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceStatus {
    Stopped,
    Starting,
    Running,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceRecord {
    pub id: String,
    pub name: String,
    pub owner_kernel_id: String,
    pub owner_machine_id: String,
    pub backend: SliceBackendKind,
    pub os: String,
    pub status: SliceStatus,
    pub workspace_mount: Option<String>,
    pub worker_kernel_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_kernel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_endpoint: Option<SliceRelayEndpoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_endpoint: Option<SliceDisplayEndpoint>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceRelayEndpoint {
    pub url: String,
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceDisplayEndpointKind {
    Novnc,
    ArrobaViewer,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceDisplayEndpointAccess {
    Local,
    Tunnel,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceDisplayEndpoint {
    pub slice_id: String,
    pub kind: SliceDisplayEndpointKind,
    pub url: String,
    pub access: SliceDisplayEndpointAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateSliceInput {
    pub name: String,
    pub backend: SliceBackendKind,
    pub os: String,
    pub workspace_mount: Option<String>,
    pub worker_kernel_ref: Option<String>,
    pub display_url: Option<String>,
    pub now_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDockerSliceAction {
    Provision,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDockerSliceRelay {
    pub relay_url: String,
    pub container_relay_url: Option<String>,
    pub relay_token: String,
}

#[derive(Debug, Clone, Default)]
pub struct SliceStore {
    inner: Arc<Mutex<SliceStoreState>>,
}

#[derive(Debug, Default)]
struct SliceStoreState {
    next_slice_number: u64,
    records: BTreeMap<String, SliceRecord>,
}

impl SliceStore {
    pub fn create(
        &self,
        owner_kernel_id: &str,
        owner_machine_id: &str,
        input: CreateSliceInput,
    ) -> Result<SliceRecord, DaemonError> {
        validate_slice_name(&input.name)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        if state
            .records
            .values()
            .any(|record| record.name == input.name || record.id == input.name)
        {
            return Err(DaemonError::LocalTransport {
                operation: "slice.create",
                message: format!("slice `{}` already exists", input.name),
            });
        }
        state.next_slice_number = state.next_slice_number.saturating_add(1);
        let id = format!("slice-{}", state.next_slice_number);
        let worker_kernel_ref = input
            .worker_kernel_ref
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("slice:{}", input.name));
        let display_url = input.display_url.or_else(|| {
            let ports = LocalDockerSlicePorts::for_slice_id(&id);
            Some(format!(
                "http://127.0.0.1:{}/vnc.html?autoconnect=true",
                ports.novnc
            ))
        });
        let display_endpoint = display_url.map(|url| SliceDisplayEndpoint {
            slice_id: id.clone(),
            kind: SliceDisplayEndpointKind::Novnc,
            url,
            access: SliceDisplayEndpointAccess::Local,
            expires_at_ms: None,
            capabilities: vec![
                "view".to_string(),
                "keyboard".to_string(),
                "mouse".to_string(),
            ],
        });
        let record = SliceRecord {
            id: id.clone(),
            name: input.name,
            owner_kernel_id: owner_kernel_id.to_string(),
            owner_machine_id: owner_machine_id.to_string(),
            backend: input.backend,
            os: input.os,
            status: SliceStatus::Stopped,
            workspace_mount: input.workspace_mount,
            worker_kernel_ref,
            worker_kernel_id: None,
            worker_machine_id: None,
            relay_endpoint: None,
            providers: Vec::new(),
            display_endpoint,
            created_at_ms: input.now_ms,
            updated_at_ms: input.now_ms,
        };
        state.records.insert(id, record.clone());
        Ok(record)
    }

    pub fn list(&self) -> Vec<SliceRecord> {
        let state = self.inner.lock().expect("slice store poisoned");
        state.records.values().cloned().collect()
    }

    pub fn restore_records(&self, records: Vec<SliceRecord>) {
        let mut state = self.inner.lock().expect("slice store poisoned");
        state.records.clear();
        state.next_slice_number = 0;
        for record in records {
            if let Some(number) = record
                .id
                .strip_prefix("slice-")
                .and_then(|value| value.parse::<u64>().ok())
            {
                state.next_slice_number = state.next_slice_number.max(number);
            }
            state.records.insert(record.id.clone(), record);
        }
    }

    pub fn resolve(&self, slice_ref: &str) -> Result<SliceRecord, DaemonError> {
        let slice_ref = slice_ref.trim();
        if slice_ref.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "slice.resolve",
                message: "slice reference must not be empty".to_string(),
            });
        }
        let state = self.inner.lock().expect("slice store poisoned");
        let mut matches = state
            .records
            .values()
            .filter(|record| record.id == slice_ref || record.name == slice_ref)
            .cloned()
            .collect::<Vec<_>>();
        match matches.len() {
            0 => Err(DaemonError::LocalTransport {
                operation: "slice.resolve",
                message: format!("unknown slice `{slice_ref}`"),
            }),
            1 => Ok(matches.remove(0)),
            _ => Err(DaemonError::LocalTransport {
                operation: "slice.resolve",
                message: format!("slice reference `{slice_ref}` is ambiguous"),
            }),
        }
    }

    pub fn set_status(
        &self,
        slice_ref: &str,
        status: SliceStatus,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.status",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.status = status;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn set_relay_endpoint(
        &self,
        slice_ref: &str,
        endpoint: Option<SliceRelayEndpoint>,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.relay_endpoint",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.relay_endpoint = endpoint;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn set_worker_presence(
        &self,
        slice_ref: &str,
        worker_kernel_id: Option<String>,
        worker_machine_id: Option<String>,
        providers: Vec<String>,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.worker_presence",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.worker_kernel_id = worker_kernel_id;
        record.worker_machine_id = worker_machine_id;
        record.providers = providers;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn delete(&self, slice_ref: &str) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        state
            .records
            .remove(&resolved.id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "slice.delete",
                message: format!("unknown slice `{slice_ref}`"),
            })
    }

    pub fn resolve_worker_kernel_ref(&self, slice_ref: &str) -> Result<String, DaemonError> {
        let record = self.resolve(slice_ref)?;
        Ok(record
            .worker_kernel_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(record.worker_kernel_ref))
    }

    pub fn resolve_by_worker_kernel_ref(&self, kernel_ref: &str) -> Option<SliceRecord> {
        let kernel_ref = kernel_ref.trim();
        if kernel_ref.is_empty() {
            return None;
        }
        let state = self.inner.lock().expect("slice store poisoned");
        state
            .records
            .values()
            .find(|record| {
                record.worker_kernel_ref == kernel_ref
                    || record.worker_kernel_id.as_deref() == Some(kernel_ref)
                    || record.worker_machine_id.as_deref() == Some(kernel_ref)
            })
            .cloned()
    }

    pub fn display_endpoint(&self, slice_ref: &str) -> Result<SliceDisplayEndpoint, DaemonError> {
        let record = self.resolve(slice_ref)?;
        record
            .display_endpoint
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "slice.display_endpoint",
                message: format!("slice `{}` has no display endpoint", record.name),
            })
    }
}

pub fn run_local_docker_slice_action(
    record: &SliceRecord,
    action: LocalDockerSliceAction,
    relay: Option<LocalDockerSliceRelay>,
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
    let script = linux_docker_slice_script()?;
    let ports = LocalDockerSlicePorts::for_slice_id(&record.id);
    let mut command = Command::new(&script);
    command
        .arg(match action {
            LocalDockerSliceAction::Provision => "provision",
            LocalDockerSliceAction::Stop => "stop",
        })
        .env("ARROBA_SLICE_NAME", local_docker_container_name(record))
        .env(
            "ARROBA_SLICE_HOME_VOLUME",
            format!("{}-home", local_docker_container_name(record)),
        )
        .env("ARROBA_SLICE_CODEX_PORT", ports.codex.to_string())
        .env("ARROBA_SLICE_OPENCODE_PORT", ports.opencode.to_string())
        .env("ARROBA_SLICE_KERNEL_PORT", ports.kernel.to_string())
        .env("ARROBA_SLICE_MCP_PORT", ports.mcp.to_string())
        .env("ARROBA_SLICE_RELAY_PORT", ports.relay.to_string())
        .env("ARROBA_SLICE_NOVNC_PORT", ports.novnc.to_string())
        .env("ARROBA_SLICE_START_DESKTOP", "1")
        .env("ARROBA_SLICE_START_PROVIDER_SERVERS", "1")
        .env("ARROBA_SLICE_START_RUNTIME", "1")
        .env("ARROBA_SLICE_IMPORT_PROVIDER_AUTH", "0")
        .env(
            "ARROBA_SLICE_DAEMON_ALIAS",
            record.worker_kernel_ref.clone(),
        )
        .env("ARROBA_SLICE_MACHINE_ID", format!("slice:{}", record.id))
        .env("ARROBA_SLICE_MACHINE_ALIAS", record.name.clone());
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

    let log_path = local_docker_slice_action_log_path(record, action);
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

pub fn local_docker_private_relay(record: &SliceRecord) -> LocalDockerSliceRelay {
    let ports = LocalDockerSlicePorts::for_slice_id(&record.id);
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
            Self::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalDockerSlicePorts {
    codex: u16,
    opencode: u16,
    kernel: u16,
    mcp: u16,
    relay: u16,
    novnc: u16,
}

impl LocalDockerSlicePorts {
    fn for_slice_id(slice_id: &str) -> Self {
        let ordinal = slice_id
            .strip_prefix("slice-")
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(1)
            .saturating_sub(1);
        Self {
            codex: 43252_u16.saturating_add(ordinal),
            opencode: 43140_u16.saturating_add(ordinal),
            kernel: 43119_u16.saturating_add(ordinal),
            mcp: 43120_u16.saturating_add(ordinal),
            relay: 43130_u16.saturating_add(ordinal),
            novnc: 6080_u16.saturating_add(ordinal),
        }
    }
}

fn local_docker_container_name(record: &SliceRecord) -> String {
    format!("arroba-slice-{}", record.name)
}

fn local_docker_slice_action_log_path(
    record: &SliceRecord,
    action: LocalDockerSliceAction,
) -> PathBuf {
    std::env::temp_dir().join(format!(
        "arroba-{}-{}.log",
        local_docker_container_name(record),
        action.as_str()
    ))
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
        .join("experiments")
        .join("slice-spike")
        .join("scripts")
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

fn validate_slice_name(name: &str) -> Result<(), DaemonError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "slice.validate",
            message: "slice name must not be empty".to_string(),
        });
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(DaemonError::LocalTransport {
            operation: "slice.validate",
            message: "slice name may only contain ASCII letters, numbers, '-', '_' or '.'"
                .to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_input(name: &str) -> CreateSliceInput {
        CreateSliceInput {
            name: name.to_string(),
            backend: SliceBackendKind::LocalDocker,
            os: "linux".to_string(),
            workspace_mount: Some("/repo".to_string()),
            worker_kernel_ref: None,
            display_url: Some("http://127.0.0.1:6080".to_string()),
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
    fn local_docker_private_relay_uses_host_endpoint_without_container_override() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let relay = local_docker_private_relay(&slice);

        assert_eq!(relay.relay_url, "ws://127.0.0.1:43130");
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
