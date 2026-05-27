use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

use crate::error::DaemonError;
use crate::provider::ProviderWriteAccessMode;

use super::approval_bodies::{
    apply_patch_review_body, command_execution_approval_body, exec_command_review_body,
    file_change_approval_body,
};
use super::json_rpc::JsonRpcMessage;
use super::permission::workspace_live_sync_codex_permission_grant;
use super::runtime_mcp::call_runtime_mcp_tool;
use super::{CodexClient, CodexSocket};

impl CodexClient {
    pub(super) fn respond_to_server_request(
        &self,
        socket: &mut CodexSocket,
        message: &JsonRpcMessage,
    ) -> Result<bool, DaemonError> {
        let Some(request_id) = message.id.as_ref() else {
            return Ok(false);
        };
        let Some(method) = message.method.as_deref() else {
            return Ok(false);
        };
        crate::logging::debug_with_fields(
            "daemon.provider.codex",
            "received codex server request",
            json!({
                "provider_run_id": self.provider_run_id,
                "method": method,
                "params": message.params,
            }),
        );
        let result = match method {
            "item/commandExecution/requestApproval" => {
                self.command_execution_approval_response(message)?
            }
            "item/fileChange/requestApproval" => self.file_change_approval_response(message)?,
            "execCommandApproval" => self.exec_command_approval_response(message)?,
            "applyPatchApproval" => self.apply_patch_approval_response(message)?,
            "item/permissions/requestApproval" => self.permissions_approval_response(message),
            "mcpServer/elicitation/request" => self.respond_to_mcp_elicitation(message),
            "item/tool/call" => self.respond_to_dynamic_tool_call(message)?,
            _ => {
                crate::logging::warn_with_fields(
                    "daemon.provider.codex",
                    "unhandled codex server request",
                    json!({
                        "provider_run_id": self.provider_run_id,
                        "method": method,
                    }),
                );
                return Ok(false);
            }
        };
        let payload = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": result,
        });
        socket
            .send(Message::Text(payload.to_string().into()))
            .map_err(|error| self.protocol_error("codex_write", error.to_string()))?;
        Ok(true)
    }

    fn respond_to_mcp_elicitation(&self, message: &JsonRpcMessage) -> Value {
        let approve = message.params.as_ref().is_some_and(|params| {
            params.get("serverName").and_then(Value::as_str) == Some("arroba")
                && params
                    .get("_meta")
                    .and_then(|meta| meta.get("codex_approval_kind"))
                    .and_then(Value::as_str)
                    == Some("mcp_tool_call")
        });
        if approve {
            json!({
                "action": "accept",
                "content": {},
                "_meta": null,
            })
        } else {
            json!({
                "action": "decline",
                "content": null,
                "_meta": null,
            })
        }
    }

    pub(super) fn permissions_approval_response(&self, message: &JsonRpcMessage) -> Value {
        let requested_permissions = message
            .params
            .as_ref()
            .and_then(|params| params.get("permissions"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        match self.write_access_mode {
            ProviderWriteAccessMode::Unrestricted => json!({
                "permissions": requested_permissions,
                "scope": "session",
            }),
            ProviderWriteAccessMode::WorkspaceLiveSyncRequired => {
                let granted_permissions = workspace_live_sync_codex_permission_grant(&requested_permissions);
                json!({
                    "permissions": granted_permissions,
                    "scope": "turn",
                })
            }
        }
    }

    fn command_execution_approval_response(
        &self,
        message: &JsonRpcMessage,
    ) -> Result<Value, DaemonError> {
        let params = message
            .params
            .as_ref()
            .cloned()
            .unwrap_or_else(|| json!({}));
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex requested command approval",
            json!({
                "provider_run_id": self.provider_run_id,
                "params": params,
            }),
        );
        let body = command_execution_approval_body(&params);
        self.request_native_permission_interaction(
            "codex-command-approval",
            Some("Command approval required".to_string()),
            body,
            crate::session::RuntimeInteractionLevel::Warning,
        )
    }

    fn file_change_approval_response(
        &self,
        message: &JsonRpcMessage,
    ) -> Result<Value, DaemonError> {
        let params = message
            .params
            .as_ref()
            .cloned()
            .unwrap_or_else(|| json!({}));
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex requested file change approval",
            json!({
                "provider_run_id": self.provider_run_id,
                "params": params,
            }),
        );
        let body = file_change_approval_body(&params);
        self.request_native_permission_interaction(
            "codex-file-change-approval",
            Some("File change approval required".to_string()),
            body,
            crate::session::RuntimeInteractionLevel::Critical,
        )
    }

    fn exec_command_approval_response(
        &self,
        message: &JsonRpcMessage,
    ) -> Result<Value, DaemonError> {
        let params = message
            .params
            .as_ref()
            .cloned()
            .unwrap_or_else(|| json!({}));
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex requested exec command approval",
            json!({
                "provider_run_id": self.provider_run_id,
                "params": params,
            }),
        );
        let body = exec_command_review_body(&params);
        self.request_native_review_interaction(
            "codex-exec-command-approval",
            Some("Command approval required".to_string()),
            body,
            crate::session::RuntimeInteractionLevel::Warning,
        )
    }

    fn apply_patch_approval_response(
        &self,
        message: &JsonRpcMessage,
    ) -> Result<Value, DaemonError> {
        let params = message
            .params
            .as_ref()
            .cloned()
            .unwrap_or_else(|| json!({}));
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex requested apply_patch approval",
            json!({
                "provider_run_id": self.provider_run_id,
                "params": params,
            }),
        );
        let body = apply_patch_review_body(&params);
        self.request_native_review_interaction(
            "codex-apply-patch-approval",
            Some("File change approval required".to_string()),
            body,
            crate::session::RuntimeInteractionLevel::Critical,
        )
    }

    fn request_native_permission_interaction(
        &self,
        prefix: &str,
        title: Option<String>,
        message: String,
        level: crate::session::RuntimeInteractionLevel,
    ) -> Result<Value, DaemonError> {
        let Some(bridge) = self.native_interaction_bridge.as_ref() else {
            return Ok(json!({ "decision": "decline" }));
        };
        let session_id = self.session_id.as_deref().ok_or_else(|| {
            self.protocol_error(
                "codex_native_permission",
                "missing session context for native permission prompt".to_string(),
            )
        })?;
        let agent_id = self.agent_id.as_deref().ok_or_else(|| {
            self.protocol_error(
                "codex_native_permission",
                "missing agent context for native permission prompt".to_string(),
            )
        })?;
        let interaction = codex_permission_interaction(prefix, title, message, level, agent_id);
        let resolution = bridge.request_blocking(session_id, interaction)?;
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex native permission interaction resolved",
            json!({
                "provider_run_id": self.provider_run_id,
                "interaction_prefix": prefix,
                "status": resolution.status,
                "choice_id": resolution.choice_id,
                "reply": resolution.reply,
            }),
        );
        Ok(Self::codex_v2_approval_decision(&resolution))
    }

    fn request_native_review_interaction(
        &self,
        prefix: &str,
        title: Option<String>,
        message: String,
        level: crate::session::RuntimeInteractionLevel,
    ) -> Result<Value, DaemonError> {
        let Some(bridge) = self.native_interaction_bridge.as_ref() else {
            return Ok(json!({ "decision": "denied" }));
        };
        let session_id = self.session_id.as_deref().ok_or_else(|| {
            self.protocol_error(
                "codex_native_review",
                "missing session context for native approval prompt".to_string(),
            )
        })?;
        let agent_id = self.agent_id.as_deref().ok_or_else(|| {
            self.protocol_error(
                "codex_native_review",
                "missing agent context for native approval prompt".to_string(),
            )
        })?;
        let interaction = codex_permission_interaction(prefix, title, message, level, agent_id);
        let resolution = bridge.request_blocking(session_id, interaction)?;
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex native review interaction resolved",
            json!({
                "provider_run_id": self.provider_run_id,
                "interaction_prefix": prefix,
                "status": resolution.status,
                "choice_id": resolution.choice_id,
                "reply": resolution.reply,
            }),
        );
        Ok(Self::codex_v2_approval_decision(&resolution))
    }

    fn codex_v2_approval_decision(
        resolution: &crate::provider::ProviderNativeInteractionResolution,
    ) -> Value {
        match resolution.choice_id.as_deref() {
            Some("allow_once") => json!({ "decision": "accept" }),
            Some("allow_session") => json!({ "decision": "acceptForSession" }),
            Some("deny") | None => json!({ "decision": "decline" }),
            Some(_) => json!({ "decision": "decline" }),
        }
    }

    fn respond_to_dynamic_tool_call(&self, message: &JsonRpcMessage) -> Result<Value, DaemonError> {
        let params = message.params.as_ref().ok_or_else(|| {
            self.protocol_error("codex_dynamic_tool_call", "missing params".to_string())
        })?;
        let tool_name = params.get("tool").and_then(Value::as_str).ok_or_else(|| {
            self.protocol_error("codex_dynamic_tool_call", "missing tool name".to_string())
        })?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let server_url = self.runtime_mcp_server_url.as_deref().ok_or_else(|| {
            self.protocol_error(
                "codex_dynamic_tool_call",
                "runtime MCP server URL is missing".to_string(),
            )
        })?;
        let auth_token = self.runtime_mcp_auth_token.as_deref().ok_or_else(|| {
            self.protocol_error(
                "codex_dynamic_tool_call",
                "runtime MCP auth token is missing".to_string(),
            )
        })?;
        match call_runtime_mcp_tool(server_url, auth_token, tool_name, arguments) {
            Ok(text) => Ok(json!({
                "contentItems": [{
                    "type": "inputText",
                    "text": text,
                }],
                "success": true,
            })),
            Err(error) => Ok(json!({
                "contentItems": [{
                    "type": "inputText",
                    "text": error.to_string(),
                }],
                "success": false,
            })),
        }
    }
}

fn codex_permission_interaction(
    prefix: &str,
    title: Option<String>,
    message: String,
    level: crate::session::RuntimeInteractionLevel,
    agent_id: &str,
) -> crate::session::RuntimeInteraction {
    crate::session::RuntimeInteraction::new(
        format!("{prefix}-{}", crate::session::unix_epoch_ms()),
        agent_id,
        crate::session::RuntimeInteractionKind::Permission,
        level,
        title,
        message,
        vec![
            crate::session::RuntimeInteractionChoice::new(
                "allow_once",
                "Allow once",
                "allow_once",
                Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "allow_session",
                "Allow for session",
                "allow_session",
                Some(crate::session::RuntimeInteractionChoiceStyle::Secondary),
            ),
            crate::session::RuntimeInteractionChoice::new(
                "deny",
                "Deny",
                "deny",
                Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        None,
        None,
    )
}
