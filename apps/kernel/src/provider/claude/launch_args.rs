use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};
use crate::provider::{
    AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest, RuntimeProviderRun,
};

pub(crate) fn claude_launch_args_for_run(
    run: &RuntimeProviderRun,
    resume_session_id: Option<&str>,
) -> Result<Vec<String>, DaemonError> {
    claude_launch_args_from_parts(
        run.model(),
        run.variant(),
        run.execution_mode(),
        run.permission_level(),
        resume_session_id,
        run.runtime_mcp_server_url(),
        run.runtime_mcp_auth_token(),
        run.mcp_servers(),
    )
}

pub(super) fn claude_launch_args(
    request: &LaunchProviderRequest,
) -> Result<Vec<String>, DaemonError> {
    claude_launch_args_from_parts(
        request.model.as_str(),
        request.variant.as_deref(),
        request.execution_mode.unwrap_or_default(),
        request.permission_level.unwrap_or_default(),
        request
            .resume_state
            .as_ref()
            .and_then(|state| state.claude_session_id()),
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.server_url.as_str()),
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.auth_token.as_str()),
        &request.mcp_servers,
    )
}

fn claude_launch_args_from_parts(
    model: &str,
    variant: Option<&str>,
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
    resume_session_id: Option<&str>,
    runtime_mcp_server_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
    mcp_servers: &[ArrobaMcpServerConfig],
) -> Result<Vec<String>, DaemonError> {
    let mut args = vec![
        "-p".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
        "--replay-user-messages".to_string(),
    ];

    let model = normalized_claude_model(model);
    if !model.is_empty() && model != "default" {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(variant) = variant.map(str::trim).filter(|value| !value.is_empty()) {
        args.extend(["--effort".to_string(), variant.to_string()]);
    }
    if let Some(session_id) = resume_session_id {
        args.extend(["--resume".to_string(), session_id.to_string()]);
    }
    if let Some(config) =
        claude_mcp_config(mcp_servers, runtime_mcp_server_url, runtime_mcp_auth_token)?
    {
        args.extend(["--mcp-config".to_string(), config]);
        args.push("--strict-mcp-config".to_string());
    }

    match (execution_mode, permission_level) {
        (AgentExecutionMode::Plan, _) => {
            args.extend(["--permission-mode".to_string(), "plan".to_string()]);
        }
        (AgentExecutionMode::Build, AgentPermissionLevel::Required) => {
            args.extend(["--permission-mode".to_string(), "default".to_string()]);
        }
        (AgentExecutionMode::Build, AgentPermissionLevel::Yolo) => {
            args.extend([
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--allow-dangerously-skip-permissions".to_string(),
            ]);
        }
    }

    Ok(args)
}

pub(super) fn claude_mcp_config(
    backing_servers: &[ArrobaMcpServerConfig],
    runtime_mcp_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
) -> Result<Option<String>, DaemonError> {
    let mut mcp_servers = serde_json::Map::new();
    let provider_mcp_servers = super::super::mcp_proxy::provider_facing_mcp_proxy_configs(
        backing_servers,
        runtime_mcp_url,
        runtime_mcp_auth_token,
    )?;
    for server in &provider_mcp_servers {
        mcp_servers.insert(server.name.clone(), claude_mcp_server_config(server));
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
    serde_json::to_string(&serde_json::json!({ "mcpServers": mcp_servers }))
        .map(Some)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "claude_mcp_config",
            message: error.to_string(),
        })
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

pub(super) fn normalized_claude_model(model: &str) -> String {
    model
        .trim()
        .strip_prefix("claude/")
        .unwrap_or_else(|| model.trim())
        .to_string()
}
