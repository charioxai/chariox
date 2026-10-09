//! MP-08 / MP-10 / MP-11: the only socket write boundary, including control,
//! response/cache, live/coalesced/replay events and terminal/attachment streams.
use super::*;
use crate::runtime::external_response::{project_response_value, redact_secret_values};

pub(super) struct OutboundBoundary {
    runtime: crate::runtime::state::KernelRuntimeState,
    peer: Option<crate::runtime::kernel_access::process::ProcessIdentity>,
    grant: Arc<std::sync::Mutex<Option<String>>>,
}

pub(super) enum Payload {
    Frame(KernelOutgoingFrame),
    Control(Message),
}

impl OutboundBoundary {
    pub(super) fn new(
        runtime: crate::runtime::state::KernelRuntimeState,
        peer: Option<crate::runtime::kernel_access::process::ProcessIdentity>,
        grant: Arc<std::sync::Mutex<Option<String>>>,
    ) -> Self {
        Self {
            runtime,
            peer,
            grant,
        }
    }

    pub(super) async fn send<S>(&self, writer: &mut S, payload: Payload) -> bool
    where
        S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        let closing = matches!(&payload, Payload::Control(Message::Close(_)));
        if !closing && !unix_access::delivery_live(&self.runtime, self.peer.as_ref(), &self.grant) {
            return false;
        }
        let message = match payload {
            Payload::Frame(frame) => {
                if self.peer.is_some() {
                    if let KernelOutgoingFrame::Event { event, .. } = &frame {
                        classify_event(event);
                    }
                    let value = match serde_json::to_value(&frame) {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    let payload = match project_payload(value) {
                        Ok(value) => {
                            match crate::transport::kernel_protocol::serialize_frame_value(value) {
                                Ok(s) => s,
                                Err(_) => return false,
                            }
                        }
                        Err(_) => match frame {
                            KernelOutgoingFrame::Response { request_id, .. } => {
                                let refusal = KernelOutgoingFrame::Response {
                                    request_id,
                                    response: Box::new(None),
                                    error: Some(withheld_error()),
                                };
                                match serialize_frame(&refusal) {
                                    Ok(s) => s,
                                    Err(_) => return false,
                                }
                            }
                            KernelOutgoingFrame::Event { .. } => return false,
                        },
                    };
                    Message::Text(payload.into())
                } else {
                    // MP-08/MP-10: protocol 466 display events are binary WebSocket
                    // messages with raw payload segments; other frames stay JSON text.
                    match frame {
                        KernelOutgoingFrame::Event { event, .. }
                            if matches!(*event, KernelEvent::KernelBrowserFrame { .. }) =>
                        {
                            match crate::transport::kernel_browser_display::encode_display_event(
                                *event,
                            ) {
                                Ok(bytes) => Message::Binary(bytes.into()),
                                Err(_) => return false,
                            }
                        }
                        frame => match serialize_frame(&frame) {
                            Ok(s) => Message::Text(s.into()),
                            Err(_) => return false,
                        },
                    }
                }
            }
            Payload::Control(Message::Close(mut frame)) => {
                if self.peer.is_some() {
                    if let Some(frame) = &mut frame {
                        frame.reason = "kernel connection closed".into();
                    }
                }
                Message::Close(frame)
            }
            // Ping is empty and Pong only echoes this peer's inbound payload.
            Payload::Control(message @ (Message::Ping(_) | Message::Pong(_))) => message,
            // No unclassified binary/text escape hatch to the socket.
            Payload::Control(_) => return false,
        };
        writer.send(message).await.is_ok()
    }
}

fn withheld_error() -> KernelTransportError {
    KernelTransportError {
        code: "kernel_access_denied".into(),
        message: "external response withheld: operation failed or reply may contain credentials; use the host terminal".into(),
        retryable: false,
    }
}

// MP-11: machine-readable kernel codes and retry flags are public protocol.
// Provider/parser text is private unless it is an exact lifecycle constant.
fn project_error(value: &Value) -> KernelTransportError {
    let code = value["code"].as_str().unwrap_or_default();
    let mut projected = withheld_error();
    if matches!(
        code,
        "session_not_found"
            | "attachment_not_found"
            | "attachment_not_in_session"
            | "no_active_provider_run"
            | "provider_run_not_found"
            | "workspace_claim_conflict"
            | "provider_adapter_not_found"
            | "provider_protocol_error"
            | "credential_vault_locked"
            | "local_transport_error"
            | "pty_spawn_failed"
            | "pty_cleanup_failed"
            | "pty_write_failed"
            | "pty_resize_failed"
            | "kernel_request_failed"
            | "kernel_access_denied"
            | "invalid_request"
            | "invalid_frame"
            | "kernel_request_overloaded"
            | "duplicate_command_unavailable"
            | "duplicate_command_conflict"
    ) {
        projected.code = code.into();
        projected.retryable = value["retryable"].as_bool().unwrap_or(false);
        projected.message = match code {
            "kernel_request_overloaded" => "kernel request admission queue overloaded",
            "duplicate_command_unavailable" => "original duplicate command result was unavailable",
            "duplicate_command_conflict" => "command_id was already used for a different request",
            _ => "external request failed; details are available only in the host terminal",
        }
        .into();
    }
    let message = value["message"].as_str().unwrap_or_default();
    if crate::runtime::external_response::public_error_message(message) {
        projected.message = message.into();
    }
    projected
}

// Both representations come from one declaration. New stream events require
// an explicit policy at compile time, not a new filtering path.
macro_rules! public_events {
    ($($variant:ident => $wire:literal),* $(,)?) => {
        fn classify_event(event: &KernelEvent) {
            match event {
                $(KernelEvent::$variant { .. } => {},)*
                KernelEvent::PasskeyPromptsChanged { .. } => {},
                // MP-11: browser display frames stay off external streams.
                KernelEvent::KernelBrowserFrame { .. } => {},
            }
        }
        fn public_event(name: &str) -> bool { matches!(name, $($wire)|*) }
    };
}
public_events! {
    TerminalOutput => "terminal_output",
    RuntimeNotices => "runtime_notices",
    AssistantMessageCompleted => "assistant_message_completed",
    SessionSnapshot => "session_snapshot",
    AgentActivityChanged => "agent_activity_changed",
    ProviderRunChanged => "provider_run_changed",
    SessionMetadataChanged => "session_metadata_changed",
    RuntimeInteractionsChanged => "runtime_interactions_changed",
    SessionUnavailable => "session_unavailable",
    RelayStatusChanged => "relay_status_changed",
    RemoteMachinesChanged => "remote_machines_changed",
    WaitingRoomInventoryChanged => "waiting_room_inventory_changed",
    WaitingRoomRowsChanged => "waiting_room_rows_changed",
    ProviderCatalogChanged => "provider_catalog_changed",
    SlicesChanged => "slices_changed",
    WorkflowDesignOp => "workflow_design_op",
    WorkflowRunUpdated => "workflow_run_updated",
    Heartbeat => "heartbeat",
    TransportResumed => "transport_resumed",
    ReplayGap => "replay_gap",
}

pub(super) fn project_payload(mut value: Value) -> Result<Value, DaemonError> {
    let deny = || crate::runtime::kernel_access::error("external outbound payload withheld");
    match value["type"].as_str() {
        Some("response") => {
            if !value["error"].is_null() {
                value["error"] =
                    serde_json::to_value(project_error(&value["error"])).map_err(|_| deny())?;
            }
            if !value["response"].is_null() {
                let body = &mut value["response"];
                if body.get("ok") == Some(&Value::Bool(true)) {
                    // Closed transport acknowledgments, not an open Value bypass.
                    #[derive(serde::Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Ack {
                        #[allow(dead_code)]
                        ok: bool,
                        #[allow(dead_code)]
                        resumed_from_event_id: Option<u64>,
                        #[allow(dead_code)]
                        replay_gap: Option<Gap>,
                    }
                    #[derive(serde::Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Gap {
                        #[allow(dead_code)]
                        requested_from_event_id: u64,
                        #[allow(dead_code)]
                        first_retained_event_id: Option<u64>,
                        #[allow(dead_code)]
                        latest_event_id: Option<u64>,
                    }
                    serde_json::from_value::<Ack>(body.clone()).map_err(|_| deny())?;
                } else {
                    project_response_value(body)?;
                }
            }
        }
        Some("event") => {
            if !value["event"]["event"].as_str().is_some_and(public_event) {
                return Err(deny());
            }
            redact_secret_values(&mut value["event"]);
        }
        _ => return Err(deny()),
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
