//! Claude Code `--permission-prompt-tool` bridge for headless (`-p`) runs.
//!
//! Claude calls `chariox.permission_prompt` with `{tool_name, input,
//! tool_use_id?}` whenever a tool needs approval. The kernel asks the user
//! through one permission `RuntimeInteraction` and answers with the JSON text
//! Claude expects: `{"behavior":"allow","updatedInput":...}` or
//! `{"behavior":"deny","message":...}`.

use super::*;
use crate::provider::{ProviderNativeInteractionResolution, RuntimeProviderRun};
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};
use crate::transport::runtime_tools::{PermissionPromptArgs, RuntimeToolResult};

const OPERATION: &str = "runtime_tool_permission_prompt";
const PERMISSION_PROMPT_TIMEOUT_SEC: u64 = 300;

impl KernelRuntimeState {
    pub(super) async fn dispatch_permission_prompt_runtime_tool_call(
        &self,
        provider_run: &RuntimeProviderRun,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if !crate::provider::provider_run_uses_claude_permission_prompt_tool(provider_run) {
            return Ok(RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "runtime tool `chariox.permission_prompt` is only available to Claude print-mode runs",
                    "tool": crate::transport::runtime_tools::PERMISSION_PROMPT_TOOL,
                }),
            });
        }
        let args = serde_json::from_value::<PermissionPromptArgs>(arguments).map_err(|error| {
            DaemonError::LocalTransport {
                operation: OPERATION,
                message: format!("invalid tool arguments: {error}"),
            }
        })?;
        // An allow echoes it as `updatedInput`, which Claude expects to be the
        // tool's input object.
        if !args.input.is_object() {
            return Err(DaemonError::LocalTransport {
                operation: OPERATION,
                message: "invalid tool arguments: `input` must be an object".to_string(),
            });
        }
        let agent_id =
            provider_run
                .agent_instance_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: OPERATION,
                    message: "provider run is not bound to an agent".to_string(),
                })?;
        let interaction = claude_permission_prompt_interaction(provider_run.id(), agent_id, &args);
        let resolution =
            crate::runtime::native_interaction_bridge::request_provider_native_interaction(
                self,
                provider_run.session_id(),
                interaction,
                OPERATION,
            )
            .await;
        let payload = match resolution {
            Ok(resolution) => claude_permission_prompt_decision(&resolution, args.input),
            Err(error) => {
                crate::logging::warn_with_fields(
                    "runtime.permission_prompt",
                    "Claude permission prompt bridge failed",
                    serde_json::json!({
                        "provider_run_id": provider_run.id(),
                        "session_id": provider_run.session_id(),
                        "tool_name": args.tool_name,
                        "error": error.to_string(),
                    }),
                );
                deny("Chariox permission bridge failed.")
            }
        };
        Ok(RuntimeToolResult { ok: true, payload })
    }
}

fn claude_permission_prompt_interaction(
    provider_run_id: &str,
    agent_id: &str,
    args: &PermissionPromptArgs,
) -> RuntimeInteraction {
    let request_id = args
        .tool_use_id
        .as_deref()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        })
        .map(str::to_string)
        .unwrap_or_else(|| format!("{:016x}", rand::random::<u64>()));
    let event = serde_json::json!({
        "tool_name": args.tool_name,
        "tool_input": args.input,
    });
    RuntimeInteraction::new(
        format!("claude-permission-{provider_run_id}-{request_id}"),
        agent_id.to_string(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some(format!("Approve Claude Code {}?", args.tool_name)),
        crate::app::format_claude_permission_message(&event),
        vec![
            RuntimeInteractionChoice::new(
                "allow_once",
                "Allow once",
                "allow",
                Some(RuntimeInteractionChoiceStyle::Primary),
            ),
            RuntimeInteractionChoice::new(
                "deny",
                "Deny",
                "deny",
                Some(RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        Some(PERMISSION_PROMPT_TIMEOUT_SEC),
        Some("deny".to_string()),
    )
}

fn claude_permission_prompt_decision(
    resolution: &ProviderNativeInteractionResolution,
    input: serde_json::Value,
) -> serde_json::Value {
    if resolution.choice_id.as_deref() == Some("allow_once") {
        return serde_json::json!({ "behavior": "allow", "updatedInput": input });
    }
    // Deny, including an unanswered prompt: its timeout picks the default
    // Deny choice.
    deny("Denied through Chariox.")
}

fn deny(message: &str) -> serde_json::Value {
    serde_json::json!({ "behavior": "deny", "message": message })
}
