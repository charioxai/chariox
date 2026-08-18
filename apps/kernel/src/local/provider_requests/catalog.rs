use crate::config::DaemonConfig;
use crate::error::DaemonError;
use crate::provider::{
    claude_provider_catalog, default_provider_command_catalogs, lease_codex_catalog_endpoint,
    lease_opencode_catalog_endpoint, resolve_claude_executable, resolve_opencode_executable,
    CodexClient, OpenCodeClient, OpenCodeProviderCatalog, OpenCodeProviderInfo, ProviderAuthStatus,
};
use chariox_relay::protocol::RelayMachinePresence;
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
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: GetProviderAuthStatusRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = registry.get(owner_user_id, &request.provider, &request.account_profile)?;
    let environment =
        registry.resolve_environment(owner_user_id, &request.provider, &profile.profile_id)?;
    match crate::provider::canonical_provider_family(&request.provider) {
        Some("codex") => {
            let endpoint = crate::provider::ensure_codex_account_endpoint(
                owner_user_id,
                &profile.profile_id,
                environment,
            )?;
            let client = CodexClient::new("provider-auth", &endpoint)?;
            let status = client.auth_status(&profile.profile_id)?;
            update_profile_auth_observation(registry, owner_user_id, &status)?;
            Ok(LocalDaemonResponse::ProviderAuthStatus { status })
        }
        Some("claude") => Ok(LocalDaemonResponse::ProviderAuthStatus {
            status: {
                let status =
                    claude_auth_status(&request.provider, &profile.profile_id, &environment)?;
                update_profile_auth_observation(registry, owner_user_id, &status)?;
                status
            },
        }),
        Some("opencode") => Ok(LocalDaemonResponse::ProviderAuthStatus {
            status: {
                let status = opencode_auth_status(&profile.profile_id, &environment)?;
                update_profile_auth_observation(registry, owner_user_id, &status)?;
                status
            },
        }),
        _ => Err(unsupported_auth_provider(
            "get_provider_auth_status",
            &request.provider,
        )),
    }
}

pub(crate) fn refresh_provider_account_profile_response(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    account_profile: &str,
) -> Result<crate::account_profile::ProviderAccountProfile, DaemonError> {
    let profile = registry.get(owner_user_id, provider, account_profile)?;
    let environment = registry.resolve_environment(owner_user_id, provider, &profile.profile_id)?;
    let (status, usage) = match crate::provider::canonical_provider_family(provider) {
        Some("codex") => {
            let endpoint = crate::provider::ensure_codex_account_endpoint(
                owner_user_id,
                &profile.profile_id,
                environment,
            )?;
            let client = CodexClient::new("provider-account-refresh", endpoint)?;
            (
                client.auth_status(&profile.profile_id)?,
                client.usage_snapshot(&profile.profile_id)?,
            )
        }
        Some("claude") => (
            claude_auth_status(provider, &profile.profile_id, &environment)?,
            profile.usage.clone(),
        ),
        Some("opencode") => (
            opencode_auth_status(&profile.profile_id, &environment)?,
            opencode_usage_snapshot(&profile.profile_id, &environment),
        ),
        _ => {
            return Err(unsupported_auth_provider(
                "refresh provider account",
                provider,
            ))
        }
    };
    registry.update_observation(
        owner_user_id,
        &status.provider,
        &status.account_profile,
        auth_state_from_status(&status.auth_state),
        status.identity_summary,
        status.plan,
        status.detected_version,
        Some(usage),
    )
}

fn claude_auth_status(
    provider: &str,
    account_profile: &str,
    environment: &BTreeMap<String, String>,
) -> Result<ProviderAuthStatus, DaemonError> {
    let executable = resolve_claude_executable()?;
    let output = Command::new(&executable)
        .args(["auth", "status", "--json"])
        .envs(environment)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .env_remove("ANTHROPIC_BASE_URL")
        .env_remove("ANTHROPIC_CUSTOM_HEADERS")
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: format!("failed to run Claude auth status: {error}"),
        })?;
    if !output.status.success() {
        return Ok(ProviderAuthStatus {
            provider: provider.to_string(),
            auth_state: "not_logged_in".to_string(),
            account_profile: account_profile.to_string(),
            identity_summary: None,
            plan: None,
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
        account_profile,
        &value,
        claude_version().ok(),
    ))
}

fn opencode_auth_status(
    account_profile: &str,
    environment: &BTreeMap<String, String>,
) -> Result<ProviderAuthStatus, DaemonError> {
    let executable = resolve_opencode_executable()?;
    let mut command = Command::new(&executable);
    command.args(["auth", "list"]).envs(environment);
    remove_account_auth_environment(&mut command, "opencode");
    let output = command
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get_provider_auth_status",
            message: format!("failed to run OpenCode auth list: {error}"),
        })?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let normalized = strip_ansi(&text);
    let has_credentials = output.status.success()
        && !normalized.trim().is_empty()
        && !normalized.to_ascii_lowercase().contains("0 credentials")
        && !normalized.to_ascii_lowercase().contains("no credentials");
    Ok(ProviderAuthStatus {
        provider: "opencode".to_string(),
        auth_state: if has_credentials {
            "authenticated"
        } else {
            "not_logged_in"
        }
        .to_string(),
        account_profile: account_profile.to_string(),
        identity_summary: has_credentials.then(|| "Provider credentials configured".to_string()),
        plan: None,
        login_hint: Some(
            "Use Provider Accounts to run `opencode auth login` for this account.".to_string(),
        ),
        detected_version: command_version(&executable).ok(),
    })
}

fn opencode_usage_snapshot(
    account_profile: &str,
    environment: &BTreeMap<String, String>,
) -> crate::account_profile::ProviderAccountUsageSnapshot {
    use crate::account_profile::{
        ProviderAccountUsageAvailability, ProviderAccountUsageMeter, ProviderAccountUsageMeterKind,
        ProviderAccountUsageMeterScope, ProviderAccountUsageMeterState,
        ProviderAccountUsageSnapshot,
    };
    let observed_at_ms = crate::session::unix_epoch_ms();
    let Ok(executable) = resolve_opencode_executable() else {
        return ProviderAccountUsageSnapshot::unavailable(account_profile, "opencode");
    };
    let mut command = Command::new(executable);
    command
        .args(["stats", "--format", "json"])
        .envs(environment);
    remove_account_auth_environment(&mut command, "opencode");
    let Ok(output) = command.output() else {
        return ProviderAccountUsageSnapshot::unavailable(account_profile, "opencode");
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return ProviderAccountUsageSnapshot::unavailable(account_profile, "opencode");
    };
    let tokens = find_numeric_field(&value, &["tokens", "totalTokens", "total_tokens"]);
    let cost = find_numeric_field(&value, &["cost", "totalCost", "total_cost"]);
    let mut meters = Vec::new();
    if let Some(used) = tokens {
        meters.push(ProviderAccountUsageMeter {
            meter_id: "local/tokens".to_string(),
            label: "Local token usage".to_string(),
            kind: ProviderAccountUsageMeterKind::TokenUsage,
            scope: ProviderAccountUsageMeterScope::Account,
            used_percent: None,
            used: Some(used),
            remaining: None,
            total: None,
            unit: Some("tokens".to_string()),
            window_duration_minutes: None,
            resets_at_ms: None,
            state: ProviderAccountUsageMeterState::Unknown,
            source: "opencode.local_stats".to_string(),
            observed_at_ms,
        });
    }
    if let Some(used) = cost {
        meters.push(ProviderAccountUsageMeter {
            meter_id: "local/cost".to_string(),
            label: "Local recorded cost".to_string(),
            kind: ProviderAccountUsageMeterKind::LocalCost,
            scope: ProviderAccountUsageMeterScope::Account,
            used_percent: None,
            used: Some(used),
            remaining: None,
            total: None,
            unit: Some("USD".to_string()),
            window_duration_minutes: None,
            resets_at_ms: None,
            state: ProviderAccountUsageMeterState::Unknown,
            source: "opencode.local_stats".to_string(),
            observed_at_ms,
        });
    }
    ProviderAccountUsageSnapshot {
        profile_id: account_profile.to_string(),
        provider: "opencode".to_string(),
        availability: if meters.is_empty() {
            ProviderAccountUsageAvailability::Unavailable
        } else {
            // OpenCode local stats cannot represent Zen or arbitrary upstream
            // provider balances, so it is intentionally never "available".
            ProviderAccountUsageAvailability::Partial
        },
        meters,
        observed_at_ms: Some(observed_at_ms),
        source: "opencode.local_stats".to_string(),
        management_url: Some("https://opencode.ai/zen".to_string()),
    }
}

fn remove_account_auth_environment(command: &mut Command, provider: &str) {
    for name in crate::account_profile::provider_auth_env_vars(provider) {
        command.env_remove(name);
    }
}

fn command_version(executable: &std::path::Path) -> Result<String, DaemonError> {
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "provider_version",
            message: error.to_string(),
        })?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty())
        .then_some(text)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "provider_version",
            message: "provider returned no version text".to_string(),
        })
}

fn find_numeric_field(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    match value {
        serde_json::Value::Object(object) => keys
            .iter()
            .find_map(|key| object.get(*key).and_then(serde_json::Value::as_f64))
            .or_else(|| {
                object
                    .values()
                    .find_map(|value| find_numeric_field(value, keys))
            }),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| find_numeric_field(value, keys)),
        _ => None,
    }
}

fn strip_ansi(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut escape = false;
    for character in value.chars() {
        if escape {
            if character.is_ascii_alphabetic() {
                escape = false;
            }
        } else if character == '\u{1b}' {
            escape = true;
        } else {
            result.push(character);
        }
    }
    result
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
    account_profile: &str,
    value: &serde_json::Value,
    detected_version: Option<String>,
) -> ProviderAuthStatus {
    let logged_in = value
        .get("loggedIn")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let identity_summary = if logged_in {
        let mut parts = Vec::new();
        for key in ["email", "orgName"] {
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
        account_profile: account_profile.to_string(),
        identity_summary,
        plan: value
            .get("subscriptionType")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        login_hint: Some("Run `claude auth login` to authenticate Claude Code.".to_string()),
        detected_version,
    }
}

pub(crate) fn start_provider_login_response(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: StartProviderLoginRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = registry.get(owner_user_id, &request.provider, &request.account_profile)?;
    let environment =
        registry.resolve_environment(owner_user_id, &request.provider, &profile.profile_id)?;
    match crate::provider::canonical_provider_family(&request.provider) {
        Some("codex") => {
            let endpoint = crate::provider::ensure_codex_account_endpoint(
                owner_user_id,
                &profile.profile_id,
                environment,
            )?;
            let client = CodexClient::new("provider-login", endpoint)?;
            Ok(LocalDaemonResponse::ProviderLoginStarted {
                login: client.start_login(&profile.profile_id)?,
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "start_provider_login",
            message: format!(
                "provider `{}` does not expose a structured login API",
                request.provider
            ),
        }),
    }
}

pub(crate) fn logout_provider_response(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    request: LogoutProviderRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = registry.get(owner_user_id, &request.provider, &request.account_profile)?;
    let environment =
        registry.resolve_environment(owner_user_id, &request.provider, &profile.profile_id)?;
    match crate::provider::canonical_provider_family(&request.provider) {
        Some("codex") => {
            crate::provider::logout_codex(&environment)?;
            crate::provider::invalidate_codex_account_endpoint(owner_user_id, &profile.profile_id);
            Ok(LocalDaemonResponse::ProviderLoggedOut {
                provider: "codex".to_string(),
                account_profile: profile.profile_id,
            })
        }
        Some("claude") => {
            let executable = resolve_claude_executable()?;
            let status = Command::new(executable)
                .args(["auth", "logout"])
                .envs(environment)
                .env_remove("ANTHROPIC_API_KEY")
                .env_remove("ANTHROPIC_AUTH_TOKEN")
                .env_remove("ANTHROPIC_BASE_URL")
                .env_remove("ANTHROPIC_CUSTOM_HEADERS")
                .status()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "logout_provider",
                    message: format!("failed to run Claude logout: {error}"),
                })?;
            if !status.success() {
                return Err(DaemonError::LocalTransport {
                    operation: "logout_provider",
                    message: format!("Claude logout failed: {status}"),
                });
            }
            Ok(LocalDaemonResponse::ProviderLoggedOut {
                provider: "claude".to_string(),
                account_profile: profile.profile_id,
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "logout_provider",
            message: format!(
                "provider `{}` does not expose a logout API",
                request.provider
            ),
        }),
    }
}

fn update_profile_auth_observation(
    registry: &crate::account_profile::ProviderAccountProfileRegistry,
    owner_user_id: &str,
    status: &ProviderAuthStatus,
) -> Result<(), DaemonError> {
    let auth_state = auth_state_from_status(&status.auth_state);
    registry.update_observation(
        owner_user_id,
        &status.provider,
        &status.account_profile,
        auth_state,
        status.identity_summary.clone(),
        status.plan.clone(),
        status.detected_version.clone(),
        None,
    )?;
    Ok(())
}

fn auth_state_from_status(status: &str) -> crate::account_profile::ProviderAccountAuthState {
    match status {
        "authenticated" => crate::account_profile::ProviderAccountAuthState::Authenticated,
        "not_logged_in" => crate::account_profile::ProviderAccountAuthState::NotConfigured,
        "expired" => crate::account_profile::ProviderAccountAuthState::Expired,
        "error" => crate::account_profile::ProviderAccountAuthState::Error,
        _ => crate::account_profile::ProviderAccountAuthState::Unknown,
    }
}

fn unsupported_auth_provider(operation: &'static str, provider: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation,
        message: format!("provider `{provider}` does not expose an account API"),
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
            std::env::temp_dir().join(format!("chariox-claude-auth-status-{}", std::process::id()));
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
        std::env::set_var("CHARIOX_CLAUDE_BIN", &path);

        let registry_root = std::env::temp_dir().join(format!(
            "chariox-claude-auth-registry-{}",
            std::process::id()
        ));
        let registry = crate::account_profile::ProviderAccountProfileRegistry::open(
            registry_root.join("profiles.json"),
        )
        .expect("profile registry should open");
        registry
            .migrate_effective_defaults("local", &registry_root.join("home"))
            .expect("default profiles should migrate");

        let response = provider_auth_status_response(
            &registry,
            "local",
            GetProviderAuthStatusRequest {
                provider: "claude-headless".to_string(),
                account_profile: "default".to_string(),
            },
        )
        .expect("claude mode auth status should resolve");

        std::env::remove_var("CHARIOX_CLAUDE_BIN");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&registry_root);

        match response {
            LocalDaemonResponse::ProviderAuthStatus { status } => {
                assert_eq!(status.provider, "claude-headless");
                assert_eq!(status.auth_state, "authenticated");
                assert_eq!(status.detected_version.as_deref(), Some("claude 1.2.3"));
                assert_eq!(status.account_profile, "default");
                assert!(status
                    .identity_summary
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
            "work",
            &json!({ "loggedIn": false }),
            Some("claude 1.2.3".to_string()),
        );

        assert_eq!(status.provider, "claude-p");
        assert_eq!(status.auth_state, "not_logged_in");
        assert_eq!(status.account_profile, "work");
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
