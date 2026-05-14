//! Runtime MCP dynamic-tool bridge used by Codex tool-call responses.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{json, Value};

use crate::error::DaemonError;

pub(super) fn call_runtime_mcp_tool(
    server_url: &str,
    auth_token: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<String, DaemonError> {
    let endpoint = parse_http_endpoint(server_url)?;
    let payload = json!({
        "jsonrpc": "2.0",
        "id": format!("codex-dynamic-tool-{}", crate::session::unix_epoch_ms()),
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments,
        }
    });
    let body = serde_json::to_vec(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "codex_dynamic_tool_serialize",
        message: error.to_string(),
    })?;
    let mut stream = TcpStream::connect((&*endpoint.host, endpoint.port)).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_connect",
            message: error.to_string(),
        }
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_timeout",
            message: error.to_string(),
        })?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_timeout",
            message: error.to_string(),
        })?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.path,
        endpoint.host,
        endpoint.port,
        auth_token,
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_write",
            message: error.to_string(),
        })?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_read",
            message: error.to_string(),
        })?;
    parse_runtime_mcp_response(&response)
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_endpoint(server_url: &str) -> Result<HttpEndpoint, DaemonError> {
    let rest = server_url
        .strip_prefix("http://")
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_endpoint",
            message: "only http runtime MCP endpoints are supported".to_string(),
        })?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, "mcp"));
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_endpoint",
            message: "runtime MCP endpoint must include an explicit port".to_string(),
        })?;
    let port = port
        .parse::<u16>()
        .map_err(|_| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_endpoint",
            message: "runtime MCP endpoint port is invalid".to_string(),
        })?;
    Ok(HttpEndpoint {
        host: host.to_string(),
        port,
        path: format!("/{path}"),
    })
}

fn parse_runtime_mcp_response(response: &[u8]) -> Result<String, DaemonError> {
    let response_text = String::from_utf8_lossy(response);
    let (head, body) =
        response_text
            .split_once("\r\n\r\n")
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "codex_dynamic_tool_response",
                message: "invalid HTTP response from runtime MCP server".to_string(),
            })?;
    let status_ok = head
        .lines()
        .next()
        .is_some_and(|line| line.contains(" 200 "));
    if !status_ok {
        return Err(DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_response",
            message: head.lines().next().unwrap_or("HTTP error").to_string(),
        });
    }
    let value =
        serde_json::from_str::<Value>(body).map_err(|error| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_response",
            message: error.to_string(),
        })?;
    if let Some(error) = value.get("error") {
        return Err(DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_response",
            message: error.to_string(),
        });
    }
    value
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "codex_dynamic_tool_response",
            message: "runtime MCP response did not include text content".to_string(),
        })
}
