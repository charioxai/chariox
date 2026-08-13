use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

use super::provider_config::{
    claude_mcp_config_paths, claude_mcp_server_sets, claude_mcp_to_chariox, codex_home_dir,
    codex_mcp_to_chariox, opencode_config_paths, opencode_mcp_to_chariox,
    remove_json_trailing_commas, strip_jsonc_comments,
};
use super::{validate_registry_name, CharioxMcpRegistry, CharioxMcpServerConfig};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpImportSkip {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpImportOutcome {
    pub imported: Vec<CharioxMcpServerConfig>,
    pub skipped: Vec<McpImportSkip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMcpImportCandidate {
    pub provider: String,
    pub name: String,
    pub source: String,
    pub source_modified_ms: u64,
    pub definition_hash: String,
    pub config: CharioxMcpServerConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderMcpImportDiscovery {
    pub candidates: Vec<ProviderMcpImportCandidate>,
    pub skipped: Vec<McpImportSkip>,
}

pub fn import_codex_mcp_servers(
    registry: &CharioxMcpRegistry,
    requested_name: Option<&str>,
) -> Result<McpImportOutcome, DaemonError> {
    let codex_home = codex_home_dir()?;
    import_codex_mcp_servers_from_config_path(
        registry,
        &codex_home.join("config.toml"),
        requested_name,
    )
}

pub fn import_opencode_mcp_servers(
    registry: &CharioxMcpRegistry,
    workspace: &Path,
    requested_name: Option<&str>,
) -> Result<McpImportOutcome, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "mcp name")?;
    }
    let mut outcome = McpImportOutcome::default();
    let mut found_config = false;
    for config_path in opencode_config_paths(workspace) {
        if !config_path.exists() {
            continue;
        }
        found_config = true;
        let partial =
            import_opencode_mcp_servers_from_config_path(registry, &config_path, requested_name)?;
        outcome.imported.extend(partial.imported);
        outcome.skipped.extend(partial.skipped);
    }
    if !found_config {
        return Ok(outcome);
    }
    if let Some(name) = requested_name {
        let found = outcome.imported.iter().any(|mcp| mcp.name == name)
            || outcome.skipped.iter().any(|skip| skip.name == name);
        if !found {
            outcome.skipped.push(McpImportSkip {
                name: name.to_string(),
                reason: "not found in OpenCode config".to_string(),
            });
        }
    }
    Ok(outcome)
}

pub fn import_claude_mcp_servers(
    registry: &CharioxMcpRegistry,
    workspace: &Path,
    requested_name: Option<&str>,
) -> Result<McpImportOutcome, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "mcp name")?;
    }
    let mut outcome = McpImportOutcome::default();
    let mut found_config = false;
    for config_path in claude_mcp_config_paths(workspace) {
        if !config_path.exists() {
            continue;
        }
        found_config = true;
        let partial = import_claude_mcp_servers_from_config_path(
            registry,
            &config_path,
            workspace,
            requested_name,
        )?;
        outcome.imported.extend(partial.imported);
        outcome.skipped.extend(partial.skipped);
    }
    if !found_config {
        return Ok(outcome);
    }
    if let Some(name) = requested_name {
        let found = outcome.imported.iter().any(|mcp| mcp.name == name)
            || outcome.skipped.iter().any(|skip| skip.name == name);
        if !found {
            outcome.skipped.push(McpImportSkip {
                name: name.to_string(),
                reason: "not found in Claude MCP config".to_string(),
            });
        }
    }
    Ok(outcome)
}

pub fn discover_provider_mcp_import_candidates(
    provider: &str,
    workspace: &Path,
    requested_name: Option<&str>,
) -> Result<ProviderMcpImportDiscovery, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "mcp name")?;
    }
    let Some(provider) = crate::provider::canonical_provider_family(provider) else {
        return Err(DaemonError::InvalidConfig {
            field: "provider",
            message: "only Codex, OpenCode, and Claude MCP import are supported",
        });
    };
    match provider {
        "codex" => {
            let config_path = codex_home_dir()?.join("config.toml");
            if !config_path.exists() {
                return Ok(missing_provider_mcp_discovery(
                    requested_name,
                    "not found in Codex config",
                ));
            }
            discover_codex_mcp_candidates_from_config_path(&config_path, requested_name)
        }
        "opencode" => {
            let mut discovery = ProviderMcpImportDiscovery::default();
            let mut found_config = false;
            for config_path in opencode_config_paths(workspace) {
                if !config_path.exists() {
                    continue;
                }
                found_config = true;
                let partial = discover_opencode_mcp_candidates_from_config_path(
                    &config_path,
                    requested_name,
                )?;
                discovery.candidates.extend(partial.candidates);
                discovery.skipped.extend(partial.skipped);
            }
            if !found_config {
                return Ok(missing_provider_mcp_discovery(
                    requested_name,
                    "not found in OpenCode config",
                ));
            }
            if let Some(name) = requested_name {
                if !mcp_discovery_contains(&discovery, name) {
                    discovery.skipped.push(McpImportSkip {
                        name: name.to_string(),
                        reason: "not found in OpenCode config".to_string(),
                    });
                }
            }
            Ok(discovery)
        }
        "claude" => {
            let mut discovery = ProviderMcpImportDiscovery::default();
            let mut found_config = false;
            for config_path in claude_mcp_config_paths(workspace) {
                if !config_path.exists() {
                    continue;
                }
                found_config = true;
                let partial = discover_claude_mcp_candidates_from_config_path(
                    &config_path,
                    workspace,
                    requested_name,
                )?;
                discovery.candidates.extend(partial.candidates);
                discovery.skipped.extend(partial.skipped);
            }
            if !found_config {
                return Ok(missing_provider_mcp_discovery(
                    requested_name,
                    "not found in Claude MCP config",
                ));
            }
            if let Some(name) = requested_name {
                if !mcp_discovery_contains(&discovery, name) {
                    discovery.skipped.push(McpImportSkip {
                        name: name.to_string(),
                        reason: "not found in Claude MCP config".to_string(),
                    });
                }
            }
            Ok(discovery)
        }
        _ => Err(DaemonError::InvalidConfig {
            field: "provider",
            message: "only Codex, OpenCode, and Claude MCP import are supported",
        }),
    }
}

pub fn import_opencode_mcp_servers_from_config_path(
    registry: &CharioxMcpRegistry,
    config_path: &Path,
    requested_name: Option<&str>,
) -> Result<McpImportOutcome, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "mcp name")?;
    }
    let payload = fs::read_to_string(config_path).map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.import.opencode",
        message: format!(
            "failed to read OpenCode MCP config `{}`: {error}",
            config_path.display()
        ),
    })?;
    let json_payload =
        strip_jsonc_comments(&payload).map_err(|message| DaemonError::LocalTransport {
            operation: "mcp.import.opencode",
            message: format!(
                "failed to strip OpenCode JSONC config `{}`: {message}",
                config_path.display()
            ),
        })?;
    let json_payload = remove_json_trailing_commas(&json_payload);
    let parsed: serde_json::Value =
        serde_json::from_str(&json_payload).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.import.opencode",
            message: format!(
                "failed to parse OpenCode MCP config `{}`: {error}",
                config_path.display()
            ),
        })?;
    let mut outcome = McpImportOutcome::default();
    let Some(servers) = parsed.get("mcp").and_then(serde_json::Value::as_object) else {
        return Ok(outcome);
    };
    for (name, value) in servers {
        if requested_name.is_some_and(|requested| requested != name) {
            continue;
        }
        if registry.get(name)?.is_some() {
            outcome.skipped.push(McpImportSkip {
                name: name.clone(),
                reason: "already installed in Chariox registry".to_string(),
            });
            continue;
        }
        match opencode_mcp_to_chariox(name, value) {
            Ok(config) => {
                registry.install(&config)?;
                outcome.imported.push(config);
            }
            Err(reason) => outcome.skipped.push(McpImportSkip {
                name: name.clone(),
                reason,
            }),
        }
    }
    Ok(outcome)
}

pub fn discover_opencode_mcp_candidates_from_config_path(
    config_path: &Path,
    requested_name: Option<&str>,
) -> Result<ProviderMcpImportDiscovery, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "mcp name")?;
    }
    let payload = fs::read_to_string(config_path).map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.discover.opencode",
        message: format!(
            "failed to read OpenCode MCP config `{}`: {error}",
            config_path.display()
        ),
    })?;
    let json_payload =
        strip_jsonc_comments(&payload).map_err(|message| DaemonError::LocalTransport {
            operation: "mcp.discover.opencode",
            message: format!(
                "failed to strip OpenCode JSONC config `{}`: {message}",
                config_path.display()
            ),
        })?;
    let json_payload = remove_json_trailing_commas(&json_payload);
    let parsed: serde_json::Value =
        serde_json::from_str(&json_payload).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.discover.opencode",
            message: format!(
                "failed to parse OpenCode MCP config `{}`: {error}",
                config_path.display()
            ),
        })?;
    let mut discovery = ProviderMcpImportDiscovery::default();
    let Some(servers) = parsed.get("mcp").and_then(serde_json::Value::as_object) else {
        return Ok(discovery);
    };
    for (name, value) in servers {
        if requested_name.is_some_and(|requested| requested != name) {
            continue;
        }
        match opencode_mcp_to_chariox(name, value) {
            Ok(config) => {
                discovery
                    .candidates
                    .push(provider_mcp_candidate("opencode", config_path, config)?)
            }
            Err(reason) => discovery.skipped.push(McpImportSkip {
                name: name.clone(),
                reason,
            }),
        }
    }
    Ok(discovery)
}

pub fn import_claude_mcp_servers_from_config_path(
    registry: &CharioxMcpRegistry,
    config_path: &Path,
    workspace: &Path,
    requested_name: Option<&str>,
) -> Result<McpImportOutcome, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "mcp name")?;
    }
    let payload = fs::read_to_string(config_path).map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.import.claude",
        message: format!(
            "failed to read Claude MCP config `{}`: {error}",
            config_path.display()
        ),
    })?;
    let parsed: serde_json::Value =
        serde_json::from_str(&payload).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.import.claude",
            message: format!(
                "failed to parse Claude MCP config `{}`: {error}",
                config_path.display()
            ),
        })?;
    let mut outcome = McpImportOutcome::default();
    let server_sets = claude_mcp_server_sets(&parsed, config_path, workspace);
    for (scope, servers) in server_sets {
        for (name, value) in servers {
            if requested_name.is_some_and(|requested| requested != name) {
                continue;
            }
            if registry.get(name)?.is_some() {
                outcome.skipped.push(McpImportSkip {
                    name: name.to_string(),
                    reason: format!("already installed in Chariox registry ({scope})"),
                });
                continue;
            }
            match claude_mcp_to_chariox(name, value) {
                Ok(config) => {
                    registry.install(&config)?;
                    outcome.imported.push(config);
                }
                Err(reason) => outcome.skipped.push(McpImportSkip {
                    name: name.to_string(),
                    reason: format!("{scope}: {reason}"),
                }),
            }
        }
    }
    Ok(outcome)
}

pub fn discover_claude_mcp_candidates_from_config_path(
    config_path: &Path,
    workspace: &Path,
    requested_name: Option<&str>,
) -> Result<ProviderMcpImportDiscovery, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "mcp name")?;
    }
    let payload = fs::read_to_string(config_path).map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.discover.claude",
        message: format!(
            "failed to read Claude MCP config `{}`: {error}",
            config_path.display()
        ),
    })?;
    let parsed: serde_json::Value =
        serde_json::from_str(&payload).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.discover.claude",
            message: format!(
                "failed to parse Claude MCP config `{}`: {error}",
                config_path.display()
            ),
        })?;
    let mut discovery = ProviderMcpImportDiscovery::default();
    for (scope, servers) in claude_mcp_server_sets(&parsed, config_path, workspace) {
        for (name, value) in servers {
            if requested_name.is_some_and(|requested| requested != name) {
                continue;
            }
            match claude_mcp_to_chariox(name, value) {
                Ok(config) => discovery.candidates.push(provider_mcp_candidate(
                    "claude",
                    config_path,
                    config,
                )?),
                Err(reason) => discovery.skipped.push(McpImportSkip {
                    name: name.to_string(),
                    reason: format!("{scope}: {reason}"),
                }),
            }
        }
    }
    Ok(discovery)
}

pub fn import_codex_mcp_servers_from_config_path(
    registry: &CharioxMcpRegistry,
    config_path: &Path,
    requested_name: Option<&str>,
) -> Result<McpImportOutcome, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "mcp name")?;
    }
    let payload = fs::read_to_string(config_path).map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.import.codex",
        message: format!(
            "failed to read Codex MCP config `{}`: {error}",
            config_path.display()
        ),
    })?;
    let parsed: toml::Value =
        toml::from_str(&payload).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.import.codex",
            message: format!(
                "failed to parse Codex MCP config `{}`: {error}",
                config_path.display()
            ),
        })?;
    let mut outcome = McpImportOutcome::default();
    let Some(servers) = parsed.get("mcp_servers").and_then(toml::Value::as_table) else {
        return Ok(outcome);
    };
    for (name, value) in servers {
        if requested_name.is_some_and(|requested| requested != name) {
            continue;
        }
        if registry.get(name)?.is_some() {
            outcome.skipped.push(McpImportSkip {
                name: name.clone(),
                reason: "already installed in Chariox registry".to_string(),
            });
            continue;
        }
        match codex_mcp_to_chariox(name, value) {
            Ok(config) => {
                registry.install(&config)?;
                outcome.imported.push(config);
            }
            Err(reason) => outcome.skipped.push(McpImportSkip {
                name: name.clone(),
                reason,
            }),
        }
    }
    if let Some(name) = requested_name {
        let found = outcome.imported.iter().any(|mcp| mcp.name == name)
            || outcome.skipped.iter().any(|skip| skip.name == name);
        if !found {
            outcome.skipped.push(McpImportSkip {
                name: name.to_string(),
                reason: "not found in Codex config".to_string(),
            });
        }
    }
    Ok(outcome)
}

pub fn discover_codex_mcp_candidates_from_config_path(
    config_path: &Path,
    requested_name: Option<&str>,
) -> Result<ProviderMcpImportDiscovery, DaemonError> {
    if let Some(name) = requested_name {
        validate_registry_name(name, "mcp name")?;
    }
    let payload = fs::read_to_string(config_path).map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.discover.codex",
        message: format!(
            "failed to read Codex MCP config `{}`: {error}",
            config_path.display()
        ),
    })?;
    let parsed: toml::Value =
        toml::from_str(&payload).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.discover.codex",
            message: format!(
                "failed to parse Codex MCP config `{}`: {error}",
                config_path.display()
            ),
        })?;
    let mut discovery = ProviderMcpImportDiscovery::default();
    let Some(servers) = parsed.get("mcp_servers").and_then(toml::Value::as_table) else {
        return Ok(discovery);
    };
    for (name, value) in servers {
        if requested_name.is_some_and(|requested| requested != name) {
            continue;
        }
        match codex_mcp_to_chariox(name, value) {
            Ok(config) => {
                discovery
                    .candidates
                    .push(provider_mcp_candidate("codex", config_path, config)?);
            }
            Err(reason) => discovery.skipped.push(McpImportSkip {
                name: name.clone(),
                reason,
            }),
        }
    }
    if let Some(name) = requested_name {
        if !mcp_discovery_contains(&discovery, name) {
            discovery.skipped.push(McpImportSkip {
                name: name.to_string(),
                reason: "not found in Codex config".to_string(),
            });
        }
    }
    Ok(discovery)
}

fn provider_mcp_candidate(
    provider: &str,
    source: &Path,
    config: CharioxMcpServerConfig,
) -> Result<ProviderMcpImportCandidate, DaemonError> {
    Ok(ProviderMcpImportCandidate {
        provider: provider.to_string(),
        name: config.name.clone(),
        source: source.display().to_string(),
        source_modified_ms: source_modified_ms(source),
        definition_hash: config.definition_hash()?,
        config,
    })
}

fn missing_provider_mcp_discovery(
    requested_name: Option<&str>,
    reason: &str,
) -> ProviderMcpImportDiscovery {
    let mut discovery = ProviderMcpImportDiscovery::default();
    if let Some(name) = requested_name {
        discovery.skipped.push(McpImportSkip {
            name: name.to_string(),
            reason: reason.to_string(),
        });
    }
    discovery
}

fn mcp_discovery_contains(discovery: &ProviderMcpImportDiscovery, name: &str) -> bool {
    discovery
        .candidates
        .iter()
        .any(|candidate| candidate.name == name)
        || discovery.skipped.iter().any(|skip| skip.name == name)
}

fn source_modified_ms(path: &Path) -> u64 {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
