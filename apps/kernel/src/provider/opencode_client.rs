use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::provider::AgentExecutionMode;
use crate::session::PromptAttachment;

mod events;

use events::parse_sse_event;

#[derive(Debug, Clone)]
pub struct OpenCodeClient {
    provider_run_id: String,
    base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeSessionSnapshot {
    pub status: String,
    pub messages: Vec<OpenCodeMessage>,
}

#[derive(Debug)]
pub struct OpenCodeEventSubscription {
    pub receiver: Receiver<OpenCodeEvent>,
    stop: Arc<AtomicBool>,
}

impl OpenCodeEventSubscription {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn for_tests(receiver: Receiver<OpenCodeEvent>) -> Self {
        Self {
            receiver,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenCodeEvent {
    MessageUpdated {
        info: OpenCodeMessageInfo,
    },
    MessagePartDelta {
        session_id: String,
        message_id: String,
        part_id: String,
        field: String,
        delta: String,
    },
    MessagePartUpdated {
        part: Box<OpenCodePart>,
    },
    SessionError {
        session_id: String,
        message: String,
    },
    SessionStatus {
        session_id: String,
        kind: String,
    },
    PermissionAsked {
        request: OpenCodePermissionRequest,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodePermissionRequest {
    pub id: String,
    pub session_id: String,
    pub permission: String,
    pub tool: Option<String>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub reason: Option<String>,
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessage {
    pub info: OpenCodeMessageInfo,
    #[serde(default)]
    pub parts: Vec<OpenCodePart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageInfo {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub role: String,
    #[serde(rename = "parentID", default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub finish: Option<String>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(rename = "providerID", default)]
    pub provider_id: Option<String>,
    #[serde(rename = "modelID", default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model: Option<OpenCodeSelectedModel>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub tokens: OpenCodeMessageTokens,
    #[serde(default)]
    pub time: OpenCodeMessageTime,
}

impl OpenCodeMessageInfo {
    pub fn is_tool_call_only_completion(&self) -> bool {
        self.finish.as_deref() == Some("tool-calls")
    }

    pub fn is_terminal_assistant_completion(&self) -> bool {
        if self.error.is_some() {
            return true;
        }
        self.time.completed.is_some()
            && self
                .finish
                .as_deref()
                .is_some_and(|finish| finish != "tool-calls" && finish != "unknown")
    }

    pub fn resolved_model(&self) -> Option<String> {
        if let (Some(provider_id), Some(model_id)) =
            (self.provider_id.as_deref(), self.model_id.as_deref())
        {
            return Some(format!("{provider_id}/{model_id}"));
        }

        self.model
            .as_ref()
            .map(|model| format!("{}/{}", model.provider_id, model.model_id))
    }

    pub fn resolved_variant(&self) -> Option<String> {
        self.variant
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    pub fn total_tokens(&self) -> u64 {
        self.tokens.total()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageTokens {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub reasoning: u64,
    #[serde(default)]
    pub cache: OpenCodeMessageCacheTokens,
}

impl OpenCodeMessageTokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.reasoning + self.cache.read + self.cache.write
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageCacheTokens {
    #[serde(default)]
    pub read: u64,
    #[serde(default)]
    pub write: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeSelectedModel {
    #[serde(rename = "providerID")]
    pub provider_id: String,
    #[serde(rename = "modelID")]
    pub model_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageTime {
    #[serde(default)]
    pub completed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodePart {
    pub id: String,
    #[serde(rename = "sessionID", default)]
    pub session_id: String,
    #[serde(rename = "messageID", default)]
    pub message_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub state: Option<OpenCodeToolState>,
    #[serde(default)]
    pub time: Option<OpenCodePartTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodeToolState {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub raw: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodePartTime {
    #[serde(default)]
    pub end: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OpenCodeHealth {
    healthy: bool,
}

#[derive(Debug, Deserialize)]
struct OpenCodeConfig {
    #[serde(default)]
    model: Option<String>,
    #[serde(rename = "default_agent", default)]
    default_agent: Option<String>,
    #[serde(default)]
    agent: BTreeMap<String, OpenCodeConfigAgent>,
    #[serde(default)]
    mode: BTreeMap<String, OpenCodeConfigAgent>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenCodeConfigAgent {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenCodeConfiguredDefaults {
    pub model: Option<String>,
    pub variant: Option<String>,
    pub selected_agent: Option<String>,
    pub agent_model: Option<String>,
    pub agent_variant: Option<String>,
    pub top_level_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeProviderCatalog {
    pub all: Vec<OpenCodeProviderInfo>,
    pub default: BTreeMap<String, String>,
    pub connected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeProviderInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub remote_machine_aliases: Vec<String>,
    #[serde(default)]
    pub models: BTreeMap<String, OpenCodeProviderModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeProviderModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub limit: Option<OpenCodeProviderModelLimit>,
    #[serde(default)]
    pub variants: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCodeProviderModelLimit {
    pub context: u64,
    #[serde(default)]
    pub input: Option<u64>,
    #[serde(default)]
    pub output: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenCodeAgentInfo {
    name: String,
    mode: String,
    hidden: Option<bool>,
    model: Option<OpenCodeSelectedModel>,
    variant: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSessionCreated {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSessionStatus {
    #[serde(rename = "type")]
    kind: String,
}

impl OpenCodeClient {
    pub fn new(
        provider_run_id: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            provider_run_id: provider_run_id.into(),
            base_url: base_url.into(),
        })
    }

    pub fn wait_until_healthy(&self, timeout: Duration) -> Result<(), DaemonError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.health() {
                Ok(()) => return Ok(()),
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub fn check_health(&self) -> Result<(), DaemonError> {
        self.health()
    }

    pub fn create_session(&self, permission: Option<Value>) -> Result<String, DaemonError> {
        let mut body = json!({});
        if let Some(permission) = permission {
            body["permission"] = permission;
        }
        let created: OpenCodeSessionCreated =
            self.send_json_request("POST", "/session", Some(&body))?;
        Ok(created.id)
    }

    pub fn create_session_with_retry(
        &self,
        permission: Option<Value>,
        timeout: Duration,
        retry_interval: Duration,
    ) -> Result<String, DaemonError> {
        let deadline = Instant::now() + timeout;
        let mut last_error = None;

        loop {
            match self.create_session(permission.clone()) {
                Ok(session_id) => return Ok(session_id),
                Err(error) if Instant::now() < deadline => {
                    last_error = Some(error);
                    std::thread::sleep(retry_interval);
                }
                Err(error) => return Err(last_error.unwrap_or(error)),
            }
        }
    }

    pub fn configured_defaults(&self) -> Result<OpenCodeConfiguredDefaults, DaemonError> {
        let config: OpenCodeConfig = match self.send_json_request("GET", "/config", None) {
            Ok(config) => config,
            Err(DaemonError::ProviderProtocol {
                operation: "opencode_http",
                message,
                ..
            }) if message == "OpenCode returned HTTP 404" => {
                return Ok(OpenCodeConfiguredDefaults::default())
            }
            Err(error) => return Err(error),
        };
        let agents = match self.send_json_request("GET", "/agent", None) {
            Ok::<serde_json::Value, _>(value) => parse_agent_infos(value),
            Err(DaemonError::ProviderProtocol {
                operation: "opencode_http",
                message,
                ..
            }) if message == "OpenCode returned HTTP 404" => Vec::new(),
            Err(error) => return Err(error),
        };
        Ok(resolve_configured_defaults(&config, &agents))
    }

    pub fn provider_catalog(&self) -> Result<OpenCodeProviderCatalog, DaemonError> {
        self.send_json_request("GET", "/provider", None)
    }

    pub fn submit_prompt(
        &self,
        session_id: &str,
        message_id: &str,
        prompt: &str,
        attachments: &[PromptAttachment],
        model: Option<&str>,
        variant: Option<&str>,
        execution_mode: AgentExecutionMode,
        disable_native_writes: bool,
        allow_native_bash: bool,
    ) -> Result<(), DaemonError> {
        let mut parts = Vec::new();
        if !prompt.is_empty() {
            parts.push(json!({
                "type": "text",
                "text": prompt,
            }));
        }
        for attachment in attachments {
            parts.push(json!({
                "type": "file",
                "mime": attachment.mime(),
                "url": attachment.url(),
                "filename": attachment.filename(),
            }));
        }
        let mut body = json!({
            "messageID": message_id,
            "parts": parts,
            "agent": opencode_agent_for_execution_mode(execution_mode),
        });
        if let Some((provider_id, model_id)) = parse_model(model) {
            body["model"] = json!({
                "providerID": provider_id,
                "modelID": model_id,
            });
        }
        if let Some(variant) = variant.map(str::trim).filter(|value| !value.is_empty()) {
            body["variant"] = json!(variant);
        }
        if disable_native_writes {
            let mut tools = serde_json::Map::from_iter([
                ("edit".to_string(), json!(false)),
                ("write".to_string(), json!(false)),
                ("apply_patch".to_string(), json!(false)),
                ("multiedit".to_string(), json!(false)),
                ("task".to_string(), json!(false)),
            ]);
            if !allow_native_bash {
                tools.insert("bash".to_string(), json!(false));
            }
            body["tools"] = serde_json::Value::Object(tools);
        }

        self.send_no_content_request(
            "POST",
            &format!("/session/{session_id}/prompt_async"),
            Some(&body),
        )?;
        Ok(())
    }

    pub fn abort_session(&self, session_id: &str) -> Result<(), DaemonError> {
        self.send_json_request::<serde_json::Value>(
            "POST",
            &format!("/session/{session_id}/abort"),
            Some(&json!({})),
        )?;
        Ok(())
    }

    pub fn reply_permission(
        &self,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<(), DaemonError> {
        let _: bool = self.send_json_request(
            "POST",
            &format!("/session/{session_id}/permissions/{permission_id}"),
            Some(&json!({ "response": response })),
        )?;
        Ok(())
    }

    pub fn snapshot(&self, session_id: &str) -> Result<OpenCodeSessionSnapshot, DaemonError> {
        let status_map: BTreeMap<String, OpenCodeSessionStatus> =
            self.send_json_request("GET", "/session/status", None)?;
        // OpenCode removes idle sessions from SessionStatus.list(), so omission means idle.
        let status = status_map
            .get(session_id)
            .map(|status| status.kind.clone())
            .unwrap_or_else(|| "idle".to_string());

        let messages = self.messages(session_id)?;

        Ok(OpenCodeSessionSnapshot { status, messages })
    }

    pub fn messages(&self, session_id: &str) -> Result<Vec<OpenCodeMessage>, DaemonError> {
        self.send_json_request("GET", &format!("/session/{session_id}/message"), None)
    }

    pub fn subscribe_events(&self) -> Result<OpenCodeEventSubscription, DaemonError> {
        let address = self.base_url.strip_prefix("http://").ok_or_else(|| {
            self.protocol_error(
                "base_url_parse",
                format!("unsupported OpenCode base URL `{}`", self.base_url),
            )
        })?;
        let mut stream = TcpStream::connect(address)
            .map_err(|error| self.protocol_error("event_subscribe", error.to_string()))?;
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|error| self.protocol_error("event_subscribe", error.to_string()))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| self.protocol_error("event_subscribe", error.to_string()))?;

        let request = format!(
            "GET /event HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\nX-Arroba-Provider-Client: kernel\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.flush())
            .map_err(|error| self.protocol_error("event_subscribe", error.to_string()))?;
        let (status_code, buffered_body) = read_http_headers(&mut stream)
            .map_err(|error| self.protocol_error("event_subscribe", error))?;
        if status_code >= 400 {
            return Err(self.protocol_error(
                "event_subscribe",
                format!("OpenCode returned HTTP {status_code}"),
            ));
        }

        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = stop.clone();
        let provider_run_id = self.provider_run_id.clone();

        thread::spawn(move || {
            let mut reader = BufReader::new(Cursor::new(buffered_body).chain(stream));
            let mut data_lines = Vec::new();

            loop {
                if stop_for_thread.load(Ordering::SeqCst) {
                    break;
                }

                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = line.trim_end_matches(['\r', '\n']);
                        if line.is_empty() {
                            if data_lines.is_empty() {
                                continue;
                            }

                            let payload = data_lines.join("\n");
                            data_lines.clear();
                            if let Some(event) = parse_sse_event(&payload, &provider_run_id) {
                                if tx.send(event).is_err() {
                                    break;
                                }
                            }
                            continue;
                        }

                        if let Some(data) = line.strip_prefix("data:") {
                            data_lines.push(data.trim_start().to_string());
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                }
            }
        });

        Ok(OpenCodeEventSubscription { receiver: rx, stop })
    }

    pub fn subscribe_events_with_retry(
        &self,
        timeout: Duration,
        retry_interval: Duration,
    ) -> Result<OpenCodeEventSubscription, DaemonError> {
        let deadline = Instant::now() + timeout;
        let mut last_error = None;

        loop {
            match self.subscribe_events() {
                Ok(subscription) => return Ok(subscription),
                Err(error) if Instant::now() < deadline => {
                    last_error = Some(error);
                    std::thread::sleep(retry_interval);
                }
                Err(error) => return Err(last_error.unwrap_or(error)),
            }
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn add_mcp_server(&self, name: &str, config: serde_json::Value) -> Result<(), DaemonError> {
        let _: serde_json::Value = self.send_json_request(
            "POST",
            "/mcp",
            Some(&serde_json::json!({ "name": name, "config": config })),
        )?;
        Ok(())
    }

    pub fn connect_mcp_server(&self, name: &str) -> Result<(), DaemonError> {
        let _: bool = self.send_json_request("POST", &format!("/mcp/{name}/connect"), None)?;
        Ok(())
    }

    fn health(&self) -> Result<(), DaemonError> {
        let health: OpenCodeHealth = self.send_json_request("GET", "/global/health", None)?;
        if health.healthy {
            Ok(())
        } else {
            Err(self.protocol_error("health", "provider reported unhealthy".to_string()))
        }
    }

    fn protocol_error(&self, operation: &'static str, message: String) -> DaemonError {
        DaemonError::ProviderProtocol {
            provider_run_id: self.provider_run_id.clone(),
            operation,
            message,
        }
    }

    fn send_json_request<T: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<T, DaemonError> {
        let (status_code, response_body) = self.send_request(method, path, body)?;
        if status_code >= 400 {
            return Err(self.protocol_error(
                method_to_operation(method, path),
                format!("OpenCode returned HTTP {status_code}"),
            ));
        }

        serde_json::from_slice(&response_body).map_err(|error| {
            self.protocol_error(
                method_to_operation(method, path),
                format!(
                    "{}; response body: {}",
                    error,
                    preview_response_body(&response_body)
                ),
            )
        })
    }

    fn send_no_content_request(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(), DaemonError> {
        let (status_code, response_body) = self.send_request(method, path, body)?;
        if status_code >= 400 {
            return Err(self.protocol_error(
                method_to_operation(method, path),
                format!("OpenCode returned HTTP {status_code}"),
            ));
        }
        if !response_body.is_empty() {
            return Err(self.protocol_error(
                method_to_operation(method, path),
                format!(
                    "expected empty response body; got {}",
                    preview_response_body(&response_body)
                ),
            ));
        }
        Ok(())
    }

    fn send_request(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(u16, Vec<u8>), DaemonError> {
        let address = self.base_url.strip_prefix("http://").ok_or_else(|| {
            self.protocol_error(
                "base_url_parse",
                format!("unsupported OpenCode base URL `{}`", self.base_url),
            )
        })?;
        let mut stream = TcpStream::connect(address).map_err(|error| {
            self.protocol_error(method_to_operation(method, path), error.to_string())
        })?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| {
                self.protocol_error(method_to_operation(method, path), error.to_string())
            })?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| {
                self.protocol_error(method_to_operation(method, path), error.to_string())
            })?;

        let body_bytes = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| {
                self.protocol_error(method_to_operation(method, path), error.to_string())
            })?
            .unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nX-Arroba-Provider-Client: kernel\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body_bytes.len()
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(&body_bytes))
            .and_then(|_| stream.flush())
            .map_err(|error| {
                self.protocol_error(method_to_operation(method, path), error.to_string())
            })?;

        read_http_response(&mut stream)
            .map_err(|error| self.protocol_error(method_to_operation(method, path), error))
    }
}

fn opencode_agent_for_execution_mode(execution_mode: AgentExecutionMode) -> &'static str {
    match execution_mode {
        AgentExecutionMode::Build => "build",
        AgentExecutionMode::Plan => "plan",
    }
}

fn method_to_operation(method: &str, path: &str) -> &'static str {
    match (method, path) {
        ("GET", "/global/health") => "health",
        ("GET", "/event") => "event_subscribe",
        ("POST", "/session") => "session_create",
        ("GET", "/session/status") => "session_status",
        _ if method == "POST" && path.ends_with("/prompt_async") => "session_prompt",
        _ if method == "POST" && path.ends_with("/message") => "session_prompt",
        _ if method == "POST" && path.ends_with("/abort") => "session_abort",
        _ if method == "GET" && path.ends_with("/message") => "session_messages",
        _ => "opencode_http",
    }
}

fn preview_response_body(body: &[u8]) -> String {
    if body.is_empty() {
        return "<empty>".to_string();
    }

    let preview = String::from_utf8_lossy(body);
    let preview = preview.trim();
    if preview.len() > 240 {
        format!("{}...", &preview[..240])
    } else {
        preview.to_string()
    }
}

fn read_http_headers(stream: &mut TcpStream) -> Result<(u16, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if size == 0 {
            return Err("connection closed before response header".to_string());
        }
        buffer.extend_from_slice(&chunk[..size]);
        if let Some(index) = find_double_crlf(&buffer) {
            let header_end = index + 4;
            let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
            let status_code = header_text
                .lines()
                .next()
                .ok_or_else(|| "missing HTTP status line".to_string())?
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| "missing HTTP status code".to_string())?
                .parse::<u16>()
                .map_err(|error| error.to_string())?;
            return Ok((status_code, buffer[header_end..].to_vec()));
        }
    }
}

fn read_http_response(stream: &mut TcpStream) -> Result<(u16, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end;
    loop {
        let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if size == 0 {
            return Err("connection closed before response header".to_string());
        }
        buffer.extend_from_slice(&chunk[..size]);
        if let Some(index) = find_double_crlf(&buffer) {
            header_end = index + 4;
            break;
        }
    }

    let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = header_text.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| "missing HTTP status line".to_string())?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "missing HTTP status code".to_string())?
        .parse::<u16>()
        .map_err(|error| error.to_string())?;
    let mut content_length = None;
    let mut is_chunked = false;
    for line in lines {
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(value) = parts.next().map(str::trim) else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse::<usize>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        {
            is_chunked = true;
        }
    }

    let body = if is_chunked {
        read_chunked_http_body(buffer[header_end..].to_vec(), stream)?
    } else {
        let mut body = buffer[header_end..].to_vec();
        let content_length = content_length.unwrap_or(0);
        while body.len() < content_length {
            let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
            if size == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..size]);
        }
        body
    };
    Ok((status_code, body))
}

fn find_double_crlf(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn read_chunked_http_body(
    mut buffered: Vec<u8>,
    stream: &mut TcpStream,
) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let header_end = loop {
            if let Some(index) = buffered.windows(2).position(|window| window == b"\r\n") {
                break index;
            }
            let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
            if size == 0 {
                return Err("unexpected EOF while reading chunk header".to_string());
            }
            buffered.extend_from_slice(&chunk[..size]);
        };

        let size_line = String::from_utf8_lossy(&buffered[..header_end]).into_owned();
        let size_hex = size_line
            .split(';')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "missing chunk size".to_string())?;
        let size = usize::from_str_radix(size_hex, 16).map_err(|error| error.to_string())?;
        buffered.drain(..header_end + 2);

        if size == 0 {
            loop {
                if buffered.len() >= 2 && &buffered[..2] == b"\r\n" {
                    buffered.drain(..2);
                    break;
                }
                let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
                if read == 0 {
                    break;
                }
                buffered.extend_from_slice(&chunk[..read]);
            }
            break;
        }

        while buffered.len() < size + 2 {
            let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("unexpected EOF while reading chunk body".to_string());
            }
            buffered.extend_from_slice(&chunk[..read]);
        }
        decoded.extend_from_slice(&buffered[..size]);
        buffered.drain(..size + 2);
    }

    Ok(decoded)
}

fn resolve_configured_defaults(
    config: &OpenCodeConfig,
    agents: &[OpenCodeAgentInfo],
) -> OpenCodeConfiguredDefaults {
    let selected_agent = config
        .default_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "build".to_string());
    let config_agent = config
        .agent
        .get(&selected_agent)
        .or_else(|| config.mode.get(&selected_agent));
    let listed_agent = agents.iter().find(|agent| {
        agent.name == selected_agent && agent.mode != "subagent" && agent.hidden != Some(true)
    });
    let top_level_model = config
        .model
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let config_agent_model = config_agent
        .and_then(|agent| agent.model.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let listed_agent_model = listed_agent
        .and_then(|agent| agent.model.as_ref())
        .map(|model| format!("{}/{}", model.provider_id, model.model_id));
    let agent_model = config_agent_model.clone().or(listed_agent_model.clone());
    let config_agent_variant = config_agent
        .and_then(|agent| agent.variant.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let listed_agent_variant = listed_agent
        .and_then(|agent| agent.variant.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let agent_variant = config_agent_variant
        .clone()
        .or(listed_agent_variant.clone());

    OpenCodeConfiguredDefaults {
        model: agent_model.clone().or(top_level_model.clone()),
        variant: agent_variant.clone(),
        selected_agent: Some(selected_agent),
        agent_model,
        agent_variant,
        top_level_model,
    }
}

fn parse_agent_infos(value: serde_json::Value) -> Vec<OpenCodeAgentInfo> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(parse_agent_info)
        .collect()
}

fn parse_agent_info(value: &serde_json::Value) -> Option<OpenCodeAgentInfo> {
    let object = value.as_object()?;
    let name = object.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }

    let mode = object
        .get("mode")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("primary")
        .to_string();
    let hidden = object.get("hidden").and_then(|value| value.as_bool());
    let model = parse_agent_model(object.get("model"));
    let variant = object
        .get("variant")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    Some(OpenCodeAgentInfo {
        name: name.to_string(),
        mode,
        hidden,
        model,
        variant,
    })
}

fn parse_agent_model(value: Option<&serde_json::Value>) -> Option<OpenCodeSelectedModel> {
    let value = value?;
    if let Some(model) = value.as_object() {
        let provider_id = model
            .get("providerID")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let model_id = model
            .get("modelID")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        return Some(OpenCodeSelectedModel {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
        });
    }

    let raw = value.as_str()?.trim();
    let (provider_id, model_id) = raw.split_once('/')?;
    let provider_id = provider_id.trim();
    let model_id = model_id.trim();
    if provider_id.is_empty() || model_id.is_empty() {
        return None;
    }

    Some(OpenCodeSelectedModel {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
    })
}

fn parse_model(model: Option<&str>) -> Option<(&str, &str)> {
    let value = model?.trim();
    if value.is_empty() || value == "default" {
        return None;
    }
    value.split_once('/')
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use crate::provider::AgentExecutionMode;

    use super::{
        parse_agent_infos, parse_model, resolve_configured_defaults, OpenCodeAgentInfo,
        OpenCodeClient, OpenCodeConfig, OpenCodeConfigAgent, OpenCodeEvent, OpenCodeMessageInfo,
        OpenCodeSelectedModel,
    };

    #[test]
    fn parses_provider_model_ids() {
        assert_eq!(
            parse_model(Some("anthropic/claude-sonnet-4")),
            Some(("anthropic", "claude-sonnet-4"))
        );
        assert_eq!(parse_model(Some("default")), None);
        assert_eq!(parse_model(None), None);
    }

    #[test]
    fn resolves_model_from_assistant_or_user_message_metadata() {
        let assistant = serde_json::from_value::<OpenCodeMessageInfo>(serde_json::json!({
            "id": "message-1",
            "sessionID": "session-1",
            "role": "assistant",
            "providerID": "openai",
            "modelID": "gpt-5.4",
            "variant": "medium"
        }))
        .expect("assistant info should parse");
        assert_eq!(
            assistant.resolved_model().as_deref(),
            Some("openai/gpt-5.4")
        );
        assert_eq!(assistant.resolved_variant().as_deref(), Some("medium"));

        let user = serde_json::from_value::<OpenCodeMessageInfo>(serde_json::json!({
            "id": "message-2",
            "sessionID": "session-1",
            "role": "user",
            "model": {
                "providerID": "openai",
                "modelID": "gpt-5.4"
            }
        }))
        .expect("user info should parse");
        assert_eq!(user.resolved_model().as_deref(), Some("openai/gpt-5.4"));
        assert_eq!(user.resolved_variant(), None);
    }

    #[test]
    fn resolves_defaults_from_default_agent_before_global_model() {
        let defaults = resolve_configured_defaults(
            &OpenCodeConfig {
                model: Some("openai/gpt-5.4".to_string()),
                default_agent: Some("build".to_string()),
                agent: BTreeMap::new(),
                mode: BTreeMap::new(),
            },
            &[
                OpenCodeAgentInfo {
                    name: "build".to_string(),
                    mode: "primary".to_string(),
                    hidden: Some(false),
                    model: Some(OpenCodeSelectedModel {
                        provider_id: "anthropic".to_string(),
                        model_id: "claude-sonnet-4".to_string(),
                    }),
                    variant: Some("medium".to_string()),
                },
                OpenCodeAgentInfo {
                    name: "plan".to_string(),
                    mode: "primary".to_string(),
                    hidden: Some(false),
                    model: None,
                    variant: None,
                },
            ],
        );
        assert_eq!(defaults.model.as_deref(), Some("anthropic/claude-sonnet-4"));
        assert_eq!(defaults.variant.as_deref(), Some("medium"));
        assert_eq!(defaults.selected_agent.as_deref(), Some("build"));
    }

    #[test]
    fn falls_back_to_global_model_when_agent_has_no_model() {
        let defaults = resolve_configured_defaults(
            &OpenCodeConfig {
                model: Some("openai/gpt-5.4".to_string()),
                default_agent: Some("build".to_string()),
                agent: BTreeMap::new(),
                mode: BTreeMap::new(),
            },
            &[OpenCodeAgentInfo {
                name: "build".to_string(),
                mode: "primary".to_string(),
                hidden: Some(false),
                model: None,
                variant: Some("low".to_string()),
            }],
        );
        assert_eq!(defaults.model.as_deref(), Some("openai/gpt-5.4"));
        assert_eq!(defaults.variant.as_deref(), Some("low"));
    }

    #[test]
    fn resolves_defaults_from_configured_build_agent_without_default_agent() {
        let defaults = resolve_configured_defaults(
            &OpenCodeConfig {
                model: None,
                default_agent: None,
                agent: BTreeMap::from([(
                    "build".to_string(),
                    OpenCodeConfigAgent {
                        model: Some("openai/gpt-5.4".to_string()),
                        variant: Some("low".to_string()),
                    },
                )]),
                mode: BTreeMap::new(),
            },
            &[],
        );
        assert_eq!(defaults.model.as_deref(), Some("openai/gpt-5.4"));
        assert_eq!(defaults.variant.as_deref(), Some("low"));
        assert_eq!(defaults.selected_agent.as_deref(), Some("build"));
    }

    #[test]
    fn parses_current_opencode_agent_payload_without_failing() {
        let agents = parse_agent_infos(serde_json::json!([
            {
                "name": "build",
                "description": "The default agent. Executes tools based on configured permissions.",
                "options": {
                    "timeout": 4000
                },
                "permission": [
                    {
                        "permission": "*",
                        "action": "allow",
                        "pattern": "*"
                    }
                ],
                "prompt": "ignored by arroba"
            },
            {
                "name": "plan",
                "mode": "subagent",
                "hidden": true,
                "model": "openai/gpt-5.4",
                "variant": "medium"
            }
        ]));

        assert_eq!(
            agents,
            vec![
                OpenCodeAgentInfo {
                    name: "build".to_string(),
                    mode: "primary".to_string(),
                    hidden: None,
                    model: None,
                    variant: None,
                },
                OpenCodeAgentInfo {
                    name: "plan".to_string(),
                    mode: "subagent".to_string(),
                    hidden: Some(true),
                    model: Some(OpenCodeSelectedModel {
                        provider_id: "openai".to_string(),
                        model_id: "gpt-5.4".to_string(),
                    }),
                    variant: Some("medium".to_string()),
                },
            ]
        );
    }

    #[test]
    fn decodes_chunked_http_json_responses() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener
            .local_addr()
            .expect("test listener should expose a local address")
            .port();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let body = r#"{"healthy":true}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n{}\r\n0\r\n\r\n",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("server should write chunked response");
        });

        let client = OpenCodeClient::new("provider-run-test", format!("http://127.0.0.1:{port}"))
            .expect("client should initialize");
        client
            .check_health()
            .expect("client should decode chunked JSON");
        server.join().expect("server thread should join");
    }

    #[test]
    fn create_session_sends_permission_rules() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener
            .local_addr()
            .expect("test listener should expose a local address")
            .port();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("read timeout should be set");
            let mut request = Vec::new();
            let mut buf = [0_u8; 1024];
            loop {
                let size = stream.read(&mut buf).expect("request should read");
                request.extend_from_slice(&buf[..size]);
                let request_text = String::from_utf8_lossy(&request);
                let Some((headers, body)) = request_text.split_once("\r\n\r\n") else {
                    continue;
                };
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .expect("request should include content length");
                if body.len() >= content_length {
                    break;
                }
            }
            let request_text = String::from_utf8_lossy(&request).into_owned();
            assert!(request_text.starts_with("POST /session "));
            assert!(request_text.contains("\"permission\""));
            assert!(request_text.contains("\"edit\""));
            assert!(request_text.contains("\"deny\""));
            let body = r#"{"id":"session-1"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("server should write response");
        });

        let client =
            OpenCodeClient::new("provider-run-1", format!("http://127.0.0.1:{port}")).unwrap();
        let session_id = client
            .create_session(Some(serde_json::json!([
                {
                    "permission": "edit",
                    "pattern": "*",
                    "action": "deny"
                }
            ])))
            .expect("session should be created");
        assert_eq!(session_id, "session-1");
        server.join().expect("server thread should join");
    }

    #[test]
    fn submit_prompt_sends_native_plan_agent() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener
            .local_addr()
            .expect("test listener should expose a local address")
            .port();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .expect("read timeout should be set");
            let mut request = Vec::new();
            let mut buf = [0_u8; 2048];
            loop {
                let size = stream.read(&mut buf).expect("request should read");
                request.extend_from_slice(&buf[..size]);
                let request_text = String::from_utf8_lossy(&request);
                let Some((headers, body)) = request_text.split_once("\r\n\r\n") else {
                    continue;
                };
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .expect("request should include content length");
                if body.len() >= content_length {
                    break;
                }
            }
            let request_text = String::from_utf8_lossy(&request).into_owned();
            let (_, body) = request_text
                .split_once("\r\n\r\n")
                .expect("request should include body");
            let body: serde_json::Value =
                serde_json::from_str(body).expect("request body should be JSON");
            assert_eq!(body.get("agent"), Some(&serde_json::json!("plan")));
            assert_eq!(
                body.get("model"),
                Some(&serde_json::json!({
                    "providerID": "openai",
                    "modelID": "gpt-5.4",
                }))
            );
            let response =
                "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream
                .write_all(response.as_bytes())
                .expect("server should write response");
        });

        let client =
            OpenCodeClient::new("provider-run-1", format!("http://127.0.0.1:{port}")).unwrap();
        client
            .submit_prompt(
                "session-1",
                "message-1",
                "make a plan",
                &[],
                Some("openai/gpt-5.4"),
                Some("low"),
                AgentExecutionMode::Plan,
                false,
                false,
            )
            .expect("prompt should be accepted");
        server.join().expect("server thread should join");
    }

    #[test]
    fn preserves_first_sse_payload_when_headers_and_body_arrive_together() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener
            .local_addr()
            .expect("test listener should expose a local address")
            .port();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let payload = serde_json::json!({
                "payload": {
                    "type": "session.error",
                    "properties": {
                        "sessionID": "session-1",
                        "error": {
                            "message": "bundled first event"
                        }
                    }
                }
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {payload}\n\n"
            );
            stream
                .write_all(response.as_bytes())
                .expect("server should write headers and first event together");
            stream.flush().expect("server should flush response");
        });

        let client =
            OpenCodeClient::new("provider-run-1", format!("http://127.0.0.1:{port}")).unwrap();
        let subscription = client
            .subscribe_events()
            .expect("client should subscribe to events");
        let event = subscription
            .receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("first SSE event should be preserved");

        match event {
            OpenCodeEvent::SessionError {
                session_id,
                message,
            } => {
                assert_eq!(session_id, "session-1");
                assert_eq!(message, "bundled first event");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        subscription.stop();
        let _ = server.join();
    }

    #[test]
    fn parses_wrapped_sse_event_payloads() {
        let payload = serde_json::json!({
            "payload": {
                "type": "message.part.delta",
                "properties": {
                    "sessionID": "session-1",
                    "messageID": "message-1",
                    "partID": "part-1",
                    "field": "text",
                    "delta": "hello"
                }
            }
        })
        .to_string();

        let event = super::parse_sse_event(&payload, "provider-run-1")
            .expect("wrapped payload should parse");

        match event {
            OpenCodeEvent::MessagePartDelta {
                session_id,
                message_id,
                part_id,
                field,
                delta,
            } => {
                assert_eq!(session_id, "session-1");
                assert_eq!(message_id, "message-1");
                assert_eq!(part_id, "part-1");
                assert_eq!(field, "text");
                assert_eq!(delta, "hello");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn snapshot_treats_missing_status_entry_as_idle() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener
            .local_addr()
            .expect("test listener should expose a local address")
            .port();

        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("client should connect");
                let mut request = [0_u8; 2048];
                let size = stream.read(&mut request).expect("request should read");
                let request_text = String::from_utf8_lossy(&request[..size]).into_owned();
                let path = request_text
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = match path {
                    "/session/status" => "{}".to_string(),
                    "/session/session-1/message" => "[]".to_string(),
                    other => panic!("unexpected path: {other}"),
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("server should write response");
                stream.flush().expect("server should flush response");
            }
        });

        let client =
            OpenCodeClient::new("provider-run-1", format!("http://127.0.0.1:{port}")).unwrap();
        let snapshot = client
            .snapshot("session-1")
            .expect("missing status entry should default to idle");
        assert_eq!(snapshot.status, "idle");
        assert!(snapshot.messages.is_empty());

        let _ = server.join();
    }
}
