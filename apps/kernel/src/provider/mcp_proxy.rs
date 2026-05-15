#![allow(dead_code)]

mod streamable_http;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};

use streamable_http::forward_streamable_http_mcp_request;

const PROXY_PATH_PREFIX: &str = "/mcp/proxy/";

pub(crate) fn provider_facing_mcp_proxy_config(
    backing: &ArrobaMcpServerConfig,
    runtime_mcp_url: &str,
    runtime_mcp_auth_token: &str,
) -> Result<ArrobaMcpServerConfig, DaemonError> {
    let proxy = ArrobaMcpServerConfig {
        name: backing.name.clone(),
        transport: ArrobaMcpTransportConfig::StreamableHttp {
            url: provider_facing_mcp_proxy_url(runtime_mcp_url, &backing.name)?,
            bearer_token_env_var: None,
            http_headers: BTreeMap::from([(
                "Authorization".to_string(),
                format!("Bearer {runtime_mcp_auth_token}"),
            )]),
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
    backing_servers: &[ArrobaMcpServerConfig],
    runtime_mcp_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
) -> Result<Vec<ArrobaMcpServerConfig>, DaemonError> {
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
    backing_servers: &[ArrobaMcpServerConfig],
    runtime_mcp_url: Option<&str>,
    runtime_mcp_auth_token: Option<&str>,
    bearer_token_env_var: &str,
) -> Result<Vec<ArrobaMcpServerConfig>, DaemonError> {
    let mut servers = provider_facing_mcp_proxy_configs(
        backing_servers,
        runtime_mcp_url,
        runtime_mcp_auth_token,
    )?;
    if runtime_mcp_url.is_some() && runtime_mcp_auth_token.is_some() {
        for server in &mut servers {
            if let ArrobaMcpTransportConfig::StreamableHttp {
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
    backing: &ArrobaMcpServerConfig,
    payload: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    let is_tools_list = payload
        .get("method")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|method| method == "tools/list");
    let mut response = match &backing.transport {
        ArrobaMcpTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => forward_streamable_http_mcp_request(
            url,
            bearer_token_env_var.as_deref(),
            http_headers,
            env_http_headers,
            payload,
            backing.tool_timeout_sec,
        ),
        ArrobaMcpTransportConfig::Stdio { .. } => {
            let process = stdio_mcp_supervisor()
                .lock()
                .expect("stdio MCP supervisor mutex poisoned")
                .process(backing)?;
            let response = process
                .lock()
                .expect("stdio MCP process mutex poisoned")
                .dispatch(payload);
            response
        }
    }?;
    if is_tools_list {
        mark_provider_proxy_tools_preapproved(&mut response);
    }
    Ok(response)
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

fn stdio_mcp_supervisor() -> &'static Mutex<StdioMcpSupervisor> {
    static SUPERVISOR: OnceLock<Mutex<StdioMcpSupervisor>> = OnceLock::new();
    SUPERVISOR.get_or_init(|| Mutex::new(StdioMcpSupervisor::default()))
}

#[derive(Default)]
struct StdioMcpSupervisor {
    processes: HashMap<String, Arc<Mutex<StdioMcpProcess>>>,
}

impl StdioMcpSupervisor {
    fn process(
        &mut self,
        backing: &ArrobaMcpServerConfig,
    ) -> Result<Arc<Mutex<StdioMcpProcess>>, DaemonError> {
        let key = backing.definition_hash()?;
        if self
            .processes
            .get(&key)
            .map(|process| {
                process
                    .lock()
                    .expect("stdio MCP process mutex poisoned")
                    .is_exited()
            })
            .transpose()?
            .unwrap_or(false)
        {
            self.processes.remove(&key);
        }
        if !self.processes.contains_key(&key) {
            let process = StdioMcpProcess::spawn(backing)?;
            self.processes
                .insert(key.clone(), Arc::new(Mutex::new(process)));
        }
        self.processes
            .get(&key)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.supervisor",
                message: format!("failed to start stdio MCP `{}`", backing.name),
            })
    }

    #[cfg(test)]
    fn stop_all(&mut self) {
        self.processes.clear();
    }
}

struct StdioMcpProcess {
    name: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    initialize_response: Option<serde_json::Value>,
    initialized_notified: bool,
}

impl StdioMcpProcess {
    fn spawn(backing: &ArrobaMcpServerConfig) -> Result<Self, DaemonError> {
        let ArrobaMcpTransportConfig::Stdio {
            command,
            args,
            env,
            env_vars,
            cwd,
        } = &backing.transport
        else {
            return Err(DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.spawn",
                message: format!("MCP `{}` is not a stdio MCP", backing.name),
            });
        };
        let mut command_builder = Command::new(command);
        command_builder
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(cwd) = cwd {
            command_builder.current_dir(cwd);
        }
        for (key, value) in env {
            command_builder.env(key, value);
        }
        for key in env_vars {
            if let Ok(value) = std::env::var(key) {
                command_builder.env(key, value);
            }
        }
        let mut child = command_builder
            .spawn()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.spawn",
                message: format!("failed to spawn stdio MCP `{}`: {error}", backing.name),
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.spawn",
                message: format!("stdio MCP `{}` did not expose stdin", backing.name),
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.spawn",
                message: format!("stdio MCP `{}` did not expose stdout", backing.name),
            })?;
        Ok(Self {
            name: backing.name.clone(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            initialize_response: None,
            initialized_notified: false,
        })
    }

    fn is_exited(&mut self) -> Result<bool, DaemonError> {
        self.child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.liveness",
                message: format!("failed to poll stdio MCP `{}`: {error}", self.name),
            })
    }

    fn dispatch(&mut self, payload: serde_json::Value) -> Result<serde_json::Value, DaemonError> {
        if self.is_exited()? {
            return Err(DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.exited",
                message: format!("stdio MCP `{}` exited", self.name),
            });
        }
        let method = payload.get("method").and_then(serde_json::Value::as_str);
        if method == Some("initialize") {
            if let Some(response) = &self.initialize_response {
                let mut response = response.clone();
                if let Some(id) = payload.get("id").cloned() {
                    response["id"] = id;
                }
                return Ok(response);
            }
            let response = self.send_request(payload)?;
            self.initialize_response = Some(response.clone());
            return Ok(response);
        }
        if method == Some("notifications/initialized") {
            if !self.initialized_notified {
                self.write_frame(&payload)?;
                self.initialized_notified = true;
            }
            return Ok(serde_json::json!({"jsonrpc": "2.0", "result": null}));
        }
        if payload.get("id").is_none() {
            self.write_frame(&payload)?;
            return Ok(serde_json::json!({"jsonrpc": "2.0", "result": null}));
        }
        self.send_request(payload)
    }

    fn send_request(
        &mut self,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        self.write_frame(&payload)?;
        self.read_frame()
    }

    fn write_frame(&mut self, payload: &serde_json::Value) -> Result<(), DaemonError> {
        let body = serde_json::to_vec(payload).map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.proxy.stdio.serialize",
            message: error.to_string(),
        })?;
        self.stdin
            .write_all(&body)
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.write",
                message: format!("failed to write stdio MCP `{}` frame: {error}", self.name),
            })
    }

    fn read_frame(&mut self) -> Result<serde_json::Value, DaemonError> {
        loop {
            let mut first_line = String::new();
            let read = self.stdout.read_line(&mut first_line).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "mcp.proxy.stdio.read",
                    message: format!("failed to read stdio MCP `{}` frame: {error}", self.name),
                }
            })?;
            if read == 0 {
                return Err(DaemonError::LocalTransport {
                    operation: "mcp.proxy.stdio.read",
                    message: format!("stdio MCP `{}` closed stdout", self.name),
                });
            }
            let first_line_trimmed = first_line.trim();
            if first_line_trimmed.is_empty() {
                continue;
            }
            if first_line_trimmed.starts_with('{') {
                return serde_json::from_str(first_line_trimmed).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "mcp.proxy.stdio.parse",
                        message: format!(
                            "stdio MCP `{}` returned invalid JSON line: {error}",
                            self.name
                        ),
                    }
                });
            }

            let mut content_length = parse_content_length_header(first_line_trimmed);
            loop {
                let mut line = String::new();
                self.stdout
                    .read_line(&mut line)
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "mcp.proxy.stdio.read",
                        message: format!(
                            "failed to read stdio MCP `{}` headers: {error}",
                            self.name
                        ),
                    })?;
                let line = line.trim();
                if line.is_empty() {
                    break;
                }
                if content_length.is_none() {
                    content_length = parse_content_length_header(line);
                }
            }
            let Some(content_length) = content_length else {
                return Err(DaemonError::LocalTransport {
                    operation: "mcp.proxy.stdio.frame",
                    message: format!("stdio MCP `{}` response missing Content-Length", self.name),
                });
            };
            let mut body = vec![0_u8; content_length];
            self.stdout
                .read_exact(&mut body)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "mcp.proxy.stdio.read",
                    message: format!("failed to read stdio MCP `{}` body: {error}", self.name),
                })?;
            return serde_json::from_slice(&body).map_err(|error| DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.parse",
                message: format!("stdio MCP `{}` returned invalid JSON: {error}", self.name),
            });
        }
    }
}

impl Drop for StdioMcpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse_content_length_header(line: &str) -> Option<usize> {
    let (name, value) = line.split_once(':')?;
    name.eq_ignore_ascii_case("content-length")
        .then(|| value.trim().parse::<usize>().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn proxy_config_preserves_policy_but_replaces_transport() {
        let mut backing =
            ArrobaMcpServerConfig::stdio("browser", "npx", vec!["@playwright/mcp@latest".into()]);
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
            ArrobaMcpTransportConfig::StreamableHttp {
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
    fn proxy_config_falls_back_when_runtime_binding_is_missing() {
        let backing = ArrobaMcpServerConfig::stdio("github", "github-mcp-server", Vec::new());
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

        let config = ArrobaMcpServerConfig {
            name: "browser".to_string(),
            transport: ArrobaMcpTransportConfig::StreamableHttp {
                url: format!("http://{address}/mcp"),
                bearer_token_env_var: None,
                http_headers: BTreeMap::from([
                    (
                        "Authorization".to_string(),
                        "Bearer secret-token".to_string(),
                    ),
                    ("X-Test".to_string(), "yes".to_string()),
                ]),
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
            ArrobaMcpServerConfig::streamable_http("chunked", format!("http://{address}/mcp"));
        let response = dispatch_provider_mcp_proxy_request(
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
            "arroba-stdio-mcp-proxy-starts-{}-{}.txt",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let script = r#"
import fs from 'node:fs'
const startFile = process.env.ARROBA_TEST_START_FILE
const current = Number(fs.existsSync(startFile) ? fs.readFileSync(startFile, 'utf8') : '0')
fs.writeFileSync(startFile, String(current + 1))
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
        let mut config = ArrobaMcpServerConfig::stdio(
            format!("stdio-test-{}", crate::session::unix_epoch_ms()),
            "node",
            vec![
                "--input-type=module".to_string(),
                "-e".to_string(),
                script.to_string(),
            ],
        );
        if let ArrobaMcpTransportConfig::Stdio { env, .. } = &mut config.transport {
            env.insert(
                "ARROBA_TEST_START_FILE".to_string(),
                start_file.to_string_lossy().to_string(),
            );
        }

        let initialize = dispatch_provider_mcp_proxy_request(
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
            &config,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .expect("initialized notification should be accepted");
        dispatch_provider_mcp_proxy_request(
            &config,
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .expect("tools/list should forward");
        let cached_initialize = dispatch_provider_mcp_proxy_request(
            &config,
            json!({"jsonrpc": "2.0", "id": 3, "method": "initialize"}),
        )
        .expect("second initialize should use cached response");
        assert_eq!(cached_initialize.get("id"), Some(&json!(3)));
        let starts = std::fs::read_to_string(&start_file).expect("start file should exist");
        assert_eq!(starts, "1");
        stdio_mcp_supervisor()
            .lock()
            .expect("test supervisor lock")
            .stop_all();
        let _ = std::fs::remove_file(start_file);
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
