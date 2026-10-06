//! MP-08 / MP-10: stop the output producer before awaiting interrupt replies.
use super::json_rpc::JsonRpcMessage;
use super::notifications::{parse_notification, rpc_error_message};
use super::{CodexClient, CodexNotification, CodexSocket};
use crate::error::DaemonError;
use serde_json::json;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

impl CodexClient {
    pub fn turn_interrupt_and_clean(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        thread_id: &str,
        turn_id: &str,
        buffered_notifications: &mut Vec<CodexNotification>,
    ) -> Result<(), DaemonError> {
        let interrupt_id = *next_request_id;
        let cleanup_id = interrupt_id + 1;
        *next_request_id += 2;
        // Two documented requests, in wire order, on the existing connection.
        // JSON-RPC replies can arrive in either order; neither ACK is optional.
        for (id, method, params) in [
            (
                interrupt_id,
                "turn/interrupt",
                json!({"threadId":thread_id,"turnId":turn_id}),
            ),
            (
                cleanup_id,
                "thread/backgroundTerminals/clean",
                json!({"threadId":thread_id}),
            ),
        ] {
            socket
                .send(Message::Text(
                    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
                        .to_string()
                        .into(),
                ))
                .map_err(|error| self.protocol_error("codex_write", error.to_string()))?;
            if id == interrupt_id {
                crate::logging::debug_with_fields(
                    "daemon.provider.codex",
                    "codex turn interrupt sent trace",
                    json!({"provider_run_id":self.provider_run_id,"turn_id":turn_id,"request_id":id}),
                );
            }
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut interrupt_result = None;
        let mut cleanup_result = None;
        while interrupt_result.is_none() || cleanup_result.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.protocol_error(
                    "codex_read",
                    "timed out awaiting interrupt/terminal cleanup replies".into(),
                ));
            }
            let raw = self.read_next_message(socket, remaining)?;
            let message: JsonRpcMessage = serde_json::from_str(&raw)
                .map_err(|error| self.protocol_error("codex_read_parse", error.to_string()))?;
            if self.respond_to_server_request(socket, &message)? {
                continue;
            }
            let method = if message.id.as_ref() == Some(&json!(interrupt_id)) {
                Some("turn/interrupt")
            } else if message.id.as_ref() == Some(&json!(cleanup_id)) {
                Some("thread/backgroundTerminals/clean")
            } else {
                None
            };
            if let Some(method) = method {
                let result = if let Some(error) = rpc_error_message(&message) {
                    Err(self.protocol_error(method, error))
                } else if message.result.is_some() {
                    Ok(())
                } else {
                    Err(self.protocol_error(method, "Codex returned no response payload".into()))
                };
                if method == "turn/interrupt" {
                    interrupt_result = Some(result);
                } else {
                    cleanup_result = Some(result);
                }
            } else if let Some(notification) = parse_notification(message) {
                super::runtime_trace::notification_received(&self.provider_run_id, &notification);
                buffered_notifications.push(notification);
            }
        }
        cleanup_result.unwrap()?;
        crate::logging::debug_with_fields(
            "daemon.provider.codex",
            "codex thread terminals cleaned trace",
            json!({"provider_run_id":self.provider_run_id}),
        );
        interrupt_result.unwrap()
    }
}
