use crate::error::DaemonError;

mod catalog;
mod defaults;
mod event_subscription;
mod events;
mod health;
mod http;
mod mcp;
mod message;
mod prompt_request;
mod session;

pub use catalog::{
    OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel,
    OpenCodeProviderModelLimit,
};
pub use defaults::OpenCodeConfiguredDefaults;
pub use event_subscription::OpenCodeEventSubscription;
pub use events::{OpenCodeEvent, OpenCodePermissionRequest};
pub use message::{
    OpenCodeMessage, OpenCodeMessageCacheTokens, OpenCodeMessageInfo, OpenCodeMessageTime,
    OpenCodeMessageTokens, OpenCodePart, OpenCodePartTime, OpenCodeSelectedModel,
    OpenCodeToolState,
};
pub use session::{OpenCodeSessionSnapshot, OpenCodeSessionStatus};

#[derive(Debug, Clone)]
pub struct OpenCodeClient {
    provider_run_id: String,
    base_url: String,
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

    pub fn base_url(&self) -> &str {
        &self.base_url
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
            Ok(Some(("anthropic", "claude-sonnet-4")))
        );
        assert_eq!(
            parse_model(Some("gpt-5.4")),
            Ok(Some(("opencode", "gpt-5.4")))
        );
        assert_eq!(parse_model(Some("default")), Ok(None));
        assert_eq!(parse_model(None), Ok(None));
    }

    #[test]
    fn resolves_model_from_assistant_or_user_message_metadata() {
        let assistant = serde_json::from_value::<OpenCodeMessageInfo>(serde_json::json!({
            "id": "message-1",
            "sessionID": "session-1",
            "role": "assistant",
            "providerID": "opencode",
            "modelID": "gpt-5.4",
            "variant": "medium"
        }))
        .expect("assistant info should parse");
        assert_eq!(
            assistant.resolved_model().as_deref(),
            Some("opencode/gpt-5.4")
        );
        assert_eq!(assistant.resolved_variant().as_deref(), Some("medium"));

        let user = serde_json::from_value::<OpenCodeMessageInfo>(serde_json::json!({
            "id": "message-2",
            "sessionID": "session-1",
            "role": "user",
            "model": {
                "providerID": "opencode",
                "modelID": "gpt-5.4"
            }
        }))
        .expect("user info should parse");
        assert_eq!(user.resolved_model().as_deref(), Some("opencode/gpt-5.4"));
        assert_eq!(user.resolved_variant(), None);
    }

    #[test]
    fn resolves_defaults_from_default_agent_before_global_model() {
        let defaults = resolve_configured_defaults(
            &OpenCodeConfig {
                model: Some("opencode/gpt-5.4".to_string()),
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
                model: Some("opencode/gpt-5.4".to_string()),
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
        assert_eq!(defaults.model.as_deref(), Some("opencode/gpt-5.4"));
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
                        model: Some("opencode/gpt-5.4".to_string()),
                        variant: Some("low".to_string()),
                    },
                )]),
                mode: BTreeMap::new(),
            },
            &[],
        );
        assert_eq!(defaults.model.as_deref(), Some("opencode/gpt-5.4"));
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
                "prompt": "ignored by chariox"
            },
            {
                "name": "plan",
                "mode": "subagent",
                "hidden": true,
                "model": "opencode/gpt-5.4",
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
                        provider_id: "opencode".to_string(),
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
                body.get("system"),
                Some(&serde_json::json!("hidden system context"))
            );
            assert_eq!(
                body.get("model"),
                Some(&serde_json::json!({
                    "providerID": "opencode",
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
                Some("hidden system context"),
                Some("opencode/gpt-5.4"),
                Some("low"),
                AgentExecutionMode::Plan,
                false,
                false,
            )
            .expect("prompt should be accepted");
        server.join().expect("server thread should join");
    }

    #[test]
    fn workspace_live_sync_prompt_disables_native_writes_but_allows_fenced_bash() {
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
            let mut buf = [0_u8; 4096];
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
            assert_eq!(
                body.get("tools"),
                Some(&serde_json::json!({
                    "edit": false,
                    "write": false,
                    "apply_patch": false,
                    "multiedit": false,
                    "task": false,
                    "bash": true,
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
                "write outside the synced root",
                &[],
                None,
                Some("opencode/gpt-5.4"),
                None,
                AgentExecutionMode::Build,
                true,
                true,
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
        assert_eq!(snapshot.status.kind, "idle");
        assert!(snapshot.messages.is_empty());

        let _ = server.join();
    }
}
