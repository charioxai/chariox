use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};
use crate::provider::{LaunchProviderRequest, RuntimeProviderRun};
use zeroize::{Zeroize, Zeroizing};

pub(crate) const CLAUDE_MCP_CONFIG_PLACEHOLDER: &str = "arroba://claude-mcp-config";
const CLAUDE_RUNTIME_FILES_PREFIX: &str = "arroba-claude-remote-native-";
const CLAUDE_MCP_CONFIG_FILE_NAME: &str = "mcp-config.json";

pub(super) struct ClaudeRuntimeFilesRoot {
    path: PathBuf,
    cleanup_on_drop: bool,
}

impl ClaudeRuntimeFilesRoot {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn persist_for_launch(&mut self) {
        self.cleanup_on_drop = false;
    }
}

impl Drop for ClaudeRuntimeFilesRoot {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            cleanup_claude_runtime_files_root(&self.path);
        }
    }
}

pub(crate) struct ClaudeMcpConfigFile {
    _root: ClaudeRuntimeFilesRoot,
    path: PathBuf,
}

impl ClaudeMcpConfigFile {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Debug for ClaudeMcpConfigFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeMcpConfigFile")
            .field("path", &self.path)
            .finish()
    }
}

pub(super) fn request_has_claude_mcp_config(
    request: &LaunchProviderRequest,
) -> Result<bool, DaemonError> {
    if let Some(binding) = request.runtime_mcp_binding.as_ref() {
        for server in &request.mcp_servers {
            claude_provider_facing_mcp_proxy_url(server, &binding.server_url)?;
        }
    }
    Ok(request.runtime_mcp_binding.is_some() || !request.mcp_servers.is_empty())
}

pub(super) fn materialize_request_claude_mcp_config(
    request: &LaunchProviderRequest,
    root: &ClaudeRuntimeFilesRoot,
) -> Result<Option<PathBuf>, DaemonError> {
    materialize_claude_mcp_config(
        root,
        &request.mcp_servers,
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.server_url.as_str()),
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.auth_token.as_str()),
    )
}

pub(crate) fn materialize_runtime_claude_mcp_config(
    run: &RuntimeProviderRun,
) -> Result<Option<ClaudeMcpConfigFile>, DaemonError> {
    let root = create_claude_runtime_files_root()?;
    let Some(path) = materialize_claude_mcp_config(
        &root,
        run.mcp_servers(),
        run.runtime_mcp_server_url(),
        run.runtime_mcp_auth_token(),
    )?
    else {
        return Ok(None);
    };
    Ok(Some(ClaudeMcpConfigFile { _root: root, path }))
}

pub(super) fn create_claude_runtime_files_root() -> Result<ClaudeRuntimeFilesRoot, DaemonError> {
    for _ in 0..16 {
        let root = std::env::temp_dir().join(format!(
            "{CLAUDE_RUNTIME_FILES_PREFIX}{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let create_result = {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            builder.create(&root)
        };
        match create_result {
            Ok(()) => {
                #[cfg(unix)]
                if let Err(error) = fs::set_permissions(&root, fs::Permissions::from_mode(0o700)) {
                    let _ = fs::remove_dir_all(&root);
                    return Err(DaemonError::LocalTransport {
                        operation: "prepare claude runtime files root",
                        message: error.to_string(),
                    });
                }
                return Ok(ClaudeRuntimeFilesRoot {
                    path: root,
                    cleanup_on_drop: true,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(DaemonError::LocalTransport {
                    operation: "prepare claude runtime files root",
                    message: error.to_string(),
                });
            }
        }
    }
    Err(DaemonError::LocalTransport {
        operation: "prepare claude runtime files root",
        message: "failed to allocate a unique Claude runtime files root".to_string(),
    })
}

fn cleanup_claude_runtime_files_root(root: &Path) {
    if root.parent() != Some(std::env::temp_dir().as_path())
        || !root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(CLAUDE_RUNTIME_FILES_PREFIX))
    {
        return;
    }
    let _ = fs::remove_dir_all(root);
}

fn materialize_claude_mcp_config(
    root: &ClaudeRuntimeFilesRoot,
    backing_servers: &[ArrobaMcpServerConfig],
    runtime_mcp_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
) -> Result<Option<PathBuf>, DaemonError> {
    let Some(config) = claude_mcp_config(backing_servers, runtime_mcp_url, runtime_mcp_auth_token)?
    else {
        return Ok(None);
    };
    validate_claude_runtime_files_root(root.path())?;
    let path = root.path().join(CLAUDE_MCP_CONFIG_FILE_NAME);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "materialize claude mcp config",
            message: error.to_string(),
        })?;
    let write_result = file
        .write_all(config.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all());
    if let Err(error) = write_result {
        let _ = fs::remove_file(&path);
        return Err(DaemonError::LocalTransport {
            operation: "materialize claude mcp config",
            message: error.to_string(),
        });
    }
    #[cfg(unix)]
    if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(&path);
        return Err(DaemonError::LocalTransport {
            operation: "materialize claude mcp config",
            message: error.to_string(),
        });
    }
    Ok(Some(path))
}

fn validate_claude_runtime_files_root(root: &Path) -> Result<(), DaemonError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| DaemonError::LocalTransport {
        operation: "materialize claude mcp config",
        message: error.to_string(),
    })?;
    if !metadata.file_type().is_dir() {
        return Err(DaemonError::LocalTransport {
            operation: "materialize claude mcp config",
            message: "Claude runtime files root must be a directory".to_string(),
        });
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(DaemonError::LocalTransport {
            operation: "materialize claude mcp config",
            message: "Claude runtime files root must not grant group or other access".to_string(),
        });
    }
    Ok(())
}

fn claude_mcp_config(
    backing_servers: &[ArrobaMcpServerConfig],
    runtime_mcp_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
) -> Result<Option<Zeroizing<String>>, DaemonError> {
    let proxy_urls = match (runtime_mcp_url, runtime_mcp_auth_token) {
        (Some(url), Some(_)) => Some(
            backing_servers
                .iter()
                .map(|server| claude_provider_facing_mcp_proxy_url(server, url))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        _ => None,
    };
    let mut mcp_servers = serde_json::Map::new();
    for (index, server) in backing_servers.iter().enumerate() {
        let config = match (proxy_urls.as_ref(), runtime_mcp_auth_token) {
            (Some(proxy_urls), Some(token)) => {
                serde_json::json!({
                    "type": "http",
                    "url": &proxy_urls[index],
                    "headers": {
                        "Authorization": format!("Bearer {token}"),
                    },
                })
            }
            _ => claude_mcp_server_config(server),
        };
        mcp_servers.insert(server.name.clone(), config);
    }
    if let (Some(url), Some(token)) = (runtime_mcp_url, runtime_mcp_auth_token) {
        mcp_servers.insert(
            "arroba".to_string(),
            serde_json::json!({
                "type": "http",
                "url": url,
                "headers": {
                    "Authorization": format!("Bearer {token}"),
                },
            }),
        );
    }
    if mcp_servers.is_empty() {
        return Ok(None);
    }
    let payload = SensitiveJsonValue(serde_json::json!({ "mcpServers": mcp_servers }));
    serde_json::to_string(&payload.0)
        .map(Zeroizing::new)
        .map(Some)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "claude_mcp_config",
            message: error.to_string(),
        })
}

fn claude_provider_facing_mcp_proxy_url(
    server: &ArrobaMcpServerConfig,
    runtime_mcp_url: &str,
) -> Result<String, DaemonError> {
    let proxy_url =
        super::super::mcp_proxy::provider_facing_mcp_proxy_url(runtime_mcp_url, &server.name)?;
    ArrobaMcpServerConfig::streamable_http(&server.name, &proxy_url).validate()?;
    Ok(proxy_url)
}

struct SensitiveJsonValue(serde_json::Value);

impl Drop for SensitiveJsonValue {
    fn drop(&mut self) {
        zeroize_json_strings(&mut self.0);
    }
}

fn zeroize_json_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(value) => value.zeroize(),
        serde_json::Value::Array(values) => {
            for value in values {
                zeroize_json_strings(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                zeroize_json_strings(value);
            }
        }
        _ => {}
    }
}

fn claude_mcp_server_config(server: &ArrobaMcpServerConfig) -> serde_json::Value {
    match &server.transport {
        ArrobaMcpTransportConfig::Stdio {
            command,
            args,
            env,
            credential_env: _,
            env_vars,
            cwd,
        } => {
            let mut resolved_env = env.clone();
            for name in env_vars {
                if let Ok(value) = std::env::var(name) {
                    resolved_env.insert(name.clone(), value);
                }
            }
            let mut config = serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": args,
                "env": resolved_env,
            });
            if let Some(cwd) = cwd {
                config["cwd"] = serde_json::Value::String(cwd.display().to_string());
            }
            config
        }
        ArrobaMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            bearer_token_credential: _,
            http_headers,
            credential_http_headers: _,
            env_http_headers,
        } => {
            let mut headers = http_headers.clone();
            for (header, env_var) in env_http_headers {
                if let Ok(value) = std::env::var(env_var) {
                    headers.insert(header.clone(), value);
                }
            }
            if let Some(env_var) = bearer_token_env_var {
                if let Ok(value) = std::env::var(env_var) {
                    headers.insert("Authorization".to_string(), format!("Bearer {value}"));
                }
            }
            serde_json::json!({
                "type": "http",
                "url": url,
                "headers": headers,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use crate::mcp::ArrobaMcpServerConfig;
    use crate::provider::{LaunchProviderRequest, RuntimeMcpBinding};

    use super::{create_claude_runtime_files_root, materialize_request_claude_mcp_config};

    #[test]
    fn materializes_private_mcp_config_and_cleans_its_root() {
        let root = create_claude_runtime_files_root().expect("runtime root should be created");
        let root_path = root.path().to_path_buf();
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "private-token",
        ));

        let path = materialize_request_claude_mcp_config(&request, &root)
            .expect("config should materialize")
            .expect("config path should exist");

        #[cfg(unix)]
        {
            assert_eq!(
                std::fs::metadata(&root_path)
                    .expect("root metadata should exist")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&path)
                    .expect("config metadata should exist")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let payload = std::fs::read_to_string(&path).expect("config should be readable by owner");
        assert!(payload.contains("Bearer private-token"));

        drop(root);
        assert!(!root_path.exists());
    }

    #[test]
    fn materializes_granted_mcp_through_the_runtime_proxy() {
        let root = create_claude_runtime_files_root().expect("runtime root should be created");
        let root_path = root.path().to_path_buf();
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "private-token",
        ))
        .with_mcp_servers(vec![ArrobaMcpServerConfig::stdio(
            "browser",
            "npx",
            vec!["@playwright/mcp@latest".to_string()],
        )]);

        let path = materialize_request_claude_mcp_config(&request, &root)
            .expect("config should materialize")
            .expect("config path should exist");
        let payload: serde_json::Value = serde_json::from_slice(
            &std::fs::read(path).expect("config should be readable by owner"),
        )
        .expect("config should be valid JSON");

        assert_eq!(
            payload.pointer("/mcpServers/browser/url"),
            Some(&serde_json::json!(
                "http://127.0.0.1:43120/mcp/proxy/browser"
            ))
        );
        assert_eq!(
            payload.pointer("/mcpServers/browser/headers/Authorization"),
            Some(&serde_json::json!("Bearer private-token"))
        );
        assert!(payload.pointer("/mcpServers/browser/command").is_none());

        drop(root);
        assert!(!root_path.exists());
    }

    #[test]
    fn runtime_root_guard_cleans_partial_setup() {
        let root = create_claude_runtime_files_root().expect("runtime root should be created");
        let root_path = root.path().to_path_buf();
        std::fs::write(root_path.join("partial-file"), "partial")
            .expect("partial setup file should write");

        drop(root);

        assert!(!root_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_materialize_credentials_in_a_broadened_root() {
        let root = create_claude_runtime_files_root().expect("runtime root should be created");
        let root_path = root.path().to_path_buf();
        std::fs::set_permissions(&root_path, std::fs::Permissions::from_mode(0o755))
            .expect("test should broaden root permissions");
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude",
            "default",
            "claude-sonnet-4-6",
        )
        .with_runtime_mcp_binding(RuntimeMcpBinding::new(
            "http://127.0.0.1:43120/mcp",
            "private-token",
        ));

        let error = materialize_request_claude_mcp_config(&request, &root)
            .expect_err("insecure root must be rejected");

        assert!(error
            .to_string()
            .contains("must not grant group or other access"));
        assert!(!root_path.join("mcp-config.json").exists());
        drop(root);
        assert!(!root_path.exists());
    }
}
