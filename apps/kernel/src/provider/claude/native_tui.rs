//! Claude native TUI hook files and launch arguments.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::DaemonError;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest};

use super::launch_args::{claude_mcp_config, normalized_claude_model};

pub(super) struct ClaudeNativeTuiFiles {
    pub(super) events_file: PathBuf,
    pub(super) context_file: PathBuf,
    pub(super) context_response_dir: PathBuf,
    pub(super) permission_response_dir: PathBuf,
    pub(super) settings_file: PathBuf,
}

pub(super) fn prepare_claude_native_tui_files() -> Result<ClaudeNativeTuiFiles, DaemonError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let root = env::temp_dir().join(format!(
        "arroba-claude-remote-native-{}-{now}",
        std::process::id()
    ));
    fs::create_dir_all(&root).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native tui files",
        message: error.to_string(),
    })?;
    let events_file = root.join("events.jsonl");
    let context_file = root.join("hidden-context.txt");
    let context_response_dir = root.join("hook-context-responses");
    let permission_response_dir = root.join("permission-responses");
    let settings_file = root.join("settings.json");
    let hook_handler_file = root.join("hook-handler.mjs");
    fs::create_dir_all(&context_response_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native context response dir",
        message: error.to_string(),
    })?;
    fs::create_dir_all(&permission_response_dir).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native permission response dir",
        message: error.to_string(),
    })?;
    fs::write(&events_file, "").map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native events file",
        message: error.to_string(),
    })?;
    fs::write(&context_file, "").map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native context file",
        message: error.to_string(),
    })?;
    fs::write(&hook_handler_file, claude_native_hook_handler()).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "prepare claude native hook handler",
            message: error.to_string(),
        }
    })?;
    let hook_command = format!(
        "node {}",
        serde_json::to_string(&hook_handler_file.display().to_string()).unwrap()
    );
    let settings = serde_json::json!({
        "hooks": {
            "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "Stop": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "StopFailure": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "SessionEnd": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "PermissionRequest": [{ "matcher": "*", "hooks": [{ "type": "command", "command": hook_command }] }],
            "PreToolUse": [{ "matcher": "*", "hooks": [{ "type": "command", "command": hook_command }] }],
            "PostToolUse": [{ "matcher": "*", "hooks": [{ "type": "command", "command": hook_command }] }]
        }
    });
    let settings =
        serde_json::to_string_pretty(&settings).map_err(|error| DaemonError::LocalTransport {
            operation: "prepare claude native settings",
            message: error.to_string(),
        })?;
    fs::write(&settings_file, settings).map_err(|error| DaemonError::LocalTransport {
        operation: "prepare claude native settings file",
        message: error.to_string(),
    })?;
    Ok(ClaudeNativeTuiFiles {
        events_file,
        context_file,
        context_response_dir,
        permission_response_dir,
        settings_file,
    })
}

fn claude_native_hook_handler() -> &'static str {
    r#"#!/usr/bin/env node
import { appendFileSync, existsSync, readFileSync, unlinkSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const raw = Buffer.concat(chunks).toString("utf8")
let input = {}
try {
  input = raw.trim() ? JSON.parse(raw) : {}
} catch (error) {
  input = { hook_event_name: "parse_error", raw, error: String(error) }
}
const eventName = input.hook_event_name ?? "unknown"
const hookContextRequestId = eventName === "UserPromptSubmit" || eventName === "PreToolUse" || eventName === "PermissionRequest"
  ? `${Date.now()}-${process.pid}-${Math.random().toString(36).slice(2)}`
  : null
appendFileSync(process.env.ARROBA_CLAUDE_NATIVE_EVENTS, JSON.stringify({
  at: new Date().toISOString(),
  hook_event_name: eventName,
  hook_context_request_id: hookContextRequestId,
  prompt: input.prompt ?? null,
  transcript_path: input.transcript_path ?? null,
  permission_mode: input.permission_mode ?? null,
  tool_name: input.tool_name ?? null,
  tool_input: input.tool_input ?? null,
  tool_response: input.tool_response ?? null,
  error: input.error ?? null,
}) + "\n")

if (eventName === "UserPromptSubmit") {
  let additionalContext = ""
  try {
    additionalContext = readFileSync(process.env.ARROBA_CLAUDE_NATIVE_CONTEXT, "utf8")
  } catch {}
  if (!additionalContext && hookContextRequestId && process.env.ARROBA_CLAUDE_NATIVE_CONTEXT_RESPONSES) {
    const responseFile = join(process.env.ARROBA_CLAUDE_NATIVE_CONTEXT_RESPONSES, `${hookContextRequestId}.txt`)
    const deadline = Date.now() + 5000
    while (Date.now() < deadline) {
      if (existsSync(responseFile)) {
        additionalContext = readFileSync(responseFile, "utf8")
        try { unlinkSync(responseFile) } catch {}
        break
      }
      await sleep(50)
    }
  }
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "UserPromptSubmit",
      additionalContext
    }
  }))
} else if (eventName === "PreToolUse" || eventName === "PermissionRequest") {
  const toolName = String(input.tool_name ?? "")
  const isArrobaRuntimeTool = toolName.startsWith("mcp__arroba__") || toolName.startsWith("arroba.")
  if (eventName === "PreToolUse" && (input.permission_mode === "bypassPermissions" || isArrobaRuntimeTool)) {
    process.exit(0)
  }
  const responseDir = process.env.ARROBA_CLAUDE_NATIVE_PERMISSION_RESPONSES
  const responseFile = responseDir && hookContextRequestId
    ? join(responseDir, `${hookContextRequestId}.json`)
    : null
  if (responseFile) {
    const deadline = Date.now() + 300000
    while (Date.now() < deadline) {
      if (existsSync(responseFile)) {
        try {
          const decision = JSON.parse(readFileSync(responseFile, "utf8"))
          try { unlinkSync(responseFile) } catch {}
          if (decision?.permissionDecision) {
            process.stdout.write(JSON.stringify({
              hookSpecificOutput: {
                hookEventName: eventName,
                permissionDecision: decision.permissionDecision,
                permissionDecisionReason: decision.permissionDecisionReason ?? "Resolved through Arroba."
              }
            }))
          }
        } catch {}
        break
      }
      await sleep(50)
    }
  }
}
"#
}

pub(super) fn claude_native_tui_args(
    request: &LaunchProviderRequest,
    settings_file: &Path,
) -> Result<Vec<String>, DaemonError> {
    let mut args = vec![
        "--settings".to_string(),
        settings_file.display().to_string(),
        "--permission-mode".to_string(),
        match (
            request.execution_mode.unwrap_or_default(),
            request.permission_level.unwrap_or_default(),
        ) {
            (AgentExecutionMode::Plan, _) => "plan".to_string(),
            (AgentExecutionMode::Build, AgentPermissionLevel::Required) => "default".to_string(),
            (AgentExecutionMode::Build, AgentPermissionLevel::Yolo) => {
                "bypassPermissions".to_string()
            }
        },
    ];
    let model = normalized_claude_model(&request.model);
    if !model.is_empty() && model != "default" {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(variant) = request
        .variant
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.extend(["--effort".to_string(), variant.to_string()]);
    }
    if request.permission_level.unwrap_or_default() == AgentPermissionLevel::Yolo {
        args.push("--allow-dangerously-skip-permissions".to_string());
    }
    if let Some(config) = claude_mcp_config(
        &request.mcp_servers,
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.server_url.as_str()),
        request
            .runtime_mcp_binding
            .as_ref()
            .map(|binding| binding.auth_token.as_str()),
    )? {
        args.extend(["--mcp-config".to_string(), config]);
        args.push("--strict-mcp-config".to_string());
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::claude_native_hook_handler;

    #[test]
    fn hook_does_not_block_bypass_or_arroba_runtime_pre_tool_use() {
        let handler = claude_native_hook_handler();

        assert!(handler.contains("input.permission_mode === \"bypassPermissions\""));
        assert!(handler.contains("toolName.startsWith(\"mcp__arroba__\")"));
        assert!(handler.contains("toolName.startsWith(\"arroba.\")"));
        assert!(handler.contains("process.exit(0)"));
    }
}
