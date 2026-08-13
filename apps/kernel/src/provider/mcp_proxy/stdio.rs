use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::mcp::{CharioxMcpServerConfig, CharioxMcpTransportConfig};

pub(super) fn stdio_mcp_supervisor() -> &'static Mutex<StdioMcpSupervisor> {
    static SUPERVISOR: OnceLock<Mutex<StdioMcpSupervisor>> = OnceLock::new();
    SUPERVISOR.get_or_init(|| Mutex::new(StdioMcpSupervisor::default()))
}

#[derive(Default)]
pub(super) struct StdioMcpSupervisor {
    processes: HashMap<String, StdioMcpEntry>,
    closed_provider_run_ids: HashSet<String>,
    closed_session_ids: HashSet<String>,
}

struct StdioMcpEntry {
    process: Arc<Mutex<StdioMcpProcess>>,
    owner_sessions_by_provider_run_id: HashMap<String, String>,
}

impl StdioMcpSupervisor {
    pub(super) fn process(
        &mut self,
        key: &str,
        owner_provider_run_id: &str,
        owner_session_id: &str,
        backing: &CharioxMcpServerConfig,
    ) -> Result<Arc<Mutex<StdioMcpProcess>>, DaemonError> {
        if self.closed_provider_run_ids.contains(owner_provider_run_id)
            || self.closed_session_ids.contains(owner_session_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.closed",
                message: format!(
                    "provider run `{owner_provider_run_id}` is already closed for stdio MCP `{}`",
                    backing.name
                ),
            });
        }
        let exited = match self.processes.get(key) {
            Some(entry) => match entry.process.try_lock() {
                Ok(mut process) => process.is_exited()?,
                Err(TryLockError::WouldBlock) => false,
                Err(TryLockError::Poisoned(_)) => panic!("stdio MCP process mutex poisoned"),
            },
            None => false,
        };
        if exited {
            self.processes.remove(key);
        }
        if !self.processes.contains_key(key) {
            let process = StdioMcpProcess::spawn(backing)?;
            self.processes.insert(
                key.to_string(),
                StdioMcpEntry {
                    process: Arc::new(Mutex::new(process)),
                    owner_sessions_by_provider_run_id: HashMap::new(),
                },
            );
        }
        let entry = self
            .processes
            .get_mut(key)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.supervisor",
                message: format!("failed to start stdio MCP `{}`", backing.name),
            })?;
        entry.owner_sessions_by_provider_run_id.insert(
            owner_provider_run_id.to_string(),
            owner_session_id.to_string(),
        );
        Ok(entry.process.clone())
    }

    pub(super) fn discard_process(
        &mut self,
        key: &str,
        expected: &Arc<Mutex<StdioMcpProcess>>,
    ) -> Option<Arc<Mutex<StdioMcpProcess>>> {
        let matches = self
            .processes
            .get(key)
            .is_some_and(|entry| Arc::ptr_eq(&entry.process, expected));
        matches
            .then(|| self.processes.remove(key).map(|entry| entry.process))
            .flatten()
    }

    pub(super) fn release_run(
        &mut self,
        provider_run_id: &str,
    ) -> Vec<Arc<Mutex<StdioMcpProcess>>> {
        self.closed_provider_run_ids
            .insert(provider_run_id.to_string());
        let mut released_keys = Vec::new();
        for (key, entry) in &mut self.processes {
            entry
                .owner_sessions_by_provider_run_id
                .remove(provider_run_id);
            if entry.owner_sessions_by_provider_run_id.is_empty() {
                released_keys.push(key.clone());
            }
        }
        released_keys
            .into_iter()
            .filter_map(|key| self.processes.remove(&key).map(|entry| entry.process))
            .collect()
    }

    pub(super) fn release_session(&mut self, session_id: &str) -> Vec<Arc<Mutex<StdioMcpProcess>>> {
        self.closed_session_ids.insert(session_id.to_string());
        let mut released_keys = Vec::new();
        for (key, entry) in &mut self.processes {
            entry
                .owner_sessions_by_provider_run_id
                .retain(|_, owner_session_id| owner_session_id != session_id);
            if entry.owner_sessions_by_provider_run_id.is_empty() {
                released_keys.push(key.clone());
            }
        }
        released_keys
            .into_iter()
            .filter_map(|key| self.processes.remove(&key).map(|entry| entry.process))
            .collect()
    }

    #[cfg(test)]
    pub(super) fn stop_all(&mut self) {
        self.processes.clear();
        self.closed_provider_run_ids.clear();
        self.closed_session_ids.clear();
    }

    #[cfg(test)]
    pub(super) fn process_count(&self) -> usize {
        self.processes.len()
    }
}

pub(super) struct StdioMcpProcess {
    name: String,
    child: Child,
    stdin: ChildStdin,
    frames: mpsc::Receiver<Result<serde_json::Value, String>>,
    request_timeout: Duration,
    initialize_response: Option<serde_json::Value>,
    initialized_notified: bool,
}

impl StdioMcpProcess {
    fn spawn(backing: &CharioxMcpServerConfig) -> Result<Self, DaemonError> {
        let CharioxMcpTransportConfig::Stdio {
            command,
            args,
            env,
            credential_env: _,
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
        let (frame_tx, frame_rx) = mpsc::channel();
        let reader_name = backing.name.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("chariox-stdio-mcp-reader".to_string())
            .spawn(move || read_frames(stdout, &reader_name, frame_tx))
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.spawn",
                message: format!(
                    "failed to start stdio MCP `{}` response reader: {error}",
                    backing.name
                ),
            });
        }
        Ok(Self {
            name: backing.name.clone(),
            child,
            stdin,
            frames: frame_rx,
            request_timeout: Duration::from_secs(backing.tool_timeout_sec.unwrap_or(30).max(1)),
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
        let request_id = payload
            .get("id")
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.request",
                message: format!("stdio MCP `{}` request is missing an id", self.name),
            })?;
        self.write_frame(&payload)?;
        let deadline = Instant::now() + self.request_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return self.request_timeout_error();
            }
            let frame = match self.frames.recv_timeout(remaining) {
                Ok(Ok(frame)) => frame,
                Ok(Err(message)) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "mcp.proxy.stdio.read",
                        message,
                    })
                }
                Err(mpsc::RecvTimeoutError::Timeout) => return self.request_timeout_error(),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "mcp.proxy.stdio.read",
                        message: format!("stdio MCP `{}` response reader stopped", self.name),
                    })
                }
            };
            if let Some(method) = frame.get("method").and_then(serde_json::Value::as_str) {
                if let Some(server_request_id) = frame.get("id").cloned() {
                    self.write_frame(&serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": server_request_id,
                        "error": {
                            "code": -32601,
                            "message": format!(
                                "server-initiated MCP request `{method}` is not supported by the Chariox stdio proxy"
                            ),
                        },
                    }))?;
                }
                continue;
            }
            match frame.get("id") {
                Some(response_id) if response_id == &request_id => return Ok(frame),
                Some(_) => continue,
                None => {
                    return Err(DaemonError::LocalTransport {
                        operation: "mcp.proxy.stdio.response",
                        message: format!(
                            "stdio MCP `{}` returned a response without an id",
                            self.name
                        ),
                    })
                }
            }
        }
    }

    fn request_timeout_error(&mut self) -> Result<serde_json::Value, DaemonError> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        Err(DaemonError::LocalTransport {
            operation: "mcp.proxy.stdio.timeout",
            message: format!(
                "stdio MCP `{}` did not respond within {} seconds",
                self.name,
                self.request_timeout.as_secs()
            ),
        })
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

fn read_frames(
    stdout: ChildStdout,
    name: &str,
    sender: mpsc::Sender<Result<serde_json::Value, String>>,
) {
    let mut stdout = BufReader::new(stdout);
    loop {
        let frame = read_frame(&mut stdout, name);
        let failed = frame.is_err();
        if sender.send(frame).is_err() || failed {
            return;
        }
    }
}

fn read_frame(
    stdout: &mut BufReader<ChildStdout>,
    name: &str,
) -> Result<serde_json::Value, String> {
    loop {
        let mut first_line = String::new();
        let read = stdout
            .read_line(&mut first_line)
            .map_err(|error| format!("failed to read stdio MCP `{name}` frame: {error}"))?;
        if read == 0 {
            return Err(format!("stdio MCP `{name}` closed stdout"));
        }
        let first_line_trimmed = first_line.trim();
        if first_line_trimmed.is_empty() {
            continue;
        }
        if first_line_trimmed.starts_with('{') {
            return serde_json::from_str(first_line_trimmed).map_err(|error| {
                format!("stdio MCP `{name}` returned invalid JSON line: {error}")
            });
        }

        let mut content_length = parse_content_length_header(first_line_trimmed);
        loop {
            let mut line = String::new();
            stdout
                .read_line(&mut line)
                .map_err(|error| format!("failed to read stdio MCP `{name}` headers: {error}"))?;
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if content_length.is_none() {
                content_length = parse_content_length_header(line);
            }
        }
        let Some(content_length) = content_length else {
            return Err(format!(
                "stdio MCP `{name}` response missing Content-Length"
            ));
        };
        let mut body = vec![0_u8; content_length];
        stdout
            .read_exact(&mut body)
            .map_err(|error| format!("failed to read stdio MCP `{name}` body: {error}"))?;
        return serde_json::from_slice(&body)
            .map_err(|error| format!("stdio MCP `{name}` returned invalid JSON: {error}"));
    }
}

#[cfg(test)]
mod tests;
