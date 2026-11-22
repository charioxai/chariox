use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::daemon_config_lock::DaemonConfigLock;
use super::paths::{default_config_dir, default_state_dir};
use super::private_file::write_private_file;
use crate::error::DaemonError;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PersistedDaemonConfig {
    #[serde(default)]
    pub(super) relay_url: Option<String>,
    #[serde(default)]
    pub(super) relay_token: Option<String>,
    #[serde(default)]
    pub(super) managed_slice_relay_recovery_token: Option<String>,
    #[serde(default)]
    pub(super) managed_slice_relay_owner_public_key: Option<String>,
    #[serde(default)]
    pub(super) cloud_relay: Option<PersistedCloudRelayProfile>,
    #[serde(default)]
    pub(super) machines: Vec<PersistedMachineRegistration>,
    /// Machines whose worker leases the owner ended by removing them. A private-slice worker has
    /// no registry entry, so this is kept apart from `machines`; approving the machine again clears it.
    #[serde(default)]
    pub(super) revoked_worker_machines: Vec<String>,
    #[serde(default)]
    pub(super) clients: Vec<PersistedClientPairing>,
    #[serde(default)]
    pub(super) relay_peer_public_keys: BTreeMap<String, String>,
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
    pub machine_credential: Option<String>,
    #[serde(default)]
    pub kernel_id: Option<String>,
    #[serde(default)]
    pub kernel_credential: Option<String>,
    #[serde(default)]
    pub kernel_unlinked: bool,
    #[serde(default)]
    pub kernel_removal_pending: bool,
    #[serde(default)]
    pub kernel_public_key_thumbprint: Option<String>,
    #[serde(default)]
    pub cloud_session_token: Option<String>,
    #[serde(default)]
    pub cloud_session_expires_at_ms: Option<u64>,
    #[serde(default)]
    pub token_expires_at_ms: Option<u64>,
}

impl PersistedCloudRelayProfile {
    pub(super) fn enrolled_kernel_id(&self) -> Option<&str> {
        self.kernel_credential
            .as_deref()
            .filter(|credential| !credential.is_empty())?;
        self.kernel_id.as_deref()
    }

    pub(crate) fn kernel_removal(&self) -> Option<CloudKernelRemoval> {
        if self.kernel_unlinked {
            Some(CloudKernelRemoval::Permanent)
        } else if self.kernel_removal_pending {
            Some(CloudKernelRemoval::Pending)
        } else {
            None
        }
    }

    pub(super) fn canonicalized(mut self) -> Self {
        self.api_url = normalize_url_without_trailing_slash(&self.api_url);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudKernelRemoval {
    Pending,
    Permanent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedMachineRegistration {
    pub machine_id: String,
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
    #[serde(default)]
    pub paired_at_ms: Option<u64>,
    #[serde(default)]
    pub approved: bool,
    /// When a relay last listed this machine; 0 until this kernel first notices it is gone.
    #[serde(default)]
    pub last_seen_ms: u64,
    /// The name the machine had when last seen, for the list of offline machines.
    #[serde(default)]
    pub display_name: Option<String>,
    /// The Cloud directory (`cloud_directory_key`) whose live pass first recorded this machine:
    /// only that directory's reads may forget it. None for a machine recorded through a private
    /// relay, a pairing or an approval: no directory owns it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
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

pub(super) fn normalized_terminal_type(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "web" | "web_terminal" | "web-terminal" => "web".to_string(),
        "ios" | "ios_terminal" | "ios-terminal" => "ios".to_string(),
        "android" | "android_terminal" | "android-terminal" => "android".to_string(),
        _ => "cli".to_string(),
    }
}

pub(super) fn default_daemon_config_path() -> PathBuf {
    default_config_dir().join("daemon").join("config.json")
}

pub(super) fn legacy_daemon_config_path() -> PathBuf {
    default_state_dir().join("daemon").join("config.json")
}

pub(super) fn load_persisted_relay_config() -> Option<PersistedDaemonConfig> {
    for path in [default_daemon_config_path(), legacy_daemon_config_path()] {
        let Ok(payload) = fs::read_to_string(path) else {
            continue;
        };
        if let Ok(mut config) = serde_json::from_str::<PersistedDaemonConfig>(&payload) {
            canonicalize_persisted_daemon_config(&mut config);
            return Some(config);
        }
    }
    None
}

pub(super) fn load_persisted_daemon_config() -> PersistedDaemonConfig {
    load_persisted_relay_config().unwrap_or_default()
}

/// Applies `update` to the freshly loaded config and saves it, as one load, change and save under
/// the config file lock, so writers in this kernel and in sibling kernels sharing `CHARIOX_HOME`
/// (a client revocation, a machine Remove, the presence refresh) never overwrite each other. An
/// unchanged config is not written again, except to create the file the first time.
pub(super) fn update_persisted_daemon_config<T>(
    operation: &'static str,
    update: impl FnOnce(&mut PersistedDaemonConfig) -> Result<T, DaemonError>,
) -> Result<T, DaemonError> {
    update_persisted_daemon_config_then(operation, update, |_| {})
}

/// As `update_persisted_daemon_config`, and `saved` runs once the change is on disk, while the
/// config is still held.
pub(super) fn update_persisted_daemon_config_then<T>(
    operation: &'static str,
    update: impl FnOnce(&mut PersistedDaemonConfig) -> Result<T, DaemonError>,
    saved: impl FnOnce(&T),
) -> Result<T, DaemonError> {
    let _writing = DaemonConfigLock::acquire(&default_daemon_config_path(), operation)?;
    let mut persisted = load_persisted_daemon_config();
    let before = persisted.clone();
    let result = update(&mut persisted)?;
    if persisted != before || !default_daemon_config_path().exists() {
        save_persisted_daemon_config(&persisted, operation)?;
    }
    saved(&result);
    Ok(result)
}

fn save_persisted_daemon_config(
    persisted: &PersistedDaemonConfig,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let path = default_daemon_config_path();
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
    write_private_file(&path, payload.as_bytes()).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    })
}

pub(crate) fn persist_managed_cloud_relay_profile(
    profile: PersistedCloudRelayProfile,
) -> Result<(), DaemonError> {
    update_persisted_daemon_config("persist managed cloud relay profile", |persisted| {
        persisted.relay_url = None;
        persisted.relay_token = None;
        persisted.cloud_relay = Some(profile.canonicalized());
        Ok(())
    })
}

pub(crate) fn load_managed_cloud_relay_profile() -> Option<PersistedCloudRelayProfile> {
    load_persisted_relay_config()?.cloud_relay
}

fn canonicalize_persisted_daemon_config(config: &mut PersistedDaemonConfig) {
    let Some(profile) = config.cloud_relay.take() else {
        return;
    };
    config.cloud_relay = Some(profile.canonicalized());
}

fn normalize_url_without_trailing_slash(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

pub(super) fn upsert_machine_registration<'a>(
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
        public_key_thumbprint: None,
        paired_at_ms: None,
        approved: false,
        last_seen_ms: 0,
        display_name: None,
        directory: None,
    });
    entries
        .last_mut()
        .expect("entry was just inserted into machine registry")
}

pub(super) fn upsert_client_pairing<'a>(
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
