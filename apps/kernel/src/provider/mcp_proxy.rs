#![allow(dead_code)]

mod stdio;
mod streamable_http;

use std::collections::BTreeMap;
#[cfg(test)]
use std::io::{Read, Write};

use crate::error::DaemonError;
use crate::mcp::{CharioxMcpServerConfig, CharioxMcpTransportConfig};

use stdio::stdio_mcp_supervisor;
use streamable_http::forward_streamable_http_mcp_request;

const PROXY_PATH_PREFIX: &str = "/mcp/proxy/";

pub(crate) fn provider_facing_mcp_proxy_config(
    backing: &CharioxMcpServerConfig,
    runtime_mcp_url: &str,
    runtime_mcp_auth_token: &str,
) -> Result<CharioxMcpServerConfig, DaemonError> {
    provider_facing_mcp_proxy_config_named(
        backing,
        &backing.name,
        runtime_mcp_url,
        runtime_mcp_auth_token,
    )
}

pub(crate) fn provider_facing_mcp_proxy_config_named(
    backing: &CharioxMcpServerConfig,
    provider_visible_name: &str,
    runtime_mcp_url: &str,
    runtime_mcp_auth_token: &str,
) -> Result<CharioxMcpServerConfig, DaemonError> {
    let proxy = CharioxMcpServerConfig {
        name: provider_visible_name.to_string(),
        transport: CharioxMcpTransportConfig::StreamableHttp {
            url: provider_facing_mcp_proxy_url(runtime_mcp_url, &backing.name)?,
            bearer_token_env_var: None,
            bearer_token_credential: None,
            http_headers: BTreeMap::from([(
                "Authorization".to_string(),
                format!("Bearer {runtime_mcp_auth_token}"),
            )]),
            credential_http_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
        },
        enabled: backing.enabled,
        required: backing.required,
        startup_timeout_sec: backing.startup_timeout_sec,
        tool_timeout_sec: backing.tool_timeout_sec,
        enabled_tools: backing.enabled_tools.clone(),
        disabled_tools: backing.disabled_tools.clone(),
        tools: backing.tools.clone(),
    };
    proxy.validate()?;
    Ok(proxy)
}

pub(crate) fn provider_facing_mcp_proxy_configs(
    backing_servers: &[CharioxMcpServerConfig],
    runtime_mcp_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
) -> Result<Vec<CharioxMcpServerConfig>, DaemonError> {
    let Some(runtime_mcp_url) = runtime_mcp_url else {
        return Ok(backing_servers.to_vec());
    };
    let Some(runtime_mcp_auth_token) = runtime_mcp_auth_token else {
        return Ok(backing_servers.to_vec());
    };
    backing_servers
        .iter()
        .map(|server| {
            provider_facing_mcp_proxy_config(server, runtime_mcp_url, runtime_mcp_auth_token)
        })
        .collect()
}

pub(crate) fn provider_facing_mcp_proxy_configs_with_bearer_env(
    backing_servers: &[CharioxMcpServerConfig],
    runtime_mcp_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
    bearer_token_env_var: &str,
) -> Result<Vec<CharioxMcpServerConfig>, DaemonError> {
    let mut servers = provider_facing_mcp_proxy_configs(
        backing_servers,
        runtime_mcp_url,
        runtime_mcp_auth_token,
    )?;
    if runtime_mcp_url.is_some() && runtime_mcp_auth_token.is_some() {
        for server in &mut servers {
            if let CharioxMcpTransportConfig::StreamableHttp {
                bearer_token_env_var: env_var,
                http_headers,
                ..
            } = &mut server.transport
            {
                http_headers.remove("Authorization");
                *env_var = Some(bearer_token_env_var.to_string());
            }
        }
    }
    Ok(servers)
}

pub(crate) fn provider_facing_mcp_proxy_url(
    runtime_mcp_url: &str,
    name: &str,
) -> Result<String, DaemonError> {
    crate::mcp::validate_registry_name(name, "mcp name")?;
    let base = runtime_mcp_url
        .trim_end_matches('/')
        .strip_suffix("/mcp")
        .unwrap_or_else(|| runtime_mcp_url.trim_end_matches('/'));
    Ok(format!("{base}{PROXY_PATH_PREFIX}{name}"))
}

pub(crate) fn dispatch_provider_mcp_proxy_request(
    owner_provider_run_id: &str,
    owner_session_id: &str,
    backing: &CharioxMcpServerConfig,
    payload: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    let is_tools_list = payload
        .get("method")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|method| method == "tools/list");
    let mut response = match &backing.transport {
        CharioxMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            bearer_token_credential: _,
            http_headers,
            credential_http_headers: _,
            env_http_headers,
        } => forward_streamable_http_mcp_request(
            url,
            bearer_token_env_var.as_deref(),
            http_headers,
            env_http_headers,
            payload,
            backing.tool_timeout_sec,
        ),
        CharioxMcpTransportConfig::Stdio { .. } => {
            let key = backing.definition_hash()?;
            let process = stdio_mcp_supervisor()
                .lock()
                .expect("stdio MCP supervisor mutex poisoned")
                .process(&key, owner_provider_run_id, owner_session_id, backing)?;
            let response = process
                .lock()
                .expect("stdio MCP process mutex poisoned")
                .dispatch(payload);
            if response.is_err() {
                let discarded = stdio_mcp_supervisor()
                    .lock()
                    .expect("stdio MCP supervisor mutex poisoned")
                    .discard_process(&key, &process);
                drop(discarded);
            }
            response
        }
    }?;
    if is_tools_list {
        mark_provider_proxy_tools_preapproved(&mut response);
    }
    Ok(response)
}

pub(crate) fn shutdown_provider_mcp_proxy_session(session_id: &str) {
    let released_processes = stdio_mcp_supervisor()
        .lock()
        .expect("stdio MCP supervisor mutex poisoned")
        .release_session(session_id);
    let released_process_count = released_processes.len();
    drop(released_processes);
    if released_process_count > 0 {
        crate::logging::info_with_fields(
            "daemon.provider.mcp_proxy",
            "released final session stdio MCP proxy process ownership",
            serde_json::json!({
                "session_id": session_id,
                "released_process_count": released_process_count,
            }),
        );
    }
}

pub(crate) fn shutdown_provider_mcp_proxy_run(provider_run_id: &str) {
    let released_processes = stdio_mcp_supervisor()
        .lock()
        .expect("stdio MCP supervisor mutex poisoned")
        .release_run(provider_run_id);
    let released_process_count = released_processes.len();
    drop(released_processes);
    if released_process_count > 0 {
        crate::logging::info_with_fields(
            "daemon.provider.mcp_proxy",
            "released final stdio MCP proxy process ownership",
            serde_json::json!({
                "provider_run_id": provider_run_id,
                "released_process_count": released_process_count,
            }),
        );
    }
}

fn mark_provider_proxy_tools_preapproved(response: &mut serde_json::Value) {
    let Some(tools) = response
        .get_mut("result")
        .and_then(|result| result.get_mut("tools"))
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for tool in tools {
        let Some(tool) = tool.as_object_mut() else {
            continue;
        };
        let annotations = tool
            .entry("annotations".to_string())
            .or_insert_with(|| serde_json::json!({}));
        let Some(annotations) = annotations.as_object_mut() else {
            continue;
        };
        annotations.insert("destructiveHint".to_string(), serde_json::json!(false));
        annotations.insert("openWorldHint".to_string(), serde_json::json!(false));
        annotations.insert("readOnlyHint".to_string(), serde_json::json!(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::ffi::OsString;
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    struct EnvironmentRestore(Vec<(&'static str, Option<OsString>)>);

    impl EnvironmentRestore {
        fn capture(names: &[&'static str]) -> Self {
            Self(
                names
                    .iter()
                    .map(|name| (*name, std::env::var_os(name)))
                    .collect(),
            )
        }
    }

    impl Drop for EnvironmentRestore {
        fn drop(&mut self) {
            for (name, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

    #[test]
    fn proxy_config_preserves_policy_but_replaces_transport() {
        let mut backing =
            CharioxMcpServerConfig::stdio("browser", "npx", vec!["@playwright/mcp@latest".into()]);
        backing.required = true;
        backing.startup_timeout_sec = Some(7);
        backing.tool_timeout_sec = Some(30);
        backing.enabled_tools = Some(vec!["browser_snapshot".into()]);

        let proxy =
            provider_facing_mcp_proxy_config(&backing, "http://127.0.0.1:43120/mcp", "token-123")
                .expect("proxy config should build");

        assert_eq!(proxy.name, "browser");
        assert!(proxy.required);
        assert_eq!(proxy.startup_timeout_sec, Some(7));
        assert_eq!(proxy.tool_timeout_sec, Some(30));
        assert_eq!(proxy.enabled_tools, Some(vec!["browser_snapshot".into()]));
        match proxy.transport {
            CharioxMcpTransportConfig::StreamableHttp {
                url, http_headers, ..
            } => {
                assert_eq!(url, "http://127.0.0.1:43120/mcp/proxy/browser");
                assert_eq!(
                    http_headers.get("Authorization").map(String::as_str),
                    Some("Bearer token-123")
                );
            }
            other => panic!("expected streamable HTTP proxy, got {other:?}"),
        }
    }

    #[test]
    fn proxy_config_can_use_provider_visible_alias() {
        let backing = CharioxMcpServerConfig::stdio("node_repl", "node", Vec::new());

        let proxy = provider_facing_mcp_proxy_config_named(
            &backing,
            "chariox_mcp_node_repl",
            "http://127.0.0.1:43120/mcp",
            "token-123",
        )
        .expect("proxy config should build");

        assert_eq!(proxy.name, "chariox_mcp_node_repl");
        match proxy.transport {
            CharioxMcpTransportConfig::StreamableHttp { url, .. } => {
                assert_eq!(url, "http://127.0.0.1:43120/mcp/proxy/node_repl");
            }
            other => panic!("expected streamable HTTP proxy, got {other:?}"),
        }
    }

    #[test]
    fn proxy_config_falls_back_when_runtime_binding_is_missing() {
        let backing = CharioxMcpServerConfig::stdio("github", "github-mcp-server", Vec::new());
        let rendered = provider_facing_mcp_proxy_configs(&[backing.clone()], None, Some("token"))
            .expect("fallback should succeed");

        assert_eq!(rendered, vec![backing]);
    }

    #[test]
    fn streamable_http_proxy_forwards_json_rpc_and_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener.local_addr().expect("listener address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("proxy should connect");
            let request = read_http_request(&mut stream);
            assert!(request.starts_with("POST /mcp HTTP/1.1\r\n"));
            assert!(request.contains("Authorization: Bearer secret-token\r\n"));
            assert!(request.contains("X-Test: yes\r\n"));
            assert!(request.contains(r#""method":"tools/list""#));

            let body = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("response should write");
        });

        let config = CharioxMcpServerConfig {
            name: "browser".to_string(),
            transport: CharioxMcpTransportConfig::StreamableHttp {
                url: format!("http://{address}/mcp"),
                bearer_token_env_var: None,
                bearer_token_credential: None,
                http_headers: BTreeMap::from([
                    (
                        "Authorization".to_string(),
                        "Bearer secret-token".to_string(),
                    ),
                    ("X-Test".to_string(), "yes".to_string()),
                ]),
                credential_http_headers: BTreeMap::new(),
                env_http_headers: BTreeMap::new(),
            },
            enabled: true,
            required: false,
            startup_timeout_sec: None,
            tool_timeout_sec: Some(2),
            enabled_tools: None,
            disabled_tools: None,
            tools: BTreeMap::new(),
        };

        let response = dispatch_provider_mcp_proxy_request(
            "provider-run-http-test",
            "session-http-test",
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .expect("proxy should forward request");
        assert_eq!(
            response,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}})
        );
        server.join().expect("test server should finish");
    }

    #[test]
    fn streamable_http_proxy_uses_configured_http_proxy_for_external_mcp() {
        let _environment_lock = crate::env_lock::lock();
        let _environment_restore = EnvironmentRestore::capture(&[
            "ALL_PROXY",
            "all_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
        ]);
        let listener = TcpListener::bind("127.0.0.1:0").expect("HTTP proxy should bind");
        let address = listener.local_addr().expect("HTTP proxy address");
        listener
            .set_nonblocking(true)
            .expect("HTTP proxy should become nonblocking");
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .expect("proxy read timeout");
                        let request = read_http_request(&mut stream);
                        assert!(request
                            .starts_with("POST http://unresolvable.invalid/mcp HTTP/1.1\r\n"));
                        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"proxied":true}}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .expect("proxy response should write");
                        return;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            panic!("MCP request did not use the configured HTTP proxy");
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("HTTP proxy accept failed: {error}"),
                }
            }
        });

        for name in [
            "ALL_PROXY",
            "all_proxy",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
        ] {
            std::env::remove_var(name);
        }
        std::env::set_var("HTTP_PROXY", format!("http://{address}"));

        let config = CharioxMcpServerConfig::streamable_http(
            "external",
            "http://unresolvable.invalid/mcp".to_string(),
        );
        let response = dispatch_provider_mcp_proxy_request(
            "provider-run-http-proxy-test",
            "session-http-proxy-test",
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .expect("external MCP request should use the configured HTTP proxy");
        assert_eq!(
            response,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"proxied": true}})
        );
        server.join().expect("HTTP proxy should finish");
    }

    #[test]
    fn streamable_http_proxy_respects_no_proxy_for_external_mcp() {
        let _environment_lock = crate::env_lock::lock();
        let _environment_restore = EnvironmentRestore::capture(&[
            "ALL_PROXY",
            "all_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
        ]);
        for name in [
            "ALL_PROXY",
            "all_proxy",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "no_proxy",
        ] {
            std::env::remove_var(name);
        }
        std::env::set_var("HTTP_PROXY", "not-a-valid-proxy://secret@example.invalid");
        std::env::set_var("NO_PROXY", ".example.invalid");

        let proxy = streamable_http::configured_proxy_for_url("http://mcp.example.invalid/api")
            .expect("NO_PROXY should bypass malformed proxy configuration");
        assert!(proxy.is_none());
    }

    #[test]
    fn streamable_https_proxy_prefers_https_proxy_configuration() {
        let _environment_lock = crate::env_lock::lock();
        let _environment_restore = EnvironmentRestore::capture(&[
            "ALL_PROXY",
            "all_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
        ]);
        for name in [
            "ALL_PROXY",
            "all_proxy",
            "http_proxy",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
        ] {
            std::env::remove_var(name);
        }
        std::env::set_var("HTTP_PROXY", "not-a-valid-proxy://secret@example.invalid");
        std::env::set_var("HTTPS_PROXY", "http://127.0.0.1:3128");

        let proxy = streamable_http::configured_proxy_for_url("https://mcp.example.invalid/api")
            .expect("HTTPS_PROXY should take precedence over HTTP_PROXY");
        assert!(proxy.is_some());
    }

    #[test]
    fn streamable_http_proxy_rejects_malformed_proxy_configuration() {
        let _environment_lock = crate::env_lock::lock();
        let _environment_restore = EnvironmentRestore::capture(&[
            "ALL_PROXY",
            "all_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
        ]);
        for name in [
            "ALL_PROXY",
            "all_proxy",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "NO_PROXY",
            "no_proxy",
        ] {
            std::env::remove_var(name);
        }
        std::env::set_var("HTTP_PROXY", "not-a-valid-proxy://secret@example.invalid");

        let error = streamable_http::configured_proxy_for_url("http://mcp.example.invalid/api")
            .expect_err("malformed proxy configuration must fail closed");
        match error {
            DaemonError::LocalTransport { operation, message } => {
                assert_eq!(operation, "mcp.proxy.http.proxy");
                assert!(message.contains("HTTP_PROXY"));
                assert!(!message.contains("secret"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn streamable_http_proxy_decodes_chunked_json_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener.local_addr().expect("listener address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("proxy should connect");
            let request = read_http_request(&mut stream);
            assert!(request.starts_with("POST /mcp HTTP/1.1\r\n"));
            let first = r#"{"jsonrpc":"2.0","id":1,"result":{"chunked":"#;
            let second = r#""ok"}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n{}\r\n{:X}\r\n{}\r\n0\r\n\r\n",
                first.len(),
                first,
                second.len(),
                second,
            )
            .expect("chunked response should write");
        });

        let config =
            CharioxMcpServerConfig::streamable_http("chunked", format!("http://{address}/mcp"));
        let response = dispatch_provider_mcp_proxy_request(
            "provider-run-chunked-test",
            "session-chunked-test",
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .expect("proxy should decode chunked response");
        assert_eq!(
            response,
            json!({"jsonrpc": "2.0", "id": 1, "result": {"chunked": "ok"}})
        );
        server.join().expect("test server should finish");
    }

    #[test]
    fn stdio_proxy_reuses_backing_process_and_caches_initialize() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        stdio_mcp_supervisor()
            .lock()
            .expect("test supervisor lock")
            .stop_all();
        let start_file = std::env::temp_dir().join(format!(
            "chariox-stdio-mcp-proxy-starts-{}-{}.txt",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let pid_file = start_file.with_extension("pid");
        let script = r#"
import fs from 'node:fs'
const startFile = process.env.CHARIOX_TEST_START_FILE
const pidFile = process.env.CHARIOX_TEST_PID_FILE
const current = Number(fs.existsSync(startFile) ? fs.readFileSync(startFile, 'utf8') : '0')
fs.writeFileSync(startFile, String(current + 1))
fs.writeFileSync(pidFile, String(process.pid))
let buffer = Buffer.alloc(0)
function write(message) {
  const body = JSON.stringify(message)
  process.stdout.write(`${body}\n`)
}
process.stdin.on('data', (chunk) => {
  buffer = Buffer.concat([buffer, chunk])
  while (true) {
    const newline = buffer.indexOf('\n')
    if (newline >= 0) {
      const line = buffer.subarray(0, newline).toString('utf8').trim()
      buffer = buffer.subarray(newline + 1)
      if (line) handle(JSON.parse(line))
      continue
    }
    const headerEnd = buffer.indexOf('\r\n\r\n')
    if (headerEnd < 0) return
    const header = buffer.subarray(0, headerEnd).toString('utf8')
    const match = /^content-length:\s*(\d+)$/im.exec(header)
    if (!match) throw new Error(`missing Content-Length: ${header}`)
    const length = Number(match[1])
    const bodyStart = headerEnd + 4
    const frameEnd = bodyStart + length
    if (buffer.length < frameEnd) return
    const message = JSON.parse(buffer.subarray(bodyStart, frameEnd).toString('utf8'))
    buffer = buffer.subarray(frameEnd)
    handle(message)
  }
})
function handle(message) {
  if (message.method === 'notifications/initialized') return
  if (message.method === 'initialize') {
    write({ jsonrpc: '2.0', id: message.id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'stdio-test', version: '1' } } })
    return
  }
  if (message.method === 'tools/list') {
    write({ jsonrpc: '2.0', id: message.id, result: { tools: [] } })
    return
  }
  write({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: `unknown ${message.method}` } })
}
"#;
        let mut config = CharioxMcpServerConfig::stdio(
            format!("stdio-test-{}", crate::session::unix_epoch_ms()),
            "node",
            vec![
                "--input-type=module".to_string(),
                "-e".to_string(),
                script.to_string(),
            ],
        );
        if let CharioxMcpTransportConfig::Stdio { env, .. } = &mut config.transport {
            env.insert(
                "CHARIOX_TEST_START_FILE".to_string(),
                start_file.to_string_lossy().to_string(),
            );
            env.insert(
                "CHARIOX_TEST_PID_FILE".to_string(),
                pid_file.to_string_lossy().to_string(),
            );
        }

        let initialize = dispatch_provider_mcp_proxy_request(
            "provider-run-a",
            "session-a",
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}),
        )
        .expect("initialize should forward");
        assert_eq!(
            initialize
                .pointer("/result/serverInfo/name")
                .and_then(serde_json::Value::as_str),
            Some("stdio-test")
        );
        dispatch_provider_mcp_proxy_request(
            "provider-run-a",
            "session-a",
            &config,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .expect("initialized notification should be accepted");
        dispatch_provider_mcp_proxy_request(
            "provider-run-a",
            "session-a",
            &config,
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .expect("tools/list should forward");
        let cached_initialize = dispatch_provider_mcp_proxy_request(
            "provider-run-b",
            "session-b",
            &config,
            json!({"jsonrpc": "2.0", "id": 3, "method": "initialize"}),
        )
        .expect("second initialize should use cached response");
        assert_eq!(cached_initialize.get("id"), Some(&json!(3)));
        let starts = std::fs::read_to_string(&start_file).expect("start file should exist");
        assert_eq!(starts, "1");
        let _child_pid = std::fs::read_to_string(&pid_file)
            .expect("PID file should exist")
            .parse::<u32>()
            .expect("PID should parse");
        shutdown_provider_mcp_proxy_run("provider-run-a");
        assert_eq!(
            stdio_mcp_supervisor()
                .lock()
                .expect("test supervisor lock")
                .process_count(),
            1,
            "a shared MCP process must survive until its last provider run ends"
        );
        #[cfg(unix)]
        assert!(crate::runtime::process_health::process_running(_child_pid));
        shutdown_provider_mcp_proxy_session("session-b");
        assert_eq!(
            stdio_mcp_supervisor()
                .lock()
                .expect("test supervisor lock")
                .process_count(),
            0,
            "the MCP process must be released after its last provider run ends"
        );
        #[cfg(unix)]
        assert!(
            !crate::runtime::process_health::process_running(_child_pid),
            "the released MCP child must be killed and reaped"
        );
        let _ = std::fs::remove_file(start_file);
        let _ = std::fs::remove_file(pid_file);
    }

    #[test]
    fn tools_list_proxy_marks_granted_tools_as_preapproved() {
        let mut response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "tools": [
                    {
                        "name": "browser_snapshot",
                        "annotations": {
                            "destructiveHint": true,
                            "openWorldHint": true
                        }
                    }
                ]
            }
        });

        mark_provider_proxy_tools_preapproved(&mut response);

        let annotations = &response["result"]["tools"][0]["annotations"];
        assert_eq!(annotations["destructiveHint"], json!(false));
        assert_eq!(annotations["openWorldHint"], json!(false));
        assert_eq!(annotations["readOnlyHint"], json!(true));
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 512];
        loop {
            let read = stream.read(&mut chunk).expect("request should read");
            assert_ne!(read, 0, "client closed before full request");
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&buffer[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                let total = header_end + 4 + content_length;
                if buffer.len() >= total {
                    return String::from_utf8(buffer).expect("request should be utf8");
                }
            }
        }
    }
}
