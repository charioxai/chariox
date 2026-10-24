//! MP-08 / MP-10: stop the output producer before awaiting interrupt replies.
use super::json_rpc::JsonRpcMessage;
use super::notifications::{parse_notification, rpc_error_message};
use super::{CodexClient, CodexNotification, CodexSocket};
use crate::error::DaemonError;
use serde_json::json;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

/// A Codex app-server answers a honoured `turn/interrupt` within milliseconds. One that stays
/// silent this long will not honour the Stop, so the abort fails here and the turn ends with
/// the cancellation-failure notice instead of waiting out the generic 30 s request timeout.
pub(crate) const CODEX_INTERRUPT_ANSWER_DEADLINE: Duration = Duration::from_secs(10);

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
        // JSON-RPC replies can arrive in either order. Cleanup is best effort;
        // an acknowledged interrupt remains authoritative if cleanup is refused.
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
        let deadline = Instant::now() + CODEX_INTERRUPT_ANSWER_DEADLINE;
        let mut interrupt_result = None;
        let mut cleanup_result = None;
        while interrupt_result.is_none() || cleanup_result.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                if interrupt_result.as_ref().is_some_and(Result::is_ok) {
                    return Ok(());
                }
                return Err(self.protocol_error(
                    "codex_read",
                    "timed out awaiting interrupt/terminal cleanup replies".into(),
                ));
            }
            let raw = match self.read_next_message(socket, remaining) {
                Ok(raw) => raw,
                Err(_) if interrupt_result.as_ref().is_some_and(Result::is_ok) => return Ok(()),
                Err(error) => return Err(error),
            };
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
        interrupt_result.unwrap()?;
        if let Err(error) = cleanup_result.unwrap() {
            crate::logging::warn_with_fields(
                "daemon.provider.codex",
                "codex thread terminal cleanup failed after acknowledged interrupt",
                json!({"provider_run_id":self.provider_run_id,"error":error.to_string()}),
            );
        }
        Ok(())
    }
}
