use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::DaemonError;
use crate::transport::relay_crypto;
use serde::{Deserialize, Serialize};

mod credentials;
mod env_loader;
mod identity;
mod pairings;
mod paths;
mod persisted_daemon;
mod provider;
mod relay_profile;
mod slices;
mod storage;
mod user_config_mutation;
mod user_config_schema;
mod validation;

pub use credentials::{
    validate_credentials, CredentialVaultAgentManagementPolicy, CredentialVaultBackend,
    CredentialVaultUnlockPolicy, UserCredentialConfig, UserCredentialInjectionConfig,
    UserCredentialMetadataConfig, UserCredentialSourceConfig, UserCredentialUse,
    UserCredentialVaultConfig,
};
#[cfg(test)]
use identity::{generate_identity_suffix, RuntimeIdentity};
#[cfg(test)]
use persisted_daemon::PersistedDaemonConfig;
#[cfg(test)]
use persisted_daemon::HOSTED_STAGING_RELAY_URL;
#[cfg(test)]
use persisted_daemon::{upsert_client_pairing, upsert_machine_registration};
pub use persisted_daemon::{
    PersistedClientPairing, PersistedCloudRelayProfile, PersistedMachineRegistration,
};
pub use provider::{UserProviderConfig, WorkspaceLiveSyncConfig, WorkspaceLiveSyncMode};
pub use slices::{
    SliceImageBuildPolicy, UserLinuxSliceConfig, UserSlicesConfig, DEFAULT_LINUX_SLICE_DOCKER_IMAGE,
};
pub use storage::{
    ArtifactOperationalBackend, HistoryArchiveMode, HistoryOperationalBackend, StateBackend,
    UserArchiveArtifactsConfig, UserArchiveHistoryConfig, UserArtifactsConfig, UserHistoryConfig,
    UserOperationalArtifactsConfig, UserOperationalHistoryConfig, UserStateConfig,
};
pub use user_config_schema::UserConfigSchemaEntry;

pub const DEFAULT_KERNEL_WEBSOCKET_WRITE_DELAY_MS: u64 = 33;
pub const DEFAULT_RELAY_HEARTBEAT_MS: u64 = 5_000;

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
    pub operational_history_read_delay_ms: u64,
    pub provider_catalog_read_delay_ms: u64,
    pub provider_process_list_delay_ms: u64,
    pub provider_process_idle_ttl_ms: u64,
    pub provider_process_orphan_ttl_ms: u64,
    pub provider_runtime_init_delay_ms: u64,
}

impl DaemonConfig {
    pub(crate) fn relay_url_uses_cloud_profile(&self, relay_url: &str) -> bool {
        let relay_url = normalize_relay_url_for_match(relay_url);
        self.cloud_relay
            .as_ref()
            .is_some_and(|profile| normalize_relay_url_for_match(&profile.relay_url) == relay_url)
    }

    pub(crate) fn apply_remote_relay_override(&mut self, relay_url: String, relay_token: String) {
        if self.relay_url_uses_cloud_profile(&relay_url) {
            return;
        }
        self.relay_url = Some(relay_url);
        self.relay_token = Some(relay_token);
        self.cloud_relay = None;
    }

    pub(crate) fn apply_missing_remote_relay_override(
        &mut self,
        relay_url: String,
        relay_token: String,
    ) {
        if self.relay_url.is_some() && self.relay_token.is_some() {
            return;
        }
        if self.relay_url_uses_cloud_profile(&relay_url) {
            return;
        }
        if self.relay_url.is_none() {
            self.relay_url = Some(relay_url);
        }
        if self.relay_token.is_none() {
            self.relay_token = Some(relay_token);
        }
        self.cloud_relay = None;
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
            kernel_websocket_write_delay_ms: DEFAULT_KERNEL_WEBSOCKET_WRITE_DELAY_MS,
            runtime_mcp_host: "127.0.0.1".to_string(),
            runtime_mcp_port: 43120,
            session_history_root: Self::default_session_history_root(),
            session_history_read_delay_ms: 0,
            operational_history_read_delay_ms: 0,
            provider_catalog_read_delay_ms: 0,
            provider_process_list_delay_ms: 0,
            provider_process_idle_ttl_ms: 300_000,
            provider_process_orphan_ttl_ms: 30_000,
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
            relay_heartbeat_ms: DEFAULT_RELAY_HEARTBEAT_MS,
            relay_request_timeout_ms: 60_000,
            accept_remote_leases: true,
            os_user: os_user.into(),
        }
    }

    pub fn for_tests() -> Self {
        static TEST_SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

        let index = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
        let mut config = Self::new("daemon-test", "machine-test", "tester");
        config.kernel_websocket_write_delay_ms = 0;
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
                .join(format!("kernel-state-{}-{}", std::process::id(), index))
                .join("state.db")
                .display()
                .to_string(),
        );
        config.user_config.workflow.max_queues_per_workflow = Some(10);
        config.user_config.providers.workspace_live_sync =
            crate::config::WorkspaceLiveSyncConfig::from_mode(
                crate::config::WorkspaceLiveSyncMode::Unrestricted,
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

    pub fn user_config_path(&self) -> &PathBuf {
        &self.user_config_path
    }

    pub fn provider_requires_workspace_live_sync(&self, _provider: &str) -> bool {
        self.user_config
            .providers
            .workspace_live_sync
            .requires_workspace_live_sync()
    }

    pub fn provider_tracks_workspace_live_sync(&self, _provider: &str) -> bool {
        self.user_config
            .providers
            .workspace_live_sync
            .tracks_workspace_live_sync()
    }

    pub fn provider_workspace_live_sync_mode(
        &self,
        _provider: &str,
    ) -> crate::config::WorkspaceLiveSyncMode {
        self.user_config.providers.workspace_live_sync.mode
    }

    pub fn max_workflow_queues_per_workflow(&self) -> usize {
        self.user_config
            .workflow
            .max_queues_per_workflow
            .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_QUEUES) as usize
    }

    pub fn session_default_max_agents(&self) -> i32 {
        self.user_config
            .workflow
            .session_default_max_agents
            .unwrap_or(crate::session::DEFAULT_SESSION_MAX_AGENTS as u32)
            .min(i32::MAX as u32) as i32
    }

    pub fn workflow_code_limits(&self) -> WorkflowCodeLimitsConfig {
        let mut limits = self.user_config.workflow.code_limits();
        limits.max_queues = limits
            .max_queues
            .min(self.max_workflow_queues_per_workflow() as u32);
        limits
    }

    pub fn operational_history_max_size_bytes(&self) -> u64 {
        self.user_config
            .history
            .operational
            .max_size_mb
            .map(|value| value as u64 * 1024 * 1024)
            .unwrap_or(crate::history::OPERATIONAL_HISTORY_HARD_MAX_BYTES)
            .clamp(1, crate::history::OPERATIONAL_HISTORY_HARD_MAX_BYTES)
    }

    pub fn set_user_config_value(
        &mut self,
        key_path: impl AsRef<str>,
        value: impl Into<String>,
    ) -> Result<(), DaemonError> {
        let key_path = key_path.as_ref();
        self.user_config.set_value(key_path, value.into())?;
        self.apply_live_user_config_value(key_path);
        persist_user_config(&self.user_config_path, &self.user_config)?;
        Ok(())
    }

    pub fn unset_user_config_value(
        &mut self,
        key_path: impl AsRef<str>,
    ) -> Result<(), DaemonError> {
        let key_path = key_path.as_ref();
        self.user_config.unset_value(key_path)?;
        self.apply_live_user_config_value(key_path);
        persist_user_config(&self.user_config_path, &self.user_config)?;
        Ok(())
    }

    pub fn user_config_schema() -> Vec<UserConfigSchemaEntry> {
        user_config_schema::entries()
    }

    fn apply_live_user_config_value(&mut self, key_path: &str) {
        if key_path == "relay.accept_remote_leases" {
            self.accept_remote_leases = self.user_config.relay.accept_remote_leases.unwrap_or(true);
        }
    }
}

fn normalize_relay_url_for_match(relay_url: &str) -> &str {
    relay_url.trim().trim_end_matches('/')
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
    pub workflow: UserWorkflowConfig,
    #[serde(default)]
    pub credential_vault: UserCredentialVaultConfig,
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
            workflow: UserWorkflowConfig::default(),
            credential_vault: UserCredentialVaultConfig::default(),
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserWorkflowConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_queues_per_workflow: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_default_max_agents: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<UserWorkflowCodeConfig>,
}

impl UserWorkflowConfig {
    pub fn code_limits(&self) -> WorkflowCodeLimitsConfig {
        self.code
            .as_ref()
            .map(UserWorkflowCodeConfig::limits)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserWorkflowCodeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_nodes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_agents: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edges: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_endpoints: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_queues: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_watchdogs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_schema_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_generated_prompt_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_memory_bytes: Option<u64>,
}

impl UserWorkflowCodeConfig {
    pub fn is_empty(&self) -> bool {
        self.max_concurrent.is_none()
            && self.max_nodes.is_none()
            && self.max_agents.is_none()
            && self.max_edges.is_none()
            && self.max_endpoints.is_none()
            && self.max_queues.is_none()
            && self.max_watchdogs.is_none()
            && self.max_schema_bytes.is_none()
            && self.max_generated_prompt_bytes.is_none()
            && self.script_timeout_ms.is_none()
            && self.script_memory_bytes.is_none()
    }

    pub fn limits(&self) -> WorkflowCodeLimitsConfig {
        WorkflowCodeLimitsConfig {
            max_concurrent: self
                .max_concurrent
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_CONCURRENT),
            max_nodes: self
                .max_nodes
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_NODES),
            max_agents: self
                .max_agents
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_AGENTS),
            max_edges: self
                .max_edges
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_EDGES),
            max_endpoints: self
                .max_endpoints
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_ENDPOINTS),
            max_queues: self
                .max_queues
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_QUEUES),
            max_watchdogs: self
                .max_watchdogs
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_WATCHDOGS),
            max_schema_bytes: self
                .max_schema_bytes
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_SCHEMA_BYTES),
            max_generated_prompt_bytes: self
                .max_generated_prompt_bytes
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_MAX_GENERATED_PROMPT_BYTES),
            script_timeout_ms: self
                .script_timeout_ms
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_SCRIPT_TIMEOUT_MS),
            script_memory_bytes: self
                .script_memory_bytes
                .unwrap_or(crate::session::DEFAULT_WORKFLOW_CODE_SCRIPT_MEMORY_BYTES),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeLimitsConfig {
    pub max_concurrent: u32,
    pub max_nodes: u32,
    pub max_agents: u32,
    pub max_edges: u32,
    pub max_endpoints: u32,
    pub max_queues: u32,
    pub max_watchdogs: u32,
    pub max_schema_bytes: u32,
    pub max_generated_prompt_bytes: u32,
    pub script_timeout_ms: u64,
    pub script_memory_bytes: u64,
}

impl Default for WorkflowCodeLimitsConfig {
    fn default() -> Self {
        UserWorkflowCodeConfig::default().limits()
    }
}

fn default_user_config_version() -> u32 {
    1
}

fn load_user_config_from_path(path: &PathBuf) -> ArrobaUserConfig {
    let Some(payload) = fs::read_to_string(path).ok() else {
        return ArrobaUserConfig::default();
    };
    let mut config = toml::from_str::<ArrobaUserConfig>(&payload).unwrap_or_default();
    clamp_operational_history_config(&mut config);
    reject_test_persistence_paths_in_default_user_config(path, &config);
    config
}

fn persist_user_config(path: &PathBuf, config: &ArrobaUserConfig) -> Result<(), DaemonError> {
    reject_test_persistence_paths_for_persist(path, config)?;
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

fn clamp_operational_history_config(config: &mut ArrobaUserConfig) {
    if let Some(max_size_mb) = config.history.operational.max_size_mb.as_mut() {
        *max_size_mb = (*max_size_mb).min(crate::history::OPERATIONAL_HISTORY_HARD_MAX_MB);
    }
}

fn reject_test_persistence_paths_in_default_user_config(path: &PathBuf, config: &ArrobaUserConfig) {
    if path != &DaemonConfig::default_user_config_path() {
        return;
    }
    if user_config_has_test_persistence_path(config) {
        panic!(
            "default Arroba user config contains test persistence paths under arroba-tests; remove history/state test paths from {}",
            path.display()
        );
    }
}

fn reject_test_persistence_paths_for_persist(
    path: &PathBuf,
    config: &ArrobaUserConfig,
) -> Result<(), DaemonError> {
    if path == &DaemonConfig::default_user_config_path()
        && user_config_has_test_persistence_path(config)
    {
        return Err(DaemonError::InvalidConfig {
            field: "user_config",
            message: "default user config must not persist test paths under arroba-tests",
        });
    }
    Ok(())
}

fn user_config_has_test_persistence_path(config: &ArrobaUserConfig) -> bool {
    [
        config.history.operational.path.as_deref(),
        config.artifacts.operational.root.as_deref(),
        config.artifacts.operational.index_path.as_deref(),
        config.state.path.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(is_test_persistence_path)
}

fn is_test_persistence_path(path: &str) -> bool {
    path.contains("/arroba-tests/") || path.contains("\\arroba-tests\\")
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
    use std::env;
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
    fn kernel_websocket_write_delay_coalesces_events_outside_test_configs() {
        let config = DaemonConfig::new("daemon", "machine", "tester");
        assert_eq!(
            config.kernel_websocket_write_delay_ms,
            DEFAULT_KERNEL_WEBSOCKET_WRITE_DELAY_MS
        );

        let test_config = DaemonConfig::for_tests();
        assert_eq!(test_config.kernel_websocket_write_delay_ms, 0);
    }

    #[test]
    fn relay_heartbeat_defaults_to_human_scale_cadence() {
        let config = DaemonConfig::new("daemon", "machine", "tester");
        assert_eq!(config.relay_heartbeat_ms, DEFAULT_RELAY_HEARTBEAT_MS);
        assert_eq!(config.relay_heartbeat_ms, 5_000);
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
        let old_cloud_relay_config = env::var_os("ARROBA_CLOUD_RELAY_CONFIG_JSON");
        unsafe {
            env::set_var("HOME", &temp_home);
            env::remove_var("XDG_CONFIG_HOME");
            env::remove_var("XDG_STATE_HOME");
            env::set_var("ARROBA_RELAY_URL", "ws://127.0.0.1:47000");
            env::set_var("ARROBA_RELAY_TOKEN", "local-drill-token");
            env::remove_var("ARROBA_CLOUD_RELAY_CONFIG_JSON");
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
            restore_env_var("ARROBA_CLOUD_RELAY_CONFIG_JSON", old_cloud_relay_config);
        }
        let _ = fs::remove_dir_all(temp_home);

        assert_eq!(config.relay_url.as_deref(), Some("ws://127.0.0.1:47000"));
        assert_eq!(config.relay_token.as_deref(), Some("local-drill-token"));
        assert_eq!(config.cloud_relay, None);
    }

    #[test]
    fn relay_url_uses_cloud_profile_tolerates_spacing_and_trailing_slashes() {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: "https://cloud.example.test".to_string(),
            email: "user@example.test".to_string(),
            account_id: "account-1".to_string(),
            user_id: "user-1".to_string(),
            account_slug: "account".to_string(),
            realm_id: "realm-1".to_string(),
            relay_url: "wss://relay.example.test/".to_string(),
            issuer_id: "issuer-1".to_string(),
            client_id: None,
            client_alias: None,
            machine_id: Some("machine-1".to_string()),
            machine_alias: None,
            machine_credential: Some("machine-secret".to_string()),
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: Some(42),
        });

        assert!(config.relay_url_uses_cloud_profile(" wss://relay.example.test "));
        assert!(config.relay_url_uses_cloud_profile("wss://relay.example.test//"));
        assert!(!config.relay_url_uses_cloud_profile("wss://other-relay.example.test"));
    }

    #[test]
    fn env_cloud_profile_can_accompany_env_relay_config_for_worker_refresh() {
        let _guard = env_test_guard().lock().expect("env test guard poisoned");
        let temp_home = std::env::temp_dir().join(format!(
            "arroba-config-env-cloud-relay-test-{}",
            generate_identity_suffix()
        ));
        let old_home = env::var_os("HOME");
        let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
        let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
        let old_relay_url = env::var_os("ARROBA_RELAY_URL");
        let old_relay_token = env::var_os("ARROBA_RELAY_TOKEN");
        let old_cloud_relay_config = env::var_os("ARROBA_CLOUD_RELAY_CONFIG_JSON");
        unsafe {
            env::set_var("HOME", &temp_home);
            env::remove_var("XDG_CONFIG_HOME");
            env::remove_var("XDG_STATE_HOME");
            env::set_var("ARROBA_RELAY_URL", "wss://195.201.123.115.sslip.io");
            env::set_var("ARROBA_RELAY_TOKEN", "runtime-token");
            env::set_var(
                "ARROBA_CLOUD_RELAY_CONFIG_JSON",
                r#"{
                  "cloud_relay": {
                    "api_url": "https://arroba-cloud-staging.osc-fr1.scalingo.io",
                    "email": "worker@example.com",
                    "account_id": "account-1",
                    "user_id": "user-1",
                    "account_slug": "account",
                    "realm_id": "realm-1",
                    "relay_url": "ws://195.201.123.115:43130",
                    "issuer_id": "arroba-cloud-staging",
                    "machine_id": "machine-1",
                    "machine_credential": "machine-credential",
                    "token_expires_at_ms": 1
                  }
                }"#,
            );
        }

        let config = DaemonConfig::load_from_env();

        unsafe {
            restore_env_var("HOME", old_home);
            restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
            restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
            restore_env_var("ARROBA_RELAY_URL", old_relay_url);
            restore_env_var("ARROBA_RELAY_TOKEN", old_relay_token);
            restore_env_var("ARROBA_CLOUD_RELAY_CONFIG_JSON", old_cloud_relay_config);
        }
        let _ = fs::remove_dir_all(temp_home);

        assert_eq!(
            config.relay_url.as_deref(),
            Some("wss://195.201.123.115.sslip.io")
        );
        assert_eq!(config.relay_token.as_deref(), Some("runtime-token"));
        let profile = config
            .cloud_relay
            .expect("env cloud profile should be loaded with env relay config");
        assert_eq!(profile.account_id, "account-1");
        assert_eq!(profile.machine_id.as_deref(), Some("machine-1"));
        assert_eq!(profile.relay_url, HOSTED_STAGING_RELAY_URL);
    }

    #[test]
    fn load_from_env_imports_cli_cloud_profile_for_kernel_startup() {
        let _guard = env_test_guard().lock().expect("env test guard poisoned");
        let temp_home = std::env::temp_dir().join(format!(
            "arroba-config-cli-cloud-import-test-{}",
            generate_identity_suffix()
        ));
        let old_home = env::var_os("HOME");
        let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
        let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
        let old_relay_url = env::var_os("ARROBA_RELAY_URL");
        let old_relay_token = env::var_os("ARROBA_RELAY_TOKEN");
        let old_cloud_relay_config = env::var_os("ARROBA_CLOUD_RELAY_CONFIG_JSON");
        unsafe {
            env::set_var("HOME", &temp_home);
            env::remove_var("XDG_CONFIG_HOME");
            env::remove_var("XDG_STATE_HOME");
            env::remove_var("ARROBA_RELAY_URL");
            env::remove_var("ARROBA_RELAY_TOKEN");
            env::remove_var("ARROBA_CLOUD_RELAY_CONFIG_JSON");
        }
        let preferences_path = temp_home.join(".arroba").join("config.json");
        fs::create_dir_all(preferences_path.parent().expect("preferences parent"))
            .expect("preferences parent should be created");
        fs::write(
            &preferences_path,
            r#"{
              "relay": {
                "cloud": {
                  "apiUrl": "https://arroba-cloud-staging.osc-fr1.scalingo.io",
                  "email": "test@example.com",
                  "accountId": "account-1",
                  "userId": "user-1",
                  "accountSlug": "account",
                  "realmId": "realm-1",
                  "relayUrl": "ws://195.201.123.115:43130",
                  "issuerId": "arroba-cloud-staging",
                  "machineId": "machine-1",
                  "machineCredential": "machine-credential",
                  "cloudSessionToken": "session-token",
                  "cloudSessionExpiresAtMs": 12345
                }
              }
            }"#,
        )
        .expect("CLI preferences should write");

        let config = DaemonConfig::load_from_env();

        unsafe {
            restore_env_var("HOME", old_home);
            restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
            restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
            restore_env_var("ARROBA_RELAY_URL", old_relay_url);
            restore_env_var("ARROBA_RELAY_TOKEN", old_relay_token);
            restore_env_var("ARROBA_CLOUD_RELAY_CONFIG_JSON", old_cloud_relay_config);
        }
        let _ = fs::remove_dir_all(temp_home);

        let profile = config
            .cloud_relay
            .expect("CLI cloud profile should seed kernel cloud relay");
        assert_eq!(profile.account_id, "account-1");
        assert_eq!(profile.machine_id.as_deref(), Some("machine-1"));
        assert_eq!(
            profile.machine_credential.as_deref(),
            Some("machine-credential")
        );
        assert_eq!(profile.relay_url, HOSTED_STAGING_RELAY_URL);
        assert_eq!(config.relay_url, None);
        assert_eq!(config.relay_token, None);
    }

    #[test]
    fn persisted_daemon_cloud_profile_takes_precedence_over_cli_profile() {
        let _guard = env_test_guard().lock().expect("env test guard poisoned");
        let temp_home = std::env::temp_dir().join(format!(
            "arroba-config-daemon-cloud-precedence-test-{}",
            generate_identity_suffix()
        ));
        let old_home = env::var_os("HOME");
        let old_xdg_config_home = env::var_os("XDG_CONFIG_HOME");
        let old_xdg_state_home = env::var_os("XDG_STATE_HOME");
        let old_relay_url = env::var_os("ARROBA_RELAY_URL");
        let old_relay_token = env::var_os("ARROBA_RELAY_TOKEN");
        let old_cloud_relay_config = env::var_os("ARROBA_CLOUD_RELAY_CONFIG_JSON");
        unsafe {
            env::set_var("HOME", &temp_home);
            env::remove_var("XDG_CONFIG_HOME");
            env::remove_var("XDG_STATE_HOME");
            env::remove_var("ARROBA_RELAY_URL");
            env::remove_var("ARROBA_RELAY_TOKEN");
            env::remove_var("ARROBA_CLOUD_RELAY_CONFIG_JSON");
        }
        let daemon_config_path = DaemonConfig::default_daemon_config_path();
        fs::create_dir_all(daemon_config_path.parent().expect("daemon config parent"))
            .expect("daemon config parent should be created");
        fs::write(
            &daemon_config_path,
            r#"{
              "cloud_relay": {
                "api_url": "https://daemon-cloud.example",
                "email": "daemon@example.com",
                "account_id": "daemon-account",
                "user_id": "daemon-user",
                "account_slug": "daemon",
                "realm_id": "daemon-realm",
                "relay_url": "wss://daemon-relay.example",
                "issuer_id": "daemon-issuer",
                "machine_credential": "daemon-machine-credential"
              }
            }"#,
        )
        .expect("daemon config should write");
        let preferences_path = temp_home.join(".arroba").join("config.json");
        fs::write(
            &preferences_path,
            r#"{
              "relay": {
                "cloud": {
                  "apiUrl": "https://cli-cloud.example",
                  "email": "cli@example.com",
                  "accountId": "cli-account",
                  "userId": "cli-user",
                  "accountSlug": "cli",
                  "realmId": "cli-realm",
                  "relayUrl": "wss://cli-relay.example",
                  "issuerId": "cli-issuer",
                  "machineCredential": "cli-machine-credential"
                }
              }
            }"#,
        )
        .expect("CLI preferences should write");

        let config = DaemonConfig::load_from_env();

        unsafe {
            restore_env_var("HOME", old_home);
            restore_env_var("XDG_CONFIG_HOME", old_xdg_config_home);
            restore_env_var("XDG_STATE_HOME", old_xdg_state_home);
            restore_env_var("ARROBA_RELAY_URL", old_relay_url);
            restore_env_var("ARROBA_RELAY_TOKEN", old_relay_token);
            restore_env_var("ARROBA_CLOUD_RELAY_CONFIG_JSON", old_cloud_relay_config);
        }
        let _ = fs::remove_dir_all(temp_home);

        let profile = config
            .cloud_relay
            .expect("daemon cloud profile should be loaded");
        assert_eq!(profile.account_id, "daemon-account");
        assert_eq!(profile.relay_url, "wss://daemon-relay.example");
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
    fn user_config_parses_slice_defaults() {
        let payload = r#"
version = 1

[slices]
root = "~/.arroba/slices-dev"

[slices.linux]
docker_image = "arroba-slice-linux-custom:local"
	build_image = "never"
	extension_dockerfile = "~/.arroba/slices/extensions/Dockerfile"
	allow_unconfined_seccomp = true
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
        assert_eq!(config.slices.linux.allow_unconfined_seccomp, Some(true));
        assert_eq!(config.slices.linux.memory_mb, Some(4096));
        assert_eq!(config.slices.linux.cpus.as_deref(), Some("2.5"));
        assert_eq!(config.slices.linux.screen_width, Some(1440));
        assert_eq!(config.slices.linux.screen_height, Some(900));
    }

    #[test]
    fn user_config_defaults_to_versioned_slice_image() {
        let config = ArrobaUserConfig::default();

        assert_eq!(
            config.slices.linux.docker_image.as_deref(),
            Some(DEFAULT_LINUX_SLICE_DOCKER_IMAGE)
        );
        assert_ne!(DEFAULT_LINUX_SLICE_DOCKER_IMAGE, "arroba-slice-linux:local");
    }

    #[test]
    fn user_config_parses_credential_vault_service() {
        let payload = r#"
version = 1

[credential_vault]
service = "arroba-test"
"#;

        let config =
            toml::from_str::<ArrobaUserConfig>(payload).expect("credential vault should parse");
        config.validate().expect("credential vault should validate");
        assert_eq!(config.credential_vault.service, "arroba-test");
    }

    #[test]
    fn user_config_parses_credential_vault_backend() {
        let payload = r#"
version = 1

[credential_vault]
backend = "process_memory"
"#;

        let config =
            toml::from_str::<ArrobaUserConfig>(payload).expect("credential vault should parse");
        config.validate().expect("credential vault should validate");
        assert_eq!(
            config.credential_vault.backend,
            CredentialVaultBackend::ProcessMemory
        );
    }

    #[test]
    fn user_config_rejects_unimplemented_credential_vault_unlock_scopes() {
        for unlock_policy in ["session", "agent"] {
            let payload = format!(
                r#"
version = 1

[credential_vault]
unlock_policy = "{unlock_policy}"
"#
            );

            let error = toml::from_str::<ArrobaUserConfig>(&payload)
                .expect_err("unimplemented unlock scope should not parse");
            assert!(error.to_string().contains("kernel_init, ttl, or always"));
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
    fn workspace_live_sync_policy_serializes_default_as_off() {
        let config = DaemonConfig::new("daemon", "machine", "tester");

        assert!(!config.provider_requires_workspace_live_sync("codex"));
        assert!(!config.provider_requires_workspace_live_sync("opencode"));
        assert!(!config.provider_requires_workspace_live_sync("default"));
        assert!(!config.provider_tracks_workspace_live_sync("codex"));
    }

    #[test]
    fn test_config_defaults_to_unrestricted_workspace_live_sync() {
        let config = DaemonConfig::for_tests();

        assert!(!config.provider_requires_workspace_live_sync("dev-stub"));
        assert!(!config.provider_tracks_workspace_live_sync("dev-stub"));
    }

    #[test]
    fn workspace_live_sync_policy_can_be_changed_and_persisted_in_user_config() {
        let path = std::env::temp_dir().join(format!(
            "arroba-user-config-test-{}-{}.toml",
            std::process::id(),
            generate_identity_suffix()
        ));
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config_path = path.clone();

        config
            .set_user_config_value("providers.workspace_live_sync", "off")
            .expect("workspace live sync policy should update");

        assert!(!config.provider_requires_workspace_live_sync("opencode"));
        assert!(!config.provider_requires_workspace_live_sync("codex"));

        let loaded = load_user_config_from_path(&path);
        assert_eq!(
            loaded.providers.workspace_live_sync.mode,
            WorkspaceLiveSyncMode::Unrestricted
        );

        config
            .set_user_config_value("providers.workspace_live_sync", "tracked")
            .expect("tracked workspace live sync policy should update");
        assert_eq!(
            config.user_config.providers.workspace_live_sync.mode,
            WorkspaceLiveSyncMode::Tracked
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn workspace_live_sync_policy_defaults_to_off() {
        let encoded =
            toml::to_string(&UserProviderConfig::default()).expect("provider config should encode");

        assert!(encoded.contains("workspace_live_sync = \"off\""));
        assert!(!encoded.contains("workspace_live_sync = \"managed\""));
    }

    #[test]
    fn remote_lease_acceptance_defaults_to_enabled_and_user_config_is_live() {
        let path = std::env::temp_dir().join(format!(
            "arroba-remote-lease-config-test-{}-{}.toml",
            std::process::id(),
            generate_identity_suffix()
        ));
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config_path = path.clone();

        assert!(config.accept_remote_leases);

        config
            .set_user_config_value("relay.accept_remote_leases", "false")
            .expect("remote lease setting should persist");
        assert!(!config.accept_remote_leases);
        assert_eq!(config.user_config.relay.accept_remote_leases, Some(false));

        config
            .set_user_config_value("relay.accept_remote_leases", "true")
            .expect("remote lease setting should update");
        assert!(config.accept_remote_leases);

        config
            .unset_user_config_value("relay.accept_remote_leases")
            .expect("remote lease setting should unset");
        assert!(config.accept_remote_leases);
        assert_eq!(config.user_config.relay.accept_remote_leases, None);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn workspace_live_sync_policy_accepts_managed_config_spelling() {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");

        config
            .set_user_config_value("providers.workspace_live_sync", "managed")
            .expect("managed workspace live sync config spelling should be accepted");

        assert_eq!(
            config.user_config.providers.workspace_live_sync.mode,
            WorkspaceLiveSyncMode::Managed
        );
    }

    #[test]
    fn workspace_live_sync_policy_rejects_legacy_boolean_aliases() {
        for alias in ["on", "true", "1", "false", "0"] {
            let mut config = DaemonConfig::new("daemon", "machine", "tester");

            let error = config
                .set_user_config_value("providers.workspace_live_sync", alias)
                .expect_err("legacy workspace live sync aliases should be rejected");

            assert!(
                matches!(
                    error,
                    DaemonError::InvalidConfig {
                        field: "providers.workspace_live_sync",
                        ..
                    }
                ),
                "alias {alias:?} should be rejected with an invalid config error"
            );
        }
    }

    #[test]
    fn user_config_schema_lists_settable_kernel_owned_keys() {
        let schema = DaemonConfig::user_config_schema();
        let workspace_live_sync = schema
            .iter()
            .find(|entry| entry.path == "providers.workspace_live_sync")
            .expect("workspace live sync schema entry should exist");

        assert!(workspace_live_sync.settable);
        assert!(workspace_live_sync.unsettable);
        assert_eq!(workspace_live_sync.effect, "provider_reload");
        assert_eq!(
            workspace_live_sync.allowed_values,
            vec!["off", "managed", "tracked"]
        );
        assert!(schema
            .iter()
            .any(|entry| entry.path == "ui.worktree_aliases.<alias>"));
        assert!(schema
            .iter()
            .any(|entry| entry.path == "workflow.session_default_max_agents"));
        for path in [
            "workflow.code.max_concurrent",
            "workflow.code.max_nodes",
            "workflow.code.max_agents",
            "workflow.code.max_edges",
            "workflow.code.max_endpoints",
            "workflow.code.max_queues",
            "workflow.code.max_watchdogs",
            "workflow.code.max_schema_bytes",
            "workflow.code.max_generated_prompt_bytes",
            "workflow.code.script_timeout_ms",
            "workflow.code.script_memory_bytes",
        ] {
            assert!(
                schema.iter().any(|entry| entry.path == path),
                "user config schema should expose `{path}`"
            );
        }
    }

    #[test]
    fn workflow_code_limits_have_large_defaults() {
        let config = DaemonConfig::new("daemon", "machine", "tester");
        let limits = config.workflow_code_limits();

        assert_eq!(
            config.session_default_max_agents(),
            crate::session::DEFAULT_SESSION_MAX_AGENTS
        );
        assert_eq!(
            config.max_workflow_queues_per_workflow(),
            crate::session::DEFAULT_WORKFLOW_CODE_MAX_QUEUES as usize
        );
        assert_eq!(
            limits.max_concurrent,
            crate::session::DEFAULT_WORKFLOW_CODE_MAX_CONCURRENT
        );
        assert_eq!(
            limits.max_agents,
            crate::session::DEFAULT_WORKFLOW_CODE_MAX_AGENTS
        );
        assert_eq!(
            limits.max_endpoints,
            crate::session::DEFAULT_WORKFLOW_CODE_MAX_ENDPOINTS
        );
        assert_eq!(
            limits.max_generated_prompt_bytes,
            crate::session::DEFAULT_WORKFLOW_CODE_MAX_GENERATED_PROMPT_BYTES
        );
    }

    #[test]
    fn workflow_code_limits_can_be_set_and_unset() {
        let path = std::env::temp_dir().join(format!(
            "arroba-workflow-code-config-test-{}-{}.toml",
            std::process::id(),
            generate_identity_suffix()
        ));
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config_path = path.clone();

        config
            .set_user_config_value("workflow.session_default_max_agents", "2048")
            .expect("session agent cap should update");
        config
            .set_user_config_value("workflow.code.max_concurrent", "64")
            .expect("workflow-code concurrency should update");
        config
            .set_user_config_value("workflow.code.max_nodes", "256")
            .expect("workflow-code node cap should update");
        config
            .set_user_config_value("workflow.code.max_endpoints", "128")
            .expect("workflow-code endpoint cap should update");

        assert_eq!(config.session_default_max_agents(), 2048);
        let limits = config.workflow_code_limits();
        assert_eq!(limits.max_concurrent, 64);
        assert_eq!(limits.max_nodes, 256);
        assert_eq!(limits.max_endpoints, 128);

        config
            .unset_user_config_value("workflow.code.max_concurrent")
            .expect("workflow-code concurrency should unset");

        assert_eq!(
            config.workflow_code_limits().max_concurrent,
            crate::session::DEFAULT_WORKFLOW_CODE_MAX_CONCURRENT
        );
        assert_eq!(
            config
                .user_config
                .workflow
                .code
                .as_ref()
                .expect("remaining workflow-code config should stay")
                .max_nodes,
            Some(256)
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn workflow_code_queue_limit_is_capped_by_runtime_queue_limit() {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config.workflow.max_queues_per_workflow = Some(2);
        config.user_config.workflow.code = Some(UserWorkflowCodeConfig {
            max_queues: Some(8),
            ..UserWorkflowCodeConfig::default()
        });

        assert_eq!(config.max_workflow_queues_per_workflow(), 2);
        assert_eq!(config.workflow_code_limits().max_queues, 2);
    }

    #[test]
    fn workflow_code_limits_reject_zero_values() {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");

        let error = config
            .set_user_config_value("workflow.code.max_concurrent", "0")
            .expect_err("zero concurrency should be rejected");

        assert!(matches!(
            error,
            DaemonError::InvalidConfig {
                field: "workflow.code.max_concurrent",
                ..
            }
        ));
    }

    #[test]
    fn workspace_live_sync_policy_rejects_per_provider_setter_keys() {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");

        let set_error = config
            .set_user_config_value("providers.workspace_live_sync.codex", "unrestricted")
            .expect_err("per-provider workspace live sync setters should be rejected");
        let unset_error = config
            .unset_user_config_value("providers.workspace_live_sync.codex")
            .expect_err("per-provider workspace live sync unsets should be rejected");

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
        assert!(!config.provider_requires_workspace_live_sync("codex"));
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
            config.user_config.history.operational.max_size_mb,
            Some(crate::history::OPERATIONAL_HISTORY_HARD_MAX_MB)
        );
        assert_eq!(
            config.operational_history_max_size_bytes(),
            crate::history::OPERATIONAL_HISTORY_HARD_MAX_BYTES
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
    fn operational_history_size_config_is_clamped_to_hard_cap() {
        let path = std::env::temp_dir().join(format!(
            "arroba-history-size-config-test-{}-{}.toml",
            std::process::id(),
            generate_identity_suffix()
        ));
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config_path = path.clone();

        config
            .set_user_config_value("history.operational.max_size_mb", "5000")
            .expect("oversized history cap should clamp");

        let loaded = load_user_config_from_path(&path);
        assert_eq!(
            loaded.history.operational.max_size_mb,
            Some(crate::history::OPERATIONAL_HISTORY_HARD_MAX_MB)
        );
        config.user_config = loaded;
        assert_eq!(
            config.operational_history_max_size_bytes(),
            crate::history::OPERATIONAL_HISTORY_HARD_MAX_BYTES
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn default_user_config_rejects_test_persistence_paths() {
        let mut config = ArrobaUserConfig::default();
        config.history.operational.path =
            Some("/tmp/arroba-tests/operational-history.db".to_string());

        let error = reject_test_persistence_paths_for_persist(
            &DaemonConfig::default_user_config_path(),
            &config,
        )
        .expect_err("default config should reject leaked test paths");

        assert!(matches!(
            error,
            DaemonError::InvalidConfig {
                field: "user_config",
                ..
            }
        ));
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

    #[test]
    fn event_counter_paths_expand_state_home_before_parent() {
        let mut config = DaemonConfig::new("daemon", "machine", "tester");
        config.user_config.state.path = Some("~/.arroba/custom/state.db".to_string());

        let kernel_counter = config.kernel_event_counter_path();
        let relay_counter = config.kernel_relay_event_counter_path();

        assert!(!kernel_counter.starts_with("~"));
        assert!(!relay_counter.starts_with("~"));
        assert!(kernel_counter.ends_with(".arroba/custom/kernel-events/daemon/event-counter.json"));
        assert!(
            relay_counter.ends_with(".arroba/custom/kernel-events/daemon/relay-event-counter.json")
        );
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
