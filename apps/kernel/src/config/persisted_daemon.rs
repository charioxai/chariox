use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::paths::{default_config_dir, default_state_dir};
use crate::error::DaemonError;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PersistedDaemonConfig {
    #[serde(default)]
    pub(super) relay_url: Option<String>,
    #[serde(default)]
    pub(super) relay_token: Option<String>,
    #[serde(default)]
    pub(super) cloud_relay: Option<PersistedCloudRelayProfile>,
    #[serde(default)]
    pub(super) machines: Vec<PersistedMachineRegistration>,
    #[serde(default)]
    pub(super) clients: Vec<PersistedClientPairing>,
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
        if let Ok(config) = serde_json::from_str::<PersistedDaemonConfig>(&payload) {
            return Some(config);
        }
    }
    None
}

pub(super) fn load_persisted_daemon_config() -> PersistedDaemonConfig {
    load_persisted_relay_config().unwrap_or_default()
}

pub(super) fn persist_daemon_config(
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
    fs::write(path, payload).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    })
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
