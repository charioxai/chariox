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
    fn rejects_invalid_mcp_names() {
        let config = ArrobaMcpServerConfig::stdio("../bad", "npx", Vec::new());
        assert!(config.validate().is_err());
    }
}
