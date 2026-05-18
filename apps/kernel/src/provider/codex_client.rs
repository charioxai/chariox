use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::mcp::ArrobaMcpServerConfig;
use crate::provider::{
    OpenCodeProviderCatalog, ProviderNativeInteractionBridge, ProviderRunTokenUsage,
    ProviderWriteAccessMode,
};

use super::codex::CODEX_MCP_TOKEN_ENV;
use super::resolve_codex_executable;

mod approval_bodies;
mod auth;
mod catalog;
mod health;
mod json_rpc;
mod json_rpc_transport;
mod notifications;
mod permission;
mod runtime_mcp;
mod server_requests;
mod socket_io;
mod thread_runtime;

mod mcp_config;

use catalog::{codex_catalog_from_models, CodexModelListResponse};
use json_rpc::JsonRpcMessage;
use notifications::parse_notification;
use permission::{
    codex_collaboration_mode, codex_permission_policy, managed_io_codex_permission_grant,
};

pub use auth::{ProviderAuthStatus, ProviderLoginStart};
pub use health::codex_endpoint_is_healthy;
pub use socket_io::CodexSocket;

#[derive(Clone)]
pub struct CodexClient {
    provider_run_id: String,
    session_id: Option<String>,
    agent_id: Option<String>,
    endpoint: String,
    runtime_mcp_server_url: Option<String>,
    runtime_mcp_auth_token: Option<String>,
    native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
    mcp_servers: Vec<ArrobaMcpServerConfig>,
    write_access_mode: ProviderWriteAccessMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexRunSelection {
    pub model: Option<String>,
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexNotification {
    AgentMessageDelta {
        item_id: String,
        delta: String,
    },
    ReasoningTextDelta {
        item_id: String,
        delta: String,
    },
    ReasoningSummaryTextDelta {
        item_id: String,
        delta: String,
    },
    ReasoningSummaryPartAdded {
        item_id: String,
        summary_index: usize,
    },
    ItemStarted {
        item: Value,
    },
    ItemCompleted {
        item: Value,
    },
    ExecCommandStarted {
        call_id: String,
        command: Value,
        cwd: Option<String>,
    },
    ExecCommandCompleted {
        call_id: String,
        command: Value,
        cwd: Option<String>,
        output: Option<String>,
        exit_code: Option<i64>,
        success: Option<bool>,
        stderr: Option<String>,
    },
    ExecCommandOutputDelta {
        call_id: String,
        chunk: String,
    },
    CommandExecutionOutputDelta {
        item_id: String,
        delta: String,
    },
    FileChangeOutputDelta {
        item_id: String,
        delta: String,
    },
    McpToolCallProgress {
        item_id: String,
        message: String,
    },
    TokenUsageUpdated {
        thread_id: String,
        turn_id: String,
        usage: ProviderRunTokenUsage,
    },
    TurnStarted {
        turn_id: String,
    },
    TurnCompleted {
        turn_id: String,
        status: String,
        error_message: Option<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodexThreadStartResponse {
    pub thread: CodexThread,
    pub model: String,
    #[serde(rename = "reasoningEffort", default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CodexThread {
    pub id: String,
}

impl CodexClient {
    pub fn new(
        provider_run_id: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            provider_run_id: provider_run_id.into(),
            session_id: None,
            agent_id: None,
            endpoint: endpoint.into(),
            runtime_mcp_server_url: None,
            runtime_mcp_auth_token: None,
            native_interaction_bridge: None,
            mcp_servers: Vec::new(),
            write_access_mode: ProviderWriteAccessMode::Unrestricted,
        })
    }

    pub fn with_runtime_context(
        mut self,
        session_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Self {
        self.session_id = session_id.map(str::to_string);
        self.agent_id = agent_id.map(str::to_string);
        self
    }

    pub fn with_runtime_mcp_binding(
        mut self,
        server_url: Option<&str>,
        auth_token: Option<&str>,
    ) -> Self {
        self.runtime_mcp_server_url = server_url.map(str::to_string);
        self.runtime_mcp_auth_token = auth_token.map(str::to_string);
        self
    }

    pub fn with_mcp_servers(mut self, mcp_servers: &[ArrobaMcpServerConfig]) -> Self {
        self.mcp_servers = mcp_servers.to_vec();
        self
    }

    pub(crate) fn with_native_interaction_bridge(
        mut self,
        bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
    ) -> Self {
        self.native_interaction_bridge = bridge;
        self
    }

    pub fn with_write_access_mode(mut self, write_access_mode: ProviderWriteAccessMode) -> Self {
        self.write_access_mode = write_access_mode;
        self
    }

    pub fn provider_catalog(&self) -> Result<OpenCodeProviderCatalog, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexModelListResponse =
            self.send_request(&mut socket, &mut next_request_id, "model/list", json!({}))?;
        Ok(codex_catalog_from_models(response.data))
    }

    pub(super) fn protocol_error(&self, operation: &'static str, message: String) -> DaemonError {
        DaemonError::ProviderProtocol {
            provider_run_id: self.provider_run_id.clone(),
            operation,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::mcp::ArrobaMcpServerConfig;
    use crate::provider::{
        AgentExecutionMode, AgentPermissionLevel, ProviderRunTokenUsage, ProviderWriteAccessMode,
    };

    use super::{
        codex_collaboration_mode, codex_permission_policy, managed_io_codex_permission_grant,
        parse_notification, CodexClient, CodexNotification, JsonRpcMessage,
    };

    #[test]
    fn managed_io_permission_policy_uses_read_only_sandbox() {
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::ManagedIoRequired,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
        );

        assert_eq!(policy.approval_policy, json!("never"));
        assert_eq!(policy.sandbox, "read-only");
        assert_eq!(policy.sandbox_policy, json!({ "type": "readOnly" }));
        assert_eq!(
            policy.config_overrides.get("include_apply_patch_tool"),
            Some(&json!(false))
        );
        assert_eq!(
            policy.config_overrides.get("features.apply_patch_freeform"),
            Some(&json!(false))
        );
    }

    #[test]
    fn unrestricted_required_policy_uses_strict_native_approval() {
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::Unrestricted,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Required,
        );

        assert_eq!(policy.approval_policy, json!("untrusted"));
        assert_eq!(policy.sandbox, "workspace-write");
        assert_eq!(policy.sandbox_policy, json!({ "type": "workspaceWrite" }));
    }

    #[test]
    fn unrestricted_yolo_build_policy_uses_full_filesystem_access() {
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::Unrestricted,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
        );

        assert_eq!(policy.approval_policy, json!("never"));
        assert_eq!(policy.sandbox, "danger-full-access");
        assert_eq!(policy.sandbox_policy, json!({ "type": "dangerFullAccess" }));
    }

    #[test]
    fn unrestricted_yolo_plan_policy_remains_read_only() {
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::Unrestricted,
            AgentExecutionMode::Plan,
            AgentPermissionLevel::Yolo,
        );

        assert_eq!(policy.approval_policy, json!("never"));
        assert_eq!(policy.sandbox, "read-only");
        assert_eq!(policy.sandbox_policy, json!({ "type": "readOnly" }));
    }

    #[test]
    fn codex_plan_mode_uses_native_collaboration_mode() {
        assert_eq!(
            codex_collaboration_mode(AgentExecutionMode::Plan, Some("gpt-5.4"), Some("low")),
            Some(json!({
                "mode": "plan",
                "settings": {
                    "model": "gpt-5.4",
                    "reasoning_effort": "low",
                    "developer_instructions": null,
                }
            }))
        );
    }

    #[test]
    fn codex_build_mode_resets_native_collaboration_mode() {
        assert_eq!(
            codex_collaboration_mode(AgentExecutionMode::Build, Some("gpt-5.4"), None),
            Some(json!({
                "mode": "default",
                "settings": {
                    "model": "gpt-5.4",
                    "reasoning_effort": null,
                    "developer_instructions": null,
                }
            }))
        );
    }

    #[test]
    fn codex_thread_start_creates_durable_threads() {
        let client =
            CodexClient::new("run-1", "ws://127.0.0.1:43123").expect("client should construct");

        let params = client
            .thread_start_params(
                Some("/tmp/worktree"),
                Some("gpt-5.5"),
                ProviderWriteAccessMode::Unrestricted,
                AgentExecutionMode::Build,
                AgentPermissionLevel::Yolo,
            )
            .expect("params should build");

        assert_eq!(params.get("ephemeral"), None);
        assert_eq!(params.get("persistExtendedHistory"), Some(&json!(true)));
        assert_eq!(params.get("serviceName"), Some(&json!("arroba")));
        assert_eq!(params.get("cwd"), Some(&json!("/tmp/worktree")));
        assert_eq!(params.get("model"), Some(&json!("gpt-5.5")));
    }

    #[test]
    fn managed_io_permission_grant_removes_filesystem_write() {
        let requested = json!({
            "network": {
                "enabled": true
            },
            "fileSystem": {
                "read": ["/tmp/input"],
                "write": ["/tmp/output"]
            }
        });

        assert_eq!(
            managed_io_codex_permission_grant(&requested),
            json!({
                "network": {
                    "enabled": true
                },
                "fileSystem": {
                    "read": ["/tmp/input"]
                }
            })
        );
    }

    #[test]
    fn managed_io_permission_grant_denies_write_only_request() {
        let requested = json!({
            "fileSystem": {
                "write": ["/tmp/output"]
            }
        });

        assert_eq!(managed_io_codex_permission_grant(&requested), json!({}));
    }

    #[test]
    fn managed_io_client_does_not_approve_codex_filesystem_writes() {
        let client = CodexClient::new("run-1", "ws://127.0.0.1:43123")
            .expect("client should construct")
            .with_write_access_mode(ProviderWriteAccessMode::ManagedIoRequired);
        let message = JsonRpcMessage {
            id: Some(json!(1)),
            method: Some("item/permissions/requestApproval".to_string()),
            params: Some(json!({
                "permissions": {
                    "fileSystem": {
                        "write": ["/tmp/output"]
                    }
                }
            })),
            result: None,
            error: None,
        };

        assert_eq!(
            client.permissions_approval_response(&message),
            json!({
                "permissions": {},
                "scope": "turn"
            })
        );
    }

    #[test]
    fn thread_config_overrides_include_runtime_mcp_binding() {
        let client = CodexClient::new("run-1", "ws://127.0.0.1:43123")
            .expect("client should construct")
            .with_runtime_mcp_binding(Some("http://127.0.0.1:43120/mcp"), Some("token-123"));
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::ManagedIoRequired,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
        );

        let overrides = client.thread_config_overrides(&policy).unwrap();

        assert_eq!(
            overrides.get("mcp_servers.arroba.url"),
            Some(&json!("http://127.0.0.1:43120/mcp"))
        );
        assert_eq!(
            overrides.get("mcp_servers.arroba.bearer_token_env_var"),
            Some(&json!("ARROBA_MCP_TOKEN"))
        );
        assert_eq!(
            overrides.get("mcp_servers.arroba.required"),
            Some(&json!(true))
        );
        assert_eq!(
            overrides.get("mcp_servers.arroba.tool_timeout_sec"),
            Some(&json!(15))
        );
        assert_eq!(overrides.get("features.shell_tool"), None);
    }

    #[test]
    fn thread_config_overrides_include_granted_mcp_servers() {
        let mut server =
            ArrobaMcpServerConfig::stdio("browser", "npx", vec!["@playwright/mcp@latest".into()]);
        server.required = true;
        server.tool_timeout_sec = Some(25);
        let client = CodexClient::new("run-1", "ws://127.0.0.1:43123")
            .expect("client should construct")
            .with_mcp_servers(&[server]);
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::Unrestricted,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
        );

        let overrides = client.thread_config_overrides(&policy).unwrap();

        assert_eq!(
            overrides.get("mcp_servers.browser.command"),
            Some(&json!("npx"))
        );
        assert_eq!(
            overrides.get("mcp_servers.browser.args"),
            Some(&json!(["@playwright/mcp@latest"]))
        );
        assert_eq!(
            overrides.get("mcp_servers.browser.required"),
            Some(&json!(true))
        );
        assert_eq!(
            overrides.get("mcp_servers.browser.tool_timeout_sec"),
            Some(&json!(25))
        );
    }

    #[test]
    fn thread_config_overrides_proxy_granted_mcp_servers_when_runtime_mcp_is_bound() {
        let mut server =
            ArrobaMcpServerConfig::stdio("browser", "npx", vec!["@playwright/mcp@latest".into()]);
        server.required = true;
        server.tool_timeout_sec = Some(25);
        let client = CodexClient::new("run-1", "ws://127.0.0.1:43123")
            .expect("client should construct")
            .with_runtime_mcp_binding(Some("http://127.0.0.1:43120/mcp"), Some("token-123"))
            .with_mcp_servers(&[server]);
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::ManagedIoRequired,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
        );

        let overrides = client.thread_config_overrides(&policy).unwrap();

        assert_eq!(
            overrides.get("mcp_servers.browser.url"),
            Some(&json!("http://127.0.0.1:43120/mcp/proxy/browser"))
        );
        assert_eq!(
            overrides.get("mcp_servers.browser.bearer_token_env_var"),
            Some(&json!("ARROBA_MCP_TOKEN"))
        );
        assert_eq!(
            overrides.get("mcp_servers.browser.http_headers.Authorization"),
            None
        );
        assert_eq!(overrides.get("mcp_servers.browser.command"), None);
        assert_eq!(overrides.get("mcp_servers.browser.args"), None);
        assert_eq!(
            overrides.get("mcp_servers.browser.required"),
            Some(&json!(true))
        );
        assert_eq!(
            overrides.get("mcp_servers.browser.tool_timeout_sec"),
            Some(&json!(25))
        );
    }

    #[test]
    fn parse_notification_recognizes_reasoning_and_tool_events() {
        let reasoning = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("item/reasoning/textDelta".to_string()),
            params: Some(json!({
                "itemId": "reason-1",
                "delta": "thinking"
            })),
            result: None,
            error: None,
        });
        assert_eq!(
            reasoning,
            Some(CodexNotification::ReasoningTextDelta {
                item_id: "reason-1".to_string(),
                delta: "thinking".to_string(),
            })
        );

        let item_started = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("item/started".to_string()),
            params: Some(json!({
                "item": {
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "pwd",
                    "cwd": "/tmp",
                    "status": "inProgress",
                    "commandActions": []
                }
            })),
            result: None,
            error: None,
        });
        assert!(matches!(
            item_started,
            Some(CodexNotification::ItemStarted { item })
                if item.get("id").and_then(serde_json::Value::as_str) == Some("cmd-1")
        ));

        let progress = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("item/mcpToolCall/progress".to_string()),
            params: Some(json!({
                "itemId": "mcp-1",
                "message": "running"
            })),
            result: None,
            error: None,
        });
        assert_eq!(
            progress,
            Some(CodexNotification::McpToolCallProgress {
                item_id: "mcp-1".to_string(),
                message: "running".to_string(),
            })
        );

        let exec_started = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("codex/event/exec_command_begin".to_string()),
            params: Some(json!({
                "msg": {
                    "type": "exec_command_begin",
                    "call_id": "cmd-event-1",
                    "command": "/bin/zsh -lc 'pwd'",
                    "cwd": "/tmp"
                }
            })),
            result: None,
            error: None,
        });
        assert_eq!(
            exec_started,
            Some(CodexNotification::ExecCommandStarted {
                call_id: "cmd-event-1".to_string(),
                command: json!("/bin/zsh -lc 'pwd'"),
                cwd: Some("/tmp".to_string()),
            })
        );

        let exec_output_delta = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("codex/event/exec_command_output_delta".to_string()),
            params: Some(json!({
                "msg": {
                    "type": "exec_command_output_delta",
                    "call_id": "cmd-event-1",
                    "chunk": "b2s=",
                }
            })),
            result: None,
            error: None,
        });
        assert_eq!(
            exec_output_delta,
            Some(CodexNotification::ExecCommandOutputDelta {
                call_id: "cmd-event-1".to_string(),
                chunk: "b2s=".to_string(),
            })
        );

        let exec_completed = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("codex/event/exec_command_end".to_string()),
            params: Some(json!({
                "msg": {
                    "type": "exec_command_end",
                    "call_id": "cmd-event-1",
                    "command": "/bin/zsh -lc 'pwd'",
                    "cwd": "/tmp",
                    "aggregated_output": "ok\n",
                    "exit_code": 0,
                    "success": true
                }
            })),
            result: None,
            error: None,
        });
        assert_eq!(
            exec_completed,
            Some(CodexNotification::ExecCommandCompleted {
                call_id: "cmd-event-1".to_string(),
                command: json!("/bin/zsh -lc 'pwd'"),
                cwd: Some("/tmp".to_string()),
                output: Some("ok\n".to_string()),
                exit_code: Some(0),
                success: Some(true),
                stderr: None,
            })
        );

        let token_usage = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("thread/tokenUsage/updated".to_string()),
            params: Some(json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "tokenUsage": {
                    "total": {
                        "totalTokens": 42100,
                        "inputTokens": 40000,
                        "cachedInputTokens": 12000,
                        "outputTokens": 2100,
                        "reasoningOutputTokens": 800
                    },
                    "last": {
                        "totalTokens": 8900,
                        "inputTokens": 7600,
                        "cachedInputTokens": 2000,
                        "outputTokens": 1300,
                        "reasoningOutputTokens": 500
                    },
                    "modelContextWindow": 128000
                }
            })),
            result: None,
            error: None,
        });
        assert_eq!(
            token_usage,
            Some(CodexNotification::TokenUsageUpdated {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                usage: ProviderRunTokenUsage {
                    total_tokens: Some(42100),
                    last_tokens: Some(8900),
                    context_tokens: Some(8900),
                    context_window: Some(128000),
                },
            })
        );

        let impossible_context_usage = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("thread/tokenUsage/updated".to_string()),
            params: Some(json!({
                "threadId": "thread-1",
                "turnId": "turn-2",
                "tokenUsage": {
                    "total": {
                        "totalTokens": 36000000
                    },
                    "last": {
                        "totalTokens": 36000000
                    },
                    "modelContextWindow": 128000
                }
            })),
            result: None,
            error: None,
        });
        assert_eq!(
            impossible_context_usage,
            Some(CodexNotification::TokenUsageUpdated {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-2".to_string(),
                usage: ProviderRunTokenUsage {
                    total_tokens: Some(36_000_000),
                    last_tokens: Some(36_000_000),
                    context_tokens: None,
                    context_window: Some(128_000),
                },
            })
        );
    }

    #[test]
    fn parse_notification_recognizes_codex_terminal_events() {
        let v2_completed = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("turn/completed".to_string()),
            params: Some(json!({
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "status": "completed",
                    "items": []
                }
            })),
            result: None,
            error: None,
        });
        assert_eq!(
            v2_completed,
            Some(CodexNotification::TurnCompleted {
                turn_id: "turn-1".to_string(),
                status: "completed".to_string(),
                error_message: None,
            })
        );

        let v2_completed_without_id = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("turn/completed".to_string()),
            params: Some(json!({
                "threadId": "thread-1",
                "turn": {
                    "status": "completed",
                    "items": []
                }
            })),
            result: None,
            error: None,
        });
        assert_eq!(v2_completed_without_id, None);

        let raw_task_complete = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("codex/event/task_complete".to_string()),
            params: Some(json!({
                "id": "turn-raw-1",
                "msg": {
                    "type": "task_complete"
                }
            })),
            result: None,
            error: None,
        });
        assert_eq!(raw_task_complete, None);

        let raw_turn_aborted = parse_notification(JsonRpcMessage {
            id: None,
            method: Some("codex/event/turn_aborted".to_string()),
            params: Some(json!({
                "id": "turn-raw-2",
                "msg": {
                    "type": "turn_aborted",
                    "reason": "interrupted"
                }
            })),
            result: None,
            error: None,
        });
        assert_eq!(raw_turn_aborted, None);
    }
}
