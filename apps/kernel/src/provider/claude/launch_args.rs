use crate::error::DaemonError;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest};

use super::mcp_config::{request_has_claude_mcp_config, CLAUDE_MCP_CONFIG_PLACEHOLDER};

/// Claude Code names MCP tools `mcp__<server>__<tool>` with characters outside
/// `[A-Za-z0-9_-]` replaced by `_`; the runtime server is named `chariox`.
pub(crate) const CLAUDE_PERMISSION_PROMPT_TOOL: &str = "mcp__chariox__chariox_permission_prompt";
const CLAUDE_RUNTIME_TOOLS_PATTERN: &str = "mcp__chariox__*";

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
        .get("chariox.metaagent_tools_only")
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

    append_claude_execution_config_args(
        &mut args,
        execution_mode,
        permission_level,
        has_mcp_config && has_runtime_mcp_binding,
    );

    Ok(args)
}

pub(crate) fn claude_args_with_execution_config(
    args: &[String],
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
    has_runtime_mcp: bool,
) -> Vec<String> {
    let mut args = claude_args_without_execution_config(args);
    append_claude_execution_config_args(
        &mut args,
        execution_mode,
        permission_level,
        has_runtime_mcp,
    );
    args
}

fn claude_args_without_execution_config(args: &[String]) -> Vec<String> {
    let mut sanitized = Vec::with_capacity(args.len());
    let mut skip_next = false;
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(
            arg.as_str(),
            "--permission-mode" | "--permission-prompt-tool"
        ) {
            skip_next = true;
            continue;
        }
        if arg == "--allowedTools"
            && args.peek().map(|value| value.as_str()) == Some(CLAUDE_RUNTIME_TOOLS_PATTERN)
        {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--permission-mode=")
            || matches!(
                arg.as_str(),
                "--allow-dangerously-skip-permissions" | "--dangerously-skip-permissions"
            )
        {
            continue;
        }
        sanitized.push(arg.clone());
    }
    sanitized
}

fn append_claude_execution_config_args(
    args: &mut Vec<String>,
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
    has_runtime_mcp: bool,
) {
    match (execution_mode, permission_level) {
        (AgentExecutionMode::Plan, _) => {
            args.extend(["--permission-mode".to_string(), "plan".to_string()]);
        }
        (AgentExecutionMode::Build, AgentPermissionLevel::Required) => {
            args.extend(["--permission-mode".to_string(), "default".to_string()]);
            // Print mode cannot show Claude's own approval prompt. Route it
            // to the kernel-owned runtime interaction instead, and pre-allow
            // Chariox runtime tools as the native TUI launch does.
            if has_runtime_mcp {
                args.extend([
                    "--permission-prompt-tool".to_string(),
                    CLAUDE_PERMISSION_PROMPT_TOOL.to_string(),
                    "--allowedTools".to_string(),
                    CLAUDE_RUNTIME_TOOLS_PATTERN.to_string(),
                ]);
            }
        }
        (AgentExecutionMode::Build, AgentPermissionLevel::Yolo) => {
            args.extend([
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
                "--allow-dangerously-skip-permissions".to_string(),
            ]);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
        has_runtime_mcp: bool,
    ) -> Vec<String> {
        claude_launch_args_from_parts(
            "sonnet",
            None,
            execution_mode,
            permission_level,
            None,
            has_runtime_mcp,
            has_runtime_mcp,
        )
        .expect("Claude args should build")
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|pair| pair == [flag, value])
    }

    #[test]
    fn permission_prompt_tool_is_only_passed_for_build_required_with_runtime_mcp() {
        let required = args(
            AgentExecutionMode::Build,
            AgentPermissionLevel::Required,
            true,
        );
        assert!(has_pair(&required, "--permission-mode", "default"));
        assert!(has_pair(
            &required,
            "--permission-prompt-tool",
            "mcp__chariox__chariox_permission_prompt"
        ));
        assert!(has_pair(&required, "--allowedTools", "mcp__chariox__*"));

        for args in [
            args(
                AgentExecutionMode::Build,
                AgentPermissionLevel::Required,
                false,
            ),
            args(AgentExecutionMode::Build, AgentPermissionLevel::Yolo, true),
            args(
                AgentExecutionMode::Plan,
                AgentPermissionLevel::Required,
                true,
            ),
            args(AgentExecutionMode::Plan, AgentPermissionLevel::Yolo, true),
        ] {
            assert!(!args.iter().any(|arg| arg == "--permission-prompt-tool"));
            assert!(!args.iter().any(|arg| arg == "--allowedTools"));
        }
    }

    #[test]
    fn permission_prompt_tool_name_matches_the_runtime_mcp_tool() {
        let normalized = crate::transport::runtime_tools::PERMISSION_PROMPT_TOOL
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        assert_eq!(
            CLAUDE_PERMISSION_PROMPT_TOOL,
            format!("mcp__chariox__{normalized}")
        );
    }

    #[test]
    fn execution_config_switch_replaces_the_permission_prompt_bridge() {
        let required = args(
            AgentExecutionMode::Build,
            AgentPermissionLevel::Required,
            true,
        );
        let yolo = claude_args_with_execution_config(
            &required,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
            true,
        );
        assert!(has_pair(&yolo, "--permission-mode", "bypassPermissions"));
        assert!(!yolo.iter().any(|arg| arg == "--permission-prompt-tool"
            || arg == CLAUDE_PERMISSION_PROMPT_TOOL
            || arg == "--allowedTools"));
        assert!(has_pair(&yolo, "--disallowedTools", "ToolSearch"));

        let required_again = claude_args_with_execution_config(
            &yolo,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Required,
            true,
        );
        assert_eq!(required_again, required);
    }
}
