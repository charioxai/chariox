use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::DaemonError;
use crate::transport::relay_crypto;
use serde::{Deserialize, Serialize};

mod credentials;
mod identity;
mod storage;

use credentials::validate_credentials;
pub use credentials::{
    CredentialVaultBackend, UserCredentialConfig, UserCredentialInjectionConfig,
    UserCredentialSourceConfig, UserCredentialUse, UserCredentialVaultConfig,
};
use identity::load_or_create_runtime_identity;
#[cfg(test)]
use identity::{generate_identity_suffix, RuntimeIdentity};
pub use storage::{
    ArtifactOperationalBackend, HistoryArchiveMode, HistoryOperationalBackend, StateBackend,
    UserArchiveArtifactsConfig, UserArchiveHistoryConfig, UserArtifactsConfig, UserHistoryConfig,
    UserOperationalArtifactsConfig, UserOperationalHistoryConfig, UserStateConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub user_config_path: PathBuf,
    pub user_config: ArrobaUserConfig,
    pub daemon_id: String,
    pub host_machine_id: String,
    pub host_machine_alias: Option<String>,
    pub os_name: String,
    pub daemon_alias: Option<String>,
    pub relay_url: Option<String>,
    pub relay_token: Option<String>,
    pub cloud_relay: Option<PersistedCloudRelayProfile>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfigSchemaEntry {
    pub path: String,
    pub value_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<String>,
    pub settable: bool,
    pub unsettable: bool,
    pub effect: String,
    pub status: String,
    pub description: String,
}

impl DaemonConfig {
    pub fn load_from_env() -> Self {
        let user_config_path = Self::default_user_config_path();
        let user_config = load_user_config_from_path(&user_config_path);
        let kernel_websocket_host =
            env::var("ARROBA_KERNEL_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let kernel_websocket_port = env::var("ARROBA_KERNEL_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(43118);
        let runtime_identity =
            load_or_create_runtime_identity(&kernel_websocket_host, kernel_websocket_port);
        let persisted_config = load_persisted_relay_config();
        let env_relay_url = env::var("ARROBA_RELAY_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let env_relay_token = env::var("ARROBA_RELAY_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let env_relay_configured = env_relay_url.is_some() || env_relay_token.is_some();
        let daemon_id = env::var("ARROBA_DAEMON_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| runtime_identity.daemon_id.clone());
        Self {
            user_config_path,
            user_config,
            local_socket_path: env::var_os("ARROBA_DAEMON_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| Self::default_local_socket_path(&daemon_id)),
            kernel_websocket_host,
            kernel_websocket_port,
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
            relay_url: env_relay_url
                .or_else(|| persisted_config.clone().and_then(|config| config.relay_url)),
            relay_token: env_relay_token.or_else(|| {
                persisted_config
                    .clone()
                    .and_then(|config| config.relay_token)
            }),
            cloud_relay: if env_relay_configured {
                None
            } else {
                persisted_config.and_then(|config| config.cloud_relay)
            },
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
            user_config_path: Self::default_user_config_path(),
            user_config: ArrobaUserConfig::default(),
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
            cloud_relay: None,
            relay_public_key,
            relay_private_key,
            relay_heartbeat_ms: 500,
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
        config.user_config.history.operational.path = Some(
            std::env::temp_dir()
                .join("arroba-tests")
                .join(format!(
                    "operational-history-{}-{}.db",
                    std::process::id(),
                    index
                ))
                .display()
                .to_string(),
        );
        config.user_config.artifacts.operational.root = Some(
            std::env::temp_dir()
                .join("arroba-tests")
                .join(format!(
                    "operational-artifacts-{}-{}",
                    std::process::id(),
                    index
                ))
                .display()
                .to_string(),
        );
        config.user_config.artifacts.operational.index_path = Some(
            std::env::temp_dir()
                .join("arroba-tests")
                .join(format!(
                    "operational-artifacts-{}-{}.db",
                    std::process::id(),
                    index
                ))
                .display()
                .to_string(),
        );
        config.user_config.state.path = Some(
            std::env::temp_dir()
                .join("arroba-tests")
                .join(format!("kernel-state-{}-{}.db", std::process::id(), index))
                .display()
                .to_string(),
        );
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
        default_config_dir().join("sessions")
    }

    pub fn operational_history_path(&self) -> PathBuf {
        self.user_config
            .history
            .operational
            .path
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| default_config_dir().join("history").join("operational.db"))
    }

    pub fn operational_artifact_root(&self) -> PathBuf {
        self.user_config
            .artifacts
            .operational
            .root
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| default_state_dir().join("artifacts"))
    }

    pub fn operational_artifact_index_path(&self) -> PathBuf {
        self.user_config
            .artifacts
            .operational
            .index_path
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| self.operational_artifact_root().join("index.db"))
    }

    pub fn durable_state_path(&self) -> PathBuf {
        self.user_config
            .state
            .path
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| {
                default_config_dir()
                    .join("kernels")
                    .join(&self.daemon_id)
                    .join("state.db")
            })
    }

    pub fn slice_root(&self) -> PathBuf {
        self.user_config
            .slices
            .root
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| default_config_dir().join("slices"))
    }

    pub fn kernel_event_counter_path(&self) -> PathBuf {
        self.event_counter_root()
            .join(&self.daemon_id)
            .join("event-counter.json")
    }

    pub fn kernel_relay_event_counter_path(&self) -> PathBuf {
        self.event_counter_root()
            .join(&self.daemon_id)
            .join("relay-event-counter.json")
    }

    fn event_counter_root(&self) -> PathBuf {
        self.user_config
            .state
            .path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .and_then(|path| PathBuf::from(path).parent().map(Path::to_path_buf))
            .map(|root| root.join("kernel-events"))
            .unwrap_or_else(|| default_state_dir().join("kernel-events"))
    }

    pub fn default_runtime_identity_path() -> PathBuf {
        default_state_dir().join("daemon").join("identity.json")
    }

    pub fn default_machine_identity_path() -> PathBuf {
        default_config_dir().join("machine").join("identity.json")
    }

    pub fn default_kernel_registry_path() -> PathBuf {
        default_config_dir().join("kernels").join("registry.json")
    }

    pub fn default_daemon_config_path() -> PathBuf {
        default_config_dir().join("daemon").join("config.json")
    }

    fn legacy_daemon_config_path() -> PathBuf {
        default_state_dir().join("daemon").join("config.json")
    }

    pub fn default_user_config_path() -> PathBuf {
        default_config_dir().join("config.toml")
    }

    pub fn user_config_path(&self) -> &PathBuf {
        &self.user_config_path
    }

    pub fn provider_requires_managed_io(&self, _provider: &str) -> bool {
        self.user_config.providers.managed_io.requires_managed_io()
    }

    pub fn set_user_config_value(
        &mut self,
        key_path: impl AsRef<str>,
        value: impl Into<String>,
    ) -> Result<(), DaemonError> {
        self.user_config
            .set_value(key_path.as_ref(), value.into())?;
        persist_user_config(&self.user_config_path, &self.user_config)?;
        Ok(())
    }

    pub fn unset_user_config_value(
        &mut self,
        key_path: impl AsRef<str>,
    ) -> Result<(), DaemonError> {
        self.user_config.unset_value(key_path.as_ref())?;
        persist_user_config(&self.user_config_path, &self.user_config)?;
        Ok(())
    }

    pub fn user_config_schema() -> Vec<UserConfigSchemaEntry> {
        user_config_schema_entries().to_vec()
    }

    pub fn persist_relay_config(&self) -> Result<(), DaemonError> {
        let mut persisted = load_persisted_daemon_config();
        persisted.relay_url = self.relay_url.clone();
        persisted.relay_token = self.relay_token.clone();
        persisted.cloud_relay = self.cloud_relay.clone();
        persist_daemon_config(&persisted, "persist relay config")
    }

    pub fn persist_cloud_relay_profile(
        &mut self,
        profile: Option<PersistedCloudRelayProfile>,
    ) -> Result<(), DaemonError> {
        self.cloud_relay = profile;
        self.persist_relay_config()
    }

    pub fn machine_registry_entries() -> Vec<PersistedMachineRegistration> {
        load_persisted_daemon_config().machines
    }

    pub fn client_pairing_entries() -> Vec<PersistedClientPairing> {
        load_persisted_daemon_config().clients
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

    pub fn pair_remote_machine(
        machine_id: impl Into<String>,
        public_key_thumbprint: impl Into<String>,
        paired_at_ms: u64,
    ) -> Result<PersistedMachineRegistration, DaemonError> {
        let machine_id = machine_id.into();
        let public_key_thumbprint = public_key_thumbprint.into().trim().to_string();
        validate_non_empty("machine_id", &machine_id)?;
        validate_non_empty("public_key_thumbprint", &public_key_thumbprint)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_machine_registration(&mut persisted.machines, &machine_id);
        entry.public_key_thumbprint = Some(public_key_thumbprint);
        entry.paired_at_ms = Some(paired_at_ms);
        entry.forgotten = false;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist machine pairing")?;
        Ok(saved)
    }

    pub fn record_paired_client(
        client_id: impl Into<String>,
        public_key_thumbprint: impl Into<String>,
        alias: Option<String>,
        paired_at_ms: u64,
    ) -> Result<PersistedClientPairing, DaemonError> {
        Self::record_paired_terminal(client_id, public_key_thumbprint, alias, paired_at_ms, "cli")
    }

    pub fn record_paired_terminal(
        client_id: impl Into<String>,
        public_key_thumbprint: impl Into<String>,
        alias: Option<String>,
        paired_at_ms: u64,
        terminal_type: impl Into<String>,
    ) -> Result<PersistedClientPairing, DaemonError> {
        let client_id = client_id.into();
        let public_key_thumbprint = public_key_thumbprint.into().trim().to_string();
        let terminal_type = normalized_terminal_type(&terminal_type.into());
        validate_non_empty("client_id", &client_id)?;
        validate_non_empty("public_key_thumbprint", &public_key_thumbprint)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_client_pairing(&mut persisted.clients, &client_id);
        entry.alias = normalized_optional(alias).or_else(|| entry.alias.clone());
        entry.public_key_thumbprint = public_key_thumbprint;
        entry.terminal_type = terminal_type;
        entry.paired_at_ms = paired_at_ms;
        entry.revoked = false;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist client pairing")?;
        Ok(saved)
    }

    pub fn revoke_paired_client(
        client_id: impl Into<String>,
    ) -> Result<PersistedClientPairing, DaemonError> {
        let client_id = client_id.into();
        validate_non_empty("client_id", &client_id)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_client_pairing(&mut persisted.clients, &client_id);
        entry.revoked = true;
        let saved = entry.clone();
        persist_daemon_config(&persisted, "persist client revocation")?;
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
        self.user_config.validate()?;
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

fn user_config_schema_entry(
    path: &str,
    value_type: &str,
    allowed_values: &[&str],
    settable: bool,
    unsettable: bool,
    effect: &str,
    status: &str,
    description: &str,
) -> UserConfigSchemaEntry {
    UserConfigSchemaEntry {
        path: path.to_string(),
        value_type: value_type.to_string(),
        allowed_values: allowed_values
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        settable,
        unsettable,
        effect: effect.to_string(),
        status: status.to_string(),
        description: description.to_string(),
    }
}

fn user_config_schema_entries() -> Vec<UserConfigSchemaEntry> {
    vec![
        user_config_schema_entry(
            "providers.managed_io",
            "enum",
            &["required", "unrestricted"],
            true,
            true,
            "provider_reload",
            "live",
            "Global managed I/O write-enforcement policy for supported provider runs.",
        ),
        user_config_schema_entry("providers.default", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted provider default; currently not used by launch defaulting."),
        user_config_schema_entry("providers.model", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted model default; currently not used by launch defaulting."),
        user_config_schema_entry("providers.account_profile", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted account profile default; currently not used by launch defaulting."),
        user_config_schema_entry("providers.effort", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted effort default; currently not used by launch defaulting."),
        user_config_schema_entry("ui.theme", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted UI theme value; terminal UI currently uses CLI preferences."),
        user_config_schema_entry("ui.multi_agent_response_layout", "string", &[], true, true, "no_runtime_effect", "unwired", "Persisted response layout value; terminal UI currently uses CLI/session preferences."),
        user_config_schema_entry("ui.max_agents_per_screen", "u32", &[], true, true, "no_runtime_effect", "unwired", "Persisted pane-count value; terminal UI currently uses CLI preferences."),
        user_config_schema_entry("ui.worktree_aliases.<alias>", "string", &[], true, true, "no_runtime_effect", "unwired", "Pattern key for a worktree alias entry."),
        user_config_schema_entry("relay.url", "string|null", &[], true, true, "no_runtime_effect", "unwired", "Persisted user-config relay URL; daemon relay connection currently uses daemon config."),
        user_config_schema_entry("relay.accept_remote_leases", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Persisted remote-lease acceptance flag; daemon runtime currently uses daemon config."),
        user_config_schema_entry("history.operational.backend", "enum", &["sqlite"], true, false, "restart_required", "boot", "Operational history storage backend."),
        user_config_schema_entry("history.operational.path", "string", &[], true, true, "restart_required", "boot", "Operational history SQLite database path."),
        user_config_schema_entry("history.operational.retention_days", "u32", &[], true, true, "no_runtime_effect", "unwired", "Retention-days setting; no pruning job currently consumes it."),
        user_config_schema_entry("history.operational.max_size_mb", "u32", &[], true, true, "no_runtime_effect", "unwired", "Max-size setting; no pruning job currently consumes it."),
        user_config_schema_entry("history.operational.keep_pinned_sessions", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Pinned-session retention setting; no pruning job currently consumes it."),
        user_config_schema_entry("history.operational.archive_inactive_after_days", "u32", &[], true, true, "no_runtime_effect", "unwired", "Inactive-session archival threshold; no archival scheduler currently consumes it."),
        user_config_schema_entry("history.operational.archive_deleted_agents", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Deleted-agent archival flag; deletion flow does not currently consume it."),
        user_config_schema_entry("history.archive.mode", "enum", &["disabled", "external"], true, true, "none", "live", "History archive mode."),
        user_config_schema_entry("history.archive.url", "string", &[], true, true, "none", "live", "External history archive endpoint."),
        user_config_schema_entry("history.archive.token_env", "string", &[], true, true, "none", "live", "Environment variable name for the history archive bearer token."),
        user_config_schema_entry("history.archive.archive_deleted_agents", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Archive-deleted-agents flag; deletion flow does not currently consume it."),
        user_config_schema_entry("history.archive.archive_before_delete", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Archive-before-delete flag; deletion flow does not currently consume it."),
        user_config_schema_entry("history.archive.delete_operational_after_verified_archive", "bool", &["true", "false"], true, true, "no_runtime_effect", "unwired", "Delete-after-archive flag; no archive cleanup flow currently consumes it."),
        user_config_schema_entry("history.archive.require_durable_acceptance", "bool", &["true", "false"], true, true, "none", "live", "Require durable archive acceptance for history events."),
        user_config_schema_entry("artifacts.operational.backend", "enum", &["filesystem"], true, false, "none", "live", "Operational artifact storage backend."),
        user_config_schema_entry("artifacts.operational.root", "string", &[], true, true, "none", "live", "Operational artifact filesystem root."),
        user_config_schema_entry("artifacts.operational.index_path", "string", &[], true, true, "none", "live", "Operational artifact SQLite index path."),
        user_config_schema_entry("artifacts.operational.retention_days", "u32", &[], true, true, "no_runtime_effect", "unwired", "Artifact retention setting; no cleanup job currently consumes it."),
        user_config_schema_entry("artifacts.archive.mode", "enum", &["disabled", "external"], true, true, "none", "live", "Artifact archive mode."),
        user_config_schema_entry("artifacts.archive.url", "string", &[], true, true, "none", "live", "External artifact archive endpoint."),
        user_config_schema_entry("artifacts.archive.token_env", "string", &[], true, true, "none", "live", "Environment variable name for the artifact archive bearer token."),
        user_config_schema_entry("artifacts.archive.require_durable_acceptance", "bool", &["true", "false"], true, true, "none", "live", "Require durable archive acceptance for artifact events."),
        user_config_schema_entry("state.backend", "enum", &["sqlite"], true, false, "restart_required", "boot", "Durable kernel state backend."),
        user_config_schema_entry("state.path", "string", &[], true, true, "restart_required", "boot", "Durable kernel state SQLite database path."),
        user_config_schema_entry("state.snapshot_interval_events", "u32", &[], true, true, "none", "live", "Number of state events between durable snapshots."),
        user_config_schema_entry("slices.root", "string", &[], true, true, "none", "live", "Arroba-owned slice metadata, logs, and build-helper root."),
        user_config_schema_entry("slices.linux.docker_image", "string", &[], true, true, "none", "live", "Docker image tag used for new Linux slices."),
        user_config_schema_entry("slices.linux.build_image", "enum", &["auto", "always", "never"], true, true, "none", "live", "Linux slice image build policy."),
        user_config_schema_entry("slices.linux.extension_dockerfile", "string", &[], true, true, "none", "live", "Optional user Dockerfile layered on top of the Linux slice image."),
        user_config_schema_entry("slices.linux.memory_mb", "u32", &[], true, true, "none", "live", "Optional Docker memory limit for new Linux slice containers."),
        user_config_schema_entry("slices.linux.cpus", "string", &[], true, true, "none", "live", "Optional Docker CPU limit for new Linux slice containers."),
        user_config_schema_entry("slices.linux.idle_timeout_minutes", "u32", &[], true, true, "no_runtime_effect", "unwired", "Future idle-stop timeout for Linux slices."),
        user_config_schema_entry("slices.linux.screen_width", "u32", &[], true, true, "none", "live", "Linux slice virtual screen width."),
        user_config_schema_entry("slices.linux.screen_height", "u32", &[], true, true, "none", "live", "Linux slice virtual screen height."),
        user_config_schema_entry("kernel.websocket_host", "string", &[], true, true, "restart_required", "boot", "Kernel websocket bind host."),
        user_config_schema_entry("kernel.websocket_port", "port", &[], true, true, "restart_required", "boot", "Kernel websocket bind port."),
        user_config_schema_entry("kernel.runtime_mcp_host", "string", &[], true, true, "restart_required", "boot", "Runtime MCP bind host."),
        user_config_schema_entry("kernel.runtime_mcp_port", "port", &[], true, true, "restart_required", "boot", "Runtime MCP bind port."),
        user_config_schema_entry("credential_vault.service", "string", &[], true, false, "none", "live", "OS keychain service namespace for vault-backed credentials."),
        user_config_schema_entry("version", "u32", &[], true, false, "none", "internal", "User config schema version; migration-owned and not recommended for manual edits."),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaUserConfig {
    #[serde(default = "default_user_config_version")]
    pub version: u32,
    #[serde(default)]
    pub providers: UserProviderConfig,
    #[serde(default)]
    pub history: UserHistoryConfig,
    #[serde(default)]
    pub artifacts: UserArtifactsConfig,
    #[serde(default)]
    pub state: UserStateConfig,
    #[serde(default)]
    pub slices: UserSlicesConfig,
    #[serde(default)]
    pub ui: UserUiConfig,
    #[serde(default)]
    pub relay: UserRelayConfig,
    #[serde(default)]
    pub kernel: UserKernelConfig,
    #[serde(default)]
    pub credential_vault: UserCredentialVaultConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<UserCredentialConfig>,
}

impl Default for ArrobaUserConfig {
    fn default() -> Self {
        Self {
            version: default_user_config_version(),
            providers: UserProviderConfig::default(),
            history: UserHistoryConfig::default(),
            artifacts: UserArtifactsConfig::default(),
            state: UserStateConfig::default(),
            slices: UserSlicesConfig::default(),
            ui: UserUiConfig::default(),
            relay: UserRelayConfig::default(),
            kernel: UserKernelConfig::default(),
            credential_vault: UserCredentialVaultConfig::default(),
            credentials: Vec::new(),
        }
    }
}

impl ArrobaUserConfig {
    pub fn validate(&self) -> Result<(), DaemonError> {
        self.providers.managed_io.validate()?;
        self.history.validate()?;
        self.artifacts.validate()?;
        self.state.validate()?;
        self.slices.validate()?;
        validate_non_empty("credential_vault.service", &self.credential_vault.service)?;
        validate_credentials(&self.credentials)?;
        Ok(())
    }

    fn set_value(&mut self, key_path: &str, value: String) -> Result<(), DaemonError> {
        let normalized = key_path.trim();
        validate_config_key_path(normalized)?;
        match normalized {
            "version" => {
                self.version = value
                    .parse::<u32>()
                    .map_err(|_| DaemonError::InvalidConfig {
                        field: "version",
                        message: "value must be an unsigned integer",
                    })?;
            }
            "providers.default" => {
                self.providers.default = Some(non_empty_config_string("providers.default", value)?)
            }
            "providers.model" => {
                self.providers.model = Some(non_empty_config_string("providers.model", value)?)
            }
            "providers.account_profile" => {
                self.providers.account_profile =
                    Some(non_empty_config_string("providers.account_profile", value)?)
            }
            "providers.effort" => {
                self.providers.effort = Some(non_empty_config_string("providers.effort", value)?)
            }
            "ui.theme" => self.ui.theme = Some(non_empty_config_string("ui.theme", value)?),
            "ui.multi_agent_response_layout" => {
                self.ui.multi_agent_response_layout = Some(non_empty_config_string(
                    "ui.multi_agent_response_layout",
                    value,
                )?)
            }
            "ui.max_agents_per_screen" => {
                self.ui.max_agents_per_screen =
                    Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| DaemonError::InvalidConfig {
                                field: "ui.max_agents_per_screen",
                                message: "value must be an unsigned integer",
                            })?,
                    );
            }
            path if path.starts_with("ui.worktree_aliases.") => {
                let key = path.trim_start_matches("ui.worktree_aliases.").trim();
                validate_config_key_path(&format!("ui.worktree_aliases.{key}"))?;
                self.ui.worktree_aliases.insert(
                    key.to_string(),
                    non_empty_config_string("ui.worktree_aliases", value)?,
                );
            }
            "relay.url" => self.relay.url = normalized_optional(Some(value)),
            "relay.accept_remote_leases" => {
                self.relay.accept_remote_leases =
                    Some(parse_config_bool("relay.accept_remote_leases", &value)?)
            }
            "history.operational.backend" => {
                self.history.operational.backend =
                    HistoryOperationalBackend::parse("history.operational.backend", &value)?
            }
            "history.operational.path" => {
                self.history.operational.path =
                    Some(non_empty_config_string("history.operational.path", value)?)
            }
            "history.operational.retention_days" => {
                self.history.operational.retention_days = Some(parse_config_u32(
                    "history.operational.retention_days",
                    &value,
                    true,
                )?)
            }
            "history.operational.max_size_mb" => {
                self.history.operational.max_size_mb = Some(parse_config_u32(
                    "history.operational.max_size_mb",
                    &value,
                    true,
                )?)
            }
            "history.operational.keep_pinned_sessions" => {
                self.history.operational.keep_pinned_sessions = Some(parse_config_bool(
                    "history.operational.keep_pinned_sessions",
                    &value,
                )?)
            }
            "history.operational.archive_inactive_after_days" => {
                self.history.operational.archive_inactive_after_days = Some(parse_config_u32(
                    "history.operational.archive_inactive_after_days",
                    &value,
                    true,
                )?)
            }
            "history.operational.archive_deleted_agents" => {
                self.history.operational.archive_deleted_agents = Some(parse_config_bool(
                    "history.operational.archive_deleted_agents",
                    &value,
                )?)
            }
            "history.archive.mode" => {
                self.history.archive.mode = HistoryArchiveMode::parse(&value)?
            }
            "history.archive.url" => {
                self.history.archive.url =
                    Some(non_empty_config_string("history.archive.url", value)?)
            }
            "history.archive.token_env" => {
                self.history.archive.token_env =
                    Some(non_empty_config_string("history.archive.token_env", value)?)
            }
            "history.archive.archive_deleted_agents" => {
                self.history.archive.archive_deleted_agents = Some(parse_config_bool(
                    "history.archive.archive_deleted_agents",
                    &value,
                )?)
            }
            "history.archive.archive_before_delete" => {
                self.history.archive.archive_before_delete = Some(parse_config_bool(
                    "history.archive.archive_before_delete",
                    &value,
                )?)
            }
            "history.archive.delete_operational_after_verified_archive" => {
                self.history
                    .archive
                    .delete_operational_after_verified_archive = Some(parse_config_bool(
                    "history.archive.delete_operational_after_verified_archive",
                    &value,
                )?)
            }
            "history.archive.require_durable_acceptance" => {
                self.history.archive.require_durable_acceptance = Some(parse_config_bool(
                    "history.archive.require_durable_acceptance",
                    &value,
                )?)
            }
            "artifacts.operational.backend" => {
                self.artifacts.operational.backend =
                    ArtifactOperationalBackend::parse("artifacts.operational.backend", &value)?
            }
            "artifacts.operational.root" => {
                self.artifacts.operational.root = Some(non_empty_config_string(
                    "artifacts.operational.root",
                    value,
                )?)
            }
            "artifacts.operational.index_path" => {
                self.artifacts.operational.index_path = Some(non_empty_config_string(
                    "artifacts.operational.index_path",
                    value,
                )?)
            }
            "artifacts.operational.retention_days" => {
                self.artifacts.operational.retention_days = Some(parse_config_u32(
                    "artifacts.operational.retention_days",
                    &value,
                    true,
                )?)
            }
            "artifacts.archive.mode" => {
                self.artifacts.archive.mode = HistoryArchiveMode::parse(&value)?
            }
            "artifacts.archive.url" => {
                self.artifacts.archive.url =
                    Some(non_empty_config_string("artifacts.archive.url", value)?)
            }
            "artifacts.archive.token_env" => {
                self.artifacts.archive.token_env = Some(non_empty_config_string(
                    "artifacts.archive.token_env",
                    value,
                )?)
            }
            "artifacts.archive.require_durable_acceptance" => {
                self.artifacts.archive.require_durable_acceptance = Some(parse_config_bool(
                    "artifacts.archive.require_durable_acceptance",
                    &value,
                )?)
            }
            "state.backend" => self.state.backend = StateBackend::parse("state.backend", &value)?,
            "state.path" => self.state.path = Some(non_empty_config_string("state.path", value)?),
            "state.snapshot_interval_events" => {
                self.state.snapshot_interval_events = Some(parse_config_u32(
                    "state.snapshot_interval_events",
                    &value,
                    true,
                )?)
            }
            "slices.root" => {
                self.slices.root = Some(non_empty_config_string("slices.root", value)?)
            }
            "slices.linux.docker_image" => {
                self.slices.linux.docker_image =
                    Some(non_empty_config_string("slices.linux.docker_image", value)?)
            }
            "slices.linux.build_image" => {
                self.slices.linux.build_image = Some(SliceImageBuildPolicy::parse(&value)?)
            }
            "slices.linux.extension_dockerfile" => {
                self.slices.linux.extension_dockerfile = Some(non_empty_config_string(
                    "slices.linux.extension_dockerfile",
                    value,
                )?)
            }
            "slices.linux.memory_mb" => {
                self.slices.linux.memory_mb =
                    Some(parse_config_u32("slices.linux.memory_mb", &value, true)?)
            }
            "slices.linux.cpus" => {
                self.slices.linux.cpus = Some(non_empty_config_string("slices.linux.cpus", value)?)
            }
            "slices.linux.idle_timeout_minutes" => {
                self.slices.linux.idle_timeout_minutes = Some(parse_config_u32(
                    "slices.linux.idle_timeout_minutes",
                    &value,
                    true,
                )?)
            }
            "slices.linux.screen_width" => {
                self.slices.linux.screen_width =
                    Some(parse_config_u32("slices.linux.screen_width", &value, true)?)
            }
            "slices.linux.screen_height" => {
                self.slices.linux.screen_height = Some(parse_config_u32(
                    "slices.linux.screen_height",
                    &value,
                    true,
                )?)
            }
            "kernel.websocket_host" => {
                self.kernel.websocket_host =
                    Some(non_empty_config_string("kernel.websocket_host", value)?)
            }
            "kernel.websocket_port" => {
                self.kernel.websocket_port =
                    Some(parse_config_port("kernel.websocket_port", &value)?)
            }
            "kernel.runtime_mcp_host" => {
                self.kernel.runtime_mcp_host =
                    Some(non_empty_config_string("kernel.runtime_mcp_host", value)?)
            }
            "kernel.runtime_mcp_port" => {
                self.kernel.runtime_mcp_port =
                    Some(parse_config_port("kernel.runtime_mcp_port", &value)?)
            }
            "credential_vault.service" => {
                self.credential_vault.service =
                    non_empty_config_string("credential_vault.service", value)?
            }
            "providers.managed_io" => {
                self.providers.managed_io =
                    ManagedIoConfig::from_mode(ManagedIoMode::parse(&value)?);
            }
            _ => {
                return Err(DaemonError::InvalidConfig {
                    field: "user_config",
                    message: "unsupported user config key",
                });
            }
        }
        self.validate()
    }

    fn unset_value(&mut self, key_path: &str) -> Result<(), DaemonError> {
        let normalized = key_path.trim();
        validate_config_key_path(normalized)?;
        match normalized {
            "providers.default" => self.providers.default = None,
            "providers.model" => self.providers.model = None,
            "providers.account_profile" => self.providers.account_profile = None,
            "providers.effort" => self.providers.effort = None,
            "ui.theme" => self.ui.theme = None,
            "ui.multi_agent_response_layout" => self.ui.multi_agent_response_layout = None,
            "ui.max_agents_per_screen" => self.ui.max_agents_per_screen = None,
            path if path.starts_with("ui.worktree_aliases.") => {
                let key = path.trim_start_matches("ui.worktree_aliases.").trim();
                validate_config_key_path(&format!("ui.worktree_aliases.{key}"))?;
                self.ui.worktree_aliases.remove(key);
            }
            "relay.url" => self.relay.url = None,
            "relay.accept_remote_leases" => self.relay.accept_remote_leases = None,
            "history.operational.backend" => {
                return Err(DaemonError::InvalidConfig {
                    field: "history.operational.backend",
                    message: "operational history backend cannot be unset",
                });
            }
            "history.operational.path" => self.history.operational.path = None,
            "history.operational.retention_days" => self.history.operational.retention_days = None,
            "history.operational.max_size_mb" => self.history.operational.max_size_mb = None,
            "history.operational.keep_pinned_sessions" => {
                self.history.operational.keep_pinned_sessions = None
            }
            "history.operational.archive_inactive_after_days" => {
                self.history.operational.archive_inactive_after_days = None
            }
            "history.operational.archive_deleted_agents" => {
                self.history.operational.archive_deleted_agents = None
            }
            "history.archive.mode" => self.history.archive.mode = HistoryArchiveMode::Disabled,
            "history.archive.url" => self.history.archive.url = None,
            "history.archive.token_env" => self.history.archive.token_env = None,
            "history.archive.archive_deleted_agents" => {
                self.history.archive.archive_deleted_agents = None
            }
            "history.archive.archive_before_delete" => {
                self.history.archive.archive_before_delete = None
            }
            "history.archive.delete_operational_after_verified_archive" => {
                self.history
                    .archive
                    .delete_operational_after_verified_archive = None
            }
            "history.archive.require_durable_acceptance" => {
                self.history.archive.require_durable_acceptance = None
            }
            "artifacts.operational.backend" => {
                return Err(DaemonError::InvalidConfig {
                    field: "artifacts.operational.backend",
                    message: "operational artifact backend cannot be unset",
                });
            }
            "artifacts.operational.root" => self.artifacts.operational.root = None,
            "artifacts.operational.index_path" => self.artifacts.operational.index_path = None,
            "artifacts.operational.retention_days" => {
                self.artifacts.operational.retention_days = None
            }
            "artifacts.archive.mode" => self.artifacts.archive.mode = HistoryArchiveMode::Disabled,
            "artifacts.archive.url" => self.artifacts.archive.url = None,
            "artifacts.archive.token_env" => self.artifacts.archive.token_env = None,
            "artifacts.archive.require_durable_acceptance" => {
                self.artifacts.archive.require_durable_acceptance = None
            }
            "state.backend" => {
                return Err(DaemonError::InvalidConfig {
                    field: "state.backend",
                    message: "state backend cannot be unset",
                });
            }
            "state.path" => self.state.path = None,
            "state.snapshot_interval_events" => self.state.snapshot_interval_events = None,
            "slices.root" => self.slices.root = None,
            "slices.linux.docker_image" => self.slices.linux.docker_image = None,
            "slices.linux.build_image" => self.slices.linux.build_image = None,
            "slices.linux.extension_dockerfile" => self.slices.linux.extension_dockerfile = None,
            "slices.linux.memory_mb" => self.slices.linux.memory_mb = None,
            "slices.linux.cpus" => self.slices.linux.cpus = None,
            "slices.linux.idle_timeout_minutes" => self.slices.linux.idle_timeout_minutes = None,
            "slices.linux.screen_width" => self.slices.linux.screen_width = None,
            "slices.linux.screen_height" => self.slices.linux.screen_height = None,
            "kernel.websocket_host" => self.kernel.websocket_host = None,
            "kernel.websocket_port" => self.kernel.websocket_port = None,
            "kernel.runtime_mcp_host" => self.kernel.runtime_mcp_host = None,
            "kernel.runtime_mcp_port" => self.kernel.runtime_mcp_port = None,
            "providers.managed_io" => self.providers.managed_io = ManagedIoConfig::default(),
            "version" => {
                return Err(DaemonError::InvalidConfig {
                    field: "version",
                    message: "version cannot be unset",
                });
            }
            _ => {
                return Err(DaemonError::InvalidConfig {
                    field: "user_config",
                    message: "unsupported user config key",
                });
            }
        }
        self.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub managed_io: ManagedIoConfig,
}

impl Default for UserProviderConfig {
    fn default() -> Self {
        Self {
            default: Some("opencode".to_string()),
            model: Some("default".to_string()),
            account_profile: Some("default".to_string()),
            effort: None,
            managed_io: ManagedIoConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ManagedIoConfigSerde", into = "ManagedIoConfigSerde")]
pub struct ManagedIoConfig {
    pub mode: ManagedIoMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum ManagedIoConfigSerde {
    Mode(ManagedIoMode),
    LegacyModes(BTreeMap<String, ManagedIoMode>),
}

impl Default for ManagedIoConfig {
    fn default() -> Self {
        Self {
            mode: ManagedIoMode::Unrestricted,
        }
    }
}

impl ManagedIoConfig {
    pub fn from_mode(mode: ManagedIoMode) -> Self {
        Self { mode }
    }

    pub fn requires_managed_io(&self) -> bool {
        self.mode.requires_managed_io()
    }

    fn validate(&self) -> Result<(), DaemonError> {
        Ok(())
    }
}

impl From<ManagedIoConfigSerde> for ManagedIoConfig {
    fn from(value: ManagedIoConfigSerde) -> Self {
        match value {
            ManagedIoConfigSerde::Mode(mode) => Self::from_mode(mode),
            ManagedIoConfigSerde::LegacyModes(modes) => {
                Self::from_mode(legacy_managed_io_mode(modes))
            }
        }
    }
}

impl From<ManagedIoConfig> for ManagedIoConfigSerde {
    fn from(value: ManagedIoConfig) -> Self {
        Self::Mode(value.mode)
    }
}

fn legacy_managed_io_mode(modes: BTreeMap<String, ManagedIoMode>) -> ManagedIoMode {
    if let Some(mode) = modes.get("default").copied() {
        return mode;
    }
    let Some(first) = modes.values().copied().next() else {
        return ManagedIoMode::Unrestricted;
    };
    if modes.values().all(|mode| *mode == first) {
        first
    } else {
        ManagedIoMode::Required
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedIoMode {
    Required,
    Unrestricted,
}

impl ManagedIoMode {
    fn parse(value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "required" | "managed" | "managed_io_required" | "on" | "true" | "1" => {
                Ok(Self::Required)
            }
            "unrestricted" | "off" | "false" | "0" => Ok(Self::Unrestricted),
            _ => Err(DaemonError::InvalidConfig {
                field: "providers.managed_io",
                message: "value must be `required` or `unrestricted`",
            }),
        }
    }

    pub fn requires_managed_io(&self) -> bool {
        matches!(self, Self::Required)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSlicesConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default)]
    pub linux: UserLinuxSliceConfig,
}

impl Default for UserSlicesConfig {
    fn default() -> Self {
        Self {
            root: Some("~/.arroba/slices".to_string()),
            linux: UserLinuxSliceConfig::default(),
        }
    }
}

impl UserSlicesConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        if let Some(root) = &self.root {
            validate_non_empty("slices.root", root)?;
        }
        self.linux.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserLinuxSliceConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_image: Option<SliceImageBuildPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_dockerfile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_height: Option<u32>,
}

impl Default for UserLinuxSliceConfig {
    fn default() -> Self {
        Self {
            docker_image: Some("arroba-slice-linux-spike:local".to_string()),
            build_image: Some(SliceImageBuildPolicy::Auto),
            extension_dockerfile: None,
            memory_mb: None,
            cpus: None,
            idle_timeout_minutes: Some(30),
            screen_width: Some(1280),
            screen_height: Some(800),
        }
    }
}

impl UserLinuxSliceConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        if let Some(image) = &self.docker_image {
            validate_non_empty("slices.linux.docker_image", image)?;
        }
        if let Some(path) = &self.extension_dockerfile {
            validate_non_empty("slices.linux.extension_dockerfile", path)?;
        }
        validate_optional_nonzero("slices.linux.memory_mb", self.memory_mb)?;
        validate_optional_nonzero(
            "slices.linux.idle_timeout_minutes",
            self.idle_timeout_minutes,
        )?;
        validate_optional_nonzero("slices.linux.screen_width", self.screen_width)?;
        validate_optional_nonzero("slices.linux.screen_height", self.screen_height)?;
        if let Some(cpus) = &self.cpus {
            validate_non_empty("slices.linux.cpus", cpus)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceImageBuildPolicy {
    Auto,
    Always,
    Never,
}

impl SliceImageBuildPolicy {
    fn parse(value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" | "off" | "false" | "0" => Ok(Self::Never),
            _ => Err(DaemonError::InvalidConfig {
                field: "slices.linux.build_image",
                message: "value must be `auto`, `always`, or `never`",
            }),
        }
    }

    pub fn as_env_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserUiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_agent_response_layout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_agents_per_screen: Option<u32>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub worktree_aliases: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRelayConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept_remote_leases: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserKernelConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_mcp_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_mcp_port: Option<u16>,
}

fn default_user_config_version() -> u32 {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedDaemonConfig {
    #[serde(default)]
    relay_url: Option<String>,
    #[serde(default)]
    relay_token: Option<String>,
    #[serde(default)]
    cloud_relay: Option<PersistedCloudRelayProfile>,
    #[serde(default)]
    machines: Vec<PersistedMachineRegistration>,
    #[serde(default)]
    clients: Vec<PersistedClientPairing>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedCloudRelayProfile {
    pub api_url: String,
    pub email: String,
    pub account_id: String,
    pub user_id: String,
    pub account_slug: String,
    pub realm_id: String,
    pub relay_url: String,
    pub issuer_id: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_alias: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub machine_alias: Option<String>,
    #[serde(default)]
    pub machine_credential: Option<String>,
    #[serde(default)]
    pub cloud_session_token: Option<String>,
    #[serde(default)]
    pub cloud_session_expires_at_ms: Option<u64>,
    #[serde(default)]
    pub token_expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedMachineRegistration {
    pub machine_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
    #[serde(default)]
    pub paired_at_ms: Option<u64>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub forgotten: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedClientPairing {
    pub client_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default = "default_terminal_type")]
    pub terminal_type: String,
    #[serde(default)]
    pub public_key_thumbprint: String,
    #[serde(default)]
    pub paired_at_ms: u64,
    #[serde(default)]
    pub revoked: bool,
}

fn default_terminal_type() -> String {
    "cli".to_string()
}

fn normalized_terminal_type(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "web" | "web_terminal" | "web-terminal" => "web".to_string(),
        "ios" | "ios_terminal" | "ios-terminal" => "ios".to_string(),
        "android" | "android_terminal" | "android-terminal" => "android".to_string(),
        _ => "cli".to_string(),
    }
}

fn load_persisted_relay_config() -> Option<PersistedDaemonConfig> {
    for path in [
        DaemonConfig::default_daemon_config_path(),
        DaemonConfig::legacy_daemon_config_path(),
    ] {
        let Ok(payload) = fs::read_to_string(path) else {
            continue;
        };
        if let Ok(config) = serde_json::from_str::<PersistedDaemonConfig>(&payload) {
            return Some(config);
        }
    }
    None
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

fn load_user_config_from_path(path: &PathBuf) -> ArrobaUserConfig {
    let Some(payload) = fs::read_to_string(path).ok() else {
        return ArrobaUserConfig::default();
    };
    toml::from_str::<ArrobaUserConfig>(&payload).unwrap_or_default()
}

fn persist_user_config(path: &PathBuf, config: &ArrobaUserConfig) -> Result<(), DaemonError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
            operation: "persist user config",
            message: error.to_string(),
        })?;
    }
    let payload = toml::to_string_pretty(config).map_err(|error| DaemonError::LocalTransport {
        operation: "persist user config",
        message: error.to_string(),
    })?;
    fs::write(path, payload).map_err(|error| DaemonError::LocalTransport {
        operation: "persist user config",
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
        public_key_thumbprint: None,
        paired_at_ms: None,
        approved: false,
        forgotten: false,
    });
    entries
        .last_mut()
        .expect("entry was just inserted into machine registry")
}

fn upsert_client_pairing<'a>(
    entries: &'a mut Vec<PersistedClientPairing>,
    client_id: &str,
) -> &'a mut PersistedClientPairing {
    if let Some(index) = entries
        .iter()
        .position(|entry| entry.client_id == client_id)
    {
        return &mut entries[index];
    }
    entries.push(PersistedClientPairing {
        client_id: client_id.to_string(),
        alias: None,
        terminal_type: default_terminal_type(),
        public_key_thumbprint: String::new(),
        paired_at_ms: 0,
        revoked: false,
    });
    entries
        .last_mut()
        .expect("entry was just inserted into client pairing registry")
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_config_key_path(key_path: &str) -> Result<(), DaemonError> {
    validate_non_empty("config_path", key_path)?;
    if !key_path.split('.').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }) {
        return Err(DaemonError::InvalidConfig {
            field: "config_path",
            message: "path must contain dot-separated alphanumeric keys",
        });
    }
    Ok(())
}

fn non_empty_config_string(field: &'static str, value: String) -> Result<String, DaemonError> {
    let value = value.trim().to_string();
    validate_non_empty(field, &value)?;
    Ok(value)
}

fn parse_config_bool(field: &'static str, value: &str) -> Result<bool, DaemonError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(DaemonError::InvalidConfig {
            field,
            message: "value must be a boolean",
        }),
    }
}

fn parse_config_port(field: &'static str, value: &str) -> Result<u16, DaemonError> {
    let port = value
        .parse::<u16>()
        .map_err(|_| DaemonError::InvalidConfig {
            field,
            message: "value must be a TCP port",
        })?;
    if port == 0 {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be zero",
        });
    }
    Ok(port)
}

fn parse_config_u32(
    field: &'static str,
    value: &str,
    require_nonzero: bool,
) -> Result<u32, DaemonError> {
    let parsed = value
        .parse::<u32>()
        .map_err(|_| DaemonError::InvalidConfig {
            field,
            message: "value must be an unsigned integer",
        })?;
    if require_nonzero && parsed == 0 {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be zero",
        });
    }
    Ok(parsed)
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

fn default_config_dir() -> PathBuf {
    if let Some(config_dir) = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return config_dir.join("arroba");
    }

    if let Some(home_dir) = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return home_dir.join(".arroba");
    }

    std::env::temp_dir().join("arroba").join("config")
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

fn expand_user_path(value: &str) -> PathBuf {
    let value = value.trim();
    if value == "~" {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(suffix) = value.strip_prefix("~/") {
        if let Some(home_dir) = env::var_os("HOME").map(PathBuf::from) {
            return home_dir.join(suffix);
        }
    }
    PathBuf::from(value)
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

fn validate_optional_nonzero(field: &'static str, value: Option<u32>) -> Result<(), DaemonError> {
    if value == Some(0) {
        return Err(DaemonError::InvalidConfig {
            field,
            message: "value must not be zero",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

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

    #[test]
    fn runtime_identity_is_stable_per_host_port() {
        let _guard = env_test_guard().lock().expect("env test guard poisoned");
        let temp_home = std::env::temp_dir().join(format!(
            "arroba-config-identity-test-{}",
            generate_identity_suffix()
        ));
        let old_home = env::var_os("HOME");
        let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
        let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
        let old_kernel_host = env::var_os("ARROBA_KERNEL_HOST");
        let old_kernel_port = env::var_os("ARROBA_KERNEL_PORT");
        unsafe {
            env::set_var("HOME", &temp_home);
            env::remove_var("XDG_CONFIG_HOME");
            env::remove_var("XDG_STATE_HOME");
            env::set_var("ARROBA_KERNEL_HOST", "127.0.0.1");
            env::set_var("ARROBA_KERNEL_PORT", "43118");
        }

        let default_identity = DaemonConfig::load_from_env();
        let restarted_default = DaemonConfig::load_from_env();
        unsafe {
            env::set_var("ARROBA_KERNEL_PORT", "43119");
        }
        let other_port = DaemonConfig::load_from_env();

        unsafe {
            restore_env_var("HOME", old_home);
            restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
            restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
            restore_env_var("ARROBA_KERNEL_HOST", old_kernel_host);
            restore_env_var("ARROBA_KERNEL_PORT", old_kernel_port);
        }
        let _ = fs::remove_dir_all(temp_home);

        assert_eq!(default_identity.daemon_id, restarted_default.daemon_id);
        assert_eq!(
            default_identity.host_machine_id,
            restarted_default.host_machine_id
        );
        assert_eq!(default_identity.host_machine_id, other_port.host_machine_id);
        assert_ne!(default_identity.daemon_id, other_port.daemon_id);
    }

    #[test]
    fn env_relay_config_takes_precedence_over_persisted_cloud_relay_profile() {
        let _guard = env_test_guard().lock().expect("env test guard poisoned");
        let temp_home = std::env::temp_dir().join(format!(
            "arroba-config-relay-env-test-{}",
            generate_identity_suffix()
        ));
        let old_home = env::var_os("HOME");
        let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
        let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
        let old_relay_url = env::var_os("ARROBA_RELAY_URL");
        let old_relay_token = env::var_os("ARROBA_RELAY_TOKEN");
        unsafe {
            env::set_var("HOME", &temp_home);
            env::remove_var("XDG_CONFIG_HOME");
            env::remove_var("XDG_STATE_HOME");
            env::set_var("ARROBA_RELAY_URL", "ws://127.0.0.1:47000");
            env::set_var("ARROBA_RELAY_TOKEN", "local-drill-token");
        }
        let daemon_config_path = DaemonConfig::default_daemon_config_path();
        if let Some(parent) = daemon_config_path.parent() {
            fs::create_dir_all(parent).expect("daemon config parent should be created");
        }
        fs::write(
            &daemon_config_path,
            r#"{
              "relay_url": "wss://cloud-relay.example",
              "relay_token": "cloud-token",
              "cloud_relay": {
                "api_url": "https://cloud.example",
                "email": "test@example.com",
                "account_id": "account-1",
                "user_id": "user-1",
                "account_slug": "account",
                "realm_id": "realm-1",
                "relay_url": "wss://cloud-relay.example",
                "issuer_id": "issuer-1",
                "machine_credential": "machine-credential",
                "token_expires_at_ms": 1
              }
            }"#,
        )
        .expect("daemon config should write");

        let config = DaemonConfig::load_from_env();

        unsafe {
            restore_env_var("HOME", old_home);
            restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
            restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
            restore_env_var("ARROBA_RELAY_URL", old_relay_url);
            restore_env_var("ARROBA_RELAY_TOKEN", old_relay_token);
        }
        let _ = fs::remove_dir_all(temp_home);

        assert_eq!(config.relay_url.as_deref(), Some("ws://127.0.0.1:47000"));
        assert_eq!(config.relay_token.as_deref(), Some("local-drill-token"));
        assert_eq!(config.cloud_relay, None);
    }

    #[test]
    fn machine_pairing_metadata_preserves_approval_state() {
        let mut entries = Vec::new();
        {
            let entry = upsert_machine_registration(&mut entries, "machine-1");
            entry.approved = true;
            entry.alias = Some("worker".to_string());
        }
        {
            let entry = upsert_machine_registration(&mut entries, "machine-1");
            entry.public_key_thumbprint = Some("thumbprint-1".to_string());
            entry.paired_at_ms = Some(42);
        }

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].machine_id, "machine-1");
        assert_eq!(entries[0].alias.as_deref(), Some("worker"));
        assert_eq!(
            entries[0].public_key_thumbprint.as_deref(),
            Some("thumbprint-1")
        );
        assert_eq!(entries[0].paired_at_ms, Some(42));
        assert!(entries[0].approved);
        assert!(!entries[0].forgotten);
    }

    #[test]
    fn client_pairing_upsert_reopens_revoked_client() {
        let mut entries = Vec::new();
        {
            let entry = upsert_client_pairing(&mut entries, "client-1");
            entry.alias = Some("laptop".to_string());
            entry.public_key_thumbprint = "old-thumbprint".to_string();
            entry.paired_at_ms = 10;
            entry.revoked = true;
        }
        {
            let entry = upsert_client_pairing(&mut entries, "client-1");
            entry.public_key_thumbprint = "new-thumbprint".to_string();
            entry.paired_at_ms = 20;
            entry.revoked = false;
        }

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].client_id, "client-1");
        assert_eq!(entries[0].alias.as_deref(), Some("laptop"));
        assert_eq!(entries[0].public_key_thumbprint, "new-thumbprint");
        assert_eq!(entries[0].paired_at_ms, 20);
        assert!(!entries[0].revoked);
    }

    #[test]
    fn user_config_parses_credential_handles_without_values() {
        let payload = r#"
version = 1

[[credentials]]
id = "github"
description = "GitHub API"
allowed_hosts = ["api.github.com"]
allowed_uses = ["http"]
source = { type = "env", name = "GH_TOKEN" }
injection = { kind = "header", name = "authorization", value = "Bearer ${secret}" }
"#;

        let config =
            toml::from_str::<ArrobaUserConfig>(payload).expect("credential config should parse");
        config
            .validate()
            .expect("credential config should validate");

        assert_eq!(config.credentials.len(), 1);
        assert_eq!(config.credentials[0].id, "github");
        assert_eq!(
            config.credentials[0].allowed_uses,
            vec![UserCredentialUse::Http]
        );
    }

    #[test]
    fn user_config_parses_slice_defaults() {
        let payload = r#"
version = 1

[slices]
root = "~/.arroba/slices-dev"

[slices.linux]
docker_image = "arroba-slice-linux-custom:local"
build_image = "never"
extension_dockerfile = "~/.arroba/slices/extensions/Dockerfile"
memory_mb = 4096
cpus = "2.5"
idle_timeout_minutes = 45
screen_width = 1440
screen_height = 900
"#;

        let config =
            toml::from_str::<ArrobaUserConfig>(payload).expect("slice config should parse");
        config.validate().expect("slice config should validate");

        assert_eq!(config.slices.root.as_deref(), Some("~/.arroba/slices-dev"));
        assert_eq!(
            config.slices.linux.docker_image.as_deref(),
            Some("arroba-slice-linux-custom:local")
        );
        assert_eq!(
            config.slices.linux.build_image,
            Some(SliceImageBuildPolicy::Never)
        );
        assert_eq!(config.slices.linux.memory_mb, Some(4096));
        assert_eq!(config.slices.linux.cpus.as_deref(), Some("2.5"));
        assert_eq!(config.slices.linux.screen_width, Some(1440));
        assert_eq!(config.slices.linux.screen_height, Some(900));
    }

    #[test]
    fn user_config_parses_vault_credential_source() {
        let payload = r#"
version = 1

[credential_vault]
service = "arroba-test"

[[credentials]]
id = "github"
source = { type = "vault", key = "github-token" }
allowed_uses = ["http"]
allowed_hosts = ["api.github.com"]
injection = { kind = "header", name = "authorization", value = "Bearer ${secret}" }
"#;

        let config =
            toml::from_str::<ArrobaUserConfig>(payload).expect("vault credential should parse");
        config.validate().expect("vault credential should validate");
        assert_eq!(config.credential_vault.service, "arroba-test");
        assert_eq!(
            config.credentials[0].source,
            UserCredentialSourceConfig::Vault {
                key: "github-token".to_string()
            }
        );
    }

    #[test]
    fn user_config_rejects_duplicate_credential_ids() {
        let payload = r#"
version = 1

[[credentials]]
id = "github"
source = { type = "env", name = "GH_TOKEN" }
injection = { kind = "query", name = "token" }

[[credentials]]
id = "github"
source = { type = "env", name = "OTHER_TOKEN" }
injection = { kind = "query", name = "token" }
"#;

        let config = toml::from_str::<ArrobaUserConfig>(payload)
            .expect("duplicate ids should parse before validation");
        let error = config
            .validate()
            .expect_err("duplicate credential ids should be invalid");
        match error {
            DaemonError::InvalidConfig { field, .. } => assert_eq!(field, "credentials"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn persisted_daemon_config_loads_legacy_machine_registry_without_pairing_fields() {
        let payload = r#"{
          "relay_url": "ws://relay",
          "relay_token": "secret",
          "machines": [
            {
              "machine_id": "machine-1",
              "alias": "worker",
              "approved": true,
              "forgotten": false
            }
          ]
        }"#;

        let persisted = serde_json::from_str::<PersistedDaemonConfig>(payload)
            .expect("legacy daemon config should decode");

        assert_eq!(persisted.clients, Vec::<PersistedClientPairing>::new());
        assert_eq!(persisted.machines.len(), 1);
        assert_eq!(persisted.machines[0].machine_id, "machine-1");
        assert_eq!(persisted.machines[0].alias.as_deref(), Some("worker"));
        assert_eq!(persisted.machines[0].public_key_thumbprint, None);
        assert_eq!(persisted.machines[0].paired_at_ms, None);
        assert!(persisted.machines[0].approved);
    }

    #[test]
    fn managed_io_policy_defaults_to_unrestricted() {
        let config = DaemonConfig::new("daemon", "machine", "tester");

        assert!(!config.provider_requires_managed_io("codex"));
        assert!(!config.provider_requires_managed_io("opencode"));
        assert!(!config.provider_requires_managed_io("default"));
    }

    #[test]
    fn managed_io_policy_can_be_changed_and_persisted_in_user_config() {
        let path = std::env::temp_dir().join(format!(
            "arroba-user-config-test-{}-{}.toml",
            std::process::id(),
            generate_identity_suffix()
        ));
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config_path = path.clone();

        config
            .set_user_config_value("providers.managed_io", "unrestricted")
            .expect("managed I/O policy should update");

        assert!(!config.provider_requires_managed_io("opencode"));
        assert!(!config.provider_requires_managed_io("codex"));

        let loaded = load_user_config_from_path(&path);
        assert_eq!(
            loaded.providers.managed_io.mode,
            ManagedIoMode::Unrestricted
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn user_config_schema_lists_settable_kernel_owned_keys() {
        let schema = DaemonConfig::user_config_schema();
        let managed_io = schema
            .iter()
            .find(|entry| entry.path == "providers.managed_io")
            .expect("managed I/O schema entry should exist");

        assert!(managed_io.settable);
        assert!(managed_io.unsettable);
        assert_eq!(managed_io.effect, "provider_reload");
        assert_eq!(managed_io.allowed_values, vec!["required", "unrestricted"]);
        assert!(schema
            .iter()
            .any(|entry| entry.path == "ui.worktree_aliases.<alias>"));
    }

    #[test]
    fn managed_io_policy_rejects_per_provider_setter_keys() {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");

        let set_error = config
            .set_user_config_value("providers.managed_io.codex", "unrestricted")
            .expect_err("per-provider managed I/O setters should be rejected");
        let unset_error = config
            .unset_user_config_value("providers.managed_io.codex")
            .expect_err("per-provider managed I/O unsets should be rejected");

        assert!(matches!(
            set_error,
            DaemonError::InvalidConfig {
                field: "user_config",
                ..
            }
        ));
        assert!(matches!(
            unset_error,
            DaemonError::InvalidConfig {
                field: "user_config",
                ..
            }
        ));
        assert!(!config.provider_requires_managed_io("codex"));
    }

    #[test]
    fn legacy_per_provider_managed_io_config_loads_into_global_mode() {
        let path = std::env::temp_dir().join(format!(
            "arroba-user-config-legacy-managed-io-test-{}-{}.toml",
            std::process::id(),
            generate_identity_suffix()
        ));
        std::fs::write(
            &path,
            r#"version = 1

[providers]
default = "opencode"

[providers.managed_io]
default = "required"
opencode = "unrestricted"
codex = "required"
"#,
        )
        .expect("legacy managed I/O config should write");

        let loaded = load_user_config_from_path(&path);
        assert_eq!(loaded.providers.managed_io.mode, ManagedIoMode::Required);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn history_and_state_config_defaults_are_available() {
        let config = DaemonConfig::new("daemon", "machine", "tester");

        assert_eq!(
            config.user_config.history.operational.backend,
            HistoryOperationalBackend::Sqlite
        );
        assert_eq!(
            config.user_config.history.operational.retention_days,
            Some(30)
        );
        assert_eq!(
            config.user_config.history.archive.mode,
            HistoryArchiveMode::Disabled
        );
        assert_eq!(config.user_config.state.backend, StateBackend::Sqlite);
        assert_eq!(
            config.user_config.state.snapshot_interval_events,
            Some(1_000)
        );
    }

    #[test]
    fn history_archive_external_requires_url() {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");

        let error = config
            .set_user_config_value("history.archive.mode", "external")
            .expect_err("external archive without a URL should be rejected");

        match error {
            DaemonError::InvalidConfig { field, .. } => {
                assert_eq!(field, "history.archive.url");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn history_and_state_config_can_be_changed_and_persisted() {
        let path = std::env::temp_dir().join(format!(
            "arroba-history-config-test-{}-{}.toml",
            std::process::id(),
            generate_identity_suffix()
        ));
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config_path = path.clone();

        config
            .set_user_config_value("history.operational.path", "~/.arroba/custom/history.db")
            .expect("operational history path should update");
        config
            .set_user_config_value("history.operational.retention_days", "10")
            .expect("retention should update");
        config
            .set_user_config_value("history.archive.url", "http://127.0.0.1:49300")
            .expect("archive URL should update");
        config
            .set_user_config_value("history.archive.mode", "external")
            .expect("archive mode should update after URL is set");
        config
            .set_user_config_value("state.snapshot_interval_events", "250")
            .expect("state snapshot interval should update");

        let loaded = load_user_config_from_path(&path);
        assert_eq!(
            loaded.history.operational.path.as_deref(),
            Some("~/.arroba/custom/history.db")
        );
        assert_eq!(loaded.history.operational.retention_days, Some(10));
        assert_eq!(loaded.history.archive.mode, HistoryArchiveMode::External);
        assert_eq!(
            loaded.history.archive.url.as_deref(),
            Some("http://127.0.0.1:49300")
        );
        assert_eq!(loaded.state.snapshot_interval_events, Some(250));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn operational_history_path_expands_home() {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config.history.operational.path =
            Some("~/.arroba/custom/history.db".to_string());

        assert!(config
            .operational_history_path()
            .ends_with(".arroba/custom/history.db"));
    }

    #[test]
    fn durable_state_path_expands_home() {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config.state.path = Some("~/.arroba/custom/state.db".to_string());

        assert!(config
            .durable_state_path()
            .ends_with(".arroba/custom/state.db"));
    }

    fn env_test_guard() -> &'static Mutex<()> {
        static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
        GUARD.get_or_init(|| Mutex::new(()))
    }

    unsafe fn restore_env_var(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => env::set_var(key, value),
            None => env::remove_var(key),
        }
    }
}
