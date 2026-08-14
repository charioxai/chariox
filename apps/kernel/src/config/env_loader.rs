use std::collections::BTreeMap;
use std::env;
use std::fs;
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
            env::var("CHARIOX_KERNEL_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let kernel_websocket_port = env::var("CHARIOX_KERNEL_PORT")
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
        let env_relay_url = env::var("CHARIOX_RELAY_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let env_relay_token = env::var("CHARIOX_RELAY_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let env_relay_configured = env_relay_url.is_some() || env_relay_token.is_some();
        let daemon_id = env::var("CHARIOX_DAEMON_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| runtime_identity.daemon_id.clone());
        let event_delivery_environment_id = env::var("CHARIOX_EVENT_ENVIRONMENT_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| daemon_id.clone());
        let default_machine_alias = runtime_display_machine_name();
        let host_machine_alias = env::var("CHARIOX_MACHINE_ALIAS")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or(runtime_identity.machine_alias)
            .or(default_machine_alias.clone());
        let daemon_alias = env::var("CHARIOX_DAEMON_ALIAS")
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
        let accept_remote_leases = env::var("CHARIOX_ACCEPT_REMOTE_LEASES")
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .or(user_config.relay.accept_remote_leases)
            .unwrap_or(true);
        Self {
            user_config_path,
            user_config,
            local_socket_path: env::var_os("CHARIOX_DAEMON_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| Self::default_local_socket_path(&daemon_id)),
            kernel_websocket_host,
            kernel_websocket_port,
            kernel_websocket_queue_capacity: env::var("CHARIOX_KERNEL_QUEUE_CAPACITY")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(128),
            kernel_websocket_write_delay_ms: env::var("CHARIOX_KERNEL_WRITE_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(super::DEFAULT_KERNEL_WEBSOCKET_WRITE_DELAY_MS),
            runtime_mcp_host: env::var("CHARIOX_MCP_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            runtime_mcp_port: env::var("CHARIOX_MCP_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(43120),
            session_history_root: env::var_os("CHARIOX_SESSION_HISTORY_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(Self::default_session_history_root),
            session_history_read_delay_ms: env::var("CHARIOX_SESSION_HISTORY_READ_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            operational_history_read_delay_ms: env::var(
                "CHARIOX_OPERATIONAL_HISTORY_READ_DELAY_MS",
            )
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0),
            provider_catalog_read_delay_ms: env::var("CHARIOX_PROVIDER_CATALOG_READ_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            provider_process_list_delay_ms: env::var("CHARIOX_PROVIDER_PROCESS_LIST_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            provider_process_idle_ttl_ms: env::var("CHARIOX_PROVIDER_PROCESS_IDLE_TTL_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(300_000),
            provider_process_orphan_ttl_ms: env::var("CHARIOX_PROVIDER_PROCESS_ORPHAN_TTL_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30_000),
            provider_runtime_init_delay_ms: env::var("CHARIOX_PROVIDER_RUNTIME_INIT_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
            daemon_id,
            host_machine_id: env::var("CHARIOX_MACHINE_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| runtime_identity.machine_id.clone()),
            host_machine_alias,
            os_name: env::var("CHARIOX_OS_NAME")
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
            relay_heartbeat_ms: env::var("CHARIOX_RELAY_HEARTBEAT_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(DEFAULT_RELAY_HEARTBEAT_MS),
            event_delivery_url: env::var("CHARIOX_AEDS_URL")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            event_delivery_token: env::var("CHARIOX_AEDS_TOKEN")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            event_delivery_environment_id,
            event_registry_url: env::var("CHARIOX_EVENT_REGISTRY_URL")
                .ok()
                .map(|value| value.trim().trim_end_matches('/').to_string())
                .filter(|value| !value.is_empty()),
            event_generator_management_targets: load_event_generator_management_targets(),
            relay_request_timeout_ms: env::var("CHARIOX_RELAY_REQUEST_TIMEOUT_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(60_000),
            accept_remote_leases,
            os_user: env::var("USER")
                .or_else(|_| env::var("USERNAME"))
                .unwrap_or_else(|_| "unknown".to_string()),
        }
    }
}

#[derive(serde::Deserialize)]
struct EventGeneratorManagementTargetInput {
    url: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    token_file: Option<PathBuf>,
}

fn load_event_generator_management_targets(
) -> BTreeMap<String, super::EventGeneratorManagementTarget> {
    let encoded = env::var("CHARIOX_AEGS_MANAGEMENT_TARGETS_JSON")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var_os("CHARIOX_AEGS_MANAGEMENT_TARGETS_FILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| {
                    fs::read_to_string(&path).unwrap_or_else(|error| {
                        panic!(
                            "failed to read CHARIOX_AEGS_MANAGEMENT_TARGETS_FILE {}: {error}",
                            path.display()
                        )
                    })
                })
        });
    let Some(encoded) = encoded else {
        return BTreeMap::new();
    };
    parse_event_generator_management_targets(&encoded).unwrap_or_else(|error| {
        panic!("invalid AEGS management targets: {error}");
    })
}

fn parse_event_generator_management_targets(
    encoded: &str,
) -> Result<BTreeMap<String, super::EventGeneratorManagementTarget>, String> {
    let values: BTreeMap<String, EventGeneratorManagementTargetInput> =
        serde_json::from_str(encoded).map_err(|error| format!("invalid JSON: {error}"))?;
    let mut targets = BTreeMap::new();
    for (raw_generator_id, value) in values {
        let generator_id =
            crate::event_connection::canonical_event_generator_id(raw_generator_id.trim());
        if generator_id.is_empty() {
            return Err("generator ID must not be empty".to_string());
        }
        let url = value.url.trim().trim_end_matches('/').to_string();
        let loopback_http = url
            .strip_prefix("http://")
            .and_then(|authority| authority.split('/').next())
            .and_then(|authority| authority.rsplit_once(':'))
            .is_some_and(|(host, port)| {
                matches!(host, "127.0.0.1" | "localhost") && port.parse::<u16>().is_ok()
            });
        let remote_https = url
            .strip_prefix("https://")
            .and_then(|authority| authority.split('/').next())
            .is_some_and(|authority| !authority.is_empty());
        if !loopback_http && !remote_https {
            return Err(format!(
                "target {generator_id} must use HTTPS or loopback HTTP"
            ));
        }
        let token = value
            .token
            .filter(|token| !token.trim().is_empty())
            .map(Ok)
            .or_else(|| {
                value.token_file.map(|path| {
                    fs::read_to_string(&path).map_err(|error| {
                        format!(
                            "failed to read token file {} for {generator_id}: {error}",
                            path.display()
                        )
                    })
                })
            })
            .transpose()?
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| format!("target {generator_id} requires a token"))?;
        if targets
            .insert(
                generator_id.clone(),
                super::EventGeneratorManagementTarget { url, token },
            )
            .is_some()
        {
            return Err(format!("duplicate target {generator_id}"));
        }
    }
    Ok(targets)
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
    use super::{
        default_kernel_alias, normalize_hostname, parse_event_generator_management_targets,
    };

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

    #[test]
    fn event_generator_management_targets_allow_https_and_loopback_http() {
        let targets = parse_event_generator_management_targets(
            r#"{
                "dev.chariox.dummy": {
                    "url": "http://127.0.0.1:43132/",
                    "token": " local-token "
                },
                "dev.chariox.github": {
                    "url": "https://events.example.test/github",
                    "token": "remote-token"
                }
            }"#,
        )
        .unwrap();
        assert_eq!(targets["dev.chariox.dummy"].url, "http://127.0.0.1:43132");
        assert_eq!(targets["dev.chariox.dummy"].token, "local-token");
        assert_eq!(
            targets["dev.chariox.github"].url,
            "https://events.example.test/github"
        );
    }

    #[test]
    fn event_generator_management_targets_migrate_removed_publisher_namespace() {
        let targets = parse_event_generator_management_targets(
            r#"{
                "dev.arroba.github": {
                    "url": "https://events.example.test/github",
                    "token": "remote-token"
                }
            }"#,
        )
        .expect("renamed target should migrate");

        assert!(targets.contains_key("dev.chariox.github"));
        assert!(!targets.contains_key("dev.arroba.github"));
    }

    #[test]
    fn event_generator_management_targets_reject_remote_plaintext() {
        let error = parse_event_generator_management_targets(
            r#"{
                "dev.chariox.github": {
                    "url": "http://events.example.test:43132",
                    "token": "token"
                }
            }"#,
        )
        .unwrap_err();
        assert!(error.contains("HTTPS or loopback HTTP"));
    }

    #[test]
    fn event_generator_management_targets_require_credentials() {
        let error = parse_event_generator_management_targets(
            r#"{
                "dev.chariox.github": {
                    "url": "https://events.example.test/github"
                }
            }"#,
        )
        .unwrap_err();
        assert!(error.contains("requires a token"));
    }
}

fn load_env_cloud_relay_profile() -> Option<PersistedCloudRelayProfile> {
    let payload = env::var("CHARIOX_CLOUD_RELAY_CONFIG_JSON").ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&payload).ok()?;
    let profile_value = value.get("cloud_relay").cloned().unwrap_or(value);
    serde_json::from_value::<PersistedCloudRelayProfile>(profile_value)
        .ok()
        .map(PersistedCloudRelayProfile::canonicalized)
}
