use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::stream::MaybeTlsStream;
use tokio_tungstenite::tungstenite::{connect, Message, WebSocket};

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};
use crate::provider::{
    AgentExecutionMode, AgentPermissionLevel, OpenCodeProviderCatalog,
    ProviderNativeInteractionBridge, ProviderRunTokenUsage, ProviderWriteAccessMode,
};

use super::codex::CODEX_MCP_TOKEN_ENV;
use super::resolve_codex_executable;

pub type CodexSocket = WebSocket<MaybeTlsStream<TcpStream>>;

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

struct CodexPermissionPolicy {
    approval_policy: Value,
    sandbox: &'static str,
    sandbox_policy: Value,
    config_overrides: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexRunSelection {
    pub model: Option<String>,
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAuthStatus {
    pub provider: String,
    pub auth_state: String,
    pub account_profile: Option<String>,
    pub login_hint: Option<String>,
    pub detected_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderLoginStart {
    pub provider: String,
    pub login_kind: String,
    pub login_id: Option<String>,
    pub auth_url: Option<String>,
    pub verification_url: Option<String>,
    pub user_code: Option<String>,
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
        turn_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcMessage {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexGetAccountResponse {
    account: Option<CodexAccount>,
    #[serde(rename = "requiresOpenaiAuth")]
    requires_openai_auth: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexAccount {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexLoginStartResponse {
    #[serde(rename = "type")]
    login_kind: String,
    #[serde(rename = "loginId", default)]
    login_id: Option<String>,
    #[serde(rename = "authUrl", default)]
    auth_url: Option<String>,
    #[serde(rename = "verificationUrl", default)]
    verification_url: Option<String>,
    #[serde(rename = "userCode", default)]
    user_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexModelListResponse {
    data: Vec<CodexModel>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexModel {
    id: String,
    model: String,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(default)]
    hidden: bool,
    #[serde(rename = "supportedReasoningEfforts", default)]
    supported_reasoning_efforts: Vec<CodexReasoningEffort>,
    #[serde(rename = "isDefault", default)]
    is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexReasoningEffort {
    #[serde(rename = "reasoningEffort")]
    reasoning_effort: String,
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

    pub fn connect_initialized(&self) -> Result<CodexSocket, DaemonError> {
        let (mut socket, _) = connect(self.endpoint.as_str())
            .map_err(|error| self.protocol_error("codex_connect", error.to_string()))?;
        set_socket_timeouts(
            &mut socket,
            Some(Duration::from_secs(10)),
            Some(Duration::from_secs(10)),
        )?;
        self.initialize_socket(&mut socket)?;
        Ok(socket)
    }

    pub fn thread_start(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        cwd: Option<&str>,
        model: Option<&str>,
        write_access_mode: ProviderWriteAccessMode,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
    ) -> Result<CodexThreadStartResponse, DaemonError> {
        let policy = codex_permission_policy(write_access_mode, execution_mode, permission_level);
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex thread/start policy",
            json!({
                "provider_run_id": self.provider_run_id,
                "write_access_mode": format!("{write_access_mode:?}"),
                "execution_mode": format!("{execution_mode:?}"),
                "permission_level": format!("{permission_level:?}"),
                "approval_policy": policy.approval_policy,
                "sandbox": policy.sandbox,
                "cwd": cwd,
                "model": model,
            }),
        );
        let mut params = json!({
            "approvalPolicy": policy.approval_policy,
            "approvalsReviewer": "user",
            "sandbox": policy.sandbox,
            "sandboxPolicy": policy.sandbox_policy,
            "personality": "pragmatic",
            "ephemeral": true,
            "serviceName": "arroba",
        });
        let config_overrides = self.thread_config_overrides(&policy)?;
        if !config_overrides.is_empty() {
            self.log_thread_config_overrides("thread/start", &config_overrides);
            params["config"] = json!(config_overrides);
        }
        if let Some(cwd) = cwd {
            params["cwd"] = json!(cwd);
        }
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        self.send_request(socket, next_request_id, "thread/start", params)
    }

    pub fn thread_resume(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        thread_id: &str,
        cwd: Option<&str>,
        model: Option<&str>,
        write_access_mode: ProviderWriteAccessMode,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
    ) -> Result<CodexThreadStartResponse, DaemonError> {
        let policy = codex_permission_policy(write_access_mode, execution_mode, permission_level);
        crate::logging::info_with_fields(
            "daemon.provider.codex",
            "codex thread/resume policy",
            json!({
                "provider_run_id": self.provider_run_id,
                "write_access_mode": format!("{write_access_mode:?}"),
                "execution_mode": format!("{execution_mode:?}"),
                "permission_level": format!("{permission_level:?}"),
                "approval_policy": policy.approval_policy,
                "sandbox": policy.sandbox,
                "cwd": cwd,
                "model": model,
            }),
        );
        let mut params = json!({
            "threadId": thread_id,
            "approvalPolicy": policy.approval_policy,
            "approvalsReviewer": "user",
            "sandbox": policy.sandbox,
            "sandboxPolicy": policy.sandbox_policy,
            "personality": "pragmatic",
        });
        let config_overrides = self.thread_config_overrides(&policy)?;
        if !config_overrides.is_empty() {
            self.log_thread_config_overrides("thread/resume", &config_overrides);
            params["config"] = json!(config_overrides);
        }
        if let Some(cwd) = cwd {
            params["cwd"] = json!(cwd);
        }
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        self.send_request(socket, next_request_id, "thread/resume", params)
    }

    pub fn turn_start(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        thread_id: &str,
        cwd: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        write_access_mode: ProviderWriteAccessMode,
        execution_mode: AgentExecutionMode,
        permission_level: AgentPermissionLevel,
        input: Vec<Value>,
        buffered_notifications: &mut Vec<CodexNotification>,
    ) -> Result<Value, DaemonError> {
        let policy = codex_permission_policy(write_access_mode, execution_mode, permission_level);
        let mut params = json!({
            "threadId": thread_id,
            "input": input,
            "approvalPolicy": policy.approval_policy,
            "approvalsReviewer": "user",
            "personality": "pragmatic",
            "sandbox": policy.sandbox,
            "sandboxPolicy": policy.sandbox_policy,
            "summary": "detailed",
        });
        if let Some(cwd) = cwd {
            params["cwd"] = json!(cwd);
        }
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        if let Some(effort) = effort {
            params["effort"] = json!(effort);
        }
        if let Some(collaboration_mode) = codex_collaboration_mode(execution_mode, model, effort) {
            params["collaborationMode"] = collaboration_mode;
        }
        self.send_request_buffering_notifications(
            socket,
            next_request_id,
            "turn/start",
            params,
            buffered_notifications,
        )
    }

    pub fn turn_interrupt(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), DaemonError> {
        let _: Value = self.send_request(
            socket,
            next_request_id,
            "turn/interrupt",
            json!({
                "threadId": thread_id,
                "turnId": turn_id,
            }),
        )?;
        Ok(())
    }

    pub fn provider_catalog(&self) -> Result<OpenCodeProviderCatalog, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexModelListResponse =
            self.send_request(&mut socket, &mut next_request_id, "model/list", json!({}))?;
        Ok(codex_catalog_from_models(response.data))
    }

    pub fn auth_status(&self) -> Result<ProviderAuthStatus, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexGetAccountResponse =
            self.send_request(&mut socket, &mut next_request_id, "account/read", json!({}))?;
        Ok(ProviderAuthStatus {
            provider: "codex".to_string(),
            auth_state: if response.account.is_some() {
                "authenticated".to_string()
            } else if response.requires_openai_auth {
                "not_logged_in".to_string()
            } else {
                "unknown".to_string()
            },
            account_profile: response.account.and_then(|account| account.email),
            login_hint: Some("Run /provider login codex to authenticate Codex.".to_string()),
            detected_version: codex_version().ok(),
        })
    }

    pub fn start_login(&self) -> Result<ProviderLoginStart, DaemonError> {
        let mut socket = self.connect_initialized()?;
        let mut next_request_id = 1;
        let response: CodexLoginStartResponse = self.send_request(
            &mut socket,
            &mut next_request_id,
            "account/login/start",
            json!({ "type": "chatgptDeviceCode" }),
        )?;
        Ok(ProviderLoginStart {
            provider: "codex".to_string(),
            login_kind: response.login_kind,
            login_id: response.login_id,
            auth_url: response.auth_url,
            verification_url: response.verification_url,
            user_code: response.user_code,
        })
    }

    pub fn send_request<T: for<'de> Deserialize<'de>>(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        method: &'static str,
        params: Value,
    ) -> Result<T, DaemonError> {
        self.send_request_buffering_notifications(
            socket,
            next_request_id,
            method,
            params,
            &mut Vec::new(),
        )
    }

    pub fn send_request_buffering_notifications<T: for<'de> Deserialize<'de>>(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        method: &'static str,
        params: Value,
        buffered_notifications: &mut Vec<CodexNotification>,
    ) -> Result<T, DaemonError> {
        let request_id = *next_request_id;
        *next_request_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        socket
            .send(Message::Text(payload.to_string().into()))
            .map_err(|error| self.protocol_error("codex_write", error.to_string()))?;

        loop {
            let raw = self.read_next_message(socket, Duration::from_secs(30))?;
            let message: JsonRpcMessage = serde_json::from_str(&raw)
                .map_err(|error| self.protocol_error("codex_read_parse", error.to_string()))?;
            if self.respond_to_server_request(socket, &message)? {
                continue;
            }
            if message.id.as_ref() == Some(&json!(request_id)) {
                if let Some(error) = rpc_error_message(&message) {
                    return Err(self.protocol_error(method, error));
                }
                let result = message.result.ok_or_else(|| {
                    self.protocol_error(method, "Codex returned no response payload".to_string())
                })?;
                return serde_json::from_value(result)
                    .map_err(|error| self.protocol_error(method, error.to_string()));
            }
            if let Some(notification) = parse_notification(message.clone()) {
                buffered_notifications.push(notification);
            } else if let Some(message_method) = message.method.as_deref() {
                crate::logging::debug_with_fields(
                    "daemon.provider.codex",
                    "ignored codex message while awaiting response",
                    json!({
                        "provider_run_id": self.provider_run_id,
                        "awaiting_method": method,
                        "message_method": message_method,
                        "has_id": message.id.is_some(),
                        "params": message.params,
                        "error": message.error,
                    }),
                );
            }
        }
    }

    pub fn read_notification(
        &self,
        socket: &mut CodexSocket,
        timeout: Duration,
    ) -> Result<Option<CodexNotification>, DaemonError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            set_socket_timeouts(socket, Some(remaining), Some(Duration::from_secs(5)))?;
            match socket.read() {
                Ok(message) => {
                    let raw = match message {
                        Message::Text(text) => text.to_string(),
                        Message::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                        Message::Close(_) => {
                            return Ok(Some(CodexNotification::Error {
                                message: "Codex app-server closed the websocket".to_string(),
                            }));
                        }
                    };
                    let message: JsonRpcMessage = serde_json::from_str(&raw).map_err(|error| {
                        self.protocol_error("codex_notification_parse", error.to_string())
                    })?;
                    if self.respond_to_server_request(socket, &message)? {
                        continue;
                    }
                    let notification = parse_notification(message.clone());
                    if let Some(notification) = notification {
                        return Ok(Some(notification));
                    }
                    if let Some(method) = message.method.as_deref() {
                        crate::logging::debug_with_fields(
                            "daemon.provider.codex",
                            "ignored codex notification",
                            json!({
                                "provider_run_id": self.provider_run_id,
                                "method": method,
                                "has_id": message.id.is_some(),
                                "params": message.params,
                                "error": message.error,
                            }),
                        );
                    }
                    continue;
                }
                Err(tokio_tungstenite::tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(self.protocol_error("codex_read", error.to_string())),
            }
        }
    }

    fn initialize_socket(&self, socket: &mut CodexSocket) -> Result<(), DaemonError> {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "arroba-kernel",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                },
            },
        });
        socket
            .send(Message::Text(initialize.to_string().into()))
            .map_err(|error| self.protocol_error("codex_initialize", error.to_string()))?;
        let response = self.read_next_message(socket, Duration::from_secs(10))?;
        let message: JsonRpcMessage = serde_json::from_str(&response)
            .map_err(|error| self.protocol_error("codex_initialize_parse", error.to_string()))?;
        if message.result.is_none() {
            return Err(self.protocol_error(
                "codex_initialize",
                rpc_error_message(&message)
                    .unwrap_or_else(|| "Codex returned no initialize result".to_string()),
            ));
        }
        let initialized = json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {},
        });
        socket
            .send(Message::Text(initialized.to_string().into()))
            .map_err(|error| self.protocol_error("codex_initialized", error.to_string()))?;
        Ok(())
    }

    fn read_next_message(
        &self,
        socket: &mut CodexSocket,
        timeout: Duration,
    ) -> Result<String, DaemonError> {
        set_socket_timeouts(socket, Some(timeout), Some(Duration::from_secs(5)))?;
        loop {
            match socket.read() {
                Ok(Message::Text(text)) => return Ok(text.to_string()),
                Ok(Message::Binary(bytes)) => {
                    return Ok(String::from_utf8_lossy(&bytes).into_owned());
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                Ok(Message::Close(_)) => {
                    return Err(self.protocol_error(
                        "codex_read",
                        "Codex app-server closed the websocket".to_string(),
                    ));
                }
                Ok(Message::Frame(_)) => continue,
                Err(error) => return Err(self.protocol_error("codex_read", error.to_string())),
            }
        }
    }

    fn respond_to_server_request(
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

    fn permissions_approval_response(&self, message: &JsonRpcMessage) -> Value {
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
            ProviderWriteAccessMode::ManagedIoRequired => {
                let granted_permissions = managed_io_codex_permission_grant(&requested_permissions);
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
        let command = params
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("<unknown command>");
        let cwd = params
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let reason = params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let mut body = format!("Approve command execution?\n\n{command}");
        if let Some(cwd) = cwd {
            body.push_str(&format!("\n\ncwd: {cwd}"));
        }
        if let Some(reason) = reason {
            body.push_str(&format!("\n\nreason: {reason}"));
        }
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
        let reason = params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let changes = params
            .get("changes")
            .map(render_pretty_json)
            .filter(|value| !value.trim().is_empty());
        let mut body = "Approve file changes?".to_string();
        if let Some(reason) = reason {
            body.push_str(&format!("\n\nreason: {reason}"));
        }
        if let Some(changes) = changes {
            body.push_str(&format!("\n\nchanges:\n{changes}"));
        }
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
        let command = params
            .get("command")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "<unknown command>".to_string());
        let cwd = params
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let reason = params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let parsed = params
            .get("parsedCmd")
            .map(render_pretty_json)
            .filter(|value| !value.trim().is_empty());
        let mut body = format!("Approve command execution?\n\n{command}");
        if let Some(cwd) = cwd {
            body.push_str(&format!("\n\ncwd: {cwd}"));
        }
        if let Some(reason) = reason {
            body.push_str(&format!("\n\nreason: {reason}"));
        }
        if let Some(parsed) = parsed {
            body.push_str(&format!("\n\nparsed:\n{parsed}"));
        }
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
        let reason = params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let grant_root = params
            .get("grantRoot")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let changes = params
            .get("fileChanges")
            .map(render_pretty_json)
            .filter(|value| !value.trim().is_empty());
        let mut body = "Approve file changes?".to_string();
        if let Some(reason) = reason {
            body.push_str(&format!("\n\nreason: {reason}"));
        }
        if let Some(grant_root) = grant_root {
            body.push_str(&format!("\n\ngrant_root: {grant_root}"));
        }
        if let Some(changes) = changes {
            body.push_str(&format!("\n\nchanges:\n{changes}"));
        }
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
        let interaction = crate::session::RuntimeInteraction::new(
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
        );
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
        let interaction = crate::session::RuntimeInteraction::new(
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
        );
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

    fn thread_config_overrides(
        &self,
        policy: &CodexPermissionPolicy,
    ) -> Result<BTreeMap<String, Value>, DaemonError> {
        let mut overrides = policy.config_overrides.clone();
        let provider_mcp_servers =
            super::mcp_proxy::provider_facing_mcp_proxy_configs_with_bearer_env(
                &self.mcp_servers,
                self.runtime_mcp_server_url.as_deref(),
                self.runtime_mcp_auth_token.as_deref(),
                CODEX_MCP_TOKEN_ENV,
            )?;
        append_codex_mcp_overrides(&mut overrides, &provider_mcp_servers);
        if let (Some(server_url), Some(auth_token)) = (
            self.runtime_mcp_server_url.as_deref(),
            self.runtime_mcp_auth_token.as_deref(),
        ) {
            append_runtime_mcp_overrides(&mut overrides, server_url, auth_token);
        }
        Ok(overrides)
    }

    fn log_thread_config_overrides(
        &self,
        operation: &'static str,
        overrides: &BTreeMap<String, Value>,
    ) {
        crate::logging::debug_with_fields(
            "daemon.provider.codex",
            "sending codex thread config overrides",
            json!({
                "provider_run_id": self.provider_run_id,
                "operation": operation,
                "runtime_mcp_binding_present": self.runtime_mcp_server_url.is_some()
                    && self.runtime_mcp_auth_token.is_some(),
                "granted_mcp_servers": self
                    .mcp_servers
                    .iter()
                    .map(|server| server.name.as_str())
                    .collect::<Vec<_>>(),
                "config_override_keys": overrides.keys().cloned().collect::<Vec<_>>(),
            }),
        );
    }

    fn protocol_error(&self, operation: &'static str, message: String) -> DaemonError {
        DaemonError::ProviderProtocol {
            provider_run_id: self.provider_run_id.clone(),
            operation,
            message,
        }
    }
}

fn append_runtime_mcp_overrides(
    overrides: &mut BTreeMap<String, Value>,
    server_url: &str,
    _auth_token: &str,
) {
    overrides.insert(
        "mcp_servers.arroba.url".to_string(),
        json!(server_url.to_string()),
    );
    overrides.insert(
        "mcp_servers.arroba.bearer_token_env_var".to_string(),
        json!(CODEX_MCP_TOKEN_ENV),
    );
    overrides.insert("mcp_servers.arroba.required".to_string(), json!(true));
    overrides.insert("mcp_servers.arroba.tool_timeout_sec".to_string(), json!(15));
}

fn append_codex_mcp_overrides(
    overrides: &mut BTreeMap<String, Value>,
    servers: &[ArrobaMcpServerConfig],
) {
    for server in servers {
        let prefix = format!("mcp_servers.{}", server.name);
        match &server.transport {
            ArrobaMcpTransportConfig::Stdio {
                command,
                args,
                env,
                env_vars,
                cwd,
            } => {
                overrides.insert(format!("{prefix}.command"), json!(command));
                if !args.is_empty() {
                    overrides.insert(format!("{prefix}.args"), json!(args));
                }
                for (key, value) in env {
                    overrides.insert(format!("{prefix}.env.{key}"), json!(value));
                }
                if !env_vars.is_empty() {
                    overrides.insert(format!("{prefix}.env_vars"), json!(env_vars));
                }
                if let Some(cwd) = cwd {
                    overrides.insert(format!("{prefix}.cwd"), json!(cwd.display().to_string()));
                }
            }
            ArrobaMcpTransportConfig::StreamableHttp {
                url,
                bearer_token_env_var,
                http_headers,
                env_http_headers,
            } => {
                overrides.insert(format!("{prefix}.url"), json!(url));
                if let Some(env_var) = bearer_token_env_var {
                    overrides.insert(format!("{prefix}.bearer_token_env_var"), json!(env_var));
                }
                for (key, value) in http_headers {
                    overrides.insert(format!("{prefix}.http_headers.{key}"), json!(value));
                }
                for (key, value) in env_http_headers {
                    overrides.insert(format!("{prefix}.env_http_headers.{key}"), json!(value));
                }
            }
        }
        overrides.insert(format!("{prefix}.enabled"), json!(server.enabled));
        if server.required {
            overrides.insert(format!("{prefix}.required"), json!(true));
        }
        if let Some(timeout) = server.startup_timeout_sec {
            overrides.insert(format!("{prefix}.startup_timeout_sec"), json!(timeout));
        }
        if let Some(timeout) = server.tool_timeout_sec {
            overrides.insert(format!("{prefix}.tool_timeout_sec"), json!(timeout));
        }
        if let Some(enabled_tools) = &server.enabled_tools {
            overrides.insert(format!("{prefix}.enabled_tools"), json!(enabled_tools));
        }
        if let Some(disabled_tools) = &server.disabled_tools {
            overrides.insert(format!("{prefix}.disabled_tools"), json!(disabled_tools));
        }
    }
}

fn codex_permission_policy(
    write_access_mode: ProviderWriteAccessMode,
    execution_mode: AgentExecutionMode,
    permission_level: AgentPermissionLevel,
) -> CodexPermissionPolicy {
    match write_access_mode {
        ProviderWriteAccessMode::Unrestricted => {
            let yolo_build = execution_mode == AgentExecutionMode::Build
                && permission_level == AgentPermissionLevel::Yolo;
            CodexPermissionPolicy {
                approval_policy: match permission_level {
                    AgentPermissionLevel::Required => json!({
                        "granular": {
                            "mcp_elicitations": true,
                            "request_permissions": true,
                            "rules": true,
                            "sandbox_approval": true
                        }
                    }),
                    AgentPermissionLevel::Yolo => json!("never"),
                },
                sandbox: match (execution_mode, yolo_build) {
                    (AgentExecutionMode::Build, true) => "danger-full-access",
                    (AgentExecutionMode::Build, false) => "workspace-write",
                    (AgentExecutionMode::Plan, _) => "read-only",
                },
                sandbox_policy: match (execution_mode, yolo_build) {
                    (AgentExecutionMode::Build, true) => json!({ "type": "dangerFullAccess" }),
                    (AgentExecutionMode::Build, false) => json!({ "type": "workspaceWrite" }),
                    (AgentExecutionMode::Plan, _) => json!({ "type": "readOnly" }),
                },
                config_overrides: BTreeMap::new(),
            }
        }
        ProviderWriteAccessMode::ManagedIoRequired => {
            let mut config_overrides = BTreeMap::new();
            config_overrides.insert("include_apply_patch_tool".to_string(), json!(false));
            config_overrides.insert("features.apply_patch_freeform".to_string(), json!(false));
            CodexPermissionPolicy {
                approval_policy: json!("never"),
                sandbox: "read-only",
                sandbox_policy: json!({ "type": "readOnly" }),
                config_overrides,
            }
        }
    }
}

fn codex_collaboration_mode(
    execution_mode: AgentExecutionMode,
    model: Option<&str>,
    effort: Option<&str>,
) -> Option<Value> {
    let model = model?.trim();
    if model.is_empty() {
        return None;
    }
    let effort = effort
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Value::from)
        .unwrap_or(Value::Null);
    let mode = match execution_mode {
        AgentExecutionMode::Build => "default",
        AgentExecutionMode::Plan => "plan",
    };
    Some(json!({
        "mode": mode,
        "settings": {
            "model": model,
            "reasoning_effort": effort,
            "developer_instructions": Value::Null,
        }
    }))
}

fn managed_io_codex_permission_grant(requested_permissions: &Value) -> Value {
    let Some(requested) = requested_permissions.as_object() else {
        return json!({});
    };
    let mut granted = serde_json::Map::new();
    if let Some(network) = requested.get("network") {
        granted.insert("network".to_string(), network.clone());
    }
    if let Some(file_system) = requested.get("fileSystem").and_then(Value::as_object) {
        let mut granted_file_system = serde_json::Map::new();
        if let Some(read) = file_system.get("read") {
            granted_file_system.insert("read".to_string(), read.clone());
        }
        if !granted_file_system.is_empty() {
            granted.insert("fileSystem".to_string(), Value::Object(granted_file_system));
        }
    }
    Value::Object(granted)
}

fn render_pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn call_runtime_mcp_tool(
    server_url: &str,
    auth_token: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<String, DaemonError> {
    let endpoint = parse_http_endpoint(server_url)?;
    let payload = json!({
        "jsonrpc": "2.0",
        "id": format!("codex-dynamic-tool-{}", crate::session::unix_epoch_ms()),
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments,
        }
    });
    let body = serde_json::to_vec(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "codex_dynamic_tool_serialize",
        message: error.to_string(),
    })?;
    let mut stream = TcpStream::connect((&*endpoint.host, endpoint.port)).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_connect",
            message: error.to_string(),
        }
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_timeout",
            message: error.to_string(),
        })?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_timeout",
            message: error.to_string(),
        })?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.path,
        endpoint.host,
        endpoint.port,
        auth_token,
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_write",
            message: error.to_string(),
        })?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_read",
            message: error.to_string(),
        })?;
    parse_runtime_mcp_response(&response)
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_endpoint(server_url: &str) -> Result<HttpEndpoint, DaemonError> {
    let rest = server_url
        .strip_prefix("http://")
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_endpoint",
            message: "only http runtime MCP endpoints are supported".to_string(),
        })?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, "mcp"));
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_endpoint",
            message: "runtime MCP endpoint must include an explicit port".to_string(),
        })?;
    let port = port
        .parse::<u16>()
        .map_err(|_| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_endpoint",
            message: "runtime MCP endpoint port is invalid".to_string(),
        })?;
    Ok(HttpEndpoint {
        host: host.to_string(),
        port,
        path: format!("/{path}"),
    })
}

fn parse_runtime_mcp_response(response: &[u8]) -> Result<String, DaemonError> {
    let response_text = String::from_utf8_lossy(response);
    let (head, body) =
        response_text
            .split_once("\r\n\r\n")
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "codex_dynamic_tool_response",
                message: "invalid HTTP response from runtime MCP server".to_string(),
            })?;
    let status_ok = head
        .lines()
        .next()
        .is_some_and(|line| line.contains(" 200 "));
    if !status_ok {
        return Err(DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_response",
            message: head.lines().next().unwrap_or("HTTP error").to_string(),
        });
    }
    let value =
        serde_json::from_str::<Value>(body).map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_response",
            message: error.to_string(),
        })?;
    if let Some(error) = value.get("error") {
        return Err(DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_response",
            message: error.to_string(),
        });
    }
    value
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_response",
            message: "runtime MCP response did not include text content".to_string(),
        })
}

pub fn codex_endpoint_is_healthy(endpoint: &str) -> bool {
    codex_readyz_is_healthy(endpoint)
        || CodexClient::new("catalog", endpoint)
            .and_then(|client| client.connect_initialized())
            .is_ok()
}

fn codex_readyz_is_healthy(endpoint: &str) -> bool {
    let Ok(mut url) = url::Url::parse(endpoint) else {
        return false;
    };
    match url.scheme() {
        "ws" => {
            let _ = url.set_scheme("http");
        }
        "wss" => {
            let _ = url.set_scheme("https");
        }
        "http" | "https" => {}
        _ => return false,
    }
    url.set_path("/readyz");
    url.set_query(None);
    match ureq::get(url.as_str()).call() {
        Ok(response) => response.status() == 200,
        Err(_) => false,
    }
}

fn codex_version() -> Result<String, DaemonError> {
    let executable = resolve_codex_executable()?;
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_version",
            message: error.to_string(),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return Ok(stderr);
    }
    Err(DaemonError::LocalTransport {
        operation: "codex_version",
        message: "codex returned no version text".to_string(),
    })
}

fn parse_notification(message: JsonRpcMessage) -> Option<CodexNotification> {
    let method = message.method?;
    let params = message.params.unwrap_or(Value::Null);
    match method.as_str() {
        "item/agentMessage/delta" => Some(CodexNotification::AgentMessageDelta {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            delta: params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "item/reasoning/textDelta" => Some(CodexNotification::ReasoningTextDelta {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            delta: params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "item/reasoning/summaryTextDelta" => Some(CodexNotification::ReasoningSummaryTextDelta {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            delta: params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "item/reasoning/summaryPartAdded" => Some(CodexNotification::ReasoningSummaryPartAdded {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            summary_index: params
                .get("summaryIndex")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
        }),
        "item/started" => Some(CodexNotification::ItemStarted {
            item: params.get("item").cloned().unwrap_or(Value::Null),
        }),
        "item/completed" => Some(CodexNotification::ItemCompleted {
            item: params.get("item").cloned().unwrap_or(Value::Null),
        }),
        "codex/event/exec_command_begin" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ExecCommandStarted {
                call_id: msg
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                command: msg.get("command").cloned().unwrap_or(Value::Null),
                cwd: msg.get("cwd").and_then(Value::as_str).map(str::to_string),
            })
        }
        "codex/event/exec_command_end" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ExecCommandCompleted {
                call_id: msg
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                command: msg.get("command").cloned().unwrap_or(Value::Null),
                cwd: msg.get("cwd").and_then(Value::as_str).map(str::to_string),
                output: msg
                    .get("aggregated_output")
                    .or_else(|| msg.get("aggregatedOutput"))
                    .or_else(|| msg.get("formatted_output"))
                    .or_else(|| msg.get("stdout"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                exit_code: msg
                    .get("exit_code")
                    .or_else(|| msg.get("exitCode"))
                    .and_then(Value::as_i64),
                success: msg.get("success").and_then(Value::as_bool),
                stderr: msg
                    .get("stderr")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        }
        "codex/event/exec_command_output_delta" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ExecCommandOutputDelta {
                call_id: msg
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                chunk: msg
                    .get("chunk")
                    .or_else(|| msg.get("delta"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        "codex/event/patch_apply_begin" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ItemStarted {
                item: legacy_codex_file_change_item(&params, msg, "inProgress"),
            })
        }
        "codex/event/patch_apply_end" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ItemCompleted {
                item: legacy_codex_file_change_item(
                    &params,
                    msg,
                    if msg.get("success").and_then(Value::as_bool) == Some(false) {
                        "failed"
                    } else {
                        "completed"
                    },
                ),
            })
        }
        "item/commandExecution/outputDelta" => {
            Some(CodexNotification::CommandExecutionOutputDelta {
                item_id: params
                    .get("itemId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                delta: params
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        "item/fileChange/outputDelta" => Some(CodexNotification::FileChangeOutputDelta {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            delta: params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "item/mcpToolCall/progress" => Some(CodexNotification::McpToolCallProgress {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message: params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "thread/tokenUsage/updated" => {
            let token_usage = params.get("tokenUsage")?;
            Some(CodexNotification::TokenUsageUpdated {
                thread_id: params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                turn_id: params
                    .get("turnId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                usage: {
                    let total_tokens = token_usage
                        .get("total")
                        .and_then(|total| total.get("totalTokens"))
                        .and_then(Value::as_i64)
                        .and_then(|value| u64::try_from(value).ok());
                    let last_tokens = token_usage
                        .get("last")
                        .and_then(|last| last.get("totalTokens"))
                        .and_then(Value::as_i64)
                        .and_then(|value| u64::try_from(value).ok());
                    let context_window = token_usage
                        .get("modelContextWindow")
                        .and_then(Value::as_i64)
                        .and_then(|value| u64::try_from(value).ok());
                    let context_tokens = match (last_tokens, context_window) {
                        (Some(tokens), Some(window)) if tokens <= window => Some(tokens),
                        _ => None,
                    };

                    ProviderRunTokenUsage {
                        total_tokens,
                        last_tokens,
                        context_tokens,
                        context_window,
                    }
                },
            })
        }
        "turn/started" => Some(CodexNotification::TurnStarted {
            turn_id: params
                .get("turn")
                .and_then(|turn| turn.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "turn/completed" => Some(CodexNotification::TurnCompleted {
            turn_id: optional_codex_turn_id(params.get("turn")),
            status: params
                .get("turn")
                .and_then(|turn| turn.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("completed")
                .to_string(),
            error_message: params
                .get("turn")
                .and_then(|turn| turn.get("error"))
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
        "codex/event/task_complete" => Some(CodexNotification::TurnCompleted {
            turn_id: optional_legacy_event_turn_id(&params),
            status: "completed".to_string(),
            error_message: None,
        }),
        "codex/event/turn_aborted" => Some(CodexNotification::TurnCompleted {
            turn_id: optional_legacy_event_turn_id(&params),
            status: "interrupted".to_string(),
            error_message: legacy_event_error_message(&params),
        }),
        "error" => Some(CodexNotification::Error {
            message: params
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Codex reported an unknown error")
                .to_string(),
        }),
        _ => None,
    }
}

fn optional_codex_turn_id(turn: Option<&Value>) -> Option<String> {
    turn.and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .filter(|turn_id| !turn_id.is_empty())
        .map(str::to_string)
}

fn optional_legacy_event_turn_id(params: &Value) -> Option<String> {
    params
        .get("id")
        .or_else(|| params.get("turn_id"))
        .or_else(|| params.get("turnId"))
        .or_else(|| params.get("msg").and_then(|msg| msg.get("turn_id")))
        .or_else(|| params.get("msg").and_then(|msg| msg.get("turnId")))
        .and_then(Value::as_str)
        .filter(|turn_id| !turn_id.is_empty())
        .map(str::to_string)
}

fn legacy_event_error_message(params: &Value) -> Option<String> {
    params
        .get("msg")
        .and_then(|msg| {
            msg.get("error")
                .or_else(|| msg.get("reason"))
                .or_else(|| msg.get("message"))
        })
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

fn legacy_codex_file_change_item(params: &Value, msg: &Value, status: &str) -> Value {
    let id = msg
        .get("call_id")
        .or_else(|| msg.get("callId"))
        .or_else(|| msg.get("id"))
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("patch");
    json!({
        "type": "fileChange",
        "id": id,
        "status": status,
        "changes": msg.get("changes").cloned().unwrap_or_else(|| json!([])),
    })
}

fn rpc_error_message(message: &JsonRpcMessage) -> Option<String> {
    message
        .error
        .as_ref()
        .and_then(|error| error.message.clone())
}

fn set_socket_timeouts(
    socket: &mut CodexSocket,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
) -> Result<(), DaemonError> {
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream
            .set_read_timeout(read_timeout)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "codex_socket_timeout",
                message: error.to_string(),
            })?;
        stream
            .set_write_timeout(write_timeout)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "codex_socket_timeout",
                message: error.to_string(),
            })?;
    }
    Ok(())
}

fn codex_catalog_from_models(models: Vec<CodexModel>) -> OpenCodeProviderCatalog {
    let mut catalog_models = BTreeMap::new();
    let mut default = BTreeMap::new();
    let mut first_model = None;

    for model in models.into_iter().filter(|model| !model.hidden) {
        let model_id = model.model.clone();
        if first_model.is_none() {
            first_model = Some(model_id.clone());
        }
        if model.is_default {
            default.insert("codex".to_string(), model_id.clone());
        }
        let variants = model
            .supported_reasoning_efforts
            .into_iter()
            .map(|entry| (entry.reasoning_effort, Value::Object(Default::default())))
            .collect::<BTreeMap<_, _>>();
        catalog_models.insert(
            model_id.clone(),
            crate::provider::OpenCodeProviderModel {
                id: model_id,
                name: model.display_name.unwrap_or_else(|| model.id.clone()),
                status: "active".to_string(),
                limit: None,
                variants,
            },
        );
    }

    if default.is_empty() {
        if let Some(model) = first_model {
            default.insert("codex".to_string(), model);
        }
    }

    OpenCodeProviderCatalog {
        all: vec![crate::provider::OpenCodeProviderInfo {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            remote_machine_aliases: Vec::new(),
            models: catalog_models,
        }],
        default,
        connected: vec!["codex".to_string()],
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
    fn unrestricted_required_policy_enables_permission_escalation_reviews() {
        let policy = codex_permission_policy(
            ProviderWriteAccessMode::Unrestricted,
            AgentExecutionMode::Build,
            AgentPermissionLevel::Required,
        );

        assert_eq!(
            policy.approval_policy,
            json!({
                "granular": {
                    "mcp_elicitations": true,
                    "request_permissions": true,
                    "rules": true,
                    "sandbox_approval": true
                }
            })
        );
        assert_eq!(policy.sandbox, "workspace-write");
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
                turn_id: Some("turn-1".to_string()),
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
        assert_eq!(
            v2_completed_without_id,
            Some(CodexNotification::TurnCompleted {
                turn_id: None,
                status: "completed".to_string(),
                error_message: None,
            })
        );

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
        assert_eq!(
            raw_task_complete,
            Some(CodexNotification::TurnCompleted {
                turn_id: Some("turn-raw-1".to_string()),
                status: "completed".to_string(),
                error_message: None,
            })
        );

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
        assert_eq!(
            raw_turn_aborted,
            Some(CodexNotification::TurnCompleted {
                turn_id: Some("turn-raw-2".to_string()),
                status: "interrupted".to_string(),
                error_message: Some("interrupted".to_string()),
            })
        );
    }
}
