use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::DaemonError;

mod defaults;
mod event_subscription;
mod events;
mod http;
mod prompt_request;

pub use defaults::OpenCodeConfiguredDefaults;
pub use event_subscription::OpenCodeEventSubscription;

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

    pub fn provider_catalog(&self) -> Result<OpenCodeProviderCatalog, DaemonError> {
        self.send_json_request("GET", "/provider", None)
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
        defaults::{
            parse_agent_infos, resolve_configured_defaults, OpenCodeAgentInfo, OpenCodeConfig,
            OpenCodeConfigAgent,
        },
        events::parse_sse_event,
        prompt_request::parse_model,
        OpenCodeClient, OpenCodeEvent, OpenCodeMessageInfo, OpenCodeSelectedModel,
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

        let event =
            parse_sse_event(&payload, "provider-run-1").expect("wrapped payload should parse");

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
