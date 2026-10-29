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
    use crate::provider::{ProviderFailure, ProviderFailureCause::*};

    /// EA2-75: OpenCode `session.error` events keep the harness message and
    /// give the typed cause from the error name and HTTP status. The error
    /// objects follow OpenCode's `ProviderAuthError` / `APIError` /
    /// `UnknownError` schema; none was recorded in earlier evidence.
    fn session_error(error: serde_json::Value) -> ProviderFailure {
        let payload = serde_json::json!({
            "type": "session.error",
            "properties": {"sessionID": "session", "error": error}
        });
        let Some(super::OpenCodeEvent::SessionError { message, .. }) =
            super::parse_sse_event(&payload.to_string(), "run")
        else {
            panic!("missing session error")
        };
        ProviderFailure::classify("opencode", &message)
    }

    #[test]
    fn opencode_errors_are_typed_from_name_and_status() {
        let expected = |cause, retryable, message: &str| ProviderFailure {
            cause,
            retryable,
            message: message.to_string(),
        };
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
