use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::DaemonError;

pub(super) fn forward_streamable_http_mcp_request(
    url: &str,
    bearer_token_env_var: Option<&str>,
    http_headers: &BTreeMap<String, String>,
    env_http_headers: &BTreeMap<String, String>,
    payload: serde_json::Value,
    timeout_sec: Option<u64>,
) -> Result<serde_json::Value, DaemonError> {
    let body = serde_json::to_vec(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.proxy.http.serialize",
        message: error.to_string(),
    })?;
    let timeout = Duration::from_secs(timeout_sec.unwrap_or(30).max(1));
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let mut request = agent
        .post(url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json");
    for (key, value) in http_headers {
        if !reserved_header(key) {
            request = request.set(key, value);
        }
    }
    for (key, env_var) in env_http_headers {
        if reserved_header(key) {
            continue;
        }
        if let Ok(value) = std::env::var(env_var) {
            request = request.set(key, &value);
        }
    }
    if let Some(env_var) = bearer_token_env_var {
        if let Ok(value) = std::env::var(env_var) {
            request = request.set("Authorization", &format!("Bearer {value}"));
        }
    }

    let response = match request.send_bytes(&body) {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let body = response
                .into_string()
                .unwrap_or_else(|error| format!("<failed to read response body: {error}>"));
            return Err(DaemonError::LocalTransport {
                operation: "mcp.proxy.http.response",
                message: format!("upstream `{url}` returned HTTP {status}: {body}"),
            });
        }
        Err(error) => {
            return Err(DaemonError::LocalTransport {
                operation: "mcp.proxy.http.request",
                message: format!("failed to forward MCP HTTP request to `{url}`: {error}"),
            });
        }
    };
    let text = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.proxy.http.read",
            message: format!("failed to read MCP HTTP response from `{url}`: {error}"),
        })?;
    serde_json::from_str(&text).map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.proxy.http.parse",
        message: format!("upstream `{url}` returned invalid JSON: {error}"),
    })
}

fn reserved_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "connection" | "content-type" | "accept"
    )
}
