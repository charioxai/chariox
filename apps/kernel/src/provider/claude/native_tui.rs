//! Claude native TUI hook files and launch arguments.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;
use crate::provider::{AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest};

use super::launch_args::normalized_claude_model;
use super::mcp_config::{
    create_claude_runtime_files_root, materialize_request_claude_mcp_config, ClaudeRuntimeFilesRoot,
};

pub(super) struct ClaudeNativeTuiFiles {
    root: ClaudeRuntimeFilesRoot,
    pub(super) events_file: PathBuf,
    pub(super) context_file: PathBuf,
    pub(super) context_response_dir: PathBuf,
    pub(super) permission_response_dir: PathBuf,
    pub(super) settings_file: PathBuf,
    mcp_config_file: Option<PathBuf>,
}

impl ClaudeNativeTuiFiles {
    pub(super) fn materialize_mcp_config(
        &mut self,
        request: &LaunchProviderRequest,
    ) -> Result<(), DaemonError> {
        self.mcp_config_file = materialize_request_claude_mcp_config(request, &self.root)?;
        Ok(())
    }

    pub(super) fn mcp_config_file(&self) -> Option<&Path> {
        self.mcp_config_file.as_deref()
    }

    pub(super) fn persist_for_launch(&mut self) {
        self.root.persist_for_launch();
    }
}

pub(super) fn prepare_claude_native_tui_files() -> Result<ClaudeNativeTuiFiles, DaemonError> {
    let root = create_claude_runtime_files_root()?;
    let events_file = root.path().join("events.jsonl");
    let context_file = root.path().join("hidden-context.txt");
    let context_response_dir = root.path().join("hook-context-responses");
    let permission_response_dir = root.path().join("permission-responses");
    let settings_file = root.path().join("settings.json");
    let hook_handler_file = root.path().join("hook-handler.mjs");
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
            "SessionStart": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "Stop": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "StopFailure": [{ "hooks": [{ "type": "command", "command": hook_command }] }],
            "SessionEnd": [{ "hooks": [{ "type": "command", "command": hook_command }] }]
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
        root,
        events_file,
        context_file,
        context_response_dir,
        permission_response_dir,
        settings_file,
        mcp_config_file: None,
    })
}

fn claude_native_hook_handler() -> &'static str {
    r#"#!/usr/bin/env node
import { appendFileSync, existsSync, readFileSync, unlinkSync } from "node:fs"
import { dirname, join } from "node:path"
import { setTimeout as setCallbackTimeout } from "node:timers"
import { setTimeout as sleep } from "node:timers/promises"

const hookWatchdog = setCallbackTimeout(() => {
  try {
    appendFileSync(`${process.env.ARROBA_CLAUDE_NATIVE_EVENTS}.watchdog`, JSON.stringify({
      at: new Date().toISOString(),
      reason: "hook_event_not_resolved"
    }) + "\n")
  } catch {}
  process.exit(0)
}, 7000)

async function readHookInput() {
  const chunks = []
  let settled = false
  return await new Promise((resolve) => {
    const finish = () => {
      if (settled) return
      settled = true
      resolve(Buffer.concat(chunks).toString("utf8"))
    }
    process.stdin.on("data", (chunk) => chunks.push(chunk))
    process.stdin.once("end", finish)
    process.stdin.once("error", finish)
    process.stdin.resume()
    setCallbackTimeout(finish, 1000)
  })
}

const raw = await readHookInput()
let input = {}
try {
  input = raw.trim() ? JSON.parse(raw) : {}
} catch (error) {
  input = { hook_event_name: "parse_error", raw, error: String(error) }
}
const eventName = input.hook_event_name ?? "unknown"
if (eventName === "SessionStart") {
  try { unlinkSync(join(dirname(process.argv[1]), "mcp-config.json")) } catch {}
}
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
  process.exit(0)
} else if (eventName === "PreToolUse" || eventName === "PermissionRequest") {
  const toolName = String(input.tool_name ?? "")
  const isArrobaRuntimeTool = toolName.startsWith("mcp__arroba__") || toolName.startsWith("arroba.")
  if (isArrobaRuntimeTool || (eventName === "PreToolUse" && input.permission_mode === "bypassPermissions")) {
    process.exit(0)
  }
  if (!toolName) {
    process.exit(0)
  }
  clearTimeout(hookWatchdog)
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
process.exit(0)
"#
}

pub(super) fn claude_native_tui_args(
    request: &LaunchProviderRequest,
    settings_file: &Path,
    mcp_config_file: Option<&Path>,
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
    if let Some(session_id) = request
        .resume_state
        .as_ref()
        .and_then(|state| state.claude_session_id())
    {
        args.extend(["--resume".to_string(), session_id.to_string()]);
    }
    if request.permission_level.unwrap_or_default() == AgentPermissionLevel::Yolo {
        args.push("--allow-dangerously-skip-permissions".to_string());
    }
    if let Some(config_file) = mcp_config_file {
        args.extend([
            "--mcp-config".to_string(),
            config_file.display().to_string(),
        ]);
        args.push("--strict-mcp-config".to_string());
        if request.runtime_mcp_binding.is_some() {
            args.extend(["--allowedTools".to_string(), "mcp__arroba__*".to_string()]);
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use crate::provider::{LaunchProviderRequest, ProviderResumeState, RuntimeMcpBinding};

    use super::{
        claude_native_hook_handler, claude_native_tui_args, prepare_claude_native_tui_files,
    };

    #[test]
    fn hook_does_not_block_bypass_or_arroba_runtime_pre_tool_use() {
        let handler = claude_native_hook_handler();

        assert!(handler.contains("input.permission_mode === \"bypassPermissions\""));
        assert!(handler.contains("toolName.startsWith(\"mcp__arroba__\")"));
        assert!(handler.contains("toolName.startsWith(\"arroba.\")"));
        assert!(handler.contains("process.exit(0)"));
    }

    #[test]
    fn session_start_hook_removes_materialized_mcp_credentials() {
        if Command::new("node").arg("--version").output().is_err() {
            return;
        }
        let mut native =
            prepare_claude_native_tui_files().expect("native files should be prepared");
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
        native
            .materialize_mcp_config(&request)
            .expect("MCP config should materialize");
        let config_path = native
            .mcp_config_file()
            .expect("MCP config path should exist")
            .to_path_buf();
        let hook_handler = native
            .events_file
            .parent()
            .expect("events file should have a root")
            .join("hook-handler.mjs");
        let mut child = Command::new("node")
            .arg(hook_handler)
            .env("ARROBA_CLAUDE_NATIVE_EVENTS", &native.events_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("hook handler should start");
        child
            .stdin
            .take()
            .expect("hook stdin should be piped")
            .write_all(br#"{"hook_event_name":"SessionStart"}"#)
            .expect("hook input should write");

        let output = child
            .wait_with_output()
            .expect("hook handler should finish");

        assert!(
            output.status.success(),
            "hook handler failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!config_path.exists());
        assert!(std::fs::read_to_string(&native.events_file)
            .expect("hook event should be recorded")
            .contains("SessionStart"));
    }

    #[test]
    fn native_tui_resumes_requested_claude_session() {
        let request = LaunchProviderRequest::new(
            "session-1",
            "claude",
            "claude-headless",
            "default",
            "sonnet",
        )
        .with_resume_state(ProviderResumeState::from_claude_session_id(
            "claude-session-1",
        ));

        let args = claude_native_tui_args(&request, Path::new("settings.json"), None)
            .expect("Claude native TUI args should resolve");

        assert!(args
            .windows(2)
            .any(|pair| pair == ["--resume", "claude-session-1"]));
    }
}
