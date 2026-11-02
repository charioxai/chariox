//! OpenCode SSE payload parsing into provider runtime events.

use serde::Deserialize;
use serde_json::Value;

use super::{session::OpenCodeSessionStatus, OpenCodeMessageInfo, OpenCodePart};

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
        status: OpenCodeSessionStatus,
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
    pub message_id: Option<String>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub reason: Option<String>,
    pub patterns: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawOpenCodeEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    properties: Value,
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
    error: Value,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSessionStatusEvent {
    #[serde(rename = "sessionID")]
    session_id: String,
    status: OpenCodeSessionStatus,
}

#[derive(Debug, Deserialize)]
struct OpenCodePermissionAskedEvent {
    id: String,
    #[serde(rename = "sessionID")]
    session_id: String,
    permission: String,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    tool: Value,
}

pub(super) fn parse_sse_event(payload: &str, provider_run_id: &str) -> Option<OpenCodeEvent> {
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
                status: properties.status,
            })
        }
        "permission.asked" => {
            let properties: OpenCodePermissionAskedEvent =
                serde_json::from_value(raw.properties).ok()?;
            let metadata = properties.metadata.as_object();
            let tool = properties
                .tool
                .as_object()
                .and_then(|value| value.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(OpenCodeEvent::PermissionAsked {
                request: OpenCodePermissionRequest {
                    id: properties.id,
                    session_id: properties.session_id,
                    permission: properties.permission,
                    tool,
                    message_id: properties
                        .tool
                        .get("messageID")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    command: metadata
                        .and_then(|value| value.get("command"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    cwd: metadata
                        .and_then(|value| value.get("cwd"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    reason: metadata
                        .and_then(|value| value.get("reason"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    patterns: properties.patterns,
                },
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

fn session_error_message(error: Value, provider_run_id: &str) -> String {
    error
        .get("data")
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .or_else(|| error.get("message").and_then(Value::as_str))
        .map(|message| super::message::opencode_error_text(&error, message))
        .unwrap_or_else(|| {
            format!("OpenCode reported an unknown session error for `{provider_run_id}`")
        })
}

#[cfg(test)]
mod failure_cause_tests {
    use crate::provider::{TurnOutcome, TurnOutcomeKind::*};

    /// EA2-75: OpenCode `session.error` events keep the harness message and
    /// give the typed cause from the error name and HTTP status. The error
    /// objects follow OpenCode's `ProviderAuthError` / `APIError` /
    /// `UnknownError` schema; none was recorded in earlier evidence.
    fn session_error(error: serde_json::Value) -> TurnOutcome {
        let payload = serde_json::json!({
            "type": "session.error",
            "properties": {"sessionID": "session", "error": error}
        });
        let Some(super::OpenCodeEvent::SessionError { message, .. }) =
            super::parse_sse_event(&payload.to_string(), "run")
        else {
            panic!("missing session error")
        };
        TurnOutcome::classify("opencode", &message)
    }

    #[test]
    fn ea2_75_opencode_opaque_errors_use_status_and_native_retryability() {
        for (status, kind, retryable) in [
            (401, SignInRequired, false),
            (429, UsageLimit, true),
            (503, Overloaded, true),
            (529, Overloaded, false),
            (408, Network, true),
            (500, Other, true),
            (403, Other, false),
        ] {
            let outcome = session_error(
                serde_json::json!({"name":"APIError", "data": {"message":"Request rejected", "statusCode":status, "isRetryable":retryable}}),
            );
            assert_eq!(
                (outcome.kind, outcome.retryable),
                (kind, retryable),
                "{status}"
            );
        }
        for (name, kind) in [
            ("ProviderAuthError", SignInRequired),
            ("ContextOverflowError", Other),
            ("MessageOutputLengthError", Other),
            ("MessageAbortedError", Cancelled),
        ] {
            let outcome = session_error(
                serde_json::json!({"name":name,"data":{"message":"Request rejected"}}),
            );
            assert_eq!((outcome.kind, outcome.retryable), (kind, false), "{name}");
        }
    }

    #[test]
    fn ea2_75_opencode_runtime_keeps_native_non_retryability() {
        use crate::provider::{
            AgentEndpointMode, LaunchProviderRequest, OpenCodeEventSubscription,
            ProviderLaunchResult, RuntimeProviderRun,
        };
        use std::collections::BTreeMap;
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                            .unwrap();
                        let mut request = [0; 2048];
                        let count = stream.read(&mut request).unwrap();
                        let request = String::from_utf8_lossy(&request[..count]);
                        let body = if request.starts_with("GET /session/status ") {
                            r#"{"session-1":{"type":"busy"}}"#
                        } else {
                            "[]"
                        };
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .unwrap();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(error) => panic!("{error}"),
                }
            }
        });
        let payload = serde_json::json!({"type":"session.error", "properties":{"sessionID":"session-1", "error":{"name":"APIError", "data":{"message":"Overloaded", "statusCode":529, "isRetryable":false}}}});
        let event = super::parse_sse_event(&payload.to_string(), "run").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(event).unwrap();
        let mut state = crate::provider::opencode_runtime::OpenCodeRuntimeState::new(
            endpoint,
            "session-1".into(),
            OpenCodeEventSubscription::for_tests(rx),
        );
        state.note_prompt_submitted("user-1".into());
        let request =
            LaunchProviderRequest::new("session", "opencode", "opencode", "default", "model");
        let run = RuntimeProviderRun::new(
            "run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "fixture".into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        let drained =
            crate::provider::opencode_runtime::drain_opencode_events(&run, &mut state, None)
                .unwrap();
        server.join().unwrap();
        let failure =
            TurnOutcome::classify("opencode", drained.terminal_failure.as_deref().unwrap());
        assert_eq!((failure.kind, failure.retryable), (Overloaded, false));
    }

    #[test]
    fn opencode_errors_are_typed_from_name_and_status() {
        let expected =
            |kind, retryable, _diagnostic: &str| TurnOutcome::provider_failure(kind, retryable);
        assert_eq!(
            session_error(serde_json::json!({
                "name": "ProviderAuthError",
                "data": {"providerID": "opencode", "message": "Invalid API key"}
            })),
            expected(SignInRequired, false, "Invalid API key")
        );
        assert_eq!(
            session_error(serde_json::json!({
                "name": "APIError",
                "data": {"message": "Overloaded", "statusCode": 529, "isRetryable": true}
            })),
            expected(Overloaded, true, "Overloaded")
        );
        assert_eq!(
            session_error(serde_json::json!({
                "name": "APIError",
                "data": {"message": "Rate limit exceeded", "statusCode": 429, "isRetryable": true}
            })),
            expected(UsageLimit, true, "Rate limit exceeded")
        );
        assert_eq!(
            session_error(serde_json::json!({
                "name": "APIError",
                "data": {"message": "Internal Server Error", "statusCode": 500, "isRetryable": true}
            })),
            expected(Other, true, "Internal Server Error")
        );
        // Recorded from the official OpenCode server with an empty native home
        // (extagent/r40/opencode-native-error.json), before any model generation.
        assert_eq!(
            session_error(serde_json::json!({
                "name": "UnknownError",
                "data": {"message": "Model not found: opencode/glm-5.3-flash. Did you mean: glm-5.3-flash-free?"}
            })),
            expected(
                Other,
                false,
                "Model not found: opencode/glm-5.3-flash. Did you mean: glm-5.3-flash-free?"
            )
        );
        for (status, message, cause) in [
            (403, "permission_error: model access denied", Other),
            (403, "Invalid API key", SignInRequired),
            (401, "Request rejected", SignInRequired),
        ] {
            assert_eq!(
                session_error(serde_json::json!({
                    "name": "APIError",
                    "data": {"message": message, "statusCode": status, "isRetryable": false}
                })),
                expected(cause, false, message)
            );
        }
        // The provider's explicit retryability wins over an HTTP-status default.
        assert_eq!(
            session_error(serde_json::json!({
                "name": "APIError",
                "data": {"message": "Overloaded", "statusCode": 529, "isRetryable": false}
            })),
            expected(Overloaded, false, "Overloaded")
        );
        assert_eq!(
            session_error(serde_json::json!({
                "name": "UnknownError",
                "data": {"message": "Unable to connect. Is the computer able to access the url?"}
            })),
            expected(
                Network,
                true,
                "Unable to connect. Is the computer able to access the url?"
            )
        );
    }
}

#[cfg(test)]
mod approval_origin_tests {
    #[test]
    fn approval_lifetime_opencode_parser_retains_permission_tool_message_identity() {
        let payload = serde_json::json!({"type":"permission.asked", "properties": {
            "id":"permission-A", "sessionID":"session", "permission":"bash", "patterns":[], "metadata":{},
            "tool":{"messageID":"assistant-A", "callID":"call-A"}
        }});
        let Some(super::OpenCodeEvent::PermissionAsked { request }) =
            super::parse_sse_event(&payload.to_string(), "run")
        else {
            panic!("missing permission event")
        };
        assert_eq!(request.message_id.as_deref(), Some("assistant-A"));
    }
}
