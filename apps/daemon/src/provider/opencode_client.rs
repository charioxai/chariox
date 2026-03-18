use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use crate::error::DaemonError;

#[derive(Debug, Clone)]
pub struct OpenCodeClient {
    provider_run_id: String,
    base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeSessionSnapshot {
    pub status: String,
    pub messages: Vec<OpenCodeMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessage {
    pub info: OpenCodeMessageInfo,
    #[serde(default)]
    pub parts: Vec<OpenCodeMessagePart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageInfo {
    pub id: String,
    pub role: String,
    #[serde(default)]
    pub time: OpenCodeMessageTime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessageTime {
    #[serde(default)]
    pub completed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OpenCodeMessagePart {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct OpenCodeHealth {
    healthy: bool,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSessionCreated {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OpenCodeSessionStatus {
    #[serde(rename = "type")]
    kind: String,
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

    pub fn wait_until_healthy(&self, timeout: Duration) -> Result<(), DaemonError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.health() {
                Ok(()) => return Ok(()),
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub fn create_session(&self) -> Result<String, DaemonError> {
        let created: OpenCodeSessionCreated =
            self.send_json_request("POST", "/session", Some(&json!({})))?;
        Ok(created.id)
    }

    pub fn submit_prompt(
        &self,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
    ) -> Result<(), DaemonError> {
        let mut body = json!({
            "parts": [
                {
                    "type": "text",
                    "text": prompt,
                }
            ],
        });
        if let Some((provider_id, model_id)) = parse_model(model) {
            body["model"] = json!({
                "providerID": provider_id,
                "modelID": model_id,
            });
        }

        self.send_json_request::<serde_json::Value>(
            "POST",
            &format!("/session/{session_id}/message"),
            Some(&body),
        )?;
        Ok(())
    }

    pub fn abort_session(&self, session_id: &str) -> Result<(), DaemonError> {
        self.send_json_request::<serde_json::Value>(
            "POST",
            &format!("/session/{session_id}/abort"),
            Some(&json!({})),
        )?;
        Ok(())
    }

    pub fn snapshot(&self, session_id: &str) -> Result<OpenCodeSessionSnapshot, DaemonError> {
        let status_map: BTreeMap<String, OpenCodeSessionStatus> =
            self.send_json_request("GET", "/session/status", None)?;
        let status = status_map
            .get(session_id)
            .map(|status| status.kind.clone())
            .ok_or_else(|| {
                self.protocol_error(
                    "session_status",
                    format!(
                        "OpenCode did not report session `{session_id}` in the session status response"
                    ),
                )
            })?;

        let messages: Vec<OpenCodeMessage> =
            self.send_json_request("GET", &format!("/session/{session_id}/message"), None)?;

        Ok(OpenCodeSessionSnapshot { status, messages })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn health(&self) -> Result<(), DaemonError> {
        let health: OpenCodeHealth = self.send_json_request("GET", "/health", None)?;
        if health.healthy {
            Ok(())
        } else {
            Err(self.protocol_error("health", "provider reported unhealthy".to_string()))
        }
    }

    fn protocol_error(&self, operation: &'static str, message: String) -> DaemonError {
        DaemonError::ProviderProtocol {
            provider_run_id: self.provider_run_id.clone(),
            operation,
            message,
        }
    }

    fn send_json_request<T: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<T, DaemonError> {
        let address = self.base_url.strip_prefix("http://").ok_or_else(|| {
            self.protocol_error(
                "base_url_parse",
                format!("unsupported OpenCode base URL `{}`", self.base_url),
            )
        })?;
        let mut stream = TcpStream::connect(address).map_err(|error| {
            self.protocol_error(method_to_operation(method, path), error.to_string())
        })?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| {
                self.protocol_error(method_to_operation(method, path), error.to_string())
            })?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| {
                self.protocol_error(method_to_operation(method, path), error.to_string())
            })?;

        let body_bytes = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| {
                self.protocol_error(method_to_operation(method, path), error.to_string())
            })?
            .unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body_bytes.len()
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(&body_bytes))
            .and_then(|_| stream.flush())
            .map_err(|error| {
                self.protocol_error(method_to_operation(method, path), error.to_string())
            })?;

        let (status_code, response_body) = read_http_response(&mut stream)
            .map_err(|error| self.protocol_error(method_to_operation(method, path), error))?;
        if status_code >= 400 {
            return Err(self.protocol_error(
                method_to_operation(method, path),
                format!("OpenCode returned HTTP {status_code}"),
            ));
        }

        serde_json::from_slice(&response_body).map_err(|error| {
            self.protocol_error(method_to_operation(method, path), error.to_string())
        })
    }
}

fn method_to_operation(method: &str, path: &str) -> &'static str {
    match (method, path) {
        ("GET", "/health") => "health",
        ("POST", "/session") => "session_create",
        ("GET", "/session/status") => "session_status",
        _ if method == "POST" && path.ends_with("/message") => "session_prompt",
        _ if method == "POST" && path.ends_with("/abort") => "session_abort",
        _ if method == "GET" && path.ends_with("/message") => "session_messages",
        _ => "opencode_http",
    }
}

fn read_http_response(stream: &mut TcpStream) -> Result<(u16, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end;
    loop {
        let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if size == 0 {
            return Err("connection closed before response header".to_string());
        }
        buffer.extend_from_slice(&chunk[..size]);
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }

    let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = header_text.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| "missing HTTP status line".to_string())?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "missing HTTP status code".to_string())?
        .parse::<u16>()
        .map_err(|error| error.to_string())?;
    let content_length = lines
        .find_map(|line| {
            let mut parts = line.splitn(2, ':');
            let name = parts.next()?.trim();
            let value = parts.next()?.trim();
            (name.eq_ignore_ascii_case("content-length")).then_some(value)
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if size == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..size]);
    }
    Ok((status_code, body))
}

fn parse_model(model: Option<&str>) -> Option<(&str, &str)> {
    let value = model?.trim();
    if value.is_empty() || value == "default" {
        return None;
    }
    value.split_once('/')
}

#[cfg(test)]
mod tests {
    use super::parse_model;

    #[test]
    fn parses_provider_model_ids() {
        assert_eq!(
            parse_model(Some("anthropic/claude-sonnet-4")),
            Some(("anthropic", "claude-sonnet-4"))
        );
        assert_eq!(parse_model(Some("default")), None);
        assert_eq!(parse_model(None), None);
    }
}
