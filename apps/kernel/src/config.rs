use std::collections::BTreeMap;
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
mod private_file;
mod provider;
mod publication_state;
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
pub(crate) use identity::{load_or_create_managed_runtime_identity, ManagedRuntimeIdentity};
#[cfg(test)]
use persisted_daemon::PersistedDaemonConfig;
#[cfg(test)]
use persisted_daemon::HOSTED_STAGING_RELAY_URL;
pub(crate) use persisted_daemon::{
    load_managed_cloud_relay_profile, persist_managed_cloud_relay_profile,
};
#[cfg(test)]
use persisted_daemon::{upsert_client_pairing, upsert_machine_registration};
pub use persisted_daemon::{
    PersistedClientPairing, PersistedCloudRelayProfile, PersistedMachineRegistration,
};
pub(crate) use private_file::write_private_file;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventGeneratorManagementTargetCredential {
    pub url: String,
    pub token: String,
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventGeneratorManagementTarget {
    pub url: String,
    pub token: String,
    /// Signed capabilities are short-lived. Static administrator-provided
    /// targets leave this unset; registry-issued targets are refreshed before
    /// this instant so actions never use an expired capability.
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
    /// Owners authorized by a registry-issued capability. Static targets do
    /// not carry this restriction because their operator token is the policy.
    #[serde(default)]
    pub owner_ids: Option<Vec<String>>,
    /// Registry-issued capabilities are cached independently per owner. A
    /// generator can serve several kernel/user owners at once; retaining only
    /// one token per generator would let concurrent resolutions overwrite one
    /// another and cause intermittent 403s.
    #[serde(default)]
    pub owner_scoped: Option<BTreeMap<String, EventGeneratorManagementTargetCredential>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub user_config_path: PathBuf,
    pub user_config: CharioxUserConfig,
    /// Publication control state survives container replacement. Provider homes,
    /// account registries and managed-context transfers must not use this root.
    pub publication_control_state_root: Option<PathBuf>,
    pub daemon_id: String,
    pub host_machine_id: String,
    pub host_machine_alias: Option<String>,
    pub os_name: String,
    pub daemon_alias: Option<String>,
    pub relay_url: Option<String>,
    pub relay_token: Option<String>,
    pub managed_slice_relay_recovery_token: Option<String>,
    pub managed_slice_relay_owner_public_key: Option<String>,
    pub cloud_relay: Option<PersistedCloudRelayProfile>,
    pub relay_public_key: String,
    pub relay_private_key: String,
    pub relay_heartbeat_ms: u64,
    pub relay_request_timeout_ms: u64,
    pub accept_remote_leases: bool,
    pub event_delivery_url: Option<String>,
    pub event_delivery_token: Option<String>,
    pub event_delivery_environment_id: String,
    pub event_registry_url: Option<String>,
    pub event_generator_management_targets: BTreeMap<String, EventGeneratorManagementTarget>,
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
            user_config: CharioxUserConfig::default(),
            publication_control_state_root: None,
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
            daemon_id: daemon_id.clone(),
            host_machine_id: host_machine_id.into(),
            host_machine_alias: None,
            os_name: default_os_name(),
            daemon_alias: None,
            relay_url: None,
            relay_token: None,
            managed_slice_relay_recovery_token: None,
            managed_slice_relay_owner_public_key: None,
            cloud_relay: None,
            relay_public_key,
            relay_private_key,
            relay_heartbeat_ms: DEFAULT_RELAY_HEARTBEAT_MS,
            relay_request_timeout_ms: 60_000,
            accept_remote_leases: true,
            event_delivery_url: None,
            event_delivery_token: None,
            event_delivery_environment_id: daemon_id.clone(),
            event_registry_url: None,
            event_generator_management_targets: BTreeMap::new(),
            os_user: os_user.into(),
        }
    }

    pub fn for_tests() -> Self {
        static TEST_SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

        let index = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
        let mut config = Self::new("daemon-test", "machine-test", "tester");
        config.kernel_websocket_write_delay_ms = 0;
        config.local_socket_path = std::env::temp_dir().join("chariox-tests").join(format!(
            "daemon-test-{}-{}.sock",
            std::process::id(),
            index
        ));
        config.session_history_root = std::env::temp_dir().join("chariox-tests").join(format!(
            "session-history-{}-{}",
            std::process::id(),
            index
        ));
        config.user_config.history.operational.path = Some(
            std::env::temp_dir()
                .join("chariox-tests")
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
                .join("chariox-tests")
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
                .join("chariox-tests")
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
                .join("chariox-tests")
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
pub struct CharioxUserConfig {
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

impl Default for CharioxUserConfig {
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

fn load_user_config_from_path(path: &PathBuf) -> CharioxUserConfig {
    let Some(payload) = fs::read_to_string(path).ok() else {
        return CharioxUserConfig::default();
    };
    let mut config = toml::from_str::<CharioxUserConfig>(&payload).unwrap_or_default();
    clamp_operational_history_config(&mut config);
    reject_test_persistence_paths_in_default_user_config(path, &config);
    config
}

fn persist_user_config(path: &PathBuf, config: &CharioxUserConfig) -> Result<(), DaemonError> {
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

fn clamp_operational_history_config(config: &mut CharioxUserConfig) {
    if let Some(max_size_mb) = config.history.operational.max_size_mb.as_mut() {
        *max_size_mb = (*max_size_mb).min(crate::history::OPERATIONAL_HISTORY_HARD_MAX_MB);
    }
}

fn reject_test_persistence_paths_in_default_user_config(
    path: &PathBuf,
    config: &CharioxUserConfig,
) {
    if path != &DaemonConfig::default_user_config_path() {
        return;
    }
    if user_config_has_test_persistence_path(config) {
        panic!(
            "default Chariox user config contains test persistence paths under chariox-tests; remove history/state test paths from {}",
            path.display()
        );
    }
}

fn reject_test_persistence_paths_for_persist(
    path: &PathBuf,
    config: &CharioxUserConfig,
) -> Result<(), DaemonError> {
    if path == &DaemonConfig::default_user_config_path()
        && user_config_has_test_persistence_path(config)
    {
        return Err(DaemonError::InvalidConfig {
            field: "user_config",
            message: "default user config must not persist test paths under chariox-tests",
        });
    }
    Ok(())
}

fn user_config_has_test_persistence_path(config: &CharioxUserConfig) -> bool {
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
    path.contains("/chariox-tests/") || path.contains("\\chariox-tests\\")
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
mod tests;
