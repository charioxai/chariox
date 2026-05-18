use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::{connect, Message};

use crate::error::DaemonError;

use super::json_rpc::JsonRpcMessage;
use super::notifications::{parse_notification, rpc_error_message};
use super::socket_io::set_socket_timeouts;
use super::{CodexClient, CodexNotification, CodexSocket};

impl CodexClient {
    pub fn connect_initialized(&self) -> Result<CodexSocket, DaemonError> {
        let (mut socket, _) = connect(self.endpoint.as_str())
            .map_err(|error| self.protocol_error("codex_connect", error.to_string()))?;
        set_socket_timeouts(
            &mut socket,
            Some(Duration::from_secs(10)),
            Some(Duration::from_secs(10)),
        )?;
        self.initialize_socket(&mut socket)?;
        Ok(socket)
    }

    pub fn send_request<T: for<'de> Deserialize<'de>>(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        method: &'static str,
        params: Value,
    ) -> Result<T, DaemonError> {
        self.send_request_buffering_notifications(
            socket,
            next_request_id,
            method,
            params,
            &mut Vec::new(),
        )
    }

    pub fn send_request_buffering_notifications<T: for<'de> Deserialize<'de>>(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        method: &'static str,
        params: Value,
        buffered_notifications: &mut Vec<CodexNotification>,
    ) -> Result<T, DaemonError> {
        let request_id = *next_request_id;
        *next_request_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        socket
            .send(Message::Text(payload.to_string().into()))
            .map_err(|error| self.protocol_error("codex_write", error.to_string()))?;

        loop {
            let raw = self.read_next_message(socket, Duration::from_secs(30))?;
            let message: JsonRpcMessage = serde_json::from_str(&raw)
                .map_err(|error| self.protocol_error("codex_read_parse", error.to_string()))?;
            if self.respond_to_server_request(socket, &message)? {
                continue;
            }
            if message.id.as_ref() == Some(&json!(request_id)) {
                if let Some(error) = rpc_error_message(&message) {
                    return Err(self.protocol_error(method, error));
                }
                let result = message.result.ok_or_else(|| {
                    self.protocol_error(method, "Codex returned no response payload".to_string())
                })?;
                return serde_json::from_value(result)
                    .map_err(|error| self.protocol_error(method, error.to_string()));
            }
            if let Some(notification) = parse_notification(message.clone()) {
                buffered_notifications.push(notification);
            } else if let Some(message_method) = message.method.as_deref() {
                crate::logging::debug_with_fields(
                    "daemon.provider.codex",
                    "ignored codex message while awaiting response",
                    json!({
                        "provider_run_id": self.provider_run_id,
                        "awaiting_method": method,
                        "message_method": message_method,
                        "has_id": message.id.is_some(),
                        "params": message.params,
                        "error": message.error,
                    }),
                );
            }
        }
    }

    pub fn read_notification(
        &self,
        socket: &mut CodexSocket,
        timeout: Duration,
    ) -> Result<Option<CodexNotification>, DaemonError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            set_socket_timeouts(socket, Some(remaining), Some(Duration::from_secs(5)))?;
            match socket.read() {
                Ok(message) => {
                    let raw = match message {
                        Message::Text(text) => text.to_string(),
                        Message::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                        Message::Close(_) => {
                            return Ok(Some(CodexNotification::Error {
                                message: "Codex app-server closed the websocket".to_string(),
                            }));
                        }
                    };
                    let message: JsonRpcMessage = serde_json::from_str(&raw).map_err(|error| {
                        self.protocol_error("codex_notification_parse", error.to_string())
                    })?;
                    if self.respond_to_server_request(socket, &message)? {
                        continue;
                    }
                    let notification = parse_notification(message.clone());
                    if let Some(notification) = notification {
                        return Ok(Some(notification));
                    }
                    if let Some(method) = message.method.as_deref() {
                        crate::logging::debug_with_fields(
                            "daemon.provider.codex",
                            "ignored codex notification",
                            json!({
                                "provider_run_id": self.provider_run_id,
                                "method": method,
                                "has_id": message.id.is_some(),
                                "params": message.params,
                                "error": message.error,
                            }),
                        );
                    }
                    continue;
                }
                Err(tokio_tungstenite::tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(self.protocol_error("codex_read", error.to_string())),
            }
        }
    }

    fn initialize_socket(&self, socket: &mut CodexSocket) -> Result<(), DaemonError> {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "arroba-kernel",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                },
            },
        });
        socket
            .send(Message::Text(initialize.to_string().into()))
            .map_err(|error| self.protocol_error("codex_initialize", error.to_string()))?;
        let response = self.read_next_message(socket, Duration::from_secs(10))?;
        let message: JsonRpcMessage = serde_json::from_str(&response)
            .map_err(|error| self.protocol_error("codex_initialize_parse", error.to_string()))?;
        if message.result.is_none() {
            return Err(self.protocol_error(
                "codex_initialize",
                rpc_error_message(&message)
                    .unwrap_or_else(|| "Codex returned no initialize result".to_string()),
            ));
        }
        let initialized = json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {},
        });
        socket
            .send(Message::Text(initialized.to_string().into()))
            .map_err(|error| self.protocol_error("codex_initialized", error.to_string()))?;
        Ok(())
    }

    fn read_next_message(
        &self,
        socket: &mut CodexSocket,
        timeout: Duration,
    ) -> Result<String, DaemonError> {
        set_socket_timeouts(socket, Some(timeout), Some(Duration::from_secs(5)))?;
        loop {
            match socket.read() {
                Ok(Message::Text(text)) => return Ok(text.to_string()),
                Ok(Message::Binary(bytes)) => {
                    return Ok(String::from_utf8_lossy(&bytes).into_owned());
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                Ok(Message::Close(_)) => {
                    return Err(self.protocol_error(
                        "codex_read",
                        "Codex app-server closed the websocket".to_string(),
                    ));
                }
                Ok(Message::Frame(_)) => continue,
                Err(error) => return Err(self.protocol_error("codex_read", error.to_string())),
            }
        }
    }
}
