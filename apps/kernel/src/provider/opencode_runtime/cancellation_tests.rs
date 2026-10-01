use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn late_abort_error_does_not_fail_follow_up_with_old_snapshot_error() {
    check_session_error(false);
}

#[test]
fn reused_session_still_reports_a_correlated_current_prompt_error() {
    check_session_error(true);
}

fn check_session_error(current_error: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        for _ in 0..if current_error { 1 } else { 2 } {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = String::new();
            while !request.contains("\r\n\r\n") {
                let mut buf = [0; 1024];
                let size = stream.read(&mut buf).unwrap();
                assert!(size > 0);
                request.push_str(std::str::from_utf8(&buf[..size]).unwrap());
            }
            let response = if request.starts_with("GET /session/status ") {
                serde_json::json!({"session-1": {"type": "busy"}})
            } else {
                assert!(request.starts_with("GET /session/session-1/message "));
                serde_json::json!([{"info": {
                    "id": "assistant-error", "sessionID": "session-1", "role": "assistant",
                    "parentID": if current_error { "user-next" } else { "user-cancelled" },
                    "error": {"name": "APIError", "data": {"message": if current_error { "Current turn failed" } else { "Aborted" }}}
                }, "parts": []}])
            }.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    });
    let (tx, rx) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        endpoint,
        "session-1".to_string(),
        crate::provider::OpenCodeEventSubscription::for_tests(rx),
    );
    state.note_prompt_submitted("user-cancelled".to_string());
    state.settle_aborted_turn(&[]);
    state.note_prompt_submitted("user-next".to_string());
    tx.send(crate::provider::OpenCodeEvent::SessionError {
        session_id: "session-1".to_string(),
        message: "Aborted".to_string(),
    })
    .unwrap();
    let result = drain_opencode_events(&tests::test_run(), &mut state, None).unwrap();
    if current_error {
        assert_eq!(
            result.terminal_failure.as_deref(),
            Some("Current turn failed")
        );
        assert!(result.prompt_completed);
        assert!(result.explicit_provider_error);
        assert!(state.active_user_message_id.is_none());
    } else {
        assert!(result.terminal_failure.is_none());
        assert!(!result.prompt_completed);
        assert_eq!(state.active_user_message_id.as_deref(), Some("user-next"));
    }
    server.join().unwrap();
}
