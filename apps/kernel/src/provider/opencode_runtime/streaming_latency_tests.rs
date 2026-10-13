//! MP-08 / MP-10: SSE text must reach clients before HTTP reconciliation.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use crate::provider::{OpenCodeEvent, OpenCodeEventSubscription};
use crate::terminal::TerminalOutputKind;

use super::{drain_opencode_events, tests::test_run, OpenCodeRuntimeState};

#[test]
fn panes_stream_opencode_busy_status_does_not_hold_ready_text_for_http() {
    check_ready_text(true);
}

#[test]
fn panes_stream_opencode_idle_reconciliation_follows_ready_text() {
    check_ready_text(false);
}

fn check_ready_text(busy_before_text: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let (stop, stopped) = mpsc::channel();
    let server = std::thread::spawn(move || {
        while stopped.try_recv().is_err() {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let size = stream.read(&mut request).unwrap();
                    let line = String::from_utf8_lossy(&request[..size])
                        .lines()
                        .next()
                        .unwrap()
                        .to_string();
                    let body = if line.starts_with("GET /session/status ") {
                        r#"{"session-1":{"type":"busy"}}"#
                    } else {
                        assert!(line.starts_with("GET /session/session-1/message "));
                        "[]"
                    };
                    seen.lock().unwrap().push(line);
                    // Reproduce a slow official-server reconciliation response.
                    std::thread::sleep(Duration::from_millis(350));
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("{error}"),
            }
        }
    });
    let (tx, rx) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        address,
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(rx),
    );
    state.note_prompt_submitted("user-1".into());
    state.live_text_from_start = true;
    state
        .message_roles
        .insert("assistant-1".into(), "assistant".into());
    state
        .message_parent_ids
        .insert("assistant-1".into(), Some("user-1".into()));
    state.part_kinds.insert("text-1".into(), "text".into());
    state
        .emitted_text_by_part
        .insert("text-1".into(), String::new());
    if busy_before_text {
        tx.send(OpenCodeEvent::SessionStatus {
            session_id: "session-1".into(),
            status: "busy".into(),
        })
        .unwrap();
    }
    tx.send(OpenCodeEvent::MessagePartDelta {
        session_id: "session-1".into(),
        message_id: "assistant-1".into(),
        part_id: "text-1".into(),
        field: "text".into(),
        delta: "CHECK:".into(),
    })
    .unwrap();
    if !busy_before_text {
        tx.send(OpenCodeEvent::SessionStatus {
            session_id: "session-1".into(),
            status: "idle".into(),
        })
        .unwrap();
    }
    let started = std::time::Instant::now();
    let result = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    let elapsed = started.elapsed();
    stop.send(()).unwrap();
    server.join().unwrap();
    assert!(result
        .chunks
        .iter()
        .any(|chunk| chunk.kind == TerminalOutputKind::ProviderOutput && chunk.bytes == b"CHECK:"));
    assert!(!result.prompt_completed);
    let requests = requests.lock().unwrap().clone();
    assert!(
        requests.is_empty(),
        "ready SSE text was held {elapsed:?} by reconciliation requests: {requests:?}"
    );
    if !busy_before_text {
        assert!(matches!(state.pending_event.as_ref(),
            Some(OpenCodeEvent::SessionStatus { status, .. }) if status.kind == "idle"));
        // A cancelled turn must not leave its deferred status for a successor.
        state.settle_aborted_turn(&[]);
        assert!(state.pending_event.is_none());
    }
}
