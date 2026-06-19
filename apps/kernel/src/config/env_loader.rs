use std::env;
use std::path::PathBuf;
use std::process::Command;

use super::identity::{load_or_create_runtime_identity, persist_runtime_display_aliases};
use super::{
    default_os_name, load_user_config_from_path,
    persisted_daemon::{
        load_cli_cloud_relay_profile, load_persisted_relay_config, PersistedCloudRelayProfile,
    },
    DaemonConfig, DEFAULT_RELAY_HEARTBEAT_MS,
};

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
        let persisted_cloud_relay = persisted_config
            .as_ref()
            .and_then(|config| config.cloud_relay.clone());
        let env_cloud_relay = load_env_cloud_relay_profile();
        let cli_cloud_relay = if persisted_cloud_relay.is_none() {
            load_cli_cloud_relay_profile()
        } else {
            None
        };
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
        let default_machine_alias = runtime_display_machine_name();
        let host_machine_alias = env::var("ARROBA_MACHINE_ALIAS")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or(runtime_identity.machine_alias)
            .or(default_machine_alias.clone());
        let daemon_alias = env::var("ARROBA_DAEMON_ALIAS")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or(runtime_identity.daemon_alias)
            .or_else(|| default_kernel_alias(host_machine_alias.as_deref(), kernel_websocket_port));
        persist_runtime_display_aliases(
            &kernel_websocket_host,
            kernel_websocket_port,
            host_machine_alias.as_deref(),
            daemon_alias.as_deref(),
        );
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
                .unwrap_or(super::DEFAULT_KERNEL_WEBSOCKET_WRITE_DELAY_MS),
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
            operational_history_read_delay_ms: env::var("ARROBA_OPERATIONAL_HISTORY_READ_DELAY_MS")
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
            host_machine_alias,
            os_name: env::var("ARROBA_OS_NAME")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(default_os_name),
            daemon_alias,
            relay_url: env_relay_url
                .or_else(|| persisted_config.clone().and_then(|config| config.relay_url)),
            relay_token: env_relay_token.or_else(|| {
                persisted_config
                    .clone()
                    .and_then(|config| config.relay_token)
            }),
            cloud_relay: if env_relay_configured {
                env_cloud_relay
            } else {
                env_cloud_relay
                    .or(persisted_cloud_relay)
                    .or(cli_cloud_relay)
            },
            relay_public_key: runtime_identity.relay_public_key,
            relay_private_key: runtime_identity.relay_private_key,
            relay_heartbeat_ms: env::var("ARROBA_RELAY_HEARTBEAT_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(DEFAULT_RELAY_HEARTBEAT_MS),
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
}

pub(super) fn runtime_display_machine_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    if let Some(name) = command_output("scutil", &["--get", "LocalHostName"]) {
        return Some(name);
    }
    command_output("hostname", &[]).map(normalize_hostname)
}

fn command_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_hostname(value: String) -> String {
    value
        .strip_suffix(".local")
        .unwrap_or(value.as_str())
        .trim()
        .to_string()
}

fn default_kernel_alias(machine_alias: Option<&str>, port: u16) -> Option<String> {
    let alias = machine_alias?.trim();
    if alias.is_empty() {
        None
    } else {
        Some(format!("{alias}.{port}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{default_kernel_alias, normalize_hostname};

    #[test]
    fn normalize_hostname_removes_bonjour_suffix() {
        assert_eq!(
            normalize_hostname("Miguels-MacBook-Pro.local".to_string()),
            "Miguels-MacBook-Pro"
        );
        assert_eq!(
            normalize_hostname("linux-worker".to_string()),
            "linux-worker"
        );
    }

    #[test]
    fn default_kernel_alias_uses_machine_alias_and_port() {
        assert_eq!(
            default_kernel_alias(Some("Miguels-MacBook-Pro"), 43118).as_deref(),
            Some("Miguels-MacBook-Pro.43118"),
        );
        assert_eq!(default_kernel_alias(Some(" "), 43118), None);
        assert_eq!(default_kernel_alias(None, 43118), None);
    }
}

fn load_env_cloud_relay_profile() -> Option<PersistedCloudRelayProfile> {
    let payload = env::var("ARROBA_CLOUD_RELAY_CONFIG_JSON").ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&payload).ok()?;
    let profile_value = value.get("cloud_relay").cloned().unwrap_or(value);
    serde_json::from_value::<PersistedCloudRelayProfile>(profile_value)
        .ok()
        .map(PersistedCloudRelayProfile::canonicalized)
}
