use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaMcpServerConfig {
    pub name: String,
    pub transport: ArrobaMcpTransportConfig,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, ArrobaMcpToolConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaMcpCredentialBinding {
    pub credential: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArrobaMcpTransportConfig {
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        credential_env: BTreeMap<String, ArrobaMcpCredentialBinding>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env_vars: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
    },
    StreamableHttp {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_token_env_var: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_token_credential: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        http_headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        credential_http_headers: BTreeMap<String, ArrobaMcpCredentialBinding>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env_http_headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaMcpToolConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrobaMcpRegistry {
    roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpImportSkip {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpImportOutcome {
    pub imported: Vec<ArrobaMcpServerConfig>,
    pub skipped: Vec<McpImportSkip>,
}

impl ArrobaMcpServerConfig {
    pub fn stdio(name: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            transport: ArrobaMcpTransportConfig::Stdio {
                command: command.into(),
                args,
                env: BTreeMap::new(),
                credential_env: BTreeMap::new(),
                env_vars: Vec::new(),
                cwd: None,
            },
            enabled: true,
            required: false,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
            tools: BTreeMap::new(),
        }
    }

    pub fn streamable_http(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: ArrobaMcpTransportConfig::StreamableHttp {
                url: url.into(),
                bearer_token_env_var: None,
                bearer_token_credential: None,
                http_headers: BTreeMap::new(),
                credential_http_headers: BTreeMap::new(),
                env_http_headers: BTreeMap::new(),
            },
            enabled: true,
            required: false,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
            tools: BTreeMap::new(),
        }
    }

    pub fn validate(&self) -> Result<(), DaemonError> {
        validate_registry_name(&self.name, "mcp name")?;
        match &self.transport {
            ArrobaMcpTransportConfig::Stdio { command, .. } => {
                if command.trim().is_empty() {
                    return invalid("mcp command", "must not be empty");
                }
            }
            ArrobaMcpTransportConfig::StreamableHttp { url, .. } => {
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return invalid("mcp url", "must start with http:// or https://");
                }
            }
        }
        Ok(())
    }

    pub fn definition_hash(&self) -> Result<String, DaemonError> {
        let bytes = serde_json::to_vec(self).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.definition_hash",
            message: format!("failed to serialize MCP `{}`: {error}", self.name),
        })?;
        let digest = Sha256::digest(&bytes);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    pub fn resolve_credential_bindings(
        &self,
        secret_service: &crate::secret::RuntimeSecretService,
    ) -> Result<Self, DaemonError> {
        let mut resolved = self.clone();
        match &mut resolved.transport {
            ArrobaMcpTransportConfig::Stdio {
                env,
                credential_env,
                ..
            } => {
                for (name, binding) in std::mem::take(credential_env) {
                    env.insert(
                        name,
                        secret_service.resolve_mcp_secret(&binding.credential)?,
                    );
                }
            }
            ArrobaMcpTransportConfig::StreamableHttp {
                bearer_token_credential,
                http_headers,
                credential_http_headers,
                ..
            } => {
                if let Some(credential_id) = bearer_token_credential.take() {
                    let secret = secret_service.resolve_mcp_secret(&credential_id)?;
                    http_headers.insert("Authorization".to_string(), format!("Bearer {secret}"));
                }
                for (name, binding) in std::mem::take(credential_http_headers) {
                    http_headers.insert(
                        name,
                        secret_service.resolve_mcp_secret(&binding.credential)?,
                    );
                }
            }
        }
        resolved.validate()?;
        Ok(resolved)
    }
}

impl ArrobaMcpRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn project_root(workspace: impl AsRef<Path>) -> PathBuf {
        if let Some(root) = managed_capability_root() {
            return root
                .join("project")
                .join(workspace_registry_hash(workspace.as_ref()))
                .join("mcps");
        }
        workspace.as_ref().join(".arroba").join("mcps")
    }

    pub fn user_root() -> Option<PathBuf> {
        if let Some(root) = managed_capability_root() {
            return Some(root.join("user").join("mcps"));
        }
        home_dir().map(|home| home.join(".arroba").join("mcps"))
    }

    pub fn install(&self, config: &ArrobaMcpServerConfig) -> Result<PathBuf, DaemonError> {
        config.validate()?;
        let root = self.primary_root()?;
        fs::create_dir_all(root).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.install",
            message: format!(
                "failed to create MCP registry `{}`: {error}",
                root.display()
            ),
        })?;
        let path = root.join(format!("{}.json", config.name));
        let payload =
            serde_json::to_string_pretty(config).map_err(|error| DaemonError::LocalTransport {
                operation: "mcp.install",
                message: format!("failed to serialize MCP `{}`: {error}", config.name),
            })?;
        atomic_write_config(&path, format!("{payload}\n").as_bytes(), "mcp.install")?;
        Ok(path)
    }

    pub fn update(&self, config: &ArrobaMcpServerConfig) -> Result<PathBuf, DaemonError> {
        config.validate()?;
        let path =
            self.find_config_path(&config.name)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "mcp.update",
                    message: format!("MCP `{}` is not installed", config.name),
                })?;
        let payload =
            serde_json::to_string_pretty(config).map_err(|error| DaemonError::LocalTransport {
                operation: "mcp.update",
                message: format!("failed to serialize MCP `{}`: {error}", config.name),
            })?;
        atomic_write_config(&path, format!("{payload}\n").as_bytes(), "mcp.update")?;
        Ok(path)
    }

    pub fn uninstall(&self, name: &str) -> Result<PathBuf, DaemonError> {
        validate_registry_name(name, "mcp name")?;
        let path = self
            .find_config_path(name)?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mcp.uninstall",
                message: format!("MCP `{name}` is not installed"),
            })?;
        fs::remove_file(&path).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.uninstall",
            message: format!("failed to remove MCP `{}`: {error}", path.display()),
        })?;
        Ok(path)
    }

    pub fn list(&self) -> Result<Vec<ArrobaMcpServerConfig>, DaemonError> {
        let mut entries = BTreeMap::new();
        for root in &self.roots {
            if !root.exists() {
                continue;
            }
            for entry in fs::read_dir(root).map_err(|error| DaemonError::LocalTransport {
                operation: "mcp.list",
                message: format!("failed to read MCP registry `{}`: {error}", root.display()),
            })? {
                let path = entry
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "mcp.list",
                        message: format!("failed to read MCP registry entry: {error}"),
                    })?
                    .path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let config = Self::read_config(&path)?;
                entries.entry(config.name.clone()).or_insert(config);
            }
        }
        Ok(entries.into_values().collect())
    }

    pub fn get(&self, name: &str) -> Result<Option<ArrobaMcpServerConfig>, DaemonError> {
        validate_registry_name(name, "mcp name")?;
        let Some(path) = self.find_config_path(name)? else {
            return Ok(None);
        };
        Self::read_config(&path).map(Some)
    }

    fn find_config_path(&self, name: &str) -> Result<Option<PathBuf>, DaemonError> {
        validate_registry_name(name, "mcp name")?;
        for root in &self.roots {
            let path = root.join(format!("{name}.json"));
            if path.exists() {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn primary_root(&self) -> Result<&PathBuf, DaemonError> {
        self.roots
            .first()
            .ok_or_else(|| DaemonError::InvalidConfig {
                field: "mcp registry roots",
                message: "must include at least one root",
            })
    }

    fn read_config(path: &Path) -> Result<ArrobaMcpServerConfig, DaemonError> {
        let payload = fs::read_to_string(path).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.read",
            message: format!("failed to read MCP `{}`: {error}", path.display()),
        })?;
        let config: ArrobaMcpServerConfig =
            serde_json::from_str(&payload).map_err(|error| DaemonError::LocalTransport {
                operation: "mcp.read",
                message: format!("failed to parse MCP `{}`: {error}", path.display()),
            })?;
        config.validate()?;
        Ok(config)
    }
}

pub fn import_codex_mcp_servers(
    registry: &ArrobaMcpRegistry,
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
    registry: &ArrobaMcpRegistry,
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
    registry: &ArrobaMcpRegistry,
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

pub fn import_opencode_mcp_servers_from_config_path(
    registry: &ArrobaMcpRegistry,
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
                reason: "already installed in Arroba registry".to_string(),
            });
            continue;
        }
        match opencode_mcp_to_arroba(name, value) {
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

pub fn import_claude_mcp_servers_from_config_path(
    registry: &ArrobaMcpRegistry,
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
                    name: name.clone(),
                    reason: format!("already installed in Arroba registry ({scope})"),
                });
                continue;
            }
            match claude_mcp_to_arroba(name, value) {
                Ok(config) => {
                    registry.install(&config)?;
                    outcome.imported.push(config);
                }
                Err(reason) => outcome.skipped.push(McpImportSkip {
                    name: name.clone(),
                    reason: format!("{scope}: {reason}"),
                }),
            }
        }
    }
    Ok(outcome)
}

pub fn import_codex_mcp_servers_from_config_path(
    registry: &ArrobaMcpRegistry,
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
                reason: "already installed in Arroba registry".to_string(),
            });
            continue;
        }
        match codex_mcp_to_arroba(name, value) {
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

fn codex_mcp_to_arroba(name: &str, value: &toml::Value) -> Result<ArrobaMcpServerConfig, String> {
    let table = value
        .as_table()
        .ok_or_else(|| "MCP entry must be a TOML table".to_string())?;
    let unsupported = unsupported_codex_mcp_fields(table);
    if !unsupported.is_empty() {
        return Err(format!(
            "unsupported Codex MCP fields: {}",
            unsupported.join(", ")
        ));
    }
    if table.contains_key("bearer_token") {
        return Err("inline bearer_token is not imported; use bearer_token_env_var".to_string());
    }

    let mut config = if let Some(command) = table.get("command") {
        if table.contains_key("url") {
            return Err("entry mixes stdio command and HTTP url transports".to_string());
        }
        for field in ["bearer_token_env_var", "http_headers", "env_http_headers"] {
            if table.contains_key(field) {
                return Err(format!("{field} is not supported for stdio MCPs"));
            }
        }
        let command = required_string(command, "command")?;
        let args = optional_string_array(table.get("args"), "args")?.unwrap_or_default();
        let mut config = ArrobaMcpServerConfig::stdio(name, command, args);
        if let ArrobaMcpTransportConfig::Stdio {
            env, env_vars, cwd, ..
        } = &mut config.transport
        {
            *env = optional_string_map(table.get("env"), "env")?.unwrap_or_default();
            *env_vars =
                optional_string_array(table.get("env_vars"), "env_vars")?.unwrap_or_default();
            *cwd = table
                .get("cwd")
                .map(|value| required_string(value, "cwd").map(PathBuf::from))
                .transpose()?;
        }
        config
    } else if let Some(url) = table.get("url") {
        for field in ["args", "env", "env_vars", "cwd"] {
            if table.contains_key(field) {
                return Err(format!("{field} is not supported for HTTP MCPs"));
            }
        }
        let url = required_string(url, "url")?;
        let mut config = ArrobaMcpServerConfig::streamable_http(name, url);
        if let ArrobaMcpTransportConfig::StreamableHttp {
            bearer_token_env_var,
            http_headers,
            env_http_headers,
            ..
        } = &mut config.transport
        {
            *bearer_token_env_var = table
                .get("bearer_token_env_var")
                .map(|value| required_string(value, "bearer_token_env_var"))
                .transpose()?;
            *http_headers =
                optional_string_map(table.get("http_headers"), "http_headers")?.unwrap_or_default();
            *env_http_headers =
                optional_string_map(table.get("env_http_headers"), "env_http_headers")?
                    .unwrap_or_default();
        }
        config
    } else {
        return Err("missing command or url transport".to_string());
    };

    config.enabled = optional_bool(table.get("enabled"), "enabled")?.unwrap_or(true);
    config.required = optional_bool(table.get("required"), "required")?.unwrap_or(false);
    config.startup_timeout_sec = optional_timeout_secs(table, "startup_timeout_sec")?
        .or(optional_legacy_timeout_ms(table, "startup_timeout_ms")?);
    config.tool_timeout_sec = optional_timeout_secs(table, "tool_timeout_sec")?;
    config.enabled_tools = optional_string_array(table.get("enabled_tools"), "enabled_tools")?;
    config.disabled_tools = optional_string_array(table.get("disabled_tools"), "disabled_tools")?;
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn unsupported_codex_mcp_fields(table: &toml::Table) -> Vec<String> {
    let supported = [
        "command",
        "args",
        "env",
        "env_vars",
        "cwd",
        "url",
        "bearer_token",
        "bearer_token_env_var",
        "http_headers",
        "env_http_headers",
        "startup_timeout_sec",
        "startup_timeout_ms",
        "tool_timeout_sec",
        "enabled",
        "required",
        "enabled_tools",
        "disabled_tools",
    ];
    table
        .keys()
        .filter(|field| !supported.contains(&field.as_str()))
        .cloned()
        .collect()
}

fn required_string(value: &toml::Value, field: &str) -> Result<String, String> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{field} must be a string"))
}

fn optional_bool(value: Option<&toml::Value>, field: &str) -> Result<Option<bool>, String> {
    value
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("{field} must be a boolean"))
        })
        .transpose()
}

fn optional_string_array(
    value: Option<&toml::Value>,
    field: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{field} must be an array of strings"))?;
    array
        .iter()
        .map(|value| required_string(value, field))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn optional_string_map(
    value: Option<&toml::Value>,
    field: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let table = value
        .as_table()
        .ok_or_else(|| format!("{field} must be a table of strings"))?;
    table
        .iter()
        .map(|(key, value)| required_string(value, field).map(|value| (key.clone(), value)))
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map(Some)
}

fn optional_timeout_secs(table: &toml::Table, field: &str) -> Result<Option<u64>, String> {
    let Some(value) = table.get(field) else {
        return Ok(None);
    };
    let secs = value
        .as_float()
        .or_else(|| value.as_integer().map(|integer| integer as f64))
        .ok_or_else(|| format!("{field} must be a number"))?;
    finite_timeout_secs(field, secs)
}

fn optional_legacy_timeout_ms(table: &toml::Table, field: &str) -> Result<Option<u64>, String> {
    let Some(value) = table.get(field) else {
        return Ok(None);
    };
    let millis = value
        .as_integer()
        .ok_or_else(|| format!("{field} must be an integer"))?;
    if millis < 0 {
        return Err(format!("{field} must be non-negative"));
    }
    finite_timeout_secs(field, millis as f64 / 1000.0)
}

fn finite_timeout_secs(field: &str, secs: f64) -> Result<Option<u64>, String> {
    if !secs.is_finite() || secs < 0.0 {
        return Err(format!("{field} must be a finite non-negative number"));
    }
    Ok(Some(secs.ceil() as u64))
}

pub(crate) fn validate_registry_name(name: &str, field: &'static str) -> Result<(), DaemonError> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        invalid(field, "must contain only letters, numbers, '-' or '_'")
    }
}

fn default_true() -> bool {
    true
}

fn invalid<T>(field: &'static str, message: &'static str) -> Result<T, DaemonError> {
    Err(DaemonError::InvalidConfig { field, message })
}

fn atomic_write_config(
    path: &Path,
    contents: &[u8],
    operation: &'static str,
) -> Result<(), DaemonError> {
    let parent = path.parent().ok_or_else(|| DaemonError::LocalTransport {
        operation,
        message: format!("MCP config path `{}` has no parent", path.display()),
    })?;
    fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!(
            "failed to create MCP config parent `{}`: {error}",
            parent.display()
        ),
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mcp-config");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp_path = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce));
    fs::write(&temp_path, contents).map_err(|error| DaemonError::LocalTransport {
        operation,
        message: format!(
            "failed to write temporary MCP config `{}`: {error}",
            temp_path.display()
        ),
    })?;
    fs::rename(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        DaemonError::LocalTransport {
            operation,
            message: format!("failed to publish MCP config `{}`: {error}", path.display()),
        }
    })
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn managed_capability_root() -> Option<PathBuf> {
    std::env::var_os("ARROBA_CAPABILITY_ISOLATION_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn workspace_registry_hash(workspace: &Path) -> String {
    let digest = Sha256::digest(workspace.to_string_lossy().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn codex_home_dir() -> Result<PathBuf, DaemonError> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home));
    }
    home_dir()
        .map(|home| home.join(".codex"))
        .ok_or(DaemonError::InvalidConfig {
            field: "CODEX_HOME",
            message: "CODEX_HOME or HOME must be set to import Codex MCPs",
        })
}

fn opencode_config_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(custom) = std::env::var_os("OPENCODE_CONFIG") {
        paths.push(PathBuf::from(custom));
    }
    if let Some(config_dir) = std::env::var_os("OPENCODE_CONFIG_DIR") {
        paths.extend(opencode_files_in_dir(Path::new(&config_dir)));
    }
    paths.extend([
        workspace.join("opencode.jsonc"),
        workspace.join("opencode.json"),
        workspace.join(".opencode").join("opencode.jsonc"),
        workspace.join(".opencode").join("opencode.json"),
    ]);
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        paths.extend(opencode_files_in_dir(
            &PathBuf::from(config_home).join("opencode"),
        ));
    } else if let Some(home) = home_dir() {
        paths.extend(opencode_files_in_dir(
            &home.join(".config").join("opencode"),
        ));
    }
    paths
}

fn claude_mcp_config_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(custom) = std::env::var_os("ARROBA_CLAUDE_CONFIG") {
        paths.push(PathBuf::from(custom));
    } else if let Some(home) = home_dir() {
        paths.push(home.join(".claude.json"));
    }
    paths.push(workspace.join(".mcp.json"));
    paths
}

fn opencode_files_in_dir(dir: &Path) -> Vec<PathBuf> {
    vec![
        dir.join("opencode.jsonc"),
        dir.join("opencode.json"),
        dir.join("config.json"),
    ]
}

fn claude_mcp_server_sets<'a>(
    parsed: &'a serde_json::Value,
    config_path: &Path,
    workspace: &Path,
) -> Vec<(String, &'a serde_json::Map<String, serde_json::Value>)> {
    let mut sets = Vec::new();
    if let Some(servers) = parsed
        .get("mcpServers")
        .and_then(serde_json::Value::as_object)
    {
        sets.push((config_path.display().to_string(), servers));
    }
    if config_path.file_name().and_then(|name| name.to_str()) == Some(".claude.json") {
        let workspace_key = workspace.to_string_lossy();
        let canonical_workspace = workspace.canonicalize().ok();
        if let Some(projects) = parsed
            .get("projects")
            .and_then(serde_json::Value::as_object)
        {
            for (project_path, project) in projects {
                let direct_match = project_path == workspace_key.as_ref();
                let canonical_match = canonical_workspace
                    .as_ref()
                    .is_some_and(|canonical| Path::new(project_path) == canonical.as_path());
                if !direct_match && !canonical_match {
                    continue;
                }
                if let Some(servers) = project
                    .get("mcpServers")
                    .and_then(serde_json::Value::as_object)
                {
                    sets.push((
                        format!("{} projects[{project_path}]", config_path.display()),
                        servers,
                    ));
                }
            }
        }
    }
    sets
}

fn opencode_mcp_to_arroba(
    name: &str,
    value: &serde_json::Value,
) -> Result<ArrobaMcpServerConfig, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "MCP entry must be an object".to_string())?;
    let mcp_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing MCP type".to_string())?;
    match mcp_type {
        "local" => opencode_local_mcp_to_arroba(name, object),
        "remote" => opencode_remote_mcp_to_arroba(name, object),
        other => Err(format!("unsupported OpenCode MCP type `{other}`")),
    }
}

fn opencode_local_mcp_to_arroba(
    name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<ArrobaMcpServerConfig, String> {
    let command_parts = object
        .get("command")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "local MCP command must be an array".to_string())?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| "local MCP command entries must be strings".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let Some((command, args)) = command_parts.split_first() else {
        return Err("local MCP command must not be empty".to_string());
    };
    let mut config = ArrobaMcpServerConfig::stdio(name, command.clone(), args.to_vec());
    config.enabled = optional_json_bool(object.get("enabled"), "enabled")?.unwrap_or(true);
    config.tool_timeout_sec = optional_json_timeout_ms(object.get("timeout"), "timeout")?;
    if let ArrobaMcpTransportConfig::Stdio { env, env_vars, .. } = &mut config.transport {
        let environment =
            optional_json_string_map(object.get("environment"), "environment")?.unwrap_or_default();
        for (key, value) in environment {
            if let Some(var_name) = env_reference(&value) {
                if var_name == key {
                    env_vars.push(key);
                } else {
                    return Err(format!(
                        "environment `{key}` references env var `{var_name}`, which cannot be represented in Arroba stdio env_vars"
                    ));
                }
            } else {
                env.insert(key, value);
            }
        }
    }
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn opencode_remote_mcp_to_arroba(
    name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<ArrobaMcpServerConfig, String> {
    let url = object
        .get("url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "remote MCP url must be a string".to_string())?;
    if object
        .get("oauth")
        .is_some_and(|value| value != &serde_json::Value::Bool(false))
    {
        return Err("OpenCode OAuth MCP config is not imported yet".to_string());
    }
    let mut config = ArrobaMcpServerConfig::streamable_http(name, url);
    config.enabled = optional_json_bool(object.get("enabled"), "enabled")?.unwrap_or(true);
    config.tool_timeout_sec = optional_json_timeout_ms(object.get("timeout"), "timeout")?;
    if let ArrobaMcpTransportConfig::StreamableHttp {
        http_headers,
        env_http_headers,
        ..
    } = &mut config.transport
    {
        let headers =
            optional_json_string_map(object.get("headers"), "headers")?.unwrap_or_default();
        for (key, value) in headers {
            if let Some(var_name) = env_reference(&value) {
                env_http_headers.insert(key, var_name);
            } else {
                http_headers.insert(key, value);
            }
        }
    }
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn claude_mcp_to_arroba(
    name: &str,
    value: &serde_json::Value,
) -> Result<ArrobaMcpServerConfig, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "MCP entry must be an object".to_string())?;
    if object.contains_key("oauth")
        || object.contains_key("oauthScopes")
        || object.contains_key("oauthResource")
    {
        return Err("Claude OAuth MCP config is not imported yet".to_string());
    }
    let mcp_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| {
            if object.contains_key("command") {
                "stdio"
            } else if object.contains_key("url") {
                "http"
            } else {
                ""
            }
        });
    match mcp_type {
        "stdio" => claude_stdio_mcp_to_arroba(name, object),
        "http" | "streamable_http" => claude_http_mcp_to_arroba(name, object),
        "sse" => Err("Claude SSE MCP config is not imported yet".to_string()),
        "" => Err("missing Claude MCP type, command, or url".to_string()),
        other => Err(format!("unsupported Claude MCP type `{other}`")),
    }
}

fn claude_stdio_mcp_to_arroba(
    name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<ArrobaMcpServerConfig, String> {
    if object.contains_key("url") {
        return Err("stdio MCP entry also contains url".to_string());
    }
    let command = object
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "stdio MCP command must be a string".to_string())?;
    let args = optional_json_string_array(object.get("args"), "args")?.unwrap_or_default();
    let mut config = ArrobaMcpServerConfig::stdio(name, command, args);
    config.enabled = optional_json_bool(object.get("enabled"), "enabled")?.unwrap_or(true);
    config.required = optional_json_bool(object.get("required"), "required")?.unwrap_or(false);
    config.startup_timeout_sec =
        optional_json_timeout_secs(object.get("startup_timeout_sec"), "startup_timeout_sec")?;
    config.tool_timeout_sec =
        optional_json_timeout_secs(object.get("tool_timeout_sec"), "tool_timeout_sec")?;
    if let ArrobaMcpTransportConfig::Stdio {
        env, env_vars, cwd, ..
    } = &mut config.transport
    {
        let environment = optional_json_string_map(object.get("env"), "env")?.unwrap_or_default();
        for (key, value) in environment {
            if let Some(var_name) = env_reference(&value) {
                if var_name == key {
                    env_vars.push(key);
                } else {
                    return Err(format!(
                        "env `{key}` references env var `{var_name}`, which cannot be represented in Arroba stdio env_vars"
                    ));
                }
            } else {
                env.insert(key, value);
            }
        }
        *cwd = object
            .get("cwd")
            .map(|value| {
                value
                    .as_str()
                    .map(PathBuf::from)
                    .ok_or_else(|| "cwd must be a string".to_string())
            })
            .transpose()?;
    }
    config.enabled_tools = optional_json_string_array(object.get("enabledTools"), "enabledTools")?;
    config.disabled_tools =
        optional_json_string_array(object.get("disabledTools"), "disabledTools")?;
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn claude_http_mcp_to_arroba(
    name: &str,
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<ArrobaMcpServerConfig, String> {
    if object.contains_key("command") {
        return Err("HTTP MCP entry also contains command".to_string());
    }
    let url = object
        .get("url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "HTTP MCP url must be a string".to_string())?;
    let mut config = ArrobaMcpServerConfig::streamable_http(name, url);
    config.enabled = optional_json_bool(object.get("enabled"), "enabled")?.unwrap_or(true);
    config.required = optional_json_bool(object.get("required"), "required")?.unwrap_or(false);
    config.startup_timeout_sec =
        optional_json_timeout_secs(object.get("startup_timeout_sec"), "startup_timeout_sec")?;
    config.tool_timeout_sec =
        optional_json_timeout_secs(object.get("tool_timeout_sec"), "tool_timeout_sec")?;
    if let ArrobaMcpTransportConfig::StreamableHttp {
        http_headers,
        env_http_headers,
        ..
    } = &mut config.transport
    {
        let headers =
            optional_json_string_map(object.get("headers"), "headers")?.unwrap_or_default();
        for (key, value) in headers {
            if let Some(var_name) = env_reference(&value) {
                env_http_headers.insert(key, var_name);
            } else if key.eq_ignore_ascii_case("authorization") {
                return Err(
                    "static Authorization headers are not imported; use an environment reference"
                        .to_string(),
                );
            } else {
                http_headers.insert(key, value);
            }
        }
    }
    config.enabled_tools = optional_json_string_array(object.get("enabledTools"), "enabledTools")?;
    config.disabled_tools =
        optional_json_string_array(object.get("disabledTools"), "disabledTools")?;
    config.validate().map_err(|error| error.to_string())?;
    Ok(config)
}

fn optional_json_bool(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<bool>, String> {
    value
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| format!("{field} must be a boolean"))
        })
        .transpose()
}

fn optional_json_timeout_ms(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let millis = value
        .as_u64()
        .ok_or_else(|| format!("{field} must be a positive integer number of milliseconds"))?;
    if millis == 0 {
        return Err(format!("{field} must be positive"));
    }
    Ok(Some((millis + 999) / 1000))
}

fn optional_json_timeout_secs(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let secs = value
        .as_f64()
        .ok_or_else(|| format!("{field} must be a positive number of seconds"))?;
    if !secs.is_finite() || secs <= 0.0 {
        return Err(format!("{field} must be positive"));
    }
    Ok(Some(secs.ceil() as u64))
}

fn optional_json_string_array(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{field} must be an array of strings"))?;
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{field} entries must be strings"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn optional_json_string_map(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| format!("{field} must be an object of strings"))?;
    object
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_string()))
                .ok_or_else(|| format!("{field}.{key} must be a string"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map(Some)
}

fn env_reference(value: &str) -> Option<String> {
    value
        .strip_prefix("{env:")
        .and_then(|rest| rest.strip_suffix('}'))
        .filter(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .map(str::to_string)
}

fn strip_jsonc_comments(input: &str) -> Result<String, String> {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }

        if ch == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut closed = false;
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        if next == '\n' {
                            output.push('\n');
                        }
                        if previous == '*' && next == '/' {
                            closed = true;
                            break;
                        }
                        previous = next;
                    }
                    if !closed {
                        return Err("unterminated block comment".to_string());
                    }
                    continue;
                }
                _ => {}
            }
        }

        output.push(ch);
    }

    Ok(output)
}

fn remove_json_trailing_commas(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }

        if ch == ',' {
            let mut lookahead = chars.clone();
            while matches!(lookahead.peek(), Some(next) if next.is_whitespace()) {
                lookahead.next();
            }
            if matches!(lookahead.peek(), Some('}' | ']')) {
                continue;
            }
        }

        output.push(ch);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "arroba-mcp-registry-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn registry_roots_can_be_isolated_for_managed_slice_runtime() {
        let _guard = crate::env_lock::lock();
        let isolation_root = temp_root("managed-slice-isolation");
        std::env::set_var("ARROBA_CAPABILITY_ISOLATION_ROOT", &isolation_root);

        let project_root = ArrobaMcpRegistry::project_root("/workspace");
        let user_root = ArrobaMcpRegistry::user_root().expect("user root should resolve");

        std::env::remove_var("ARROBA_CAPABILITY_ISOLATION_ROOT");
        let _ = fs::remove_dir_all(&isolation_root);

        assert!(project_root.starts_with(isolation_root.join("project")));
        assert!(project_root.ends_with("mcps"));
        assert_eq!(user_root, isolation_root.join("user").join("mcps"));
    }

    #[test]
    fn registry_round_trips_stdio_mcp_config() {
        let root = temp_root("round-trip");
        let registry = ArrobaMcpRegistry::new(vec![root.clone()]);
        let mut config = ArrobaMcpServerConfig::stdio(
            "browser",
            "npx",
            vec!["@playwright/mcp@latest".to_string()],
        );
        if let ArrobaMcpTransportConfig::Stdio { env_vars, .. } = &mut config.transport {
            env_vars.push("BROWSER_TOKEN".to_string());
        }

        let path = registry.install(&config).expect("install should succeed");
        assert_eq!(path, root.join("browser.json"));

        let listed = registry.list().expect("list should succeed");
        assert_eq!(listed, vec![config.clone()]);
        assert_eq!(registry.get("browser").unwrap(), Some(config));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn registry_updates_and_uninstalls_existing_mcp_config() {
        let root = temp_root("update-remove");
        let registry = ArrobaMcpRegistry::new(vec![root.clone()]);
        let original = ArrobaMcpServerConfig::stdio("browser", "npx", vec!["old".to_string()]);
        registry.install(&original).unwrap();

        let updated = ArrobaMcpServerConfig::stdio("browser", "node", vec!["new".to_string()]);
        let path = registry.update(&updated).expect("update should succeed");
        assert_eq!(path, root.join("browser.json"));
        assert_eq!(registry.get("browser").unwrap(), Some(updated));

        let removed = registry
            .uninstall("browser")
            .expect("uninstall should succeed");
        assert_eq!(removed, root.join("browser.json"));
        assert_eq!(registry.get("browser").unwrap(), None);
        assert!(!removed.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn imports_codex_mcp_servers_from_config() {
        let root = temp_root("codex-import-registry");
        let codex_root = temp_root("codex-import-config");
        fs::create_dir_all(&codex_root).unwrap();
        fs::write(
            codex_root.join("config.toml"),
            r#"
[mcp_servers.docs]
command = "docs-server"
args = ["--verbose"]
env_vars = ["DOCS_TOKEN"]
startup_timeout_sec = 2.2

[mcp_servers.docs.env]
ALPHA = "1"

[mcp_servers.web]
url = "https://example.test/mcp"
bearer_token_env_var = "WEB_TOKEN"
enabled_tools = ["search"]

[mcp_servers.oauth]
url = "https://example.test/oauth"
oauth_resource = "unsupported"
"#,
        )
        .unwrap();

        let registry = ArrobaMcpRegistry::new(vec![root.clone()]);
        let outcome = import_codex_mcp_servers_from_config_path(
            &registry,
            &codex_root.join("config.toml"),
            None,
        )
        .unwrap();

        assert_eq!(
            outcome
                .imported
                .iter()
                .map(|mcp| mcp.name.as_str())
                .collect::<Vec<_>>(),
            vec!["docs", "web"]
        );
        assert_eq!(outcome.skipped.len(), 1);
        assert_eq!(outcome.skipped[0].name, "oauth");
        assert!(outcome.skipped[0].reason.contains("oauth_resource"));
        let docs = registry.get("docs").unwrap().expect("docs import");
        assert_eq!(docs.startup_timeout_sec, Some(3));
        match docs.transport {
            ArrobaMcpTransportConfig::Stdio {
                command,
                args,
                env,
                env_vars,
                ..
            } => {
                assert_eq!(command, "docs-server");
                assert_eq!(args, vec!["--verbose"]);
                assert_eq!(env.get("ALPHA"), Some(&"1".to_string()));
                assert_eq!(env_vars, vec!["DOCS_TOKEN"]);
            }
            other => panic!("unexpected transport {other:?}"),
        }
        let web = registry.get("web").unwrap().expect("web import");
        assert_eq!(web.enabled_tools, Some(vec!["search".to_string()]));

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(codex_root);
    }

    #[test]
    fn imports_opencode_mcp_servers_from_jsonc_config() {
        let root = temp_root("opencode-import-registry");
        let opencode_root = temp_root("opencode-import-config");
        fs::create_dir_all(&opencode_root).unwrap();
        fs::write(
            opencode_root.join("opencode.jsonc"),
            r#"
{
  // OpenCode MCPs
      "mcp": {
        "docs": {
          "type": "local",
          "command": ["docs-server", "--verbose"],
          "environment": {
            "ALPHA": "1",
            "DOCS_TOKEN": "{env:DOCS_TOKEN}",
          },
          "timeout": 2500,
        },
        "web": {
          "type": "remote",
          "url": "https://example.test/mcp",
          "headers": {
            "X-Static": "42",
            "Authorization": "{env:WEB_TOKEN}",
          },
          "oauth": false,
          "enabled": false,
        },
        "oauth": {
          "type": "remote",
          "url": "https://example.test/oauth",
          "oauth": {},
        },
      },
}
"#,
        )
        .unwrap();

        let registry = ArrobaMcpRegistry::new(vec![root.clone()]);
        let outcome = import_opencode_mcp_servers_from_config_path(
            &registry,
            &opencode_root.join("opencode.jsonc"),
            None,
        )
        .unwrap();

        assert_eq!(
            outcome
                .imported
                .iter()
                .map(|mcp| mcp.name.as_str())
                .collect::<Vec<_>>(),
            vec!["docs", "web"]
        );
        assert_eq!(outcome.skipped.len(), 1);
        assert_eq!(outcome.skipped[0].name, "oauth");
        assert!(outcome.skipped[0].reason.contains("OAuth"));
        let docs = registry.get("docs").unwrap().expect("docs import");
        assert_eq!(docs.tool_timeout_sec, Some(3));
        match docs.transport {
            ArrobaMcpTransportConfig::Stdio {
                command,
                args,
                env,
                env_vars,
                ..
            } => {
                assert_eq!(command, "docs-server");
                assert_eq!(args, vec!["--verbose"]);
                assert_eq!(env.get("ALPHA"), Some(&"1".to_string()));
                assert_eq!(env_vars, vec!["DOCS_TOKEN"]);
            }
            other => panic!("unexpected transport {other:?}"),
        }
        let web = registry.get("web").unwrap().expect("web import");
        assert!(!web.enabled);
        match web.transport {
            ArrobaMcpTransportConfig::StreamableHttp {
                http_headers,
                env_http_headers,
                ..
            } => {
                assert_eq!(http_headers.get("X-Static"), Some(&"42".to_string()));
                assert_eq!(
                    env_http_headers.get("Authorization"),
                    Some(&"WEB_TOKEN".to_string())
                );
            }
            other => panic!("unexpected transport {other:?}"),
        }

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(opencode_root);
    }

    #[test]
    fn imports_claude_mcp_servers_from_config() {
        let root = temp_root("claude-import-registry");
        let workspace = temp_root("claude-import-workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            workspace.join(".mcp.json"),
            r#"
{
  "mcpServers": {
    "docs": {
      "type": "stdio",
      "command": "docs-server",
      "args": ["--verbose"],
      "env": {
        "ALPHA": "1",
        "DOCS_TOKEN": "{env:DOCS_TOKEN}"
      },
      "cwd": "/tmp/docs",
      "startup_timeout_sec": 2.2,
      "enabledTools": ["search"]
    },
    "web": {
      "type": "http",
      "url": "https://example.test/mcp",
      "headers": {
        "X-Static": "42",
        "Authorization": "{env:WEB_TOKEN}"
      },
      "disabledTools": ["write"]
    },
    "oauth": {
      "type": "sse",
      "url": "https://example.test/sse"
    },
    "inline_auth": {
      "type": "http",
      "url": "https://example.test/mcp",
      "headers": {
        "Authorization": "Bearer secret"
      }
    }
  }
}
"#,
        )
        .unwrap();

        let registry = ArrobaMcpRegistry::new(vec![root.clone()]);
        let outcome = import_claude_mcp_servers_from_config_path(
            &registry,
            &workspace.join(".mcp.json"),
            &workspace,
            None,
        )
        .unwrap();

        assert_eq!(
            outcome
                .imported
                .iter()
                .map(|mcp| mcp.name.as_str())
                .collect::<Vec<_>>(),
            vec!["docs", "web"]
        );
        let mut skipped_names = outcome
            .skipped
            .iter()
            .map(|skip| skip.name.as_str())
            .collect::<Vec<_>>();
        skipped_names.sort_unstable();
        assert_eq!(skipped_names, vec!["inline_auth", "oauth"]);
        assert!(outcome
            .skipped
            .iter()
            .any(|skip| skip.reason.contains("Authorization")));

        let docs = registry.get("docs").unwrap().expect("docs import");
        assert_eq!(docs.startup_timeout_sec, Some(3));
        assert_eq!(docs.enabled_tools, Some(vec!["search".to_string()]));
        match docs.transport {
            ArrobaMcpTransportConfig::Stdio {
                command,
                args,
                env,
                env_vars,
                cwd,
                ..
            } => {
                assert_eq!(command, "docs-server");
                assert_eq!(args, vec!["--verbose"]);
                assert_eq!(env.get("ALPHA"), Some(&"1".to_string()));
                assert_eq!(env_vars, vec!["DOCS_TOKEN"]);
                assert_eq!(cwd, Some(PathBuf::from("/tmp/docs")));
            }
            other => panic!("unexpected transport {other:?}"),
        }
        let web = registry.get("web").unwrap().expect("web import");
        assert_eq!(web.disabled_tools, Some(vec!["write".to_string()]));
        match web.transport {
            ArrobaMcpTransportConfig::StreamableHttp {
                http_headers,
                env_http_headers,
                ..
            } => {
                assert_eq!(http_headers.get("X-Static"), Some(&"42".to_string()));
                assert_eq!(
                    env_http_headers.get("Authorization"),
                    Some(&"WEB_TOKEN".to_string())
                );
            }
            other => panic!("unexpected transport {other:?}"),
        }

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn imports_matching_project_mcp_servers_from_claude_user_config() {
        let root = temp_root("claude-project-import-registry");
        let workspace = temp_root("claude-project-import-workspace");
        let config_root = temp_root("claude-project-import-config");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&config_root).unwrap();
        let config_path = config_root.join(".claude.json");
        fs::write(
            &config_path,
            format!(
                r#"{{
  "mcpServers": {{
    "global_docs": {{
      "command": "global-docs"
    }}
  }},
  "projects": {{
    "{}": {{
      "mcpServers": {{
        "project_docs": {{
          "command": "project-docs"
        }}
      }}
    }},
    "/elsewhere": {{
      "mcpServers": {{
        "other_docs": {{
          "command": "other-docs"
        }}
      }}
    }}
  }}
}}"#,
                workspace.display()
            ),
        )
        .unwrap();

        let registry = ArrobaMcpRegistry::new(vec![root.clone()]);
        let outcome =
            import_claude_mcp_servers_from_config_path(&registry, &config_path, &workspace, None)
                .unwrap();

        assert_eq!(
            outcome
                .imported
                .iter()
                .map(|mcp| mcp.name.as_str())
                .collect::<Vec<_>>(),
            vec!["global_docs", "project_docs"]
        );
        assert!(registry.get("other_docs").unwrap().is_none());

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(config_root);
    }

    #[test]
    fn rejects_invalid_mcp_names() {
        let config = ArrobaMcpServerConfig::stdio("../bad", "npx", Vec::new());
        assert!(config.validate().is_err());
    }
}
