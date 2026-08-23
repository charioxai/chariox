//! OpenCode MCP server endpoint operations.

use std::time::{Duration, Instant};

use crate::error::DaemonError;

use super::OpenCodeClient;

impl OpenCodeClient {
    pub fn add_mcp_server(&self, name: &str, config: serde_json::Value) -> Result<(), DaemonError> {
        let _: serde_json::Value = self.send_json_request(
            "POST",
            "/mcp",
            Some(&serde_json::json!({ "name": name, "config": config })),
        )?;
        Ok(())
    }

    pub fn connect_mcp_server(&self, name: &str) -> Result<(), DaemonError> {
        let _: bool = self.send_json_request("POST", &format!("/mcp/{name}/connect"), None)?;
        Ok(())
    }

    pub fn connect_mcp_server_with_retry(
        &self,
        name: &str,
        timeout: Duration,
        retry_interval: Duration,
    ) -> Result<(), DaemonError> {
        let deadline = Instant::now() + timeout;
        let mut last_error = None;

        loop {
            match self.connect_mcp_server(name) {
                Ok(()) => return Ok(()),
                Err(error) if Instant::now() < deadline => {
                    last_error = Some(error);
                    std::thread::sleep(retry_interval);
                }
                Err(error) => return Err(last_error.unwrap_or(error)),
            }
        }
    }

    pub fn wait_until_mcp_server_connected(
        &self,
        name: &str,
        timeout: Duration,
        retry_interval: Duration,
    ) -> Result<(), DaemonError> {
        let deadline = Instant::now() + timeout;

        loop {
            let statuses: serde_json::Value = self.send_json_request("GET", "/mcp", None)?;
            let status = statuses
                .get(name)
                .and_then(|entry| entry.get("status"))
                .and_then(serde_json::Value::as_str);
            match status {
                Some("connected") => return Ok(()),
                Some(state @ ("failed" | "needs_auth" | "needs_client_registration")) => {
                    let detail = statuses
                        .get(name)
                        .and_then(|entry| entry.get("error"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(state);
                    return Err(self.protocol_error(
                        "opencode_mcp_ready",
                        format!("OpenCode MCP server `{name}` is {state}: {detail}"),
                    ));
                }
                _ if Instant::now() < deadline => std::thread::sleep(retry_interval),
                _ => {
                    return Err(self.protocol_error(
                        "opencode_mcp_ready",
                        format!(
                            "timed out waiting for OpenCode MCP server `{name}` to become connected"
                        ),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    use super::OpenCodeClient;

    #[test]
    fn waits_for_mcp_status_to_be_connected() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener should bind");
        let port = listener
            .local_addr()
            .expect("test listener should expose an address")
            .port();
        let server = thread::spawn(move || {
            for body in [
                r#"{"chariox":{"status":"disabled"}}"#,
                r#"{"chariox":{"status":"connected"}}"#,
            ] {
                let (mut stream, _) = listener.accept().expect("client should connect");
                let mut request = [0_u8; 1024];
                let size = stream.read(&mut request).expect("request should read");
                assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET /mcp "));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("server should write response");
            }
        });

        let client = OpenCodeClient::new("provider-run-1", format!("http://127.0.0.1:{port}"))
            .expect("client should initialize");
        client
            .wait_until_mcp_server_connected("chariox", Duration::from_secs(1), Duration::ZERO)
            .expect("client should wait for connected status");
        server.join().expect("server thread should join");
    }
}
