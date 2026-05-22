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
                Err(error) if is_retryable_opencode_http_error(&error) => {
                    last_error = Some(error);
                    if attempt + 1 < Self::REQUEST_RETRY_ATTEMPTS {
                        std::thread::sleep(Self::REQUEST_RETRY_DELAY);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            self.protocol_error(
                method_to_operation(method, path),
                "OpenCode request failed".to_string(),
            )
        }))
    }

    fn send_request_once(
        &self,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(u16, Vec<u8>), DaemonError> {
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
            "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nX-Arroba-Provider-Client: kernel\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body_bytes.len()
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(&body_bytes))
            .and_then(|_| stream.flush())
            .map_err(|error| {
                self.protocol_error(method_to_operation(method, path), error.to_string())
            })?;

        read_http_response(&mut stream)
            .map_err(|error| self.protocol_error(method_to_operation(method, path), error))
    }
}

fn is_retryable_opencode_http_error(error: &DaemonError) -> bool {
    let DaemonError::ProviderProtocol {
        operation, message, ..
    } = error
    else {
        return false;
    };
    if *operation != "opencode_http" {
        return false;
    }
    matches!(
        message.as_str(),
        "Resource temporarily unavailable (os error 35)"
            | "Connection refused (os error 61)"
            | "Connection reset by peer (os error 54)"
    ) || message.contains("timed out")
        || message.contains("connection closed before response header")
        || message.contains("Broken pipe")
        || retryable_io_error_kind(message).is_some()
}

fn retryable_io_error_kind(message: &str) -> Option<ErrorKind> {
    [
        ErrorKind::WouldBlock,
        ErrorKind::Interrupted,
        ErrorKind::TimedOut,
        ErrorKind::ConnectionRefused,
        ErrorKind::ConnectionReset,
        ErrorKind::BrokenPipe,
    ]
    .into_iter()
    .find(|kind| message == kind.to_string())
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

pub(super) fn read_http_response(stream: &mut TcpStream) -> Result<(u16, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end;
    loop {
        let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if size == 0 {
            return Err("connection closed before response header".to_string());
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
        .ok_or_else(|| "missing HTTP status line".to_string())?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "missing HTTP status code".to_string())?
        .parse::<u16>()
        .map_err(|error| error.to_string())?;
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
            let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
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
) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let header_end = loop {
            if let Some(index) = buffered.windows(2).position(|window| window == b"\r\n") {
                break index;
            }
            let size = stream.read(&mut chunk).map_err(|error| error.to_string())?;
            if size == 0 {
                return Err("unexpected EOF while reading chunk header".to_string());
            }
            buffered.extend_from_slice(&chunk[..size]);
        };

        let size_line = String::from_utf8_lossy(&buffered[..header_end]).into_owned();
        let size_hex = size_line
            .split(';')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "missing chunk size".to_string())?;
        let size = usize::from_str_radix(size_hex, 16).map_err(|error| error.to_string())?;
        buffered.drain(..header_end + 2);

        if size == 0 {
            loop {
                if buffered.len() >= 2 && &buffered[..2] == b"\r\n" {
                    buffered.drain(..2);
                    break;
                }
                let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
                if read == 0 {
                    break;
                }
                buffered.extend_from_slice(&chunk[..read]);
            }
            break;
        }

        while buffered.len() < size + 2 {
            let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
            if read == 0 {
                return Err("unexpected EOF while reading chunk body".to_string());
            }
            buffered.extend_from_slice(&chunk[..read]);
        }
        decoded.extend_from_slice(&buffered[..size]);
        buffered.drain(..size + 2);
    }

    Ok(decoded)
}
