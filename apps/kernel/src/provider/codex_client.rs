use crate::error::DaemonError;
use crate::mcp::ArrobaMcpServerConfig;
use crate::provider::{ProviderNativeInteractionBridge, ProviderWriteAccessMode};
use std::path::PathBuf;

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

use json_rpc::JsonRpcMessage;
#[cfg(test)]
use notifications::parse_notification;
#[cfg(test)]
use permission::{
    codex_collaboration_mode, codex_permission_policy, workspace_live_sync_codex_permission_grant,
};

pub use auth::{ProviderAuthStatus, ProviderLoginStart};
pub use health::codex_endpoint_is_healthy;
pub use notifications::CodexNotification;
pub use socket_io::CodexSocket;
pub use thread_runtime::{CodexThread, CodexThreadStartResponse};

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
    workspace_live_sync_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexRunSelection {
    pub model: Option<String>,
    pub variant: Option<String>,
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
            workspace_live_sync_roots: Vec::new(),
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

    pub fn with_workspace_live_sync_roots(mut self, roots: &[PathBuf]) -> Self {
        self.workspace_live_sync_roots = roots.to_vec();
        self
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
        codex_collaboration_mode, codex_permission_policy, parse_notification,
        workspace_live_sync_codex_permission_grant, CodexClient, CodexNotification, JsonRpcMessage,
    };

    #[test]
    fn workspace_live_sync_permission_policy_uses_platform_fenced_sandbox() {
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::WorkspaceLiveSyncManaged,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
        );

        assert_eq!(policy.approval_policy, json!("never"));
        #[cfg(target_os = "macos")]
        {
            assert_eq!(policy.sandbox, "danger-full-access");
            assert_eq!(policy.sandbox_policy, json!({ "type": "dangerFullAccess" }));
        }
        #[cfg(not(target_os = "macos"))]
        {
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
        #[cfg(target_os = "macos")]
        assert!(policy.config_overrides.is_empty());
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
    fn tracked_live_sync_build_policy_does_not_sandbox_outside_repositories() {
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::WorkspaceLiveSyncTracked,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Required,
        );

        assert_eq!(policy.approval_policy, json!("untrusted"));
        assert_eq!(policy.sandbox, "danger-full-access");
        assert_eq!(policy.sandbox_policy, json!({ "type": "dangerFullAccess" }));
        assert!(policy.config_overrides.is_empty());
    }

    #[test]
    fn tracked_live_sync_thread_start_uses_full_access_payload() {
        let client =
            CodexClient::new("run-1", "ws://127.0.0.1:43123").expect("client should construct");

        let params = client
            .thread_start_params(
                Some("/repo/selected"),
                Some("gpt-5.2"),
                ProviderWriteAccessMode::WorkspaceLiveSyncTracked,
                AgentExecutionMode::Build,
                AgentPermissionLevel::Required,
            )
            .expect("thread start params should render");

        assert_eq!(params.get("approvalPolicy"), Some(&json!("untrusted")));
        assert_eq!(params.get("sandbox"), Some(&json!("danger-full-access")));
        assert_eq!(
            params.get("sandboxPolicy"),
            Some(&json!({ "type": "dangerFullAccess" }))
        );
        assert_eq!(params.get("cwd"), Some(&json!("/repo/selected")));
        assert_eq!(params.get("model"), Some(&json!("gpt-5.2")));
    }

    #[test]
    fn tracked_live_sync_plan_policy_remains_read_only() {
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::WorkspaceLiveSyncTracked,
            AgentExecutionMode::Plan,
            AgentPermissionLevel::Yolo,
        );

        assert_eq!(policy.approval_policy, json!("never"));
        assert_eq!(policy.sandbox, "read-only");
        assert_eq!(policy.sandbox_policy, json!({ "type": "readOnly" }));
        assert!(policy.config_overrides.is_empty());
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
            codex_collaboration_mode(AgentExecutionMode::Plan, Some("gpt-5.4"), Some("low"), None),
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
            codex_collaboration_mode(AgentExecutionMode::Build, Some("gpt-5.4"), None, None),
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
    fn codex_collaboration_mode_carries_hidden_developer_instructions() {
        let mode = codex_collaboration_mode(
            AgentExecutionMode::Build,
            Some("gpt-5.4"),
            None,
            Some("hidden system context"),
        )
        .expect("collaboration mode should build");

        assert_eq!(
            mode["settings"]["developer_instructions"],
            json!("hidden system context")
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
    fn workspace_live_sync_permission_grant_removes_filesystem_write() {
        let requested = json!({
            "network": {
                "enabled": true
            },
            "fileSystem": {
                "read": ["/tmp/input"],
                "write": ["/repo/main/output"]
            }
        });

        assert_eq!(
            workspace_live_sync_codex_permission_grant(
                &requested,
                &[std::path::PathBuf::from("/repo/main")],
                None,
            ),
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
    fn workspace_live_sync_permission_grant_denies_write_only_request() {
        let requested = json!({
            "fileSystem": {
                "write": ["/repo/main/output"]
            }
        });

        assert_eq!(
            workspace_live_sync_codex_permission_grant(
                &requested,
                &[std::path::PathBuf::from("/repo/main")],
                None,
            ),
            json!({})
        );
    }

    #[test]
    fn workspace_live_sync_permission_grant_allows_writes_outside_protected_roots() {
        let requested = json!({
            "fileSystem": {
                "write": [
                    "/repo/main/src/lib.rs",
                    "/repo/main/../main/src/config.rs",
                    "/other-repo/src/lib.rs"
                ]
            }
        });

        assert_eq!(
            workspace_live_sync_codex_permission_grant(
                &requested,
                &[std::path::PathBuf::from("/repo/main")],
                None,
            ),
            json!({
                "fileSystem": {
                    "write": ["/other-repo/src/lib.rs"]
                }
            })
        );
    }

    #[test]
    fn workspace_live_sync_permission_grant_resolves_relative_writes_against_cwd() {
        let requested = json!({
            "fileSystem": {
                "write": [
                    "src/lib.rs",
                    "../main/src/config.rs"
                ]
            }
        });

        assert_eq!(
            workspace_live_sync_codex_permission_grant(
                &requested,
                &[std::path::PathBuf::from("/repo/main")],
                Some(std::path::Path::new("/repo/other")),
            ),
            json!({
                "fileSystem": {
                    "write": ["src/lib.rs"]
                }
            })
        );

        assert_eq!(
            workspace_live_sync_codex_permission_grant(
                &requested,
                &[std::path::PathBuf::from("/repo/main")],
                Some(std::path::Path::new("/repo/main")),
            ),
            json!({})
        );
    }

    #[test]
    fn workspace_live_sync_client_does_not_approve_codex_filesystem_writes() {
        let client = CodexClient::new("run-1", "ws://127.0.0.1:43123")
            .expect("client should construct")
            .with_write_access_mode(ProviderWriteAccessMode::WorkspaceLiveSyncManaged)
            .with_workspace_live_sync_roots(&[std::path::PathBuf::from("/repo/main")]);
        let message = JsonRpcMessage {
            id: Some(json!(1)),
            method: Some("item/permissions/requestApproval".to_string()),
            params: Some(json!({
                "permissions": {
                    "fileSystem": {
                        "write": ["/repo/main/output"]
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
    fn workspace_live_sync_client_approves_relative_writes_outside_protected_roots() {
        let client = CodexClient::new("run-1", "ws://127.0.0.1:43123")
            .expect("client should construct")
            .with_write_access_mode(ProviderWriteAccessMode::WorkspaceLiveSyncManaged)
            .with_workspace_live_sync_roots(&[std::path::PathBuf::from("/repo/main")]);
        let message = JsonRpcMessage {
            id: Some(json!(1)),
            method: Some("item/permissions/requestApproval".to_string()),
            params: Some(json!({
                "cwd": "/repo/other",
                "permissions": {
                    "fileSystem": {
                        "write": ["src/lib.rs", "../main/src/lib.rs"]
                    }
                }
            })),
            result: None,
            error: None,
        };

        assert_eq!(
            client.permissions_approval_response(&message),
            json!({
                "permissions": {
                    "fileSystem": {
                        "write": ["src/lib.rs"]
                    }
                },
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
            ProviderWriteAccessMode::WorkspaceLiveSyncManaged,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
        );

        let overrides = client.thread_config_overrides(&policy).unwrap();

        assert_eq!(
            overrides.get("mcp_servers.arroba.url"),
            Some(&json!("http://127.0.0.1:43120/mcp"))
        );
        assert_eq!(
            overrides.get("mcp_servers.arroba.http_headers.Authorization"),
            Some(&json!("Bearer token-123"))
        );
        assert_eq!(
            overrides.get("mcp_servers.arroba.bearer_token_env_var"),
            None
        );
        assert_eq!(
            overrides.get("mcp_servers.arroba.required"),
            Some(&json!(true))
        );
        assert_eq!(
            overrides.get("mcp_servers.arroba.tool_timeout_sec"),
            Some(&json!(300))
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
            ProviderWriteAccessMode::WorkspaceLiveSyncManaged,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Yolo,
        );

        let overrides = client.thread_config_overrides(&policy).unwrap();

        assert_eq!(
            overrides.get("mcp_servers.browser.url"),
            Some(&json!("http://127.0.0.1:43120/mcp/proxy/browser"))
        );
        assert_eq!(
            overrides.get("mcp_servers.browser.http_headers.Authorization"),
            Some(&json!("Bearer token-123"))
        );
        assert_eq!(
            overrides.get("mcp_servers.browser.bearer_token_env_var"),
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
