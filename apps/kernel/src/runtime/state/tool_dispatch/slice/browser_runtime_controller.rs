use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

use crate::error::DaemonError;

use super::SliceScreenCommandOutput;

const DEFAULT_SOCKET_PATH: &str = "/opt/chariox-slice/private/browser-runtime-mcp.sock";
const DEFAULT_AUTH_FILE: &str = "/opt/chariox-slice/private/browser-runtime-mcp.auth";
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const DEFAULT_CALL_TIMEOUT_MS: u64 = 70_000;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 10_000;
const REQUEST_ID_COUNTER_START: u64 = 1;

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(REQUEST_ID_COUNTER_START);

pub(super) fn is_browser_runtime_mcp_tool(tool_name: &str) -> bool {
    matches!(
        crate::transport::runtime_tools::canonical_slice_tool_name(tool_name),
        Some(
            crate::transport::runtime_tools::SLICE_BROWSER_STATUS_TOOL
                | crate::transport::runtime_tools::SLICE_BROWSER_FIND_TOOL
                | crate::transport::runtime_tools::SLICE_BROWSER_FILL_TOOL
                | crate::transport::runtime_tools::SLICE_BROWSER_CLICK_TOOL
                | crate::transport::runtime_tools::SLICE_BROWSER_SUBMIT_TOOL
                | crate::transport::runtime_tools::SLICE_BROWSER_DIALOG_TOOL
                | crate::transport::runtime_tools::SLICE_BROWSER_TEXT_TOOL
                | crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_TEXT_TOOL
                | crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_SELECTOR_TOOL
                | crate::transport::runtime_tools::SLICE_BROWSER_WAIT_FOR_IDLE_TOOL
        )
    )
}

pub(super) async fn run_browser_runtime_mcp_call(
    tool_name: &str,
    arguments: Value,
) -> Result<SliceScreenCommandOutput, DaemonError> {
    let request_id = next_request_id();
    let auth_file = std::env::var("CHARIOX_BROWSER_RUNTIME_MCP_AUTH_FILE")
        .unwrap_or_else(|_| DEFAULT_AUTH_FILE.to_string());
    let auth_token = match std::fs::read_to_string(&auth_file) {
        Ok(token) => token.trim().to_string(),
        Err(_) => {
            return Ok(controller_failure(
                "browser controller authentication is unavailable",
            ));
        }
    };
    if auth_token.is_empty() {
        return Ok(controller_failure(
            "browser controller authentication is unavailable",
        ));
    }

    let request = serde_json::json!({
        "type": "tool_call",
        "request_id": request_id,
        "auth_token": auth_token,
        "tool_name": tool_name,
        "arguments": arguments,
    });
    let encoded = match serde_json::to_vec(&request) {
        Ok(encoded) if encoded.len() <= MAX_REQUEST_BYTES => encoded,
        Ok(_) => {
            return Ok(controller_failure(
                "browser controller request exceeds the byte limit",
            ));
        }
        Err(_) => {
            return Ok(controller_failure(
                "browser controller request could not be encoded",
            ));
        }
    };

    let socket_path = std::env::var("CHARIOX_BROWSER_RUNTIME_MCP_SOCKET")
        .unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string());
    let timeout_ms = browser_runtime_mcp_timeout_ms(&arguments);
    let result = timeout(
        Duration::from_millis(timeout_ms),
        exchange_with_controller(&socket_path, &encoded, &request_id),
    )
    .await;

    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(message)) => Ok(controller_failure(&message)),
        Err(_) => Ok(controller_failure(&format!(
            "browser controller request timed out after {timeout_ms}ms"
        ))),
    }
}

async fn exchange_with_controller(
    socket_path: &str,
    encoded_request: &[u8],
    request_id: &str,
) -> Result<SliceScreenCommandOutput, String> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|_| "browser controller endpoint is unavailable".to_string())?;
    stream
        .write_all(encoded_request)
        .await
        .map_err(|_| "browser controller request could not be sent".to_string())?;
    stream
        .write_all(b"\n")
        .await
        .map_err(|_| "browser controller request could not be sent".to_string())?;
    stream
        .flush()
        .await
        .map_err(|_| "browser controller request could not be sent".to_string())?;

    let response = read_bounded_line(&mut stream).await?;
    parse_controller_response(&response, request_id)
}

async fn read_bounded_line(stream: &mut UnixStream) -> Result<Vec<u8>, String> {
    let mut response = Vec::with_capacity(4096);
    let mut buffer = [0_u8; 8192];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|_| "browser controller response could not be read".to_string())?;
        if read == 0 {
            return Err("browser controller returned an incomplete response".to_string());
        }
        let newline = buffer[..read].iter().position(|byte| *byte == b'\n');
        let chunk_len = newline.map(|index| index + 1).unwrap_or(read);
        if response.len().saturating_add(chunk_len) > MAX_RESPONSE_BYTES {
            return Err("browser controller response exceeds the byte limit".to_string());
        }
        response.extend_from_slice(&buffer[..chunk_len]);
        if newline.is_some() {
            response.pop();
            return Ok(response);
        }
    }
}

fn parse_controller_response(
    response: &[u8],
    request_id: &str,
) -> Result<SliceScreenCommandOutput, String> {
    let value = serde_json::from_slice::<Value>(response)
        .map_err(|_| "browser controller returned malformed JSON".to_string())?;
    if value.get("type").and_then(Value::as_str) != Some("tool_result")
        || value.get("request_id").and_then(Value::as_str) != Some(request_id)
    {
        return Err("browser controller returned a mismatched response".to_string());
    }

    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        let code = value
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            .filter(|code| !code.is_empty())
            .unwrap_or("RUNTIME_MCP_ERROR");
        return Ok(controller_failure(&format!(
            "browser controller request failed: {code}"
        )));
    }

    let result = value
        .get("result")
        .ok_or_else(|| "browser controller response did not include a result".to_string())?;
    let stdout = match result {
        Value::String(value) => value.clone(),
        _ => serde_json::to_string(result)
            .map_err(|_| "browser controller result could not be encoded".to_string())?,
    };
    let semantic_success = result
        .as_object()
        .and_then(|object| object.get("ok"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    Ok(SliceScreenCommandOutput {
        success: semantic_success,
        status_code: Some(if semantic_success { 0 } else { 1 }),
        stdout,
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
    })
}

fn controller_failure(message: &str) -> SliceScreenCommandOutput {
    SliceScreenCommandOutput {
        success: false,
        status_code: Some(1),
        stdout: String::new(),
        stderr: message.to_string(),
        stdout_truncated: false,
        stderr_truncated: false,
    }
}

fn next_request_id() -> String {
    let sequence = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    format!("kernel-{}-{sequence}", std::process::id())
}

fn browser_runtime_mcp_timeout_ms(arguments: &Value) -> u64 {
    let requested_timeout = arguments
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
    let derived_timeout = requested_timeout.saturating_add(5_000);
    let configured_timeout = std::env::var("CHARIOX_BROWSER_RUNTIME_MCP_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value >= 100);
    configured_timeout
        .unwrap_or(derived_timeout)
        .clamp(100, DEFAULT_CALL_TIMEOUT_MS)
}
