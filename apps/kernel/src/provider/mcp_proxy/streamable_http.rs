use std::collections::BTreeMap;
use std::net::IpAddr;
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
    let agent = streamable_http_agent(url, timeout)?;
    let mut request = agent
        .post(url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json, text/event-stream");
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

fn streamable_http_agent(url: &str, timeout: Duration) -> Result<ureq::Agent, DaemonError> {
    let mut builder = ureq::AgentBuilder::new().timeout(timeout);
    if let Some(proxy) = configured_proxy_for_url(url)? {
        builder = builder.proxy(proxy);
    }
    Ok(builder.build())
}

pub(super) fn configured_proxy_for_url(url: &str) -> Result<Option<ureq::Proxy>, DaemonError> {
    let parsed = url::Url::parse(url).map_err(|error| DaemonError::LocalTransport {
        operation: "mcp.proxy.http.proxy",
        message: format!("failed to parse MCP HTTP URL: {error}"),
    })?;
    let host = parsed
        .host_str()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "mcp.proxy.http.proxy",
            message: "MCP HTTP URL does not contain a host".to_string(),
        })?;
    let host = normalized_proxy_host(host);
    let port = parsed.port_or_known_default();
    if is_loopback_host(host) || no_proxy_matches(host, port) {
        return Ok(None);
    }

    let proxy_env_names: &[&str] = match parsed.scheme() {
        "https" => &[
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ],
        "http" => &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"],
        _ => return Ok(None),
    };
    for name in proxy_env_names {
        let Ok(value) = std::env::var(name) else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        let proxy = ureq::Proxy::new(&value).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.proxy.http.proxy",
            message: format!("MCP HTTP proxy configured by {name} is invalid: {error}"),
        })?;
        return Ok(Some(proxy));
    }
    Ok(None)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn normalized_proxy_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
        .trim_end_matches('.')
}

fn no_proxy_matches(host: &str, port: Option<u16>) -> bool {
    let no_proxy = std::env::var("NO_PROXY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var("no_proxy").ok());
    let Some(no_proxy) = no_proxy else {
        return false;
    };
    no_proxy.split(',').any(|entry| {
        let entry = entry.trim();
        if entry == "*" {
            return true;
        }
        let (entry_host, entry_port) = split_no_proxy_entry(entry);
        if entry_host.is_empty() || entry_port.is_some_and(|entry_port| Some(entry_port) != port) {
            return false;
        }
        let entry_host = normalized_proxy_host(entry_host.trim_start_matches('.'));
        host.eq_ignore_ascii_case(entry_host)
            || host
                .to_ascii_lowercase()
                .ends_with(&format!(".{}", entry_host.to_ascii_lowercase()))
    })
}

fn split_no_proxy_entry(entry: &str) -> (&str, Option<u16>) {
    if let Some(rest) = entry.strip_prefix('[') {
        if let Some((host, suffix)) = rest.split_once(']') {
            return (
                host,
                suffix.strip_prefix(':').and_then(|port| port.parse().ok()),
            );
        }
    }
    if entry.matches(':').count() == 1 {
        if let Some((host, port)) = entry.rsplit_once(':') {
            if let Ok(port) = port.parse() {
                return (host, Some(port));
            }
        }
    }
    (entry, None)
}

fn reserved_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "connection" | "content-type" | "accept"
    )
}
