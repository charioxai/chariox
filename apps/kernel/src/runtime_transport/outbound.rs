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
                let payload = if self.peer.is_some() {
                    if let KernelOutgoingFrame::Event { event, .. } = &frame {
                        classify_event(event);
                    }
                    let value = match serde_json::to_value(&frame) {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    match project_payload(value) {
                        Ok(value) => match serde_json::to_string(&value) {
                            Ok(s) => s,
                            Err(_) => return false,
                        },
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
                    }
                } else {
                    match serialize_frame(&frame) {
                        Ok(s) => s,
                        Err(_) => return false,
                    }
                };
                Message::Text(payload.into())
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

// Both representations come from one declaration. New stream events require
// an explicit policy at compile time, not a new filtering path.
macro_rules! public_events {
    ($($variant:ident => $wire:literal),* $(,)?) => {
        fn classify_event(event: &KernelEvent) {
            match event {
                $(KernelEvent::$variant { .. } => {},)*
                KernelEvent::PasskeyPromptsChanged { .. } => {},
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
                // Parser/provider errors may echo stored values. Preserve only
                // exact kernel authority constants, never arbitrary suffixes.
                let message = value["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let safe = crate::runtime::external_response::public_error_message(&message);
                value["error"] = serde_json::to_value(withheld_error()).map_err(|_| deny())?;
                if safe {
                    value["error"]["message"] = message.into();
                }
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
