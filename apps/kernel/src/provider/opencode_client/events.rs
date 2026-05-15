//! OpenCode SSE payload parsing into provider runtime events.

use serde::Deserialize;
use serde_json::Value;

use super::{
    OpenCodeEvent, OpenCodeMessageInfo, OpenCodePart, OpenCodePermissionRequest,
    OpenCodeSessionStatus,
};

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
                kind: properties.status.kind,
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
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!("OpenCode reported an unknown session error for `{provider_run_id}`")
        })
}
