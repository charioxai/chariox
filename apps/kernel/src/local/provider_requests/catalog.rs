use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{
    claude_provider_catalog, default_provider_command_catalogs, ensure_codex_catalog_endpoint,
    lease_codex_catalog_endpoint, lease_opencode_catalog_endpoint, resolve_claude_executable,
    CodexClient, OpenCodeClient, OpenCodeProviderCatalog, OpenCodeProviderInfo, ProviderAuthStatus,
};
use arroba_relay::protocol::RelayMachinePresence;
use std::collections::BTreeMap;
use std::process::Command;
use std::thread;
use std::time::Duration;

use super::super::api::{
    GetProviderAuthStatusRequest, LocalDaemonResponse, LogoutProviderRequest,
    StartProviderLoginRequest,
};
use super::blocking::block_on_relay_query;

pub(crate) const PROVIDER_CATALOG_CACHE_TTL: Duration = Duration::from_secs(5);

pub(crate) fn provider_command_catalogs_response() -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::ProviderCommandCatalogs {
        catalogs: default_provider_command_catalogs(),
    })
}

pub(crate) fn load_provider_catalog(
    config: DaemonConfig,
) -> Result<OpenCodeProviderCatalog, DaemonError> {
    if config.provider_catalog_read_delay_ms > 0 {
        thread::sleep(Duration::from_millis(config.provider_catalog_read_delay_ms));
    }

    let mut catalogs = vec![claude_provider_catalog()];
    if crate::provider::dev_stub_public_inventory_enabled() {
        catalogs.push(dev_stub_provider_catalog());
    }
    let mut source_errors = Vec::new();

    match lease_opencode_catalog_endpoint() {
        Ok(endpoint) => match OpenCodeClient::new("catalog", endpoint.as_str()) {
            Ok(client) => match client.provider_catalog() {
                Ok(catalog) => catalogs.push(opencode_backend_catalog(catalog)),
                Err(error) => source_errors.push(format!("opencode catalog request: {error}")),
            },
            Err(error) => source_errors.push(format!("opencode client: {error}")),
        },
        Err(error) => source_errors.push(format!("opencode endpoint: {error}")),
    }
    match lease_codex_catalog_endpoint() {
        Ok(endpoint) => match CodexClient::new("catalog", endpoint.as_str()) {
            Ok(client) => match client.provider_catalog() {
                Ok(catalog) => catalogs.push(catalog),
                Err(error) => source_errors.push(format!("codex catalog request: {error}")),
            },
            Err(error) => source_errors.push(format!("codex client: {error}")),
        },
        Err(error) => source_errors.push(format!("codex endpoint: {error}")),
    }

    let remote_machines = if config.relay_url.is_some() && config.relay_token.is_some() {
        match block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        )) {
            Ok(machines) => machines,
            Err(error) => {
                source_errors.push(format!("relay live machines: {error}"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let approved_remote_machines =
        approved_live_remote_machines(&remote_machines, &config.host_machine_id);
    if !source_errors.is_empty() {
        crate::logging::warn_with_fields(
            "daemon.local",
            "Some provider catalog sources were unavailable",
            serde_json::json!({
                "source_errors": &source_errors,
            }),
        );
    }

    let mut catalog = merge_provider_catalogs(catalogs)
        .or_else(|| {
            remote_only_provider_catalog(&approved_remote_machines, &config.host_machine_id)
        })
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "get_provider_catalog",
            message: if source_errors.is_empty() {
                "no provider catalog sources were reachable".to_string()
            } else {
                format!(
                    "no provider catalog sources were reachable: {}",
                    source_errors.join("; ")
                )
            },
        })?;
    annotate_remote_machine_providers(
        &mut catalog,
        &approved_remote_machines,
        &config.host_machine_id,
    );
    crate::logging::info_with_fields(
        "daemon.local",
        "Retrieved merged provider catalog",
        serde_json::json!({
            "provider_count": catalog.all.len(),
            "model_count": catalog.all.iter().map(|provider| provider.models.len()).sum::<usize>(),
            "remote_provider_count": catalog.all.iter().filter(|provider| !provider.remote_machine_aliases.is_empty()).count(),
            "connected": &catalog.connected,
        }),
    );
    Ok(catalog)
}

pub(crate) fn provider_auth_status_response(
    request: GetProviderAuthStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match crate::provider::canonical_provider_family(&request.provider) {
        Some("codex") => {
            let endpoint = lease_codex_catalog_endpoint()?;
            let client = CodexClient::new("provider-auth", endpoint.as_str())?;
            Ok(LocalDaemonResponse::ProviderAuthStatus {
                status: client.auth_status()?,
            })
        }
        Some("claude") => Ok(LocalDaemonResponse::ProviderAuthStatus {
            status: claude_auth_status(&request.provider)?,
        }),
        _ => Err(DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: format!(
                "provider `{}` does not expose an auth status API",
                request.provider
            ),
        }),
    }
}

fn claude_auth_status(provider: &str) -> Result<ProviderAuthStatus, DaemonError> {
    let executable = resolve_claude_executable()?;
    let output = Command::new(&executable)
        .args(["auth", "status", "--json"])
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: format!("failed to run Claude auth status: {error}"),
        })?;
    if !output.status.success() {
        return Ok(ProviderAuthStatus {
            provider: provider.to_string(),
            auth_state: "not_logged_in".to_string(),
            account_profile: None,
            login_hint: Some("Run `claude auth login` to authenticate Claude Code.".to_string()),
            detected_version: claude_version().ok(),
        });
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: format!("Claude auth status returned invalid JSON: {error}"),
        })?;
    Ok(claude_auth_status_from_value(
        provider,
        &value,
        claude_version().ok(),
    ))
}

fn claude_version() -> Result<String, DaemonError> {
    let executable = resolve_claude_executable()?;
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: format!("failed to read Claude version: {error}"),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: "Claude version command failed".to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn claude_auth_status_from_value(
    provider: &str,
    value: &serde_json::Value,
    detected_version: Option<String>,
) -> ProviderAuthStatus {
    let logged_in = value
        .get("loggedIn")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let account_profile = if logged_in {
        let mut parts = Vec::new();
        for key in ["email", "orgName", "subscriptionType", "authMethod"] {
            if let Some(text) = value.get(key).and_then(serde_json::Value::as_str) {
                if !text.is_empty() {
                    parts.push(text.to_string());
                }
            }
        }
        (!parts.is_empty()).then(|| parts.join(" / "))
    } else {
        None
    };
    ProviderAuthStatus {
        provider: provider.to_string(),
        auth_state: if logged_in {
            "authenticated".to_string()
        } else {
            "not_logged_in".to_string()
        },
        account_profile,
        login_hint: Some("Run `claude auth login` to authenticate Claude Code.".to_string()),
        detected_version,
    }
}

pub(crate) fn start_provider_login_response(
    request: StartProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request.provider.as_str() {
        "codex" => {
            let endpoint = ensure_codex_catalog_endpoint()?;
            let client = CodexClient::new("provider-login", endpoint)?;
            Ok(LocalDaemonResponse::ProviderLoginStarted {
                login: client.start_login()?,
            })
        }
        provider => Err(DaemonError::LocalTransport {
            operation: "start_provider_login",
            message: format!("provider `{provider}` does not expose a login API"),
        }),
    }
}

pub(crate) fn logout_provider_response(
    request: LogoutProviderRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request.provider.as_str() {
        "codex" => {
            crate::provider::logout_codex()?;
            Ok(LocalDaemonResponse::ProviderLoggedOut {
                provider: "codex".to_string(),
            })
        }
        provider => Err(DaemonError::LocalTransport {
            operation: "logout_provider",
            message: format!("provider `{provider}` does not expose a logout API"),
        }),
    }
}

fn merge_provider_catalogs(
    catalogs: Vec<OpenCodeProviderCatalog>,
) -> Option<OpenCodeProviderCatalog> {
    let mut iter = catalogs.into_iter();
    let mut merged = iter.next()?;
    for catalog in iter {
        merged.connected.extend(catalog.connected);
        merged.connected.sort();
        merged.connected.dedup();
        for (provider_id, model_id) in catalog.default {
            merged.default.insert(provider_id, model_id);
        }
        for provider in catalog.all {
            if let Some(existing) = merged.all.iter_mut().find(|item| item.id == provider.id) {
                for (model_id, model) in provider.models {
                    existing.models.insert(model_id, model);
                }
            } else {
                merged.all.push(provider);
            }
        }
    }
    merged
        .all
        .sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Some(merged)
}

fn dev_stub_provider_catalog() -> OpenCodeProviderCatalog {
    OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "dev-stub".to_string(),
            name: "Dev Stub".to_string(),
            remote_machine_aliases: Vec::new(),
            models: Default::default(),
        }],
        default: Default::default(),
        connected: vec!["dev-stub".to_string()],
    }
}

fn opencode_backend_catalog(catalog: OpenCodeProviderCatalog) -> OpenCodeProviderCatalog {
    let mut models = BTreeMap::new();
    let mut first_model = None;
    let mut connected = false;

    for provider in catalog
        .all
        .into_iter()
        .filter(|provider| provider.id == "opencode")
    {
        connected = connected || catalog.connected.iter().any(|id| id == &provider.id);
        for (model_id, model) in provider.models {
            if first_model.is_none() {
                first_model = Some(model_id.clone());
            }
            models.insert(model_id, model);
        }
    }

    if models.is_empty() {
        return OpenCodeProviderCatalog {
            all: Vec::new(),
            default: Default::default(),
            connected: Vec::new(),
        };
    }

    let default_model = catalog
        .default
        .get("opencode")
        .filter(|model_id| models.contains_key(*model_id))
        .cloned()
        .or(first_model);

    let default = default_model
        .map(|model_id| BTreeMap::from([("opencode".to_string(), model_id)]))
        .unwrap_or_default();

    OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "opencode".to_string(),
            name: "OpenCode".to_string(),
            remote_machine_aliases: Vec::new(),
            models,
        }],
        default,
        connected: if connected {
            vec!["opencode".to_string()]
        } else {
            Vec::new()
        },
    }
}

fn remote_only_provider_catalog(
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) -> Option<OpenCodeProviderCatalog> {
    let mut provider_ids = live_machines
        .iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .flat_map(|machine| machine.available_providers.iter().cloned())
        .collect::<Vec<_>>();
    crate::provider::retain_public_inventory_providers(&mut provider_ids);
    provider_ids.sort();
    provider_ids.dedup();
    if provider_ids.is_empty() {
        return None;
    }

    let all = provider_ids
        .into_iter()
        .map(|provider_id| OpenCodeProviderInfo {
            name: display_name_for_provider(&provider_id),
            id: provider_id,
            remote_machine_aliases: Vec::new(),
            models: Default::default(),
        })
        .collect::<Vec<_>>();

    Some(OpenCodeProviderCatalog {
        connected: all.iter().map(|provider| provider.id.clone()).collect(),
        all,
        default: Default::default(),
    })
}

fn display_name_for_provider(provider_id: &str) -> String {
    match provider_id {
        "codex" => "Codex".to_string(),
        "opencode" => "OpenCode".to_string(),
        other => other.to_string(),
    }
}

fn annotate_remote_machine_providers(
    catalog: &mut OpenCodeProviderCatalog,
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) {
    for provider in &mut catalog.all {
        provider.remote_machine_aliases =
            remote_machine_aliases_for_provider(&provider.id, live_machines, local_machine_id);
    }
}

fn remote_machine_aliases_for_provider(
    provider_id: &str,
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) -> Vec<String> {
    let mut aliases = live_machines
        .iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .filter(|machine| {
            machine
                .available_providers
                .iter()
                .any(|provider| provider == provider_id)
        })
        .map(|machine| {
            machine
                .machine_alias
                .as_deref()
                .map(str::trim)
                .filter(|alias| !alias.is_empty())
                .unwrap_or(machine.machine_id.as_str())
                .to_string()
        })
        .collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

fn approved_live_remote_machines(
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) -> Vec<RelayMachinePresence> {
    let registry = DaemonConfig::machine_registry_entries();
    live_machines
        .iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .filter_map(|machine| {
            registry
                .iter()
                .find(|entry| {
                    entry.machine_id == machine.machine_id && entry.approved && !entry.forgotten
                })
                .map(|entry| {
                    let mut machine = machine.clone();
                    if entry.alias.is_some() {
                        machine.machine_alias = entry.alias.clone();
                    }
                    machine
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::OpenCodeProviderModel;
    use serde_json::json;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn drill_provider_catalog_exposes_dev_stub_without_restricting_fixture_models() {
        let catalog = dev_stub_provider_catalog();

        assert_eq!(catalog.connected, vec!["dev-stub"]);
        assert_eq!(catalog.all.len(), 1);
        assert_eq!(catalog.all[0].id, "dev-stub");
        assert!(catalog.all[0].models.is_empty());
    }

    #[test]
    fn provider_auth_status_accepts_claude_provider_modes() {
        let _guard = crate::env_lock::lock();
        let path =
            std::env::temp_dir().join(format!("arroba-claude-auth-status-{}", std::process::id()));
        fs::write(
            &path,
            r#"#!/bin/sh
set -eu
if [ "$#" -ge 3 ] && [ "$1" = "auth" ] && [ "$2" = "status" ] && [ "$3" = "--json" ]; then
  printf '%s\n' '{"loggedIn":true,"authMethod":"claude.ai","email":"dev@example.test","orgName":"Example Org","subscriptionType":"pro"}'
  exit 0
fi
if [ "$#" -ge 1 ] && [ "$1" = "--version" ]; then
  printf '%s\n' 'claude 1.2.3'
  exit 0
fi
exit 2
"#,
        )
        .expect("fixture should exist");
        let mut permissions = fs::metadata(&path)
            .expect("fixture metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("fixture should be executable");
        std::env::set_var("ARROBA_CLAUDE_BIN", &path);

        let response = provider_auth_status_response(GetProviderAuthStatusRequest {
            provider: "claude-headless".to_string(),
        })
        .expect("claude mode auth status should resolve");

        std::env::remove_var("ARROBA_CLAUDE_BIN");
        let _ = fs::remove_file(&path);

        match response {
            LocalDaemonResponse::ProviderAuthStatus { status } => {
                assert_eq!(status.provider, "claude-headless");
                assert_eq!(status.auth_state, "authenticated");
                assert_eq!(status.detected_version.as_deref(), Some("claude 1.2.3"));
                assert!(status
                    .account_profile
                    .as_deref()
                    .unwrap_or_default()
                    .contains("dev@example.test"));
            }
            response => panic!("unexpected response: {response:?}"),
        }
    }

    #[test]
    fn claude_auth_status_parser_reports_not_logged_in() {
        let status = claude_auth_status_from_value(
            "claude-p",
            &json!({ "loggedIn": false }),
            Some("claude 1.2.3".to_string()),
        );

        assert_eq!(status.provider, "claude-p");
        assert_eq!(status.auth_state, "not_logged_in");
        assert_eq!(status.account_profile, None);
        assert_eq!(status.detected_version.as_deref(), Some("claude 1.2.3"));
    }

    #[test]
    fn annotates_remote_machine_provider_aliases_without_including_local_machine() {
        let mut catalog = OpenCodeProviderCatalog {
            all: vec![
                OpenCodeProviderInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: Default::default(),
                },
                OpenCodeProviderInfo {
                    id: "opencode".to_string(),
                    name: "OpenCode".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: Default::default(),
                },
            ],
            default: Default::default(),
            connected: vec!["codex".to_string(), "opencode".to_string()],
        };
        let live_machines = vec![
            RelayMachinePresence {
                machine_id: "machine-local".to_string(),
                machine_alias: Some("home".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string()],
                provider_accounts: Vec::new(),
            },
            RelayMachinePresence {
                machine_id: "machine-remote-a".to_string(),
                machine_alias: Some("builder-west".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string(), "opencode".to_string()],
                provider_accounts: Vec::new(),
            },
            RelayMachinePresence {
                machine_id: "machine-remote-b".to_string(),
                machine_alias: None,
                kernel_count: 1,
                available_providers: vec!["codex".to_string()],
                provider_accounts: Vec::new(),
            },
        ];

        annotate_remote_machine_providers(&mut catalog, &live_machines, "machine-local");

        let codex = catalog
            .all
            .iter()
            .find(|provider| provider.id == "codex")
            .unwrap();
        let opencode = catalog
            .all
            .iter()
            .find(|provider| provider.id == "opencode")
            .unwrap();

        assert_eq!(
            codex.remote_machine_aliases,
            vec!["builder-west".to_string(), "machine-remote-b".to_string()]
        );
        assert_eq!(
            opencode.remote_machine_aliases,
            vec!["builder-west".to_string()]
        );
    }

    #[test]
    fn builds_remote_only_catalog_when_local_provider_sources_are_unavailable() {
        let live_machines = vec![
            RelayMachinePresence {
                machine_id: "machine-local".to_string(),
                machine_alias: Some("home".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string()],
                provider_accounts: Vec::new(),
            },
            RelayMachinePresence {
                machine_id: "machine-remote".to_string(),
                machine_alias: Some("builder".to_string()),
                kernel_count: 1,
                available_providers: vec![
                    "codex".to_string(),
                    "opencode".to_string(),
                    "dev-stub".to_string(),
                ],
                provider_accounts: Vec::new(),
            },
        ];

        let mut catalog = remote_only_provider_catalog(&live_machines, "machine-local")
            .expect("remote providers should create a catalog");
        annotate_remote_machine_providers(&mut catalog, &live_machines, "machine-local");

        assert_eq!(
            catalog.connected,
            vec!["codex".to_string(), "opencode".to_string()]
        );
        assert!(!catalog.all.iter().any(|provider| provider.id == "dev-stub"));
        assert_eq!(catalog.default.len(), 0);
        let codex = catalog
            .all
            .iter()
            .find(|provider| provider.id == "codex")
            .unwrap();
        let opencode = catalog
            .all
            .iter()
            .find(|provider| provider.id == "opencode")
            .unwrap();
        assert_eq!(codex.name, "Codex");
        assert_eq!(codex.remote_machine_aliases, vec!["builder".to_string()]);
        assert_eq!(opencode.name, "OpenCode");
        assert_eq!(opencode.remote_machine_aliases, vec!["builder".to_string()]);
    }

    #[test]
    fn opencode_backend_catalog_hides_upstream_provider_ids() {
        let catalog = opencode_backend_catalog(OpenCodeProviderCatalog {
            all: vec![
                OpenCodeProviderInfo {
                    id: "openai".to_string(),
                    name: "OpenAI".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: BTreeMap::from([(
                        "gpt-5.2".to_string(),
                        OpenCodeProviderModel {
                            id: "gpt-5.2".to_string(),
                            name: "GPT-5.2".to_string(),
                            status: "active".to_string(),
                            limit: None,
                            variants: Default::default(),
                        },
                    )]),
                },
                OpenCodeProviderInfo {
                    id: "opencode".to_string(),
                    name: "OpenCode Zen".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: BTreeMap::from([(
                        "gpt-5.2".to_string(),
                        OpenCodeProviderModel {
                            id: "gpt-5.2".to_string(),
                            name: "GPT-5.2".to_string(),
                            status: "active".to_string(),
                            limit: None,
                            variants: BTreeMap::from([("low".to_string(), json!({}))]),
                        },
                    )]),
                },
            ],
            default: BTreeMap::from([
                ("openai".to_string(), "gpt-5.2".to_string()),
                ("opencode".to_string(), "gpt-5.2".to_string()),
            ]),
            connected: vec!["openai".to_string(), "opencode".to_string()],
        });

        assert_eq!(catalog.connected, vec!["opencode".to_string()]);
        assert_eq!(
            catalog.default.get("opencode"),
            Some(&"gpt-5.2".to_string())
        );
        assert_eq!(catalog.all.len(), 1);
        assert_eq!(catalog.all[0].id, "opencode");
        assert!(catalog.all[0].models.contains_key("gpt-5.2"));
    }
}
