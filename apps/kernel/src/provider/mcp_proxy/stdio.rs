use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
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
    backing: CharioxMcpServerConfig,
    initialize_request: Option<serde_json::Value>,
    initialize_response: Option<serde_json::Value>,
    restart_required: bool,
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
        let mut environment = env.clone();
        for key in env_vars {
            if let Ok(value) = std::env::var(key) {
                environment.insert(key.clone(), value);
            }
        }
        let mut command_builder = crate::provider::managed_isolated_utility_command(
            command.clone(),
            args.clone(),
            environment,
            cwd.as_deref().map(std::path::PathBuf::from),
            &format!("mcp:{}", backing.name),
        )?;
        command_builder
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = crate::process_spawn::spawn_command(command_builder).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "mcp.proxy.stdio.spawn",
                message: format!("failed to spawn stdio MCP `{}`: {error}", backing.name),
            }
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
            if child.id() > 1 {
                let _ = child.kill();
            }
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
            backing: backing.clone(),
            initialize_request: None,
            initialize_response: None,
            restart_required: false,
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
        let method = payload.get("method").and_then(serde_json::Value::as_str);
        // MP-08 / MP-10: provider clients keep their established HTTP MCP
        // connection across a failed stdio request. A replacement child must
        // restore that handshake before accepting the next tool request.
        if self.restart_required || self.is_exited()? {
            let initialize = self.initialize_request.clone();
            let notified = self.initialized_notified;
            let backing = self.backing.clone();
            *self = Self::spawn(&backing)?;
            if method != Some("initialize") {
                if let Some(initialize) = initialize {
                    self.dispatch(initialize)?;
                    if notified {
                        self.dispatch(serde_json::json!({
                            "jsonrpc": "2.0", "method": "notifications/initialized"
                        }))?;
                    }
                }
            }
        }

        if method == Some("initialize") {
            if let Some(response) = &self.initialize_response {
                let mut response = response.clone();
                if let Some(id) = payload.get("id").cloned() {
                    response["id"] = id;
                }
                return Ok(response);
            }
            self.initialize_request = Some(payload.clone());
            let response = self.send_request(payload)?;
            if response.get("result").is_some() {
                // The proxy is the backing server's MCP client. Complete its
                // handshake even when the HTTP front end acknowledges the
                // provider's initialized notification without forwarding it.
                self.write_frame(&serde_json::json!({
                    "jsonrpc": "2.0", "method": "notifications/initialized"
                }))?;
                self.initialized_notified = true;
            }
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
        let response = self.send_request_inner(payload);
        if response.is_err() {
            self.restart_required = true;
            if self.child.id() > 1 {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        }
        response
    }

    fn send_request_inner(
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
        if self.child.id() > 1 {
            let _ = self.child.kill();
        }
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
        if self.child.id() > 1 {
            let _ = self.child.kill();
        }
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
