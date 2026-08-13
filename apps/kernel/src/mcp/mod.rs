use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

mod provider_config;
mod provider_import;
#[cfg(test)]
mod tests;

pub use provider_import::{
    discover_claude_mcp_candidates_from_config_path,
    discover_codex_mcp_candidates_from_config_path,
    discover_opencode_mcp_candidates_from_config_path, discover_provider_mcp_import_candidates,
    import_claude_mcp_servers, import_claude_mcp_servers_from_config_path,
    import_codex_mcp_servers, import_codex_mcp_servers_from_config_path,
    import_opencode_mcp_servers, import_opencode_mcp_servers_from_config_path, McpImportOutcome,
    McpImportSkip, ProviderMcpImportCandidate, ProviderMcpImportDiscovery,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharioxMcpServerConfig {
    pub name: String,
    pub transport: CharioxMcpTransportConfig,
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
    pub tools: BTreeMap<String, CharioxMcpToolConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharioxMcpCredentialBinding {
    pub credential: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CharioxMcpTransportConfig {
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        credential_env: BTreeMap<String, CharioxMcpCredentialBinding>,
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
        credential_http_headers: BTreeMap<String, CharioxMcpCredentialBinding>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env_http_headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharioxMcpToolConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharioxMcpRegistry {
    roots: Vec<PathBuf>,
}

impl CharioxMcpServerConfig {
    pub fn stdio(name: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            transport: CharioxMcpTransportConfig::Stdio {
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
            transport: CharioxMcpTransportConfig::StreamableHttp {
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
            CharioxMcpTransportConfig::Stdio { command, .. } => {
                if command.trim().is_empty() {
                    return invalid("mcp command", "must not be empty");
                }
            }
            CharioxMcpTransportConfig::StreamableHttp { url, .. } => {
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
            CharioxMcpTransportConfig::Stdio {
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
            CharioxMcpTransportConfig::StreamableHttp {
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

impl CharioxMcpRegistry {
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
        workspace.as_ref().join(".chariox").join("mcps")
    }

    pub fn user_root() -> Option<PathBuf> {
        if let Some(root) = managed_capability_root() {
            return Some(root.join("user").join("mcps"));
        }
        home_dir().map(|home| home.join(".chariox").join("mcps"))
    }

    pub fn install(&self, config: &CharioxMcpServerConfig) -> Result<PathBuf, DaemonError> {
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

    pub fn update(&self, config: &CharioxMcpServerConfig) -> Result<PathBuf, DaemonError> {
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

    pub fn list(&self) -> Result<Vec<CharioxMcpServerConfig>, DaemonError> {
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

    pub fn get(&self, name: &str) -> Result<Option<CharioxMcpServerConfig>, DaemonError> {
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

    fn read_config(path: &Path) -> Result<CharioxMcpServerConfig, DaemonError> {
        let payload = fs::read_to_string(path).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.read",
            message: format!("failed to read MCP `{}`: {error}", path.display()),
        })?;
        let config: CharioxMcpServerConfig =
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
    std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn workspace_registry_hash(workspace: &Path) -> String {
    let digest = Sha256::digest(workspace.to_string_lossy().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
