//! Minimal HTTP response parsing for the OpenCode local app-server client.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde::Deserialize;

use crate::error::DaemonError;

use super::OpenCodeClient;

impl OpenCodeClient {
    const REQUEST_RETRY_ATTEMPTS: usize = 5;
    const REQUEST_RETRY_DELAY: Duration = Duration::from_millis(100);

    pub(super) fn send_json_request<T: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<T, DaemonError> {
        let (status_code, response_body) = self.send_request(method, path, body)?;
        if status_code >= 400 {
            return Err(self.protocol_error(
                method_to_operation(method, path),
                format!("OpenCode returned HTTP {status_code}"),
            ));
        }

        serde_json::from_slice(&response_body).map_err(|error| {
            self.protocol_error(
                method_to_operation(method, path),
                format!(
                    "{}; response body: {}",
                    error,
                    preview_response_body(&response_body)
                ),
            )
        })
    }

    pub(super) fn send_no_content_request(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(), DaemonError> {
        let (status_code, response_body) = self.send_request(method, path, body)?;
        if status_code >= 400 {
            return Err(self.protocol_error(
                method_to_operation(method, path),
                format!("OpenCode returned HTTP {status_code}"),
            ));
        }
        if !response_body.is_empty() {
            return Err(self.protocol_error(
                method_to_operation(method, path),
                format!(
                    "expected empty response body; got {}",
                    preview_response_body(&response_body)
                ),
            ));
        }
        Ok(())
    }

    pub(super) fn send_request(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(u16, Vec<u8>), DaemonError> {
        let mut last_error = None;
        for attempt in 0..Self::REQUEST_RETRY_ATTEMPTS {
            match self.send_request_once(method, path, body) {
                Ok(response) => return Ok(response),
                Err(error) if is_retryable_opencode_http_error(method, path, &error) => {
                    last_error = Some(error.into_daemon_error(self, method, path));
                    if attempt + 1 < Self::REQUEST_RETRY_ATTEMPTS {
                        std::thread::sleep(retry_delay_for_attempt(attempt));
                    }
                }
                Err(error) => return Err(error.into_daemon_error(self, method, path)),
            }
        }
        let operation = method_to_operation(method, path);
        let message = last_error
            .map(|error| {
                format!(
                    "OpenCode request failed after {} attempts: {error}",
                    Self::REQUEST_RETRY_ATTEMPTS
                )
            })
            .unwrap_or_else(|| "OpenCode request failed".to_string());
        Err(self.protocol_error(operation, message))
    }

    fn send_request_once(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(u16, Vec<u8>), OpenCodeHttpFailure> {
        let address = self.base_url.strip_prefix("http://").ok_or_else(|| {
            OpenCodeHttpFailure::protocol(format!(
                "unsupported OpenCode base URL `{}`",
                self.base_url
            ))
        })?;
        let mut stream = TcpStream::connect(address).map_err(OpenCodeHttpFailure::io)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(OpenCodeHttpFailure::io)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(OpenCodeHttpFailure::io)?;

        let body_bytes = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| OpenCodeHttpFailure::protocol(error.to_string()))?
            .unwrap_or_default();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nX-Arroba-Provider-Client: kernel\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body_bytes.len()
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(&body_bytes))
            .and_then(|_| stream.flush())
            .map_err(OpenCodeHttpFailure::io)?;

        read_http_response(&mut stream).map_err(OpenCodeHttpFailure::from)
    }
}

#[derive(Debug)]
struct OpenCodeHttpFailure {
    message: String,
    io_kind: Option<ErrorKind>,
}

impl OpenCodeHttpFailure {
    fn io(error: std::io::Error) -> Self {
        Self {
            message: error.to_string(),
            io_kind: Some(error.kind()),
        }
    }

    fn protocol(message: String) -> Self {
        Self {
            message,
            io_kind: None,
        }
    }

    fn into_daemon_error(
        self,
        client: &OpenCodeClient,
        method: &'static str,
        path: &str,
    ) -> DaemonError {
        client.protocol_error(method_to_operation(method, path), self.message)
    }
}

impl std::fmt::Display for OpenCodeHttpFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl From<HttpResponseReadError> for OpenCodeHttpFailure {
    fn from(error: HttpResponseReadError) -> Self {
        Self {
            message: error.message,
            io_kind: error.io_kind,
        }
    }
}

#[derive(Debug)]
pub(super) struct HttpResponseReadError {
    message: String,
    io_kind: Option<ErrorKind>,
}

impl HttpResponseReadError {
    fn io(error: std::io::Error) -> Self {
        Self {
            message: error.to_string(),
            io_kind: Some(error.kind()),
        }
    }

    fn protocol(message: String) -> Self {
        Self {
            message,
            io_kind: None,
        }
    }
}

fn is_retryable_opencode_http_error(
    method: &'static str,
    path: &str,
    error: &OpenCodeHttpFailure,
) -> bool {
    if !is_retryable_request(method, path) {
        return false;
    }
    error.io_kind.is_some_and(is_retryable_io_error_kind)
        || error
            .message
            .contains("connection closed before response header")
}

fn is_retryable_request(method: &'static str, path: &str) -> bool {
    method == "GET"
        // OpenCode prompt submission is retried because the body carries Arroba's stable
        // messageID, so a retry can be deduplicated by the provider side.
        || (method == "POST" && path.ends_with("/prompt_async"))
}

fn is_retryable_io_error_kind(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::WouldBlock
            | ErrorKind::Interrupted
            | ErrorKind::TimedOut
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::BrokenPipe
    )
}

fn retry_delay_for_attempt(attempt: usize) -> Duration {
    OpenCodeClient::REQUEST_RETRY_DELAY * (attempt as u32 + 1)
}

fn method_to_operation(method: &str, path: &str) -> &'static str {
    match (method, path) {
        ("GET", "/global/health") => "health",
        ("GET", "/event") => "event_subscribe",
        ("POST", "/session") => "session_create",
        ("GET", "/session/status") => "session_status",
        _ if method == "POST" && path.ends_with("/prompt_async") => "session_prompt",
        _ if method == "POST" && path.ends_with("/message") => "session_prompt",
        _ if method == "POST" && path.ends_with("/abort") => "session_abort",
        _ if method == "GET" && path.ends_with("/message") => "session_messages",
        _ => "opencode_http",
    }
}

pub(super) fn preview_response_body(body: &[u8]) -> String {
    if body.is_empty() {
        return "<empty>".to_string();
    }

    let preview = String::from_utf8_lossy(body);
    let preview = preview.trim();
    if preview.len() > 240 {
        format!("{}...", &preview[..240])
    } else {
        preview.to_string()
    }
}

pub(super) fn read_http_headers(stream: &mut TcpStream) -> Result<(u16, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if size == 0 {
            return Err("connection closed before response header".to_string());
        }
        buffer.extend_from_slice(&chunk[..size]);
        if let Some(index) = find_double_crlf(&buffer) {
            let header_end = index + 4;
            let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
            let status_code = header_text
                .lines()
                .next()
                .ok_or_else(|| "missing HTTP status line".to_string())?
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| "missing HTTP status code".to_string())?
                .parse::<u16>()
                .map_err(|error| error.to_string())?;
            return Ok((status_code, buffer[header_end..].to_vec()));
        }
    }
}

pub(super) fn read_http_response(
    stream: &mut TcpStream,
) -> Result<(u16, Vec<u8>), HttpResponseReadError> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end;
    loop {
        let size = stream.read(&mut chunk).map_err(HttpResponseReadError::io)?;
        if size == 0 {
            return Err(HttpResponseReadError::protocol(
                "connection closed before response header".to_string(),
            ));
        }
        buffer.extend_from_slice(&chunk[..size]);
        if let Some(index) = find_double_crlf(&buffer) {
            header_end = index + 4;
            break;
        }
    }

    let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = header_text.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| HttpResponseReadError::protocol("missing HTTP status line".to_string()))?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| HttpResponseReadError::protocol("missing HTTP status code".to_string()))?
        .parse::<u16>()
        .map_err(|error| HttpResponseReadError::protocol(error.to_string()))?;
    let mut content_length = None;
    let mut is_chunked = false;
    for line in lines {
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(value) = parts.next().map(str::trim) else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse::<usize>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        {
            is_chunked = true;
        }
    }

    let body = if is_chunked {
        read_chunked_http_body(buffer[header_end..].to_vec(), stream)?
    } else {
        let mut body = buffer[header_end..].to_vec();
        let content_length = content_length.unwrap_or(0);
        while body.len() < content_length {
            let size = stream.read(&mut chunk).map_err(HttpResponseReadError::io)?;
            if size == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..size]);
        }
        body
    };
    Ok((status_code, body))
}

fn find_double_crlf(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn read_chunked_http_body(
    mut buffered: Vec<u8>,
    stream: &mut TcpStream,
) -> Result<Vec<u8>, HttpResponseReadError> {
    let mut decoded = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let header_end = loop {
            if let Some(index) = buffered.windows(2).position(|window| window == b"\r\n") {
                break index;
            }
            let size = stream.read(&mut chunk).map_err(HttpResponseReadError::io)?;
            if size == 0 {
                return Err(HttpResponseReadError::protocol(
                    "unexpected EOF while reading chunk header".to_string(),
                ));
            }
            buffered.extend_from_slice(&chunk[..size]);
        };

        let size_line = String::from_utf8_lossy(&buffered[..header_end]).into_owned();
        let size_hex = size_line
            .split(';')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| HttpResponseReadError::protocol("missing chunk size".to_string()))?;
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|error| HttpResponseReadError::protocol(error.to_string()))?;
        buffered.drain(..header_end + 2);

        if size == 0 {
            loop {
                if buffered.len() >= 2 && &buffered[..2] == b"\r\n" {
                    buffered.drain(..2);
                    break;
                }
                let read = stream.read(&mut chunk).map_err(HttpResponseReadError::io)?;
                if read == 0 {
                    break;
                }
                buffered.extend_from_slice(&chunk[..read]);
            }
            break;
        }

        while buffered.len() < size + 2 {
            let read = stream.read(&mut chunk).map_err(HttpResponseReadError::io)?;
            if read == 0 {
                return Err(HttpResponseReadError::protocol(
                    "unexpected EOF while reading chunk body".to_string(),
                ));
            }
            buffered.extend_from_slice(&chunk[..read]);
        }
        decoded.extend_from_slice(&buffered[..size]);
        buffered.drain(..size + 2);
    }

    Ok(decoded)
}
