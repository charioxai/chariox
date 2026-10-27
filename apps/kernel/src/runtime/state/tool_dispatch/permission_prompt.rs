//! Claude Code `--permission-prompt-tool` bridge for headless (`-p`) runs.
//!
//! Claude calls `chariox.permission_prompt` with `{tool_name, input,
//! tool_use_id?}` whenever a tool needs approval. The kernel asks the user
//! through one permission `RuntimeInteraction` and answers with the JSON text
//! Claude expects: `{"behavior":"allow","updatedInput":...}` or
//! `{"behavior":"deny","message":...}`. "Allow for this session" also answers with
//! session-scoped rules, so Claude stops asking for the same kind of call in that
//! run.

use super::*;
use crate::provider::{ProviderNativeInteractionResolution, RuntimeProviderRun};
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};
use crate::transport::runtime_tools::{PermissionPromptArgs, RuntimeToolResult};
use std::path::PathBuf;

use super::super::claude_permission_rules::{
    allow_for_session, session_grant, GrantScope, SessionGrant,
};

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
        let working_directory = provider_run.working_directory().map(PathBuf::as_path);
        let grant = session_grant(&args.tool_name, &args.input, working_directory);
        let scope = GrantScope {
            working_directory: provider_run.working_directory().cloned(),
            permission_level: provider_run.permission_level(),
            execution_mode: provider_run.execution_mode(),
        };
        // A new Claude process (after a cancel, a restart or a change of model) has none of the
        // rules the person granted earlier in this agent's session. A call they already cover is
        // allowed at once and the process gets the rules again.
        if let Some(standing) = grant.as_ref().and_then(|wanted| {
            self.claude_session_grants
                .covering(agent_id, &scope, wanted)
        }) {
            crate::logging::info_with_fields(
                "runtime.permission_prompt",
                "Claude permission prompt covered by the session grant",
                serde_json::json!({
                    "provider_run_id": provider_run.id(),
                    "tool_name": args.tool_name,
                    "rules": standing.allow_rules(),
                }),
            );
            return Ok(RuntimeToolResult {
                ok: true,
                payload: allow_for_session(args.input.clone(), &standing),
            });
        }
        let interaction = claude_permission_prompt_interaction(
            provider_run.id(),
            agent_id,
            &args,
            grant.as_ref(),
        )
        .with_native_origin(self.owned.capture_native_interaction_origin(
            provider_run.session_id(),
            agent_id,
            provider_run.id(),
        ));
        let resolution =
            crate::runtime::native_interaction_bridge::request_provider_native_interaction(
                self,
                provider_run.session_id(),
                interaction,
                OPERATION,
            )
            .await;
        let payload = match resolution {
            Ok(resolution) => {
                if let (Some("allow_session"), Some(grant)) =
                    (resolution.choice_id.as_deref(), grant.as_ref())
                {
                    self.claude_session_grants.remember(agent_id, scope, grant);
                }
                claude_permission_prompt_decision(&resolution, &args, grant.as_ref())
            }
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
    grant: Option<&SessionGrant>,
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
    let mut message = crate::app::format_claude_permission_message(&event);
    if let Some(grant) = grant {
        message.push_str("\n\n");
        message.push_str(&grant.description());
    }
    let mut choices = vec![RuntimeInteractionChoice::new(
        "allow_once",
        "Allow once",
        "allow",
        Some(RuntimeInteractionChoiceStyle::Primary),
    )];
    if let Some(grant) = grant {
        choices.push(RuntimeInteractionChoice::new(
            "allow_session",
            grant.label(),
            "allow_session",
            Some(RuntimeInteractionChoiceStyle::Secondary),
        ));
    }
    choices.push(RuntimeInteractionChoice::new(
        "deny",
        "Deny",
        "deny",
        Some(RuntimeInteractionChoiceStyle::Danger),
    ));
    RuntimeInteraction::new(
        format!("claude-permission-{provider_run_id}-{request_id}"),
        agent_id.to_string(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some(format!("Approve Claude Code {}?", args.tool_name)),
        message,
        choices,
        None,
        Some(PERMISSION_PROMPT_TIMEOUT_SEC),
        Some("deny".to_string()),
    )
}

fn claude_permission_prompt_decision(
    resolution: &ProviderNativeInteractionResolution,
    args: &PermissionPromptArgs,
    grant: Option<&SessionGrant>,
) -> serde_json::Value {
    let input = args.input.clone();
    match resolution.choice_id.as_deref() {
        Some("allow_once") => {
            return serde_json::json!({ "behavior": "allow", "updatedInput": input })
        }
        // The choice is only listed when rules can be stated, so a missing
        // grant here is a bug: deny rather than allow more than was approved.
        Some("allow_session") => {
            if let Some(grant) = grant {
                return allow_for_session(input, grant);
            }
        }
        _ => {}
    }
    // Deny, including an unanswered prompt: its timeout picks the default
    // Deny choice.
    deny("Denied through Chariox.")
}

fn deny(message: &str) -> serde_json::Value {
    serde_json::json!({ "behavior": "deny", "message": message })
}
