use std::collections::BTreeMap;
use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::stream::MaybeTlsStream;
use tokio_tungstenite::tungstenite::{connect, Message, WebSocket};

use crate::error::DaemonError;
use crate::provider::{OpenCodeProviderCatalog, ProviderWriteAccessMode};

use super::resolve_codex_executable;

pub type CodexSocket = WebSocket<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone)]
pub struct CodexClient {
    provider_run_id: String,
    endpoint: String,
}

struct CodexPermissionPolicy {
    approval_policy: &'static str,
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

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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
            endpoint: endpoint.into(),
        })
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
    ) -> Result<CodexThreadStartResponse, DaemonError> {
        let policy = codex_permission_policy(write_access_mode);
        let mut params = json!({
            "approvalPolicy": policy.approval_policy,
            "sandbox": policy.sandbox,
            "sandboxPolicy": policy.sandbox_policy,
            "personality": "pragmatic",
            "ephemeral": true,
            "serviceName": "arroba",
        });
        if !policy.config_overrides.is_empty() {
            params["config"] = json!(policy.config_overrides);
        }
        if let Some(cwd) = cwd {
            params["cwd"] = json!(cwd);
        }
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        self.send_request(socket, next_request_id, "thread/start", params)
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
        input: Vec<Value>,
        buffered_notifications: &mut Vec<CodexNotification>,
    ) -> Result<Value, DaemonError> {
        let policy = codex_permission_policy(write_access_mode);
        let mut params = json!({
            "threadId": thread_id,
            "input": input,
            "approvalPolicy": policy.approval_policy,
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
        turn_id: &str,
    ) -> Result<(), DaemonError> {
        let _: Value = self.send_request(
            socket,
            next_request_id,
            "turn/interrupt",
            json!({ "turnId": turn_id }),
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
            if let Some(notification) = parse_notification(message) {
                buffered_notifications.push(notification);
            }
        }
    }

    pub fn read_notification(
        &self,
        socket: &mut CodexSocket,
        timeout: Duration,
    ) -> Result<Option<CodexNotification>, DaemonError> {
        set_socket_timeouts(socket, Some(timeout), Some(Duration::from_secs(5)))?;
        match socket.read() {
            Ok(message) => {
                let raw = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Message::Ping(_) | Message::Pong(_) => return Ok(None),
                    Message::Close(_) => {
                        return Ok(Some(CodexNotification::Error {
                            message: "Codex app-server closed the websocket".to_string(),
                        }))
                    }
                    Message::Frame(_) => return Ok(None),
                };
                let message: JsonRpcMessage = serde_json::from_str(&raw).map_err(|error| {
                    self.protocol_error("codex_notification_parse", error.to_string())
                })?;
                if self.respond_to_server_request(socket, &message)? {
                    return Ok(None);
                }
                Ok(parse_notification(message))
            }
            Err(tokio_tungstenite::tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(self.protocol_error("codex_read", error.to_string())),
        }
    }

    fn initialize_socket(&self, socket: &mut CodexSocket) -> Result<(), DaemonError> {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": 2,
                "clientInfo": {
                    "name": "arroba-daemon",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {},
                "notifications": [],
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
                    return Ok(String::from_utf8_lossy(&bytes).into_owned())
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                Ok(Message::Close(_)) => {
                    return Err(self.protocol_error(
                        "codex_read",
                        "Codex app-server closed the websocket".to_string(),
                    ))
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
        let result = match method {
            "item/commandExecution/requestApproval" => json!({ "decision": "decline" }),
            "item/fileChange/requestApproval" => json!({ "decision": "decline" }),
            "item/permissions/requestApproval" => json!({ "permissions": {} }),
            _ => return Ok(false),
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

    fn protocol_error(&self, operation: &'static str, message: String) -> DaemonError {
        DaemonError::ProviderProtocol {
            provider_run_id: self.provider_run_id.clone(),
            operation,
            message,
        }
    }
}

fn codex_permission_policy(write_access_mode: ProviderWriteAccessMode) -> CodexPermissionPolicy {
    match write_access_mode {
        ProviderWriteAccessMode::Unrestricted => CodexPermissionPolicy {
            approval_policy: "never",
            sandbox: "danger-full-access",
            sandbox_policy: json!({ "type": "dangerFullAccess" }),
            config_overrides: BTreeMap::new(),
        },
        ProviderWriteAccessMode::ManagedIoRequired => {
            let mut config_overrides = BTreeMap::new();
            config_overrides.insert("features.shell_tool".to_string(), json!(false));
            config_overrides.insert("include_apply_patch_tool".to_string(), json!(false));
            config_overrides.insert("features.apply_patch_freeform".to_string(), json!(false));
            CodexPermissionPolicy {
                approval_policy: "on-request",
                sandbox: "read-only",
                sandbox_policy: json!({
                    "type": "readOnly",
                    "access": {
                        "type": "restricted",
                        "includePlatformDefaults": true,
                        "readableRoots": []
                    }
                }),
                config_overrides,
            }
        }
    }
}

pub fn codex_endpoint_is_healthy(endpoint: &str) -> bool {
    CodexClient::new("catalog", endpoint)
        .and_then(|client| client.connect_initialized())
        .is_ok()
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
        "turn/started" => Some(CodexNotification::TurnStarted {
            turn_id: params
                .get("turn")
                .and_then(|turn| turn.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "turn/completed" => Some(CodexNotification::TurnCompleted {
            turn_id: params
                .get("turn")
                .and_then(|turn| turn.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
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

    use crate::provider::ProviderWriteAccessMode;

    use super::{codex_permission_policy, parse_notification, CodexNotification, JsonRpcMessage};

    #[test]
    fn managed_io_permission_policy_uses_read_only_sandbox() {
        let policy = codex_permission_policy(ProviderWriteAccessMode::ManagedIoRequired);

        assert_eq!(policy.approval_policy, "on-request");
        assert_eq!(policy.sandbox, "read-only");
        assert_eq!(
            policy.sandbox_policy,
            json!({
                "type": "readOnly",
                "access": {
                    "type": "restricted",
                    "includePlatformDefaults": true,
                    "readableRoots": []
                }
            })
        );
        assert_eq!(
            policy.config_overrides.get("features.shell_tool"),
            Some(&json!(false))
        );
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
    }
}
