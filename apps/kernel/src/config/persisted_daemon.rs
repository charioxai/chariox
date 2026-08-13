use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::paths::{default_config_dir, default_state_dir};
use crate::error::DaemonError;

const HOSTED_STAGING_API_URL: &str = "https://chariox-cloud-staging.osc-fr1.scalingo.io";
pub(super) const HOSTED_STAGING_RELAY_URL: &str = "wss://195.201.123.115.sslip.io";

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

impl PersistedCloudRelayProfile {
    pub(super) fn canonicalized(mut self) -> Self {
        self.api_url = normalize_url_without_trailing_slash(&self.api_url);
        self.relay_url = canonicalize_hosted_cloud_relay_url(&self.api_url, &self.relay_url);
        self
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CliPreferences {
    #[serde(default)]
    relay: Option<CliRelayPreferences>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CliRelayPreferences {
    #[serde(default)]
    cloud: Option<CliCloudRelayProfile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliCloudRelayProfile {
    api_url: String,
    email: String,
    account_id: String,
    user_id: String,
    account_slug: String,
    realm_id: String,
    relay_url: String,
    issuer_id: String,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_alias: Option<String>,
    #[serde(default)]
    machine_id: Option<String>,
    #[serde(default)]
    machine_alias: Option<String>,
    #[serde(default)]
    machine_credential: Option<String>,
    #[serde(default)]
    cloud_session_token: Option<String>,
    #[serde(default)]
    cloud_session_expires_at_ms: Option<u64>,
    #[serde(default)]
    token_expires_at_ms: Option<u64>,
}

impl From<CliCloudRelayProfile> for PersistedCloudRelayProfile {
    fn from(profile: CliCloudRelayProfile) -> Self {
        PersistedCloudRelayProfile {
            api_url: profile.api_url,
            email: profile.email,
            account_id: profile.account_id,
            user_id: profile.user_id,
            account_slug: profile.account_slug,
            realm_id: profile.realm_id,
            relay_url: profile.relay_url,
            issuer_id: profile.issuer_id,
            client_id: profile.client_id,
            client_alias: profile.client_alias,
            machine_id: profile.machine_id,
            machine_alias: profile.machine_alias,
            machine_credential: profile.machine_credential,
            cloud_session_token: profile.cloud_session_token,
            cloud_session_expires_at_ms: profile.cloud_session_expires_at_ms,
            token_expires_at_ms: profile.token_expires_at_ms,
        }
        .canonicalized()
    }
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
        if let Ok(mut config) = serde_json::from_str::<PersistedDaemonConfig>(&payload) {
            canonicalize_persisted_daemon_config(&mut config);
            return Some(config);
        }
    }
    None
}

pub(super) fn load_cli_cloud_relay_profile() -> Option<PersistedCloudRelayProfile> {
    let payload = fs::read_to_string(cli_preferences_path()).ok()?;
    let preferences = serde_json::from_str::<CliPreferences>(&payload).ok()?;
    preferences
        .relay?
        .cloud
        .map(PersistedCloudRelayProfile::from)
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

fn cli_preferences_path() -> PathBuf {
    default_config_dir().join("config.json")
}

fn canonicalize_persisted_daemon_config(config: &mut PersistedDaemonConfig) {
    let Some(profile) = config.cloud_relay.take() else {
        return;
    };
    let profile = profile.canonicalized();
    if let Some(relay_url) = config.relay_url.clone() {
        config.relay_url = Some(canonicalize_hosted_cloud_relay_url(
            &profile.api_url,
            &relay_url,
        ));
    }
    config.cloud_relay = Some(profile);
}

fn normalize_url_without_trailing_slash(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn canonicalize_hosted_cloud_relay_url(api_url: &str, relay_url: &str) -> String {
    let normalized_api = normalize_url_without_trailing_slash(api_url);
    let relay_url = relay_url.trim();
    if normalized_api == HOSTED_STAGING_API_URL
        && matches!(
            relay_url,
            HOSTED_STAGING_RELAY_URL
                | "ws://195.201.123.115:43130"
                | "ws://195.201.123.115.sslip.io"
                | "wss://195.201.123.115.sslip.io/"
        )
    {
        return HOSTED_STAGING_RELAY_URL.to_string();
    }
    relay_url.to_string()
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
