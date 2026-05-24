use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};
use crate::provider::LaunchProviderRequest;

use super::CODEX_MCP_TOKEN_ENV;

const CODEX_AUTH_ENV_VARS: &[&str] = &[
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_ORGANIZATION",
    "OPENAI_PROJECT",
];

pub(super) fn runtime_mcp_config(
    request: Option<&LaunchProviderRequest>,
) -> Result<(Vec<String>, BTreeMap<String, String>), DaemonError> {
    let Some(request) = request else {
        return Ok((Vec::new(), inherited_codex_auth_env()));
    };
    if request.runtime_mcp_binding.is_none() && request.mcp_servers.is_empty() {
        return Ok((Vec::new(), inherited_codex_auth_env()));
    }
    let mut args = vec!["-c".to_string(), "mcp_servers={}".to_string()];
    if request.requires_managed_io() {
        let model_catalog_path = write_managed_io_model_catalog(request.model.as_str())?;
        args.splice(
            0..0,
            [
                "-c".to_string(),
                format!("model_catalog_json={:?}", model_catalog_path),
                "-c".to_string(),
                "features.apply_patch_freeform=false".to_string(),
                "-c".to_string(),
                "include_apply_patch_tool=false".to_string(),
                "-c".to_string(),
                "approval_policy=\"never\"".to_string(),
            ],
        );
    }
    let mut env = inherited_codex_auth_env();
    let provider_mcp_servers =
        super::super::mcp_proxy::provider_facing_mcp_proxy_configs_with_bearer_env(
            &request.mcp_servers,
            request
                .runtime_mcp_binding
                .as_ref()
                .map(|binding| binding.server_url.as_str()),
            request
                .runtime_mcp_binding
                .as_ref()
                .map(|binding| binding.auth_token.as_str()),
            CODEX_MCP_TOKEN_ENV,
        )?;
    for server in &provider_mcp_servers {
        append_codex_mcp_config(&mut args, server);
    }
    if let Some(binding) = request.runtime_mcp_binding.as_ref() {
        args.extend([
            "-c".to_string(),
            format!("mcp_servers.arroba.url={:?}", binding.server_url),
            "-c".to_string(),
            format!(
                "mcp_servers.arroba.bearer_token_env_var={:?}",
                CODEX_MCP_TOKEN_ENV
            ),
            "-c".to_string(),
            "mcp_servers.arroba.required=true".to_string(),
            "-c".to_string(),
            "mcp_servers.arroba.tool_timeout_sec=15".to_string(),
        ]);
        env.insert(CODEX_MCP_TOKEN_ENV.to_string(), binding.auth_token.clone());
    }
    Ok((args, env))
}

fn inherited_codex_auth_env() -> BTreeMap<String, String> {
    CODEX_AUTH_ENV_VARS
        .iter()
        .filter_map(|name| {
            env::var(name)
                .ok()
                .map(|value| ((*name).to_string(), value))
        })
        .collect()
}

fn append_codex_mcp_config(args: &mut Vec<String>, server: &ArrobaMcpServerConfig) {
    let prefix = format!("mcp_servers.{}", server.name);
    match &server.transport {
        ArrobaMcpTransportConfig::Stdio {
            command,
            args: server_args,
            env,
            credential_env: _,
            env_vars,
            cwd,
        } => {
            push_codex_config(args, format!("{prefix}.command={command:?}"));
            if !server_args.is_empty() {
                push_codex_config(args, format!("{prefix}.args={server_args:?}"));
            }
            for (key, value) in env {
                push_codex_config(args, format!("{prefix}.env.{key}={value:?}"));
            }
            if !env_vars.is_empty() {
                push_codex_config(args, format!("{prefix}.env_vars={env_vars:?}"));
            }
            if let Some(cwd) = cwd {
                push_codex_config(
                    args,
                    format!("{prefix}.cwd={:?}", cwd.display().to_string()),
                );
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
            let mut fields = vec![format!("url={url:?}")];
            if let Some(env_var) = bearer_token_env_var {
                fields.push(format!("bearer_token_env_var={env_var:?}"));
            }
            if !http_headers.is_empty() {
                fields.push(format!(
                    "http_headers={{{}}}",
                    http_headers
                        .iter()
                        .map(|(key, value)| format!("{key:?}={value:?}"))
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            if !env_http_headers.is_empty() {
                fields.push(format!(
                    "env_http_headers={{{}}}",
                    env_http_headers
                        .iter()
                        .map(|(key, value)| format!("{key:?}={value:?}"))
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            if server.required {
                fields.push("required=true".to_string());
            }
            if let Some(timeout) = server.startup_timeout_sec {
                fields.push(format!("startup_timeout_sec={timeout}"));
            }
            if let Some(timeout) = server.tool_timeout_sec {
                fields.push(format!("tool_timeout_sec={timeout}"));
            }
            if let Some(tools) = &server.enabled_tools {
                fields.push(format!("enabled_tools={tools:?}"));
            }
            if let Some(tools) = &server.disabled_tools {
                fields.push(format!("disabled_tools={tools:?}"));
            }
            push_codex_config(args, format!("{prefix}={{{}}}", fields.join(",")));
            return;
        }
    }
    if server.required {
        push_codex_config(args, format!("{prefix}.required=true"));
    }
    if let Some(timeout) = server.startup_timeout_sec {
        push_codex_config(args, format!("{prefix}.startup_timeout_sec={timeout}"));
    }
    if let Some(timeout) = server.tool_timeout_sec {
        push_codex_config(args, format!("{prefix}.tool_timeout_sec={timeout}"));
    }
}

fn push_codex_config(args: &mut Vec<String>, value: String) {
    args.push("-c".to_string());
    args.push(value);
}

fn write_managed_io_model_catalog(model: &str) -> Result<PathBuf, DaemonError> {
    let slug = model.rsplit('/').next().unwrap_or(model);
    let catalog = serde_json::json!({
        "models": [{
            "slug": slug,
            "display_name": slug,
            "description": "Arroba managed-I/O model metadata overlay",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [
                { "effort": "low", "description": "Fast responses with lighter reasoning" },
                { "effort": "medium", "description": "Balanced reasoning" },
                { "effort": "high", "description": "Greater reasoning depth" }
            ],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 0,
            "availability_nux": null,
            "upgrade": null,
            "base_instructions": "You are Codex, a coding agent. Follow the user instructions and use available tools exactly as requested.",
            "supports_reasoning_summaries": true,
            "default_reasoning_summary": "auto",
            "support_verbosity": true,
            "default_verbosity": "low",
            "apply_patch_tool_type": null,
            "web_search_tool_type": "text",
            "truncation_policy": { "mode": "tokens", "limit": 10000 },
            "supports_parallel_tool_calls": true,
            "supports_image_detail_original": true,
            "context_window": 272000,
            "effective_context_window_percent": 95,
            "experimental_supported_tools": [],
            "input_modalities": ["text", "image"]
        }]
    });
    let mut hasher = DefaultHasher::new();
    model.hash(&mut hasher);
    let path = env::temp_dir().join(format!(
        "arroba-codex-managed-io-models-{:x}.json",
        hasher.finish()
    ));
    let content = serde_json::to_string(&catalog).map_err(|error| DaemonError::LocalTransport {
        operation: "codex_managed_io_model_catalog",
        message: error.to_string(),
    })?;
    fs::write(&path, content).map_err(|error| DaemonError::LocalTransport {
        operation: "codex_managed_io_model_catalog",
        message: format!("failed to write managed-I/O Codex model catalog: {error}"),
    })?;
    Ok(path)
}
