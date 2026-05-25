use std::collections::BTreeMap;
use std::env;

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};
use crate::provider::LaunchProviderRequest;

const OPENCODE_CONFIG_CONTENT_ENV: &str = "OPENCODE_CONFIG_CONTENT";

pub(super) fn runtime_mcp_env(
    request: Option<&LaunchProviderRequest>,
) -> Result<BTreeMap<String, String>, DaemonError> {
    let mut env = BTreeMap::new();
    let Some(request) = request else {
        return Ok(env);
    };
    let mut config = serde_json::Map::new();
    let mut mcp = serde_json::Map::new();
    let provider_mcp_servers = super::super::mcp_proxy::provider_facing_mcp_proxy_configs(
        &request.mcp_servers,
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.server_url.as_str()),
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.auth_token.as_str()),
    )?;
    for server in &provider_mcp_servers {
        mcp.insert(server.name.clone(), opencode_mcp_config(server));
    }
    if let Some(binding) = request.runtime_mcp_binding.as_ref() {
        mcp.insert(
            "arroba".to_string(),
            serde_json::json!({
                "type": "remote",
                "url": binding.server_url,
                "enabled": true,
                "oauth": false,
                "timeout": 45_000,
                "headers": {
                    "Authorization": format!("Bearer {}", binding.auth_token),
                }
            }),
        );
    }
    if !mcp.is_empty() {
        config.insert("mcp".to_string(), serde_json::Value::Object(mcp));
    }
    if !config.is_empty() {
        env.insert(
            OPENCODE_CONFIG_CONTENT_ENV.to_string(),
            serde_json::Value::Object(config).to_string(),
        );
    }
    Ok(env)
}

fn opencode_mcp_config(server: &ArrobaMcpServerConfig) -> serde_json::Value {
    match &server.transport {
        ArrobaMcpTransportConfig::Stdio {
            command,
            args,
            env: static_env,
            credential_env: _,
            env_vars,
            cwd,
        } => {
            let mut environment = static_env.clone();
            for name in env_vars {
                if let Ok(value) = env::var(name) {
                    environment.insert(name.clone(), value);
                }
            }
            let command_parts = std::iter::once(command.clone())
                .chain(args.iter().cloned())
                .collect::<Vec<_>>();
            let mut config = serde_json::json!({
                "type": "local",
                "command": command_parts,
                "enabled": server.enabled,
                "environment": environment,
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
                if let Ok(value) = env::var(env_var) {
                    headers.insert(header.clone(), value);
                }
            }
            if let Some(env_var) = bearer_token_env_var {
                if let Ok(value) = env::var(env_var) {
                    headers.insert("Authorization".to_string(), format!("Bearer {value}"));
                }
            }
            serde_json::json!({
                "type": "remote",
                "url": url,
                "enabled": server.enabled,
                "oauth": false,
                "timeout": opencode_mcp_timeout_ms(server),
                "headers": headers,
            })
        }
    }
}

fn opencode_mcp_timeout_ms(server: &ArrobaMcpServerConfig) -> u64 {
    server
        .tool_timeout_sec
        .or(server.startup_timeout_sec)
        .unwrap_or(45)
        .saturating_mul(1_000)
}
