use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::DaemonError;
use crate::mcp::{ArrobaMcpServerConfig, ArrobaMcpTransportConfig};

pub(super) fn stdio_mcp_supervisor() -> &'static Mutex<StdioMcpSupervisor> {
    static SUPERVISOR: OnceLock<Mutex<StdioMcpSupervisor>> = OnceLock::new();
    SUPERVISOR.get_or_init(|| Mutex::new(StdioMcpSupervisor::default()))
}

#[derive(Default)]
pub(super) struct StdioMcpSupervisor {
    processes: HashMap<String, Arc<Mutex<StdioMcpProcess>>>,
}

impl StdioMcpSupervisor {
    pub(super) fn process(
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
    pub(super) fn stop_all(&mut self) {
        self.processes.clear();
    }
}

pub(super) struct StdioMcpProcess {
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

    pub(super) fn dispatch(
        &mut self,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
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
