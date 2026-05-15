//! Minimal HTTP response parsing for the OpenCode local app-server client.

use std::io::Read;
use std::net::TcpStream;

pub(super) fn method_to_operation(method: &str, path: &str) -> &'static str {
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
