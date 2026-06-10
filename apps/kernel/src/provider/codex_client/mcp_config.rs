//! Codex `config` override mapping for runtime and granted MCP servers.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};

pub(super) fn append_runtime_mcp_overrides(
    overrides: &mut BTreeMap<String, Value>,
    server_url: &str,
    auth_token: &str,
) {
    overrides.insert(
        "mcp_servers.arroba.url".to_string(),
        json!(server_url.to_string()),
    );
    overrides.insert(
        "mcp_servers.arroba.http_headers.Authorization".to_string(),
        json!(format!("Bearer {auth_token}")),
    );
    overrides.insert("mcp_servers.arroba.required".to_string(), json!(true));
    overrides.insert(
        "mcp_servers.arroba.startup_timeout_sec".to_string(),
        json!(90),
    );
    overrides.insert(
        "mcp_servers.arroba.tool_timeout_sec".to_string(),
        json!(300),
    );
}

pub(super) fn append_codex_mcp_overrides(
    overrides: &mut BTreeMap<String, Value>,
    servers: &[ArrobaMcpServerConfig],
) {
    for server in servers {
        let prefix = format!("mcp_servers.{}", server.name);
        match &server.transport {
            ArrobaMcpTransportConfig::Stdio {
                command,
                args,
                env,
                credential_env: _,
                env_vars,
                cwd,
            } => {
                overrides.insert(format!("{prefix}.command"), json!(command));
                if !args.is_empty() {
                    overrides.insert(format!("{prefix}.args"), json!(args));
                }
                for (key, value) in env {
                    overrides.insert(format!("{prefix}.env.{key}"), json!(value));
                }
                if !env_vars.is_empty() {
                    overrides.insert(format!("{prefix}.env_vars"), json!(env_vars));
                }
                if let Some(cwd) = cwd {
                    overrides.insert(format!("{prefix}.cwd"), json!(cwd.display().to_string()));
                }
            }
            ArrobaMcpTransportConfig::StreamableHttp {
                url,
                bearer_token_env_var,
                bearer_token_credential: _,
                http_headers,
                credential_http_headers: _,
                env_http_headers,
            } => {
                overrides.insert(format!("{prefix}.url"), json!(url));
                if let Some(env_var) = bearer_token_env_var {
                    overrides.insert(format!("{prefix}.bearer_token_env_var"), json!(env_var));
                }
                for (key, value) in http_headers {
                    overrides.insert(format!("{prefix}.http_headers.{key}"), json!(value));
                }
                for (key, value) in env_http_headers {
                    overrides.insert(format!("{prefix}.env_http_headers.{key}"), json!(value));
                }
            }
        }
        overrides.insert(format!("{prefix}.enabled"), json!(server.enabled));
        if server.required {
            overrides.insert(format!("{prefix}.required"), json!(true));
        }
        if let Some(timeout) = server.startup_timeout_sec {
            overrides.insert(format!("{prefix}.startup_timeout_sec"), json!(timeout));
        }
        if let Some(timeout) = server.tool_timeout_sec {
            overrides.insert(format!("{prefix}.tool_timeout_sec"), json!(timeout));
        }
        if let Some(enabled_tools) = &server.enabled_tools {
            overrides.insert(format!("{prefix}.enabled_tools"), json!(enabled_tools));
        }
        if let Some(disabled_tools) = &server.disabled_tools {
            overrides.insert(format!("{prefix}.disabled_tools"), json!(disabled_tools));
        }
    }
}
