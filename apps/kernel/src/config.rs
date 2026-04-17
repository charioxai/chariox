use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::DaemonError;
use crate::transport::relay_crypto;
use rand::RngCore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub daemon_id: String,
    pub host_machine_id: String,
    pub host_machine_alias: Option<String>,
    pub os_name: String,
    pub daemon_alias: Option<String>,
    pub relay_url: Option<String>,
    pub relay_token: Option<String>,
    pub relay_public_key: String,
    pub relay_private_key: String,
    pub relay_heartbeat_ms: u64,
    pub relay_request_timeout_ms: u64,
    pub accept_remote_leases: bool,
    pub os_user: String,
    pub local_socket_path: PathBuf,
    pub kernel_websocket_host: String,
    pub kernel_websocket_port: u16,
    pub kernel_websocket_queue_capacity: usize,
    pub kernel_websocket_write_delay_ms: u64,
    pub runtime_mcp_host: String,
    pub runtime_mcp_port: u16,
    pub session_history_root: PathBuf,
    pub session_history_read_delay_ms: u64,
    pub provider_catalog_read_delay_ms: u64,
    pub provider_process_list_delay_ms: u64,
    pub provider_runtime_init_delay_ms: u64,
}

impl DaemonConfig {
    pub fn load_from_env() -> Self {
        let runtime_identity = load_or_create_runtime_identity();
        let daemon_id = env::var("ARROBA_DAEMON_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| runtime_identity.daemon_id.clone());
        Self {
            local_socket_path: env::var_os("ARROBA_DAEMON_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| Self::default_local_socket_path(&daemon_id)),
            kernel_websocket_host: env::var("ARROBA_KERNEL_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            kernel_websocket_port: env::var("ARROBA_KERNEL_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(43118),
            kernel_websocket_queue_capacity: env::var("ARROBA_KERNEL_QUEUE_CAPACITY")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(128),
            kernel_websocket_write_delay_ms: env::var("ARROBA_KERNEL_WRITE_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            runtime_mcp_host: env::var("ARROBA_MCP_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            runtime_mcp_port: env::var("ARROBA_MCP_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(43120),
            session_history_root: env::var_os("ARROBA_SESSION_HISTORY_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(Self::default_session_history_root),
            session_history_read_delay_ms: env::var("ARROBA_SESSION_HISTORY_READ_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            provider_catalog_read_delay_ms: env::var("ARROBA_PROVIDER_CATALOG_READ_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            provider_process_list_delay_ms: env::var("ARROBA_PROVIDER_PROCESS_LIST_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            provider_runtime_init_delay_ms: env::var("ARROBA_PROVIDER_RUNTIME_INIT_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            daemon_id,
            host_machine_id: env::var("ARROBA_MACHINE_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| runtime_identity.machine_id.clone()),
            host_machine_alias: env::var("ARROBA_MACHINE_ALIAS")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or(runtime_identity.machine_alias),
            os_name: env::var("ARROBA_OS_NAME")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(default_os_name),
            daemon_alias: env::var("ARROBA_DAEMON_ALIAS")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or(runtime_identity.daemon_alias),
            relay_url: env::var("ARROBA_RELAY_URL")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| load_persisted_relay_config().and_then(|config| config.relay_url)),
            relay_token: env::var("ARROBA_RELAY_TOKEN")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| load_persisted_relay_config().and_then(|config| config.relay_token)),
            relay_public_key: runtime_identity.relay_public_key,
            relay_private_key: runtime_identity.relay_private_key,
            relay_heartbeat_ms: env::var("ARROBA_RELAY_HEARTBEAT_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(5_000),
            relay_request_timeout_ms: env::var("ARROBA_RELAY_REQUEST_TIMEOUT_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(60_000),
            accept_remote_leases: env::var("ARROBA_ACCEPT_REMOTE_LEASES")
                .ok()
                .map(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
                .unwrap_or(false),
            os_user: env::var("USER")
                .or_else(|_| env::var("USERNAME"))
                .unwrap_or_else(|_| "unknown".to_string()),
        }
    }

    pub fn new(
        daemon_id: impl Into<String>,
        host_machine_id: impl Into<String>,
        os_user: impl Into<String>,
    ) -> Self {
        let daemon_id = daemon_id.into();
        let relay_private_key = relay_crypto::generate_private_key_base64();
        let relay_public_key = relay_crypto::public_key_from_private_key_base64(&relay_private_key)
            .unwrap_or_default();
        Self {
            local_socket_path: Self::default_local_socket_path(&daemon_id),
            kernel_websocket_host: "127.0.0.1".to_string(),
            kernel_websocket_port: 43118,
            kernel_websocket_queue_capacity: 128,
            kernel_websocket_write_delay_ms: 0,
            runtime_mcp_host: "127.0.0.1".to_string(),
            runtime_mcp_port: 43120,
            session_history_root: Self::default_session_history_root(),
            session_history_read_delay_ms: 0,
            provider_catalog_read_delay_ms: 0,
            provider_process_list_delay_ms: 0,
            provider_runtime_init_delay_ms: 0,
            daemon_id,
            host_machine_id: host_machine_id.into(),
            host_machine_alias: None,
            os_name: default_os_name(),
            daemon_alias: None,
            relay_url: None,
            relay_token: None,
            relay_public_key,
            relay_private_key,
            relay_heartbeat_ms: 5_000,
            relay_request_timeout_ms: 60_000,
            accept_remote_leases: false,
            os_user: os_user.into(),
        }
    }

    pub fn for_tests() -> Self {
        static TEST_SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

        let index = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
        let mut config = Self::new("daemon-test", "machine-test", "tester");
        config.local_socket_path = std::env::temp_dir().join("arroba-tests").join(format!(
            "daemon-test-{}-{}.sock",
            std::process::id(),
            index
        ));
        config.session_history_root = std::env::temp_dir().join("arroba-tests").join(format!(
            "session-history-{}-{}",
            std::process::id(),
            index
        ));
        config
    }

    pub fn with_local_socket_path(mut self, path: PathBuf) -> Self {
        self.local_socket_path = path;
        self
    }

    pub fn with_session_history_root(mut self, path: PathBuf) -> Self {
        self.session_history_root = path;
        self
    }

    pub fn kernel_websocket_url(&self) -> String {
        format!(
            "ws://{}:{}/kernel",
            self.kernel_websocket_host, self.kernel_websocket_port
        )
    }

    pub fn runtime_mcp_url(&self) -> String {
        format!(
            "http://{}:{}/mcp",
            self.runtime_mcp_host, self.runtime_mcp_port
        )
    }

    pub fn default_local_socket_path(daemon_id: &str) -> PathBuf {
        default_runtime_dir().join(format!("{daemon_id}.sock"))
    }

    pub fn default_session_history_root() -> PathBuf {
        default_state_dir().join("sessions")
    }

    pub fn default_runtime_identity_path() -> PathBuf {
        default_state_dir().join("daemon").join("identity.json")
    }

    pub fn default_daemon_config_path() -> PathBuf {
        default_state_dir().join("daemon").join("config.json")
    }

    pub fn persist_relay_config(&self) -> Result<(), DaemonError> {
        let mut persisted = load_persisted_daemon_config();
        persisted.relay_url = self.relay_url.clone();
        persisted.relay_token = self.relay_token.clone();
        persist_daemon_config(&persisted, "persist relay config")
    }

    pub fn machine_registry_entries() -> Vec<PersistedMachineRegistration> {
        load_persisted_daemon_config().machines
    }

    pub fn approve_remote_machine(
        machine_id: impl Into<String>,
        alias: Option<String>,
    ) -> Result<PersistedMachineRegistration, DaemonError> {
        let machine_id = machine_id.into();
        let alias = normalized_optional(alias);
        validate_non_empty("machine_id", &machine_id)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_machine_registration(&mut persisted.machines, &machine_id);
        entry.alias = alias.or_else(|| entry.alias.clone());
        entry.approved = true;
        entry.forgotten = false;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist machine approval")?;
        Ok(saved)
    }

    pub fn forget_remote_machine(
        machine_id: impl Into<String>,
    ) -> Result<PersistedMachineRegistration, DaemonError> {
        let machine_id = machine_id.into();
        validate_non_empty("machine_id", &machine_id)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_machine_registration(&mut persisted.machines, &machine_id);
        entry.approved = false;
        entry.forgotten = true;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist forgotten machine")?;
        Ok(saved)
    }

    pub fn rename_remote_machine(
        machine_id: impl Into<String>,
        alias: impl Into<String>,
    ) -> Result<PersistedMachineRegistration, DaemonError> {
        let machine_id = machine_id.into();
        let alias = alias.into().trim().to_string();
        validate_non_empty("machine_id", &machine_id)?;
        validate_non_empty("machine_alias", &alias)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_machine_registration(&mut persisted.machines, &machine_id);
        entry.alias = Some(alias);
        entry.approved = true;
        entry.forgotten = false;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist machine rename")?;
        Ok(saved)
    }

    pub fn resolve_registered_machine_ref(machine_ref: &str) -> Option<String> {
        let machine_ref = machine_ref.trim();
        if machine_ref.is_empty() {
            return None;
        }
        Self::machine_registry_entries()
            .into_iter()
            .filter(|entry| !entry.forgotten)
            .find(|entry| {
                entry.machine_id == machine_ref || entry.alias.as_deref() == Some(machine_ref)
            })
            .map(|entry| entry.machine_id)
    }

    pub fn validate(&self) -> Result<(), DaemonError> {
        validate_non_empty("daemon_id", &self.daemon_id)?;
        validate_non_empty("host_machine_id", &self.host_machine_id)?;
        if self
            .relay_url
            .as_deref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
            && self.relay_token.is_none()
        {
            return Err(DaemonError::InvalidConfig {
                field: "relay_token",
                message: "value must be set when relay_url is configured",
            });
        }
        validate_non_empty("os_user", &self.os_user)?;
        if self.local_socket_path.as_os_str().is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "local_socket_path",
                message: "value must not be empty",
            });
        }
        validate_non_empty("os_name", &self.os_name)?;
        validate_non_empty("kernel_websocket_host", &self.kernel_websocket_host)?;
        if self.kernel_websocket_port == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "kernel_websocket_port",
                message: "value must not be zero",
            });
        }
        if self.kernel_websocket_queue_capacity == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "kernel_websocket_queue_capacity",
                message: "value must not be zero",
            });
        }
        validate_non_empty("runtime_mcp_host", &self.runtime_mcp_host)?;
        if self.runtime_mcp_port == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "runtime_mcp_port",
                message: "value must not be zero",
            });
        }
        if self.session_history_root.as_os_str().is_empty() {
            return Err(DaemonError::InvalidConfig {
                field: "session_history_root",
                message: "value must not be empty",
            });
        }
        validate_non_empty("relay_public_key", &self.relay_public_key)?;
        validate_non_empty("relay_private_key", &self.relay_private_key)?;
        if self.relay_heartbeat_ms == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "relay_heartbeat_ms",
                message: "value must not be zero",
            });
        }
        if self.relay_request_timeout_ms == 0 {
            return Err(DaemonError::InvalidConfig {
                field: "relay_request_timeout_ms",
                message: "value must not be zero",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedDaemonConfig {
    #[serde(default)]
    relay_url: Option<String>,
    #[serde(default)]
    relay_token: Option<String>,
    #[serde(default)]
    machines: Vec<PersistedMachineRegistration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedMachineRegistration {
    pub machine_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub forgotten: bool,
}

fn load_persisted_relay_config() -> Option<PersistedDaemonConfig> {
    let path = DaemonConfig::default_daemon_config_path();
    let payload = fs::read_to_string(path).ok()?;
    serde_json::from_str::<PersistedDaemonConfig>(&payload).ok()
}

fn load_persisted_daemon_config() -> PersistedDaemonConfig {
    load_persisted_relay_config().unwrap_or_default()
}

fn persist_daemon_config(
    persisted: &PersistedDaemonConfig,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let path = DaemonConfig::default_daemon_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
            operation,
            message: error.to_string(),
        })?;
    }
    let payload =
        serde_json::to_string_pretty(persisted).map_err(|error| DaemonError::LocalTransport {
            operation,
            message: error.to_string(),
        })?;
    fs::write(path, payload).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    })
}

fn upsert_machine_registration<'a>(
    entries: &'a mut Vec<PersistedMachineRegistration>,
    machine_id: &str,
) -> &'a mut PersistedMachineRegistration {
    if let Some(index) = entries
        .iter()
        .position(|entry| entry.machine_id == machine_id)
    {
        return &mut entries[index];
    }
    entries.push(PersistedMachineRegistration {
        machine_id: machine_id.to_string(),
        alias: None,
        approved: false,
        forgotten: false,
    });
    entries
        .last_mut()
        .expect("entry was just inserted into machine registry")
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RuntimeIdentity {
    daemon_id: String,
    machine_id: String,
    #[serde(default)]
    machine_alias: Option<String>,
    #[serde(default)]
    daemon_alias: Option<String>,
    relay_public_key: String,
    relay_private_key: String,
}

fn load_or_create_runtime_identity() -> RuntimeIdentity {
    let path = DaemonConfig::default_runtime_identity_path();
    if let Ok(contents) = fs::read_to_string(&path) {
        if let Ok(identity) = serde_json::from_str::<RuntimeIdentity>(&contents) {
            if !identity.daemon_id.trim().is_empty()
                && !identity.machine_id.trim().is_empty()
                && !identity.relay_public_key.trim().is_empty()
                && !identity.relay_private_key.trim().is_empty()
            {
                return identity;
            }
        }
    }

    let relay_private_key = relay_crypto::generate_private_key_base64();
    let relay_public_key =
        relay_crypto::public_key_from_private_key_base64(&relay_private_key).unwrap_or_default();
    let identity = RuntimeIdentity {
        daemon_id: format!("daemon-{}", generate_identity_suffix()),
        machine_id: format!("machine-{}", generate_identity_suffix()),
        machine_alias: None,
        daemon_alias: None,
        relay_public_key,
        relay_private_key,
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(contents) = serde_json::to_string_pretty(&identity) {
        let _ = fs::write(&path, contents);
    }
    identity
}

fn generate_identity_suffix() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn default_state_dir() -> PathBuf {
    if let Some(state_dir) = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return state_dir.join("arroba");
    }

    if let Some(home_dir) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return home_dir.join(".local").join("state").join("arroba");
    }

    std::env::temp_dir().join("arroba")
}

fn default_runtime_dir() -> PathBuf {
    if let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return runtime_dir.join("arroba");
    }

    if let Some(home_dir) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return home_dir.join(".arroba").join("run");
    }

    std::env::temp_dir().join("arroba")
}

fn default_os_name() -> String {
    match std::env::consts::OS {
        "macos" => "macOS".to_string(),
        "windows" => "Windows".to_string(),
        "linux" => "Linux".to_string(),
        "ios" => "iOS".to_string(),
        "android" => "Android".to_string(),
        other => other.to_string(),
    }
}

fn validate_non_empty(field: &'static str, value: &str) -> Result<(), DaemonError> {
    if value.trim().is_empty() {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be empty",
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_tests_uses_fixed_runtime_identity() {
        let config = DaemonConfig::for_tests();
        assert_eq!(config.daemon_id, "daemon-test");
        assert_eq!(config.host_machine_id, "machine-test");
        assert_eq!(config.host_machine_alias, None);
        assert_eq!(config.daemon_alias, None);
    }

    #[test]
    fn generated_runtime_identity_has_expected_prefixes() {
        let relay_private_key = relay_crypto::generate_private_key_base64();
        let relay_public_key = relay_crypto::public_key_from_private_key_base64(&relay_private_key)
            .expect("relay public key should derive");
        let identity = RuntimeIdentity {
            daemon_id: format!("daemon-{}", generate_identity_suffix()),
            machine_id: format!("machine-{}", generate_identity_suffix()),
            machine_alias: None,
            daemon_alias: None,
            relay_public_key,
            relay_private_key,
        };
        assert!(identity.daemon_id.starts_with("daemon-"));
        assert!(identity.machine_id.starts_with("machine-"));
        assert!(identity.daemon_id.len() > "daemon-".len());
        assert!(identity.machine_id.len() > "machine-".len());
    }
}
