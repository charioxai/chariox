#![allow(dead_code)]

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use chariox_kernel::local::{GetProviderRunRequest, LocalDaemonRequest};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

pub const UX_RESPONSE_BUDGET: Duration = Duration::from_millis(250);

pub fn reserved_kernel_listener() -> (u16, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("should bind an ephemeral port");
    let port = listener
        .local_addr()
        .expect("listener address should exist")
        .port();
    (port, listener)
}

pub fn unused_tcp_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("should bind an ephemeral port");
    listener
        .local_addr()
        .expect("listener address should exist")
        .port()
}

pub async fn connect_with_retry(url: &str) -> WebSocketStream<MaybeTlsStream<TcpStream>> {
    for _ in 0..20 {
        match connect_async(url).await {
            Ok((socket, _)) => return socket,
            Err(_) => sleep(Duration::from_millis(25)).await,
        }
    }

    let (socket, _) = connect_async(url)
        .await
        .expect("kernel websocket should accept connections");
    socket
}

pub async fn send_request(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
    request: LocalDaemonRequest,
) -> Value {
    send_frame(
        socket,
        json!({
            "type": "request",
            "request_id": request_id,
            "request": request,
        }),
    )
    .await;
    wait_for_response(socket, request_id).await
}

pub async fn send_request_with_ux_budget(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
    request: LocalDaemonRequest,
) -> Value {
    send_frame(
        socket,
        json!({
            "type": "request",
            "request_id": request_id,
            "request": request,
        }),
    )
    .await;
    wait_for_response_with_timeout(socket, request_id, UX_RESPONSE_BUDGET).await
}

pub async fn send_frame(socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>, frame: Value) {
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .expect("kernel websocket frame should send");
}

pub async fn wait_for_response(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "response" && frame["request_id"] == request_id {
                assert!(
                    frame["error"].is_null(),
                    "kernel websocket response should not contain an error: {frame}"
                );
                return frame;
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("timed out waiting for kernel websocket response `{request_id}` after {deadline:?}")
    })
}

pub async fn wait_for_response_and_event(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
    event_name: &str,
) -> (Value, Value) {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        let mut response = None;
        let mut event = None;
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "response" && frame["request_id"] == request_id {
                assert!(
                    frame["error"].is_null(),
                    "kernel websocket response should not contain an error: {frame}"
                );
                response = Some(frame);
            } else if frame["type"] == "event" && frame["event"]["event"] == event_name {
                event = Some(frame);
            }
            if let (Some(response), Some(event)) = (response.clone(), event.clone()) {
                return (response, event);
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket response and event")
}

pub async fn wait_for_responses(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_ids: &[&str],
) -> BTreeMap<String, Value> {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        let mut responses = BTreeMap::new();
        while responses.len() < request_ids.len() {
            let frame = next_json_frame(socket).await;
            if frame["type"] != "response" {
                continue;
            }
            let Some(request_id) = frame["request_id"].as_str() else {
                continue;
            };
            if request_ids.contains(&request_id) {
                assert!(
                    frame["error"].is_null(),
                    "kernel websocket response should not contain an error: {frame}"
                );
                responses.insert(request_id.to_string(), frame);
            }
        }
        responses
    })
    .await
    .expect("timed out waiting for kernel websocket responses")
}

pub async fn wait_for_first_response(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_ids: &[&str],
    deadline: Duration,
) -> Value {
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] != "response" {
                continue;
            }
            let Some(request_id) = frame["request_id"].as_str() else {
                continue;
            };
            if request_ids.contains(&request_id) {
                assert!(
                    frame["error"].is_null(),
                    "kernel websocket response should not contain an error: {frame}"
                );
                return frame;
            }
        }
    })
    .await
    .expect("timed out waiting for first kernel websocket response")
}

pub async fn wait_for_response_with_timeout(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
    deadline: Duration,
) -> Value {
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "response" && frame["request_id"] == request_id {
                assert!(
                    frame["error"].is_null(),
                    "kernel websocket response should not contain an error: {frame}"
                );
                return frame;
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("timed out waiting for kernel websocket response `{request_id}` after {deadline:?}")
    })
}

pub async fn wait_for_error_response(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "response" && frame["request_id"] == request_id {
                assert!(
                    !frame["error"].is_null(),
                    "kernel websocket response should contain an error: {frame}"
                );
                return frame;
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket error response")
}

pub async fn wait_for_error_code(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    code: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "response" && frame["error"]["code"] == code {
                return frame;
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket error code")
}

pub fn provider_run_id_from_launch_response(frame: &Value) -> String {
    let response = &frame["response"];
    let provider_run = response
        .get("ProviderRunLaunched")
        .or_else(|| response.get("ProviderRunLaunchAccepted"))
        .and_then(|value| value.get("provider_run"))
        .unwrap_or_else(|| panic!("expected provider launch response, got: {frame}"));
    provider_run["id"]
        .as_str()
        .expect("provider run id should be present")
        .to_string()
}

pub async fn wait_for_provider_run_state(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    provider_run_id: &str,
    expected_state: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        let mut attempt = 0_u64;
        loop {
            let request_id = format!("provider-run-state-{provider_run_id}-{attempt}");
            let frame = send_request(
                socket,
                &request_id,
                LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
                    provider_run_id: provider_run_id.to_string(),
                }),
            )
            .await;
            if response_variant(&frame, "ProviderRun")["provider_run"]["state"] == expected_state {
                return frame;
            }
            attempt += 1;
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("timed out waiting for provider run state")
}

pub async fn wait_for_event(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    event_name: &str,
) -> Value {
    wait_for_event_with_timeout(socket, event_name, Duration::from_secs(5)).await
}

/// Session subscriptions heartbeat every 5 s (`WATCH_INTERVAL_MS * HEARTBEAT_INTERVAL_TICKS`)
/// and emit the first heartbeat immediately on subscribe. Waiting for a *later* heartbeat
/// therefore needs a budget longer than one interval, or it races the kernel's clock.
pub const SUBSCRIPTION_HEARTBEAT_BUDGET: Duration = Duration::from_secs(10);

pub async fn wait_for_event_with_timeout(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    event_name: &str,
    deadline: Duration,
) -> Value {
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "event" && frame["event"]["event"] == event_name {
                return frame;
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("timed out waiting for kernel websocket event `{event_name}` after {deadline:?}")
    })
}

/// Collect the first event of each name, in whatever order the kernel emits them.
pub async fn wait_for_events(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    event_names: &[&str],
) -> BTreeMap<String, Value> {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        let mut events = BTreeMap::new();
        while events.len() < event_names.len() {
            let frame = next_json_frame(socket).await;
            if frame["type"] != "event" {
                continue;
            }
            let Some(event_name) = frame["event"]["event"].as_str() else {
                continue;
            };
            if event_names.contains(&event_name) && !events.contains_key(event_name) {
                events.insert(event_name.to_string(), frame);
            }
        }
        events
    })
    .await
    .unwrap_or_else(|_| {
        panic!("timed out waiting for kernel websocket events {event_names:?} after {deadline:?}")
    })
}

pub async fn next_json_frame(socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) -> Value {
    loop {
        let message = socket
            .next()
            .await
            .expect("kernel websocket should yield a frame")
            .expect("kernel websocket frame should decode");

        match message {
            Message::Text(text) => {
                return serde_json::from_str(&text)
                    .expect("kernel websocket text frame should be valid json");
            }
            Message::Binary(bytes) => {
                return serde_json::from_slice(&bytes)
                    .expect("kernel websocket binary frame should be valid json");
            }
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(frame) => panic!("kernel websocket closed unexpectedly: {frame:?}"),
            Message::Frame(_) => {}
        }
    }
}

pub async fn wait_for_close(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> (Option<u16>, Option<String>) {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let message = socket
                .next()
                .await
                .expect("kernel websocket should yield a frame")
                .expect("kernel websocket frame should decode");

            if let Message::Close(frame) = message {
                return (
                    frame.as_ref().map(|frame| u16::from(frame.code)),
                    frame.as_ref().map(|frame| frame.reason.to_string()),
                );
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket close")
}

pub fn response_variant<'a>(frame: &'a Value, variant: &str) -> &'a Value {
    frame["response"][variant]
        .as_object()
        .map(|_| &frame["response"][variant])
        .unwrap_or_else(|| panic!("expected response variant `{variant}`, got: {frame}"))
}

// session.create (and every launch preflight) requires workspace/worktree ids to
// be existing absolute directories, so fixtures own a real temporary directory
// that is removed when the fixture drops. `new()` adds a Git repository for
// workflow/provider execution; `directory()` is enough for transport-only tests.
pub struct ExecutionWorkspace(std::path::PathBuf);

impl ExecutionWorkspace {
    pub fn new() -> Self {
        let workspace = Self::directory("execution");
        for args in [
            vec!["init", "--quiet"],
            vec![
                "-c",
                "user.name=Workflow fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "fixture",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&workspace.0)
                .status()
                .expect("run fixture git")
                .success());
        }
        workspace
    }

    /// A plain (non-Git) isolated directory, unique per call.
    pub fn directory(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-websocket-{label}-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).expect("create isolated session workspace");
        Self(root)
    }

    /// The absolute directory path used as the session workspace/worktree id.
    pub fn path(&self) -> &str {
        self.0
            .to_str()
            .expect("temporary workspace path should be UTF-8")
    }

    pub fn session_request(&self) -> chariox_kernel::session::CreateSessionRequest {
        chariox_kernel::session::CreateSessionRequest::new(self.path(), self.path())
    }
}

impl Drop for ExecutionWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
