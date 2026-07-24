use crate::error::DaemonError;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest};

use super::mcp_config::{request_has_claude_mcp_config, CLAUDE_MCP_CONFIG_PLACEHOLDER};

pub(super) fn claude_launch_args(
    request: &LaunchProviderRequest,
) -> Result<Vec<String>, DaemonError> {
    let mut args = claude_launch_args_from_parts(
        request.model.as_str(),
        request.variant.as_deref(),
        request.execution_mode.unwrap_or_default(),
        request.permission_level.unwrap_or_default(),
        request
            .resume_state
            .as_ref()
            .and_then(|state| state.claude_session_id()),
        request_has_claude_mcp_config(request)?,
        request.runtime_mcp_binding.is_some(),
    )?;
    if request_uses_metaagent_tools_only(request) {
        args.extend(["--tools".to_string(), String::new()]);
    }
    Ok(args)
}

pub(super) fn request_uses_metaagent_tools_only(request: &LaunchProviderRequest) -> bool {
    request
        .provider_config_overrides
        .get("arroba.metaagent_tools_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn claude_launch_args_from_parts(
    model: &str,
    variant: Option<&str>,
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
    resume_session_id: Option<&str>,
    has_mcp_config: bool,
    has_runtime_mcp_binding: bool,
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
    if has_mcp_config {
        args.extend([
            "--mcp-config".to_string(),
            CLAUDE_MCP_CONFIG_PLACEHOLDER.to_string(),
        ]);
        args.push("--strict-mcp-config".to_string());
        if has_runtime_mcp_binding {
            args.extend(["--disallowedTools".to_string(), "ToolSearch".to_string()]);
        }
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

pub(super) fn normalized_claude_model(model: &str) -> String {
    let model = model.trim();
    for prefix in ["claude/", "claude-headless/", "claude-p/"] {
        if let Some(stripped) = model.strip_prefix(prefix) {
            return stripped.to_string();
        }
    }
    model.to_string()
}
