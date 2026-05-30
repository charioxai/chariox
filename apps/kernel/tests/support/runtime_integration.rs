#![allow(dead_code)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use arroba_kernel::local::{
    GetSessionStateRequest, LocalDaemonClient, LocalDaemonRequest, LocalDaemonResponse,
    PumpTerminalOutputRequest,
};
use arroba_kernel::DaemonApp;
use serde_json::{json, Value};

static OPENCODE_ENV_LOCK: Mutex<()> = Mutex::new(());

pub fn opencode_env_guard() -> std::sync::MutexGuard<'static, ()> {
    OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

pub fn wait_for_terminal_output(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
) -> Vec<arroba_kernel::terminal::TerminalOutputRecord> {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let records = arroba_kernel::transport::TransportService::pump_terminal_output(
            app,
            session_id,
            attachment_id,
        )
        .expect("terminal output should fan out");
        if !records.is_empty() {
            return records;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for terminal output after {timeout_ms}ms"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_for_local_terminal_output(
    client: &LocalDaemonClient,
    session_id: &str,
    attachment_id: &str,
) -> String {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let response = client
            .send(LocalDaemonRequest::PumpTerminalOutput(
                PumpTerminalOutputRequest {
                    session_id: session_id.to_string(),
                    attachment_id: attachment_id.to_string(),
                },
            ))
            .expect("terminal output polling should succeed");

        let records = match response {
            LocalDaemonResponse::TerminalOutput { records } => records,
            _ => panic!("unexpected local response"),
        };

        if !records.is_empty() {
            let bytes = records
                .into_iter()
                .flat_map(|record| record.bytes)
                .collect::<Vec<u8>>();
            return String::from_utf8_lossy(&bytes).into_owned();
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for local terminal output after {timeout_ms}ms"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_for_local_provider_run_ready(
    client: &LocalDaemonClient,
    session_id: &str,
    provider_run_id: &str,
) {
    let timeout_ms = output_timeout_ms();
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let response = client
            .send(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session_id.to_string(),
                },
            ))
            .expect("session state polling should succeed");

        if let LocalDaemonResponse::SessionState { session, .. } = response {
            if session.active_provider_run_id() == Some(provider_run_id) {
                return;
            }
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider run activation after {timeout_ms}ms"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn collect_terminal_output_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    done: F,
) -> String
where
    F: Fn(&str, &arroba_kernel::session::RuntimeSession) -> bool,
{
    let timeout_ms = output_timeout_ms().max(8_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut output = Vec::new();

    loop {
        let records = arroba_kernel::transport::TransportService::pump_terminal_output(
            app,
            session_id,
            attachment_id,
        )
        .expect("terminal output should fan out");
        for record in records {
            output.extend(record.bytes);
        }

        let output_text = String::from_utf8_lossy(&output).into_owned();
        let session = app
            .sessions()
            .get_session(session_id)
            .expect("session should still exist");
        if done(&output_text, &session) {
            return output_text;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for terminal output after {timeout_ms}ms: {output_text}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

pub fn collect_provider_output_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    recipient_attachment_ids: Vec<String>,
    done: F,
) -> String
where
    F: Fn(&str, &DaemonApp) -> bool,
{
    let timeout_ms = output_timeout_ms().max(8_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut output = Vec::new();

    loop {
        let records = arroba_kernel::transport::TransportService::pump_provider_output(
            app,
            session_id,
            provider_run_id,
            recipient_attachment_ids.clone(),
        )
        .expect("provider output should fan out");
        for record in records {
            output.extend(record.bytes);
        }

        let output_text = String::from_utf8_lossy(&output).into_owned();
        if done(&output_text, app) {
            return output_text;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider output after {timeout_ms}ms: {output_text}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

pub fn collect_provider_records_until<F>(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
    recipient_attachment_ids: Vec<String>,
    done: F,
) -> Vec<arroba_kernel::terminal::TerminalOutputRecord>
where
    F: Fn(&[arroba_kernel::terminal::TerminalOutputRecord], &DaemonApp) -> bool,
{
    let timeout_ms = output_timeout_ms().max(4_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut records = Vec::new();

    loop {
        let next = arroba_kernel::transport::TransportService::pump_provider_output(
            app,
            session_id,
            provider_run_id,
            recipient_attachment_ids.clone(),
        )
        .expect("provider output should fan out");
        records.extend(next);

        if done(&records, app) {
            return records;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider records after {timeout_ms}ms: {}",
            render_terminal_output(&records)
        );
        thread::sleep(Duration::from_millis(25));
    }
}

pub fn wait_for_provider_runtime_state(
    app: &DaemonApp,
    provider_run_id: &str,
    expected_bound: bool,
    context: &str,
) {
    let deadline = Instant::now() + Duration::from_millis(output_timeout_ms().max(4_000));
    while app
        .providers()
        .structured_runtime_state_bound_for_tests(provider_run_id)
        != expected_bound
    {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for provider runtime state to become {expected_bound} while {context}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn render_terminal_output(records: &[arroba_kernel::terminal::TerminalOutputRecord]) -> String {
    let mut output = Vec::new();
    for record in records {
        output.extend_from_slice(&record.bytes);
    }
    String::from_utf8_lossy(&output).into_owned()
}

pub struct MockOpenCodeServer {
    port: u16,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<MockOpenCodeState>>,
    thread: Option<thread::JoinHandle<()>>,
}

struct MockOpenCodeState {
    abort_count: u64,
    disconnect_next_event_stream: bool,
    emit_idle_before_completion: bool,
    emit_tool_call_before_completion: bool,
    fail_next_event_stream_attempts: u64,
    event_subscribers: Vec<mpsc::Sender<String>>,
    next_prompt_error: Option<String>,
    prompt_async_response_delay: Duration,
    abort_response_delay: Duration,
    message_response_delay: Duration,
    response_delay: Duration,
    omit_session_status: bool,
    sessions: BTreeMap<String, MockOpenCodeSessionState>,
    next_session_number: u64,
    next_message_number: u64,
}

struct MockOpenCodeSessionState {
    status: String,
    messages: Vec<Value>,
}

impl MockOpenCodeServer {
    pub fn start(response_delay: Duration) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("mock server should bind");
        listener
            .set_nonblocking(true)
            .expect("mock server should become non-blocking");
        let port = listener
            .local_addr()
            .expect("mock server should have local addr")
            .port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let state = Arc::new(Mutex::new(MockOpenCodeState {
            abort_count: 0,
            disconnect_next_event_stream: false,
            emit_idle_before_completion: false,
            emit_tool_call_before_completion: false,
            fail_next_event_stream_attempts: 0,
            event_subscribers: Vec::new(),
            next_prompt_error: None,
            prompt_async_response_delay: Duration::ZERO,
            abort_response_delay: Duration::ZERO,
            message_response_delay: Duration::ZERO,
            response_delay,
            omit_session_status: false,
            sessions: BTreeMap::new(),
            next_session_number: 0,
            next_message_number: 0,
        }));
        let state_for_thread = state.clone();
        let stop_for_thread_loop = stop_flag.clone();
        let thread = thread::spawn(move || {
            while !stop_flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state_for_connection = state_for_thread.clone();
                        let stop_for_connection = stop_for_thread_loop.clone();
                        thread::spawn(move || {
                            handle_mock_opencode_request(
                                stream,
                                &state_for_connection,
                                &stop_for_connection,
                            );
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            port,
            stop,
            state,
            thread: Some(thread),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn set_omit_session_status(&self, omit_session_status: bool) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .omit_session_status = omit_session_status;
    }

    pub fn abort_count(&self) -> u64 {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .abort_count
    }

    pub fn disconnect_next_event_stream(&self) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .disconnect_next_event_stream = true;
    }

    pub fn set_emit_tool_call_before_completion(&self, emit_tool_call_before_completion: bool) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .emit_tool_call_before_completion = emit_tool_call_before_completion;
    }

    pub fn set_prompt_async_response_delay(&self, delay: Duration) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .prompt_async_response_delay = delay;
    }

    pub fn set_abort_response_delay(&self, delay: Duration) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .abort_response_delay = delay;
    }

    fn event_subscriber_count(&self) -> usize {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .event_subscribers
            .len()
    }

    pub fn fail_next_event_stream_attempts(&self, count: u64) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .fail_next_event_stream_attempts = count;
    }

    pub fn fail_next_prompt(&self, message: impl Into<String>) {
        self.state
            .lock()
            .expect("mock state should not be poisoned")
            .next_prompt_error = Some(message.into());
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn wait_for_mock_opencode_event_subscription(mock_server: &MockOpenCodeServer) {
    let deadline = Instant::now() + Duration::from_millis(output_timeout_ms().max(4_000));
    while mock_server.event_subscriber_count() == 0 {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for mock OpenCode event subscription"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn handle_mock_opencode_request(
    mut stream: std::net::TcpStream,
    state: &Arc<Mutex<MockOpenCodeState>>,
    stop: &Arc<AtomicBool>,
) {
    let request = read_http_request(&mut stream);
    let Some(request) = request else {
        return;
    };

    if request.method == "GET" && request.path == "/event" {
        handle_mock_opencode_event_stream(stream, state, stop);
        return;
    }

    let response = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/global/health") => json!({ "healthy": true, "version": "test" }),
        ("POST", "/session") => {
            let session_id = {
                let mut state = state.lock().expect("mock state should not be poisoned");
                state.next_session_number += 1;
                let session_id = format!("mock-session-{}", state.next_session_number);
                state.sessions.insert(
                    session_id.clone(),
                    MockOpenCodeSessionState {
                        status: "idle".to_string(),
                        messages: Vec::new(),
                    },
                );
                session_id
            };
            json!({ "id": session_id })
        }
        ("GET", "/session/status") => {
            let state = state.lock().expect("mock state should not be poisoned");
            if state.omit_session_status {
                json!({})
            } else {
                let status_map = state
                    .sessions
                    .iter()
                    .map(|(session_id, session_state)| {
                        (
                            session_id.clone(),
                            json!({
                                "type": session_state.status,
                            }),
                        )
                    })
                    .collect::<serde_json::Map<String, Value>>();
                Value::Object(status_map)
            }
        }
        ("POST", path) if path.starts_with("/mcp/") && path.ends_with("/connect") => json!(true),
        ("GET", path) if path.starts_with("/session/") && path.ends_with("/message") => {
            let response_delay = state
                .lock()
                .expect("mock state should not be poisoned")
                .message_response_delay;
            thread::sleep(response_delay);
            let state = state.lock().expect("mock state should not be poisoned");
            let session_id = path
                .strip_prefix("/session/")
                .and_then(|value| value.strip_suffix("/message"))
                .unwrap_or_default();
            Value::Array(
                state
                    .sessions
                    .get(session_id)
                    .map(|session| session.messages.clone())
                    .unwrap_or_default(),
            )
        }
        ("POST", path) if path.starts_with("/session/") && path.ends_with("/prompt_async") => {
            let payload: Value =
                serde_json::from_slice(&request.body).expect("prompt body should parse");
            let prompt = payload["parts"][0]["text"]
                .as_str()
                .expect("prompt should include a text part")
                .trim_end_matches('\n')
                .to_string();
            let user_message_id = payload["messageID"]
                .as_str()
                .expect("prompt should include a user message id")
                .to_string();
            let session_id = path
                .strip_prefix("/session/")
                .and_then(|value| value.strip_suffix("/prompt_async"))
                .expect("prompt path should include a session id")
                .to_string();
            schedule_mock_response(state.clone(), session_id, user_message_id, prompt);
            let response_delay = state
                .lock()
                .expect("mock state should not be poisoned")
                .prompt_async_response_delay;
            thread::sleep(response_delay);
            write_http_empty_response(&mut stream, 204);
            return;
        }
        ("POST", path) if path.starts_with("/session/") && path.ends_with("/abort") => {
            let response_delay = {
                let mut state = state.lock().expect("mock state should not be poisoned");
                state.abort_count += 1;
                state.abort_response_delay
            };
            thread::sleep(response_delay);
            let mut state = state.lock().expect("mock state should not be poisoned");
            let session_id = path
                .strip_prefix("/session/")
                .and_then(|value| value.strip_suffix("/abort"))
                .expect("abort path should include a session id")
                .to_string();
            if let Some(session_state) = state.sessions.get_mut(&session_id) {
                session_state.status = "idle".to_string();
                publish_mock_event(
                    &mut state,
                    json!({
                        "type": "session.status",
                        "properties": {
                            "sessionID": session_id,
                            "status": {
                                "type": "idle"
                            }
                        }
                    }),
                );
            }
            json!(true)
        }
        _ => {
            write_http_response(&mut stream, 404, json!({ "error": "not found" }));
            return;
        }
    };

    write_http_response(&mut stream, 200, response);
}

fn handle_mock_opencode_event_stream(
    mut stream: std::net::TcpStream,
    state: &Arc<Mutex<MockOpenCodeState>>,
    stop: &Arc<AtomicBool>,
) {
    let (tx, rx) = mpsc::channel();
    let (disconnect_immediately, fail_with_http_error) = {
        let mut state = state.lock().expect("mock state should not be poisoned");
        if state.fail_next_event_stream_attempts > 0 {
            state.fail_next_event_stream_attempts -= 1;
            (false, true)
        } else {
            state.event_subscribers.push(tx);
            let disconnect = state.disconnect_next_event_stream;
            state.disconnect_next_event_stream = false;
            (disconnect, false)
        }
    };

    if fail_with_http_error {
        let response = "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        return;
    }

    if write_sse_connected_response(&mut stream, disconnect_immediately).is_err() {
        return;
    }

    if disconnect_immediately {
        return;
    }

    while !stop.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(payload) => {
                if write_sse_event(&mut stream, &payload).is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn publish_mock_event(state: &mut MockOpenCodeState, payload: Value) {
    let payload = payload.to_string();
    state
        .event_subscribers
        .retain(|subscriber| subscriber.send(payload.clone()).is_ok());
}

fn write_sse_headers(stream: &mut std::net::TcpStream) {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    stream
        .write_all(response.as_bytes())
        .expect("mock SSE headers should write");
    let _ = stream.flush();
}

fn write_sse_connected_response(
    stream: &mut std::net::TcpStream,
    include_event_with_headers: bool,
) -> std::io::Result<()> {
    let connected = json!({
        "type": "server.connected",
        "properties": {}
    })
    .to_string();
    if include_event_with_headers {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\ndata: {connected}\n\n"
        );
        stream.write_all(response.as_bytes())?;
        return stream.flush();
    }

    write_sse_headers(stream);
    write_sse_event(stream, &connected)
}

fn write_sse_event(stream: &mut std::net::TcpStream, payload: &str) -> std::io::Result<()> {
    stream.write_all(format!("data: {payload}\n\n").as_bytes())?;
    stream.flush()
}

fn mock_user_prompt_text(prompt: &str) -> &str {
    prompt
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or_else(|| prompt.trim())
}

fn schedule_mock_response(
    state: Arc<Mutex<MockOpenCodeState>>,
    session_id: String,
    user_message_id: String,
    prompt: String,
) {
    {
        let mut state = state.lock().expect("mock state should not be poisoned");
        if let Some(session_state) = state.sessions.get_mut(&session_id) {
            session_state.status = "busy".to_string();
        }
        publish_mock_event(
            &mut state,
            json!({
                "type": "session.status",
                "properties": {
                    "sessionID": session_id,
                    "status": {
                        "type": "busy"
                    }
                }
            }),
        );
    }

    thread::spawn(move || {
        let (response_delay, emit_idle_before_completion, emit_tool_call_before_completion) = {
            let state = state.lock().expect("mock state should not be poisoned");
            (
                state.response_delay,
                state.emit_idle_before_completion,
                state.emit_tool_call_before_completion,
            )
        };
        thread::sleep(response_delay);

        if emit_tool_call_before_completion {
            let mut state = state.lock().expect("mock state should not be poisoned");
            state.next_message_number += 1;
            let message_id = format!("assistant-tool-message-{}", state.next_message_number);
            let part_id = format!("assistant-tool-part-{}", state.next_message_number);
            if let Some(session_state) = state.sessions.get_mut(&session_id) {
                session_state.messages.push(json!({
                    "info": {
                        "id": message_id.clone(),
                        "sessionID": session_id.clone(),
                        "role": "assistant",
                        "parentID": user_message_id.clone(),
                        "finish": "tool-calls",
                        "time": {
                            "completed": 1,
                        }
                    },
                    "parts": [
                        {
                            "id": part_id.clone(),
                            "sessionID": session_id.clone(),
                            "messageID": message_id.clone(),
                            "type": "tool",
                            "tool": "read",
                            "state": {
                                "status": "completed",
                                "input": {
                                    "filePath": "./.arroba/mock-instructions.md"
                                },
                                "output": "<content>mock tool output</content>",
                                "title": "mock read"
                            }
                        }
                    ]
                }));
            }
            publish_mock_event(
                &mut state,
                json!({
                    "type": "message.updated",
                    "properties": {
                        "info": {
                            "id": message_id.clone(),
                            "sessionID": session_id.clone(),
                            "role": "assistant",
                            "parentID": user_message_id.clone(),
                            "finish": "tool-calls",
                            "time": {
                                "completed": 1
                            }
                        }
                    }
                }),
            );
            publish_mock_event(
                &mut state,
                json!({
                    "type": "message.part.updated",
                    "properties": {
                        "part": {
                            "id": part_id,
                            "sessionID": session_id.clone(),
                            "messageID": message_id,
                            "type": "tool",
                            "tool": "read",
                            "state": {
                                "status": "completed",
                                "input": {
                                    "filePath": "./.arroba/mock-instructions.md"
                                },
                                "output": "<content>mock tool output</content>",
                                "title": "mock read"
                            }
                        }
                    }
                }),
            );
            drop(state);
            thread::sleep(response_delay);
        }

        if emit_idle_before_completion {
            let mut state = state.lock().expect("mock state should not be poisoned");
            if let Some(session_state) = state.sessions.get_mut(&session_id) {
                session_state.status = "idle".to_string();
            }
            publish_mock_event(
                &mut state,
                json!({
                    "type": "session.status",
                    "properties": {
                        "sessionID": session_id.clone(),
                        "status": {
                            "type": "idle"
                        }
                    }
                }),
            );
            drop(state);
            thread::sleep(response_delay);
        }

        let mut state = state.lock().expect("mock state should not be poisoned");
        if let Some(error_message) = state.next_prompt_error.take() {
            if let Some(session_state) = state.sessions.get_mut(&session_id) {
                session_state.status = "idle".to_string();
            }
            publish_mock_event(
                &mut state,
                json!({
                    "type": "session.error",
                    "properties": {
                        "sessionID": session_id.clone(),
                        "error": {
                            "message": error_message
                        }
                    }
                }),
            );
            publish_mock_event(
                &mut state,
                json!({
                    "type": "session.status",
                    "properties": {
                        "sessionID": session_id,
                        "status": {
                            "type": "idle"
                        }
                    }
                }),
            );
            return;
        }

        state.next_message_number += 1;
        let message_id = format!("assistant-message-{}", state.next_message_number);
        let part_id = format!("assistant-part-{}", state.next_message_number);
        let response_text = format!("fixture response: {}\n", mock_user_prompt_text(&prompt));
        if let Some(session_state) = state.sessions.get_mut(&session_id) {
            session_state.messages.push(json!({
                "info": {
                    "id": message_id.clone(),
                    "sessionID": session_id.clone(),
                    "role": "assistant",
                    "parentID": user_message_id.clone(),
                    "finish": "stop",
                    "time": {
                        "completed": 1,
                    }
                },
                "parts": [
                    {
                        "id": part_id.clone(),
                        "sessionID": session_id.clone(),
                        "messageID": message_id.clone(),
                        "type": "text",
                        "text": response_text.clone(),
                        "time": {
                            "end": 1
                        }
                    }
                ]
            }));
            session_state.status = "idle".to_string();
        }
        publish_mock_event(
            &mut state,
            json!({
                "type": "message.part.delta",
                "properties": {
                    "sessionID": session_id.clone(),
                    "messageID": message_id.clone(),
                    "partID": part_id.clone(),
                    "field": "text",
                    "delta": response_text.clone(),
                }
            }),
        );
        publish_mock_event(
            &mut state,
            json!({
                "type": "message.updated",
                "properties": {
                    "info": {
                        "id": message_id.clone(),
                        "sessionID": session_id.clone(),
                        "role": "assistant",
                        "parentID": user_message_id.clone(),
                        "finish": "stop",
                        "time": {
                            "completed": 1
                        }
                    }
                }
            }),
        );
        publish_mock_event(
            &mut state,
            json!({
                "type": "session.status",
                "properties": {
                    "sessionID": session_id,
                    "status": {
                        "type": "idle"
                    }
                }
            }),
        );
    });
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut std::net::TcpStream) -> Option<HttpRequest> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("mock request stream should accept timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end;
    loop {
        let size = stream.read(&mut chunk).ok()?;
        if size == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..size]);
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }

    let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = header_text.lines();
    let request_line = lines.next()?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next()?.to_string();
    let path = request_parts.next()?.to_string();
    let content_length = lines
        .find_map(|line| {
            let mut parts = line.splitn(2, ':');
            let name = parts.next()?.trim();
            let value = parts.next()?.trim();
            (name.eq_ignore_ascii_case("content-length")).then_some(value)
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let size = stream.read(&mut chunk).ok()?;
        if size == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..size]);
    }

    Some(HttpRequest { method, path, body })
}

fn write_http_response(stream: &mut std::net::TcpStream, status: u16, body: Value) {
    let body = serde_json::to_vec(&body).expect("mock response should encode");
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("mock response header should write");
    stream
        .write_all(&body)
        .expect("mock response body should write");
    let _ = stream.flush();
}

fn write_http_empty_response(stream: &mut std::net::TcpStream, status: u16) {
    let status_text = match status {
        204 => "No Content",
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(response.as_bytes())
        .expect("mock empty response should write");
    let _ = stream.flush();
}

pub fn create_opencode_fixture_script(delay_seconds: u64) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "arroba-opencode-fixture-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be monotonic enough")
            .as_nanos()
    ));
    fs::write(&path, fixture_script_contents(delay_seconds))
        .expect("fixture script should be created");
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&path)
            .expect("fixture script should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("fixture script should be executable");
    }
    path
}

fn fixture_script_contents(delay_seconds: u64) -> String {
    format!(
        r#"#!/bin/sh
PORT=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --port)
      PORT="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

if [ -z "$PORT" ] || [ -z "$ARROBA_OPENCODE_PORT" ]; then
  exit 2
fi

export ARROBA_OPENCODE_FIXTURE_LISTEN_PORT="$PORT"
export ARROBA_OPENCODE_FIXTURE_MAX_SECONDS="{delay_seconds}"
python3 - <<'PY'
import os
import signal
import socket
import sys
import threading
import time

listen_port = int(os.environ["ARROBA_OPENCODE_FIXTURE_LISTEN_PORT"])
target_port = int(os.environ["ARROBA_OPENCODE_PORT"])
max_seconds = float(os.environ["ARROBA_OPENCODE_FIXTURE_MAX_SECONDS"])
deadline = time.monotonic() + max_seconds
stopping = threading.Event()

def stop(_signum=None, _frame=None):
    stopping.set()

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

def relay(source, destination):
    try:
        while not stopping.is_set():
            chunk = source.recv(65536)
            if not chunk:
                break
            destination.sendall(chunk)
    except OSError:
        pass
    finally:
        for sock in (source, destination):
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                sock.close()
            except OSError:
                pass

def handle(client):
    try:
        upstream = socket.create_connection(("127.0.0.1", target_port), timeout=10)
    except OSError:
        client.close()
        return
    threading.Thread(target=relay, args=(client, upstream), daemon=True).start()
    threading.Thread(target=relay, args=(upstream, client), daemon=True).start()

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", listen_port))
    server.listen()
    server.settimeout(0.1)
    while not stopping.is_set() and time.monotonic() < deadline:
        try:
            client, _addr = server.accept()
        except socket.timeout:
            continue
        except OSError:
            break
        threading.Thread(target=handle, args=(client,), daemon=True).start()

sys.exit(0)
PY
"#
    )
}

pub fn output_timeout_ms() -> u64 {
    env::var("ARROBA_HARNESS_OUTPUT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2_000)
}
