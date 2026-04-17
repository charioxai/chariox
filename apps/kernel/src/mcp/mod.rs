use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArrobaMcpTransportConfig {
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env_vars: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
    },
    StreamableHttp {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_token_env_var: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        http_headers: BTreeMap<String, String>,
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
                http_headers: BTreeMap::new(),
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
}

impl ArrobaMcpRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn project_root(workspace: impl AsRef<Path>) -> PathBuf {
        workspace.as_ref().join(".arroba").join("mcps")
    }

    pub fn user_root() -> Option<PathBuf> {
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
        fs::write(&path, format!("{payload}\n")).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.install",
            message: format!("failed to write MCP `{}`: {error}", path.display()),
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
        for root in &self.roots {
            let path = root.join(format!("{name}.json"));
            if path.exists() {
                return Self::read_config(&path).map(Some);
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

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
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
    fn rejects_invalid_mcp_names() {
        let config = ArrobaMcpServerConfig::stdio("../bad", "npx", Vec::new());
        assert!(config.validate().is_err());
    }
}
