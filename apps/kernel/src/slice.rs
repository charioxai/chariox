use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::config::{DaemonConfig, SliceImageBuildPolicy};
use crate::error::DaemonError;
use crate::slice_provider_auth::SliceProviderAuthSummary;

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
#[serde(rename_all = "snake_case")]
pub enum SliceDisplayMode {
    Headless,
    Headed,
}

impl Default for SliceDisplayMode {
    fn default() -> Self {
        Self::Headless
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceRecord {
    pub id: String,
    pub name: String,
    pub owner_kernel_id: String,
    pub owner_machine_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_ids: Vec<String>,
    pub backend: SliceBackendKind,
    pub os: String,
    #[serde(default)]
    pub display_mode: SliceDisplayMode,
    pub status: SliceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_auth: Vec<SliceProviderAuthSummary>,
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
    pub display_mode: SliceDisplayMode,
    pub workspace_id: Option<String>,
    pub worktree_id: Option<String>,
    pub workspace_mount: Option<String>,
    pub worker_kernel_ref: Option<String>,
    pub display_url: Option<String>,
    pub provider_auth: Vec<SliceProviderAuthSummary>,
    pub now_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDockerSliceAction {
    Provision,
    ImportProviderAuth,
    Stop,
    Destroy,
}

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
                .unwrap_or_else(|| "arroba-slice-linux-spike:local".to_string()),
            build_image: linux.build_image.unwrap_or(SliceImageBuildPolicy::Auto),
            extension_dockerfile: linux
                .extension_dockerfile
                .as_deref()
                .map(expand_user_path_for_slice),
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
        let display_url = if input.display_mode == SliceDisplayMode::Headed {
            input.display_url.or_else(|| {
                let ports = LocalDockerSlicePorts::for_slice_id(&id);
                Some(format!(
                    "http://127.0.0.1:{}/vnc.html?autoconnect=true",
                    ports.novnc
                ))
            })
        } else {
            None
        };
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
            session_id: None,
            session_ids: Vec::new(),
            agent_ids: Vec::new(),
            backend: input.backend,
            os: input.os,
            display_mode: input.display_mode,
            status: SliceStatus::Stopped,
            workspace_id: input.workspace_id,
            worktree_id: input.worktree_id,
            workspace_mount: input.workspace_mount,
            worker_kernel_ref,
            worker_kernel_id: None,
            worker_machine_id: None,
            relay_endpoint: None,
            providers: Vec::new(),
            provider_auth: input.provider_auth,
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

    pub fn set_provider_auth(
        &self,
        slice_ref: &str,
        provider_auth: Vec<SliceProviderAuthSummary>,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.provider_auth",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.provider_auth = provider_auth;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn set_provider_auth_alias(
        &self,
        slice_ref: &str,
        provider: &str,
        alias: Option<&str>,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let provider = provider.trim();
        if provider.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "slice.provider_auth_alias",
                message: "provider must not be empty".to_string(),
            });
        }
        let alias = alias
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.provider_auth_alias",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        let Some(auth) = record
            .provider_auth
            .iter_mut()
            .find(|auth| auth.provider == provider)
        else {
            return Err(DaemonError::LocalTransport {
                operation: "slice.provider_auth_alias",
                message: format!("slice `{}` has no `{provider}` auth summary", record.name),
            });
        };
        auth.alias = alias;
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn delete(&self, slice_ref: &str) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        if !resolved.agent_ids.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "slice.delete",
                message: format!(
                    "slice `{}` still has {} active agent(s)",
                    resolved.name,
                    resolved.agent_ids.len()
                ),
            });
        }
        state
            .records
            .remove(&resolved.id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "slice.delete",
                message: format!("unknown slice `{slice_ref}`"),
            })
    }

    pub fn attach_session(
        &self,
        slice_ref: &str,
        session_id: &str,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.attach_session",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        if !record.session_ids.iter().any(|value| value == session_id) {
            record.session_ids.push(session_id.to_string());
        }
        record.session_id = Some(session_id.to_string());
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn detach_session(
        &self,
        slice_ref: &str,
        session_id: &str,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.detach_session",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.session_ids.retain(|value| value != session_id);
        if record.session_id.as_deref() == Some(session_id) {
            record.session_id = record.session_ids.last().cloned();
        }
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn attach_agent(
        &self,
        slice_ref: &str,
        session_id: &str,
        agent_id: &str,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.attach_agent",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        if !record.session_ids.iter().any(|value| value == session_id) {
            record.session_ids.push(session_id.to_string());
        }
        record.session_id = Some(session_id.to_string());
        if !record.agent_ids.iter().any(|value| value == agent_id) {
            record.agent_ids.push(agent_id.to_string());
        }
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn detach_agent(
        &self,
        slice_ref: &str,
        agent_id: &str,
        now_ms: u64,
    ) -> Result<SliceRecord, DaemonError> {
        let resolved = self.resolve(slice_ref)?;
        let mut state = self.inner.lock().expect("slice store poisoned");
        let record =
            state
                .records
                .get_mut(&resolved.id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "slice.detach_agent",
                    message: format!("unknown slice `{slice_ref}`"),
                })?;
        record.agent_ids.retain(|value| value != agent_id);
        record.updated_at_ms = now_ms;
        Ok(record.clone())
    }

    pub fn ensure_worktree_scope(
        &self,
        slice_ref: &str,
        workspace_id: Option<&str>,
        worktree_id: Option<&str>,
    ) -> Result<SliceRecord, DaemonError> {
        let record = self.resolve(slice_ref)?;
        if let (Some(expected), Some(actual)) = (workspace_id, record.workspace_id.as_deref()) {
            if expected != actual {
                return Err(DaemonError::LocalTransport {
                    operation: "slice.scope",
                    message: format!(
                        "slice `{}` belongs to workspace `{actual}`, not `{expected}`",
                        record.name
                    ),
                });
            }
        }
        if let (Some(expected), Some(actual)) = (worktree_id, record.worktree_id.as_deref()) {
            if expected != actual {
                return Err(DaemonError::LocalTransport {
                    operation: "slice.scope",
                    message: format!(
                        "slice `{}` belongs to worktree `{actual}`, not `{expected}`",
                        record.name
                    ),
                });
            }
        }
        Ok(record)
    }

    pub fn list_by_session(&self, session_id: &str) -> Vec<SliceRecord> {
        let state = self.inner.lock().expect("slice store poisoned");
        state
            .records
            .values()
            .filter(|record| {
                record.session_id.as_deref() == Some(session_id)
                    || record.session_ids.iter().any(|value| value == session_id)
            })
            .cloned()
            .collect()
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
    let script = linux_docker_slice_script()?;
    let ports = LocalDockerSlicePorts::for_slice_id(&record.id);
    let mut command = Command::new(&script);
    command
        .arg(match action {
            LocalDockerSliceAction::Provision => "provision",
            LocalDockerSliceAction::ImportProviderAuth => "import-provider-auth",
            LocalDockerSliceAction::Stop => "stop",
            LocalDockerSliceAction::Destroy => "destroy",
        })
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
            Self::ImportProviderAuth => "import-provider-auth",
            Self::Stop => "stop",
            Self::Destroy => "destroy",
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
            kernel: 53119_u16.saturating_add(ordinal),
            mcp: 53120_u16.saturating_add(ordinal),
            relay: 53130_u16.saturating_add(ordinal),
            novnc: 16080_u16.saturating_add(ordinal),
        }
    }

    fn codex_range(self) -> String {
        let start = 43362_u16.saturating_add(self.ordinal_offset());
        format!("{start}-{}", start.saturating_add(19))
    }

    fn opencode_range(self) -> String {
        let start = 43150_u16.saturating_add(self.ordinal_offset());
        format!("{start}-{}", start.saturating_add(19))
    }

    fn ordinal_offset(self) -> u16 {
        self.kernel.saturating_sub(53119).saturating_mul(20)
    }
}

fn local_docker_container_name(record: &SliceRecord) -> String {
    format!("arroba-slice-{}", record.name)
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
    fn local_docker_private_relay_uses_host_endpoint_without_container_override() {
        let store = SliceStore::default();
        let slice = store
            .create("kernel-1", "machine-1", create_input("dev"))
            .expect("slice should create");

        let relay = local_docker_private_relay(&slice);

        assert_eq!(relay.relay_url, "ws://127.0.0.1:53130");
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
