use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::provider::opencode_client::OpenCodeEventSubscription;

use super::{drain_opencode_events, tests::test_run, OpenCodeRuntimeState};

// Answers each request by its path and returns the request lines it saw.
fn serve(
    routes: Vec<(&'static str, String)>,
    requests: usize,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while seen.len() < requests && std::time::Instant::now() < deadline {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) => panic!("provider fixture accept: {error}"),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let mut bytes = [0; 1024];
                let count = stream.read(&mut bytes).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&bytes[..count]);
            }
            let line = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap()
                .to_string();
            let body = routes
                .iter()
                .find(|(path, _)| line.contains(&format!(" {path} ")))
                .map(|(_, body)| body.clone())
                .unwrap_or_else(|| "true".to_string());
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            seen.push(line);
        }
        seen
    });
    (address, server)
}

// The sender keeps the event stream connected for the drain.
fn busy_state(
    address: String,
) -> (
    OpenCodeRuntimeState,
    mpsc::Sender<crate::provider::OpenCodeEvent>,
) {
    let (sender, receiver) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        address,
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(receiver),
    );
    state.note_prompt_submitted("message-user".into());
    state.backdate_active_prompt_for_tests(Duration::from_secs(181));
    (state, sender)
}

const BUSY: &str = r#"{"session-1":{"type":"busy"}}"#;

const USER_ONLY: &str =
    r#"[{"info":{"id":"message-user","sessionID":"session-1","role":"user"},"parts":[]}]"#;

#[test]
fn busy_turn_without_an_assistant_message_fails_visibly_after_the_timeout() {
    assert_unstarted_turn_fails(BUSY);
}

#[test]
fn unreadable_status_without_an_assistant_message_fails_visibly_after_the_timeout() {
    assert_unstarted_turn_fails("not a session status");
}

fn assert_unstarted_turn_fails(status: &str) {
    let (address, server) = serve(
        vec![
            ("/session/status", status.to_string()),
            ("/session/session-1/message", USER_ONLY.to_string()),
        ],
        3,
    );
    let (mut state, _events) = busy_state(address);
    let drained = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    let requests = server.join().unwrap();

    assert!(drained.prompt_completed);
    assert!(drained
        .terminal_failure
        .as_deref()
        .is_some_and(|failure| failure.contains("did not start a response")));
    assert!(requests
        .iter()
        .any(|line| line.starts_with("POST /session/session-1/abort ")));
}

#[test]
fn busy_turn_with_a_started_assistant_message_keeps_running() {
    let started = r#"[{"info":{"id":"message-user","sessionID":"session-1","role":"user"},"parts":[]},
        {"info":{"id":"message-assistant","sessionID":"session-1","role":"assistant","parentID":"message-user"},"parts":[]}]"#;
    let (address, server) = serve(
        vec![
            ("/session/status", BUSY.to_string()),
            ("/session/session-1/message", started.to_string()),
        ],
        2,
    );
    let (mut state, _events) = busy_state(address);
    let drained = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    server.join().unwrap();

    assert!(!drained.prompt_completed);
    assert!(drained.terminal_failure.is_none());
    // Later polls trust the observed assistant and do not refetch messages.
    assert!(state.active_prompt_has_assistant());
}
