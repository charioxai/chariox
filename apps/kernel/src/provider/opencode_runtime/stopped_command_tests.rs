//! A turn whose command lost its process ends through the drain.
use super::stuck_tool::{ChildProcess, RunningTools, ServerChildren};
use super::*;
use crate::terminal::TerminalOutputKind;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Children(Arc<Mutex<Vec<ChildProcess>>>);

impl ServerChildren for Children {
    fn children(&mut self, _port: u16) -> Option<Vec<ChildProcess>> {
        Some(self.0.lock().unwrap().clone())
    }
}

/// A scripted OpenCode: busy until the abort arrives, then idle.
fn opencode(
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut aborted = false;
        while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = String::new();
            while !request.contains("\r\n\r\n") {
                let mut buf = [0; 1024];
                let size = stream.read(&mut buf).unwrap_or(0);
                if size == 0 {
                    break;
                }
                request.push_str(&String::from_utf8_lossy(&buf[..size]));
            }
            let line = request.lines().next().unwrap_or_default().to_string();
            requests.lock().unwrap().push(line.clone());
            aborted |= line.starts_with("POST /session/session-1/abort ");
            let body = if line.starts_with("GET /session/status ") && !aborted {
                serde_json::json!({"session-1": {"type": "busy"}})
            } else if line.starts_with("GET /session/status ") {
                serde_json::json!({})
            } else if line.starts_with("GET /session/session-1/message") {
                serde_json::json!([])
            } else {
                serde_json::json!(true)
            }
            .to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (endpoint, handle)
}

fn running_bash() -> crate::provider::opencode_client::OpenCodePart {
    serde_json::from_value(serde_json::json!({
        "id": "part-bash", "sessionID": "session-1", "messageID": "assistant-1",
        "type": "tool", "tool": "bash",
        "state": {"status": "running", "input": {"command": "sleep 240"}, "time": {"start": 1}},
    }))
    .unwrap()
}

/// The sender stays with the caller: a dropped sender reads as a lost event
/// stream, and the drain would resubscribe instead of polling the status.
fn state(
    endpoint: String,
    children: Arc<Mutex<Vec<ChildProcess>>>,
) -> (
    OpenCodeRuntimeState,
    std::sync::mpsc::Sender<crate::provider::OpenCodeEvent>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        endpoint,
        "session-1".to_string(),
        crate::provider::OpenCodeEventSubscription::for_tests(rx),
    );
    state.running_tools = RunningTools::with_children(Box::new(Children(children)));
    state.note_prompt_submitted("user-1".to_string());
    state
        .running_tools
        .note(&running_bash(), Instant::now() - Duration::from_secs(30), 1);
    (state, tx)
}

#[test]
fn a_turn_whose_command_process_is_gone_is_aborted_and_ends_with_a_stopped_row() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (endpoint, server) = opencode(requests.clone(), stop.clone());
    let (mut state, _events) = state(endpoint, Arc::new(Mutex::new(Vec::new())));
    state.running_tools.backdate_gone(Duration::from_secs(10));

    let drained = drain_opencode_events(&tests::test_run(), &mut state, None).unwrap();

    assert!(drained.prompt_completed);
    assert!(drained.terminal_failure.is_none());
    let row = drained
        .chunks
        .iter()
        .find(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
        .expect("the command's row");
    assert_eq!(row.merge_key.as_deref(), Some("part-bash"));
    let row: serde_json::Value = serde_json::from_slice(&row.bytes).unwrap();
    assert_eq!(row["status"], "error");
    assert_eq!(row["error"], "The command was stopped.");
    assert_eq!(row["input"]["command"], "sleep 240");
    assert!(state.active_user_message_id.is_none());
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.starts_with("POST /session/session-1/abort ")));
    drop(state);
    stop.store(true, Ordering::Relaxed);
    let _ = server.join();
}

#[test]
fn a_turn_whose_command_process_is_alive_is_left_running() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (endpoint, server) = opencode(requests.clone(), stop.clone());
    let alive = ChildProcess {
        pid: 4242,
        started_ms: 10,
        argv: vec!["bash".into(), "-c".into(), "sleep 240".into()],
    };
    let (mut state, _events) = state(endpoint, Arc::new(Mutex::new(vec![alive])));

    let drained = drain_opencode_events(&tests::test_run(), &mut state, None).unwrap();

    assert!(!drained.prompt_completed);
    // The status poll ran, so the command was looked at and left alone.
    assert!(requests
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.starts_with("GET /session/status ")));
    assert!(drained
        .chunks
        .iter()
        .all(|chunk| chunk.kind != TerminalOutputKind::ProviderTool));
    assert_eq!(state.active_user_message_id.as_deref(), Some("user-1"));
    assert!(!requests
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.contains("/abort")));
    drop(state);
    stop.store(true, Ordering::Relaxed);
    let _ = server.join();
}
