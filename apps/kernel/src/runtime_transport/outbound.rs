//! MP-08 / MP-10 / MP-11: the only socket write boundary, including control,
//! response/cache, live/coalesced/replay events and terminal/attachment streams.
use super::*;
use crate::runtime::external_response::{project_response_value, redact_secret_values};

pub(super) struct OutboundBoundary {
    runtime: crate::runtime::state::KernelRuntimeState,
    peer: Option<crate::runtime::kernel_access::process::ProcessIdentity>,
    grant: Arc<std::sync::Mutex<Option<String>>>,
    connection: Arc<Mutex<ConnectionState>>,
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
        connection: Arc<Mutex<ConnectionState>>,
    ) -> Self {
        Self {
            runtime,
            peer,
            grant,
            connection,
        }
    }

    pub(super) async fn send<S>(&self, writer: &mut S, payload: Payload) -> bool
    where
        S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        let mut payload = payload;
        let closing = matches!(&payload, Payload::Control(Message::Close(_)));
        if !closing && !unix_access::delivery_live(&self.runtime, self.peer.as_ref(), &self.grant) {
            self.runtime.pump_kernel_access();
            let request = match &payload {
                Payload::Frame(KernelOutgoingFrame::Response { request_id, .. }) => {
                    Some(request_id.clone())
                }
                _ => None,
            };
            let admitted = if let Some(request) = request.as_ref() {
                self.connection
                    .lock()
                    .await
                    .outside_wait_requests
                    .contains(request)
            } else {
                false
            };
            if admitted {
                if !self.replace_ending_receipt(&mut payload) {
                    return false;
                }
            } else {
                // No data, heartbeat or unrelated reply may race the admitted wait's ending.
                return !self
                    .connection
                    .lock()
                    .await
                    .outside_wait_requests
                    .is_empty();
            }
        }
        let response_id = match &payload {
            Payload::Frame(KernelOutgoingFrame::Response { request_id, .. }) => {
                Some(request_id.clone())
            }
            _ => None,
        };
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
                                    kernel: None,
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
        let sent = writer.send(message).await.is_ok();
        if let Some(id) = response_id {
            self.connection
                .lock()
                .await
                .outside_wait_requests
                .remove(&id);
        }
        sent
    }

    fn replace_ending_receipt(&self, payload: &mut Payload) -> bool {
        if !self.peer.as_ref().is_some_and(|peer| peer.alive()) {
            return false;
        }
        let Some(grant) = self.grant.lock().expect("bound grant poisoned").clone() else {
            return false;
        };
        let Ok(Some(receipt)) = self.runtime.outside_ended_response(&grant) else {
            return false;
        };
        let Payload::Frame(KernelOutgoingFrame::Response {
            kernel: None,
            response,
            error,
            ..
        }) = payload
        else {
            return false;
        };
        **response = Some(serde_json::to_value(receipt).expect("typed ending receipt serializes"));
        *error = None;
        true
    }
}

fn withheld_error() -> KernelTransportError {
    KernelTransportError {
        code: "kernel_access_denied".into(),
        message: "external response withheld: operation failed or reply may contain credentials; use the host terminal".into(),
        retryable: false,
    }
}

/// A refusal reaches the grant holder with the code, retry flag and words the kernel
/// gave it, so the agent and the owner reading its report can act on the reason. Only
/// secret-shaped text is cut out of the message.
fn project_error(value: &Value) -> KernelTransportError {
    let code = value["code"]
        .as_str()
        .filter(|code| !code.is_empty())
        .unwrap_or("kernel_request_failed");
    let message = value["message"].as_str().unwrap_or_default();
    KernelTransportError {
        code: crate::secret_redaction::redact_secrets(code).into_owned(),
        message: crate::secret_redaction::redact_secrets(message).into_owned(),
        retryable: value["retryable"].as_bool().unwrap_or(false),
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
                // Only the relay viewer's own subscription tells its client that its task ended.
                KernelEvent::SubscriptionEnded { .. } => {},
                // Transcript turns are for the session's own clients, not external streams.
                KernelEvent::AgentTurnChanged { .. } => {},
                // External agents follow trees through their own `agent_tree` event.
                KernelEvent::AgentTreeChanged { .. } => {},
                KernelEvent::HomeExtensionAuditAppended { .. } => {},
                KernelEvent::HomeExtensionAuditLagged { .. } => {},
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
    SessionProcessesChanged => "session_processes_changed",
    SessionJobsChanged => "session_jobs_changed",
    SessionSnapshot => "session_snapshot",
    AgentTaskUpdated => "agent_task_updated",
    AgentActivityChanged => "agent_activity_changed",
    ProviderRunChanged => "provider_run_changed",
    SessionMetadataChanged => "session_metadata_changed",
    RuntimeInteractionsChanged => "runtime_interactions_changed",
    SessionUnavailable => "session_unavailable",
    RelayStatusChanged => "relay_status_changed",
    RemoteMachinesChanged => "remote_machines_changed",
    RoomListChanged => "room_list_changed",
    KernelResourceChanged => "kernel_resource_changed",
    RoomListRowsChanged => "room_list_rows_changed",
    ClientCatalogChanged => "client_catalog_changed",
    ProviderCatalogChanged => "provider_catalog_changed",
    SlicesChanged => "slices_changed",
    WorkflowDesignOp => "workflow_design_op",
    WorkflowRunUpdated => "workflow_run_updated",
    RoomWorkflowsChanged => "room_workflows_changed",
    Heartbeat => "heartbeat",
    TransportResumed => "transport_resumed",
    ReplayGap => "replay_gap",
}

/// Whether an external grant holder may see this event: the list `project_payload` enforces.
pub(super) fn is_public_event(event: &KernelEvent) -> bool {
    serde_json::to_value(event)
        .ok()
        .and_then(|value| value["event"].as_str().map(public_event))
        .unwrap_or(false)
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
