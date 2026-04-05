use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::DaemonError;
use crate::session::PromptAttachment;

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
    #[serde(default)]
    pub finish: Option<String>,
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

#[derive(Debug, Deserialize)]
struct OpenCodeAgentInfo {
    name: String,
    mode: String,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    model: Option<OpenCodeSelectedModel>,
    #[serde(default)]
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

#[derive(Debug, Deserialize)]
struct RawOpenCodeEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    properties: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct RawOpenCodeEventEnvelope {
    payload: RawOpenCodeEvent,
}

#[derive(Debug, Deserialize)]
struct OpenCodeMessageUpdatedEvent {
    info: OpenCodeMessageInfo,
}

#[derive(Debug, Deserialize)]
struct OpenCodeMessagePartDeltaEvent {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "messageID")]
    message_id: String,
    #[serde(rename = "partID")]
    part_id: String,
    field: String,
    delta: String,
}

#[derive(Debug, Deserialize)]
struct OpenCodeMessagePartUpdatedEvent {
    part: OpenCodePart,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSessionErrorEvent {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(default)]
    error: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSessionStatusEvent {
    #[serde(rename = "sessionID")]
    session_id: String,
    status: OpenCodeSessionStatus,
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

    pub fn create_session(&self) -> Result<String, DaemonError> {
        let created: OpenCodeSessionCreated =
            self.send_json_request("POST", "/session", Some(&json!({})))?;
        Ok(created.id)
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
        let agents: Vec<OpenCodeAgentInfo> = match self.send_json_request("GET", "/agent", None) {
            Ok(agents) => agents,
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
        prompt: &str,
        attachments: &[PromptAttachment],
        model: Option<&str>,
        variant: Option<&str>,
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
            "parts": parts,
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

    pub fn snapshot(&self, session_id: &str) -> Result<OpenCodeSessionSnapshot, DaemonError> {
        let status_map: BTreeMap<String, OpenCodeSessionStatus> =
            self.send_json_request("GET", "/session/status", None)?;
        let status = status_map
            .get(session_id)
            .map(|status| status.kind.clone())
            .ok_or_else(|| {
                self.protocol_error(
                    "session_status",
                    format!(
                        "OpenCode did not report session `{session_id}` in the session status response"
                    ),
                )
            })?;

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
            "GET /event HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n"
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
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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

fn parse_sse_event(payload: &str, provider_run_id: &str) -> Option<OpenCodeEvent> {
    let raw = serde_json::from_str::<RawOpenCodeEventEnvelope>(payload)
        .map(|envelope| envelope.payload)
        .or_else(|_| serde_json::from_str::<RawOpenCodeEvent>(payload))
        .ok()?;
    match raw.kind.as_str() {
        "server.connected" | "server.heartbeat" => None,
        "message.updated" => {
            let properties: OpenCodeMessageUpdatedEvent =
                serde_json::from_value(raw.properties).ok()?;
            Some(OpenCodeEvent::MessageUpdated {
                info: properties.info,
            })
        }
        "message.part.delta" => {
            let properties: OpenCodeMessagePartDeltaEvent =
                serde_json::from_value(raw.properties).ok()?;
            Some(OpenCodeEvent::MessagePartDelta {
                session_id: properties.session_id,
                message_id: properties.message_id,
                part_id: properties.part_id,
                field: properties.field,
                delta: properties.delta,
            })
        }
        "message.part.updated" => {
            let properties: OpenCodeMessagePartUpdatedEvent =
                serde_json::from_value(raw.properties).ok()?;
            Some(OpenCodeEvent::MessagePartUpdated {
                part: Box::new(properties.part),
            })
        }
        "session.status" => {
            let properties: OpenCodeSessionStatusEvent =
                serde_json::from_value(raw.properties).ok()?;
            Some(OpenCodeEvent::SessionStatus {
                session_id: properties.session_id,
                kind: properties.status.kind,
            })
        }
        "session.error" => {
            let properties: OpenCodeSessionErrorEvent =
                serde_json::from_value(raw.properties).ok()?;
            Some(OpenCodeEvent::SessionError {
                session_id: properties.session_id,
                message: session_error_message(properties.error, provider_run_id),
            })
        }
        _ => None,
    }
}

fn session_error_message(error: serde_json::Value, provider_run_id: &str) -> String {
    error
        .get("data")
        .and_then(|value| value.get("message"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| error.get("message").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!("OpenCode reported an unknown session error for `{provider_run_id}`")
        })
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
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
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
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
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
    let content_length = lines
        .find_map(|line| {
            let mut parts = line.splitn(2, ':');
            let name = parts.next()?.trim();
            let value = parts.next()?.trim();
            (name.eq_ignore_ascii_case("content-length")).then_some(value)
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if size == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..size]);
    }
    Ok((status_code, body))
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

    use super::{
        parse_model, resolve_configured_defaults, OpenCodeAgentInfo, OpenCodeClient,
        OpenCodeConfig, OpenCodeConfigAgent, OpenCodeEvent, OpenCodeMessageInfo,
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
}
