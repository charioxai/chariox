use std::collections::BTreeMap;
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
    pub user_config_path: PathBuf,
    pub user_config: ArrobaUserConfig,
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
        let user_config_path = Self::default_user_config_path();
        let user_config = load_user_config_from_path(&user_config_path);
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
        default_state_dir().join("sessions")
    }

    pub fn operational_history_path(&self) -> PathBuf {
        self.user_config
            .history
            .operational
            .path
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| default_state_dir().join("history").join("operational.db"))
    }

    pub fn durable_state_path(&self) -> PathBuf {
        self.user_config
            .state
            .path
            .as_deref()
            .map(expand_user_path)
            .unwrap_or_else(|| default_state_dir().join("state").join("kernel.db"))
    }

    pub fn default_runtime_identity_path() -> PathBuf {
        default_state_dir().join("daemon").join("identity.json")
    }

    pub fn default_daemon_config_path() -> PathBuf {
        default_state_dir().join("daemon").join("config.json")
    }

    pub fn default_user_config_path() -> PathBuf {
        default_config_dir().join("config.toml")
    }

    pub fn user_config_path(&self) -> &PathBuf {
        &self.user_config_path
    }

    pub fn provider_requires_managed_io(&self, provider: &str) -> bool {
        self.user_config
            .providers
            .managed_io
            .mode_for(provider)
            .requires_managed_io()
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

    pub fn persist_relay_config(&self) -> Result<(), DaemonError> {
        let mut persisted = load_persisted_daemon_config();
        persisted.relay_url = self.relay_url.clone();
        persisted.relay_token = self.relay_token.clone();
        persist_daemon_config(&persisted, "persist relay config")
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
        let client_id = client_id.into();
        let public_key_thumbprint = public_key_thumbprint.into().trim().to_string();
        validate_non_empty("client_id", &client_id)?;
        validate_non_empty("public_key_thumbprint", &public_key_thumbprint)?;
        let mut persisted = load_persisted_daemon_config();
        let entry = upsert_client_pairing(&mut persisted.clients, &client_id);
        entry.alias = normalized_optional(alias).or_else(|| entry.alias.clone());
        entry.public_key_thumbprint = public_key_thumbprint;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaUserConfig {
    #[serde(default = "default_user_config_version")]
    pub version: u32,
    #[serde(default)]
    pub providers: UserProviderConfig,
    #[serde(default)]
    pub history: UserHistoryConfig,
    #[serde(default)]
    pub state: UserStateConfig,
    #[serde(default)]
    pub ui: UserUiConfig,
    #[serde(default)]
    pub relay: UserRelayConfig,
    #[serde(default)]
    pub kernel: UserKernelConfig,
}

impl Default for ArrobaUserConfig {
    fn default() -> Self {
        Self {
            version: default_user_config_version(),
            providers: UserProviderConfig::default(),
            history: UserHistoryConfig::default(),
            state: UserStateConfig::default(),
            ui: UserUiConfig::default(),
            relay: UserRelayConfig::default(),
            kernel: UserKernelConfig::default(),
        }
    }
}

impl ArrobaUserConfig {
    pub fn validate(&self) -> Result<(), DaemonError> {
        self.providers.managed_io.validate()?;
        self.history.validate()?;
        self.state.validate()?;
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
            "state.backend" => self.state.backend = StateBackend::parse("state.backend", &value)?,
            "state.path" => self.state.path = Some(non_empty_config_string("state.path", value)?),
            "state.snapshot_interval_events" => {
                self.state.snapshot_interval_events = Some(parse_config_u32(
                    "state.snapshot_interval_events",
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
            path if path.starts_with("providers.managed_io.") => {
                let provider = path
                    .trim_start_matches("providers.managed_io.")
                    .trim()
                    .to_string();
                validate_config_provider_key(&provider)?;
                self.providers
                    .managed_io
                    .set_mode(provider, ManagedIoMode::parse(&value)?);
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
            "state.backend" => {
                return Err(DaemonError::InvalidConfig {
                    field: "state.backend",
                    message: "state backend cannot be unset",
                });
            }
            "state.path" => self.state.path = None,
            "state.snapshot_interval_events" => self.state.snapshot_interval_events = None,
            "kernel.websocket_host" => self.kernel.websocket_host = None,
            "kernel.websocket_port" => self.kernel.websocket_port = None,
            "kernel.runtime_mcp_host" => self.kernel.runtime_mcp_host = None,
            "kernel.runtime_mcp_port" => self.kernel.runtime_mcp_port = None,
            path if path.starts_with("providers.managed_io.") => {
                let provider = path.trim_start_matches("providers.managed_io.").trim();
                validate_config_provider_key(provider)?;
                self.providers.managed_io.remove_mode(provider);
            }
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
pub struct ManagedIoConfig {
    #[serde(flatten)]
    pub modes: BTreeMap<String, ManagedIoMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserHistoryConfig {
    #[serde(default)]
    pub operational: UserOperationalHistoryConfig,
    #[serde(default)]
    pub archive: UserArchiveHistoryConfig,
}

impl Default for UserHistoryConfig {
    fn default() -> Self {
        Self {
            operational: UserOperationalHistoryConfig::default(),
            archive: UserArchiveHistoryConfig::default(),
        }
    }
}

impl UserHistoryConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        self.operational.validate()?;
        self.archive.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserOperationalHistoryConfig {
    #[serde(default)]
    pub backend: HistoryOperationalBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_pinned_sessions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_inactive_after_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_deleted_agents: Option<bool>,
}

impl Default for UserOperationalHistoryConfig {
    fn default() -> Self {
        Self {
            backend: HistoryOperationalBackend::Sqlite,
            path: Some("~/.arroba/history/operational.db".to_string()),
            retention_days: Some(30),
            max_size_mb: Some(5_000),
            keep_pinned_sessions: Some(true),
            archive_inactive_after_days: Some(30),
            archive_deleted_agents: Some(true),
        }
    }
}

impl UserOperationalHistoryConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        if let Some(path) = &self.path {
            validate_non_empty("history.operational.path", path)?;
        }
        validate_optional_nonzero("history.operational.retention_days", self.retention_days)?;
        validate_optional_nonzero("history.operational.max_size_mb", self.max_size_mb)?;
        validate_optional_nonzero(
            "history.operational.archive_inactive_after_days",
            self.archive_inactive_after_days,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryOperationalBackend {
    Sqlite,
}

impl Default for HistoryOperationalBackend {
    fn default() -> Self {
        Self::Sqlite
    }
}

impl HistoryOperationalBackend {
    fn parse(field: &'static str, value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sqlite" => Ok(Self::Sqlite),
            _ => Err(DaemonError::InvalidConfig {
                field,
                message: "value must be `sqlite`",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserArchiveHistoryConfig {
    #[serde(default)]
    pub mode: HistoryArchiveMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_deleted_agents: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_before_delete: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_operational_after_verified_archive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_durable_acceptance: Option<bool>,
}

impl Default for UserArchiveHistoryConfig {
    fn default() -> Self {
        Self {
            mode: HistoryArchiveMode::Disabled,
            url: None,
            token_env: None,
            archive_deleted_agents: Some(true),
            archive_before_delete: Some(true),
            delete_operational_after_verified_archive: Some(true),
            require_durable_acceptance: Some(true),
        }
    }
}

impl UserArchiveHistoryConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        match self.mode {
            HistoryArchiveMode::Disabled => Ok(()),
            HistoryArchiveMode::External => {
                let Some(url) = self.url.as_deref() else {
                    return Err(DaemonError::InvalidConfig {
                        field: "history.archive.url",
                        message: "value must be set when archive mode is external",
                    });
                };
                validate_non_empty("history.archive.url", url)?;
                if let Some(token_env) = &self.token_env {
                    validate_non_empty("history.archive.token_env", token_env)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryArchiveMode {
    Disabled,
    External,
}

impl Default for HistoryArchiveMode {
    fn default() -> Self {
        Self::Disabled
    }
}

impl HistoryArchiveMode {
    fn parse(value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "disabled" | "off" | "false" | "0" | "none" => Ok(Self::Disabled),
            "external" => Ok(Self::External),
            _ => Err(DaemonError::InvalidConfig {
                field: "history.archive.mode",
                message: "value must be `disabled` or `external`",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserStateConfig {
    #[serde(default)]
    pub backend: StateBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_interval_events: Option<u32>,
}

impl Default for UserStateConfig {
    fn default() -> Self {
        Self {
            backend: StateBackend::Sqlite,
            path: Some("~/.arroba/state/kernel.db".to_string()),
            snapshot_interval_events: Some(1_000),
        }
    }
}

impl UserStateConfig {
    fn validate(&self) -> Result<(), DaemonError> {
        if let Some(path) = &self.path {
            validate_non_empty("state.path", path)?;
        }
        validate_optional_nonzero(
            "state.snapshot_interval_events",
            self.snapshot_interval_events,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateBackend {
    Sqlite,
}

impl Default for StateBackend {
    fn default() -> Self {
        Self::Sqlite
    }
}

impl StateBackend {
    fn parse(field: &'static str, value: &str) -> Result<Self, DaemonError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sqlite" => Ok(Self::Sqlite),
            _ => Err(DaemonError::InvalidConfig {
                field,
                message: "value must be `sqlite`",
            }),
        }
    }
}

impl Default for ManagedIoConfig {
    fn default() -> Self {
        Self {
            modes: BTreeMap::from([
                ("default".to_string(), ManagedIoMode::Required),
                ("codex".to_string(), ManagedIoMode::Required),
                ("opencode".to_string(), ManagedIoMode::Required),
                ("dev-stub".to_string(), ManagedIoMode::Unrestricted),
                ("managed-dev-stub".to_string(), ManagedIoMode::Required),
            ]),
        }
    }
}

impl ManagedIoConfig {
    pub fn mode_for(&self, provider: &str) -> ManagedIoMode {
        let provider = match provider {
            "default" => "opencode",
            other => other,
        };
        self.modes
            .get(provider)
            .copied()
            .unwrap_or(ManagedIoMode::Unrestricted)
    }

    fn set_mode(&mut self, provider: String, mode: ManagedIoMode) {
        self.modes.insert(provider, mode);
    }

    fn remove_mode(&mut self, provider: &str) {
        self.modes.remove(provider);
    }

    fn validate(&self) -> Result<(), DaemonError> {
        for provider in self.modes.keys() {
            validate_config_provider_key(provider)?;
        }
        Ok(())
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserUiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_agent_response_layout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_agents_per_screen: Option<u32>,
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
    machines: Vec<PersistedMachineRegistration>,
    #[serde(default)]
    clients: Vec<PersistedClientPairing>,
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
    #[serde(default)]
    pub public_key_thumbprint: String,
    #[serde(default)]
    pub paired_at_ms: u64,
    #[serde(default)]
    pub revoked: bool,
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

fn validate_config_provider_key(provider: &str) -> Result<(), DaemonError> {
    validate_non_empty("providers.managed_io", provider)?;
    if !provider
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(DaemonError::InvalidConfig {
            field: "providers.managed_io",
            message: "provider keys may only contain alphanumeric characters, `_`, `-`, or `.`",
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
    fn managed_io_policy_defaults_to_required_for_supported_providers() {
        let config = DaemonConfig::new("daemon", "machine", "tester");

        assert!(config.provider_requires_managed_io("codex"));
        assert!(config.provider_requires_managed_io("opencode"));
        assert!(config.provider_requires_managed_io("default"));
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
            .set_user_config_value("providers.managed_io.opencode", "unrestricted")
            .expect("managed I/O policy should update");

        assert!(!config.provider_requires_managed_io("opencode"));
        assert!(config.provider_requires_managed_io("codex"));

        let loaded = load_user_config_from_path(&path);
        assert_eq!(
            loaded.providers.managed_io.mode_for("opencode"),
            ManagedIoMode::Unrestricted
        );

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
}
