use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use crate::provider::opencode_client::OpenCodeEventSubscription;
use crate::terminal::TerminalOutputKind;

use super::{drain_opencode_events, tests::test_run, OpenCodeRuntimeState};

#[test]
fn polling_retains_native_retry_reason_without_ending_the_turn() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 2048];
        let count = stream.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /session/status "));
        let body = r#"{"session-1":{"type":"retry","message":"Monthly subscription usage limit reached.","attempt":1,"next":1788528000000}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let (_sender, receiver) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        format!("http://{address}"),
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(receiver),
    );
    state.note_prompt_submitted("message-user".into());
    let result = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    server.join().unwrap();
    let status = result
        .chunks
        .iter()
        .find(|chunk| chunk.kind == TerminalOutputKind::ProviderStatus)
        .unwrap();
    let text = String::from_utf8_lossy(&status.bytes);
    assert!(
        text.contains("Monthly subscription usage limit reached."),
        "actual status: {text}"
    );
    assert!(
        !text.contains("connection interrupted"),
        "a subscription limit is not a network failure"
    );
    assert!(text.contains("Attempt 1."));
    assert!(text.contains("Next retry: 2026-09-04 13:20:00 UTC."));
    assert!(!result.prompt_completed);
    assert!(result.terminal_failure.is_none());
}

fn serve(responses: Vec<(&'static str, &'static str, String)>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        for (route, content_type, body) in responses {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "provider fixture request missing"
                        );
                        thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(error) => panic!("provider fixture accept: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            while !request.windows(2).any(|pair| pair == b"\r\n") {
                let mut bytes = [0; 1024];
                let count = stream.read(&mut bytes).unwrap();
                assert!(count > 0 && request.len() + count <= 8192);
                request.extend_from_slice(&bytes[..count]);
            }
            assert!(String::from_utf8_lossy(&request).starts_with(&format!("GET {route} ")));
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    (address, server)
}

#[test]
fn live_event_retains_the_same_retry_details_as_polling() {
    use crate::provider::opencode_client::{OpenCodeClient, OpenCodeEvent};
    let event = serde_json::json!({"type":"session.status","properties":{
        "sessionID":"session-1", "status":{"type":"retry","message":"Monthly subscription usage limit reached.","attempt":1,"next":1788528000000u64}
    }});
    let (address, server) = serve(vec![(
        "/event",
        "text/event-stream",
        format!("data: {event}\n\n"),
    )]);
    let client = OpenCodeClient::new("provider-run-1", address).unwrap();
    let subscription = client.subscribe_events().unwrap();
    let event = subscription
        .receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    server.join().unwrap();
    let OpenCodeEvent::SessionStatus { session_id, status } = event else {
        panic!("expected provider status")
    };
    assert_eq!(session_id, "session-1");
    assert_eq!(
        status.message.as_deref(),
        Some("Monthly subscription usage limit reached.")
    );
    assert_eq!(status.attempt, Some(1));
    assert_eq!(status.next, Some(1788528000000));
}

#[test]
fn reconnect_snapshot_preserves_retry_details() {
    use crate::provider::opencode_client::OpenCodeClient;
    let status = serde_json::json!({"session-1":{"type":"retry","message":"Temporarily unavailable.","attempt":2,"next":1788528000000u64}});
    let (address, server) = serve(vec![
        ("/session/status", "application/json", status.to_string()),
        (
            "/session/session-1/message",
            "application/json",
            "[]".into(),
        ),
    ]);
    let snapshot = OpenCodeClient::new("provider-run-1", address)
        .unwrap()
        .snapshot("session-1")
        .unwrap();
    server.join().unwrap();
    assert_eq!(snapshot.status.kind, "retry");
    assert_eq!(
        snapshot.status.message.as_deref(),
        Some("Temporarily unavailable.")
    );
    assert_eq!(snapshot.status.attempt, Some(2));
    assert_eq!(snapshot.status.next, Some(1788528000000));
    assert!(snapshot.messages.is_empty());
}

#[test]
fn changed_retry_details_update_the_shared_status_without_duplicate_output() {
    let first = serde_json::json!({"session-1":{"type":"retry","message":"Waiting for provider.","attempt":1}}).to_string();
    let next = serde_json::json!({"session-1":{"type":"retry","message":"Monthly subscription usage limit reached.","attempt":2}}).to_string();
    let (address, server) = serve(vec![
        ("/session/status", "application/json", first.clone()),
        ("/session/status", "application/json", first),
        ("/session/status", "application/json", next),
    ]);
    let (_sender, receiver) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        address,
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(receiver),
    );
    state.note_prompt_submitted("message-user".into());
    let initial = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    let duplicate = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    let changed = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    server.join().unwrap();
    assert_eq!(initial.chunks.len(), 1);
    assert!(duplicate.chunks.is_empty());
    assert_eq!(changed.chunks.len(), 1);
    assert!(String::from_utf8_lossy(&changed.chunks[0].bytes)
        .contains("Monthly subscription usage limit reached."));
    assert_eq!(initial.chunks[0].merge_key, changed.chunks[0].merge_key);
    assert_eq!(
        changed.chunks[0].merge_key.as_deref(),
        Some(crate::provider::PROVIDER_CONNECTION_RETRY_MERGE_KEY)
    );
    assert!(!changed.prompt_completed);
    assert!(changed.terminal_failure.is_none());
}

#[test]
fn retry_messages_are_bounded_and_invalid_dates_do_not_panic() {
    use crate::provider::opencode_client::OpenCodeClient;
    let status = serde_json::json!({"session-1":{"type":"retry","message":"\u{1b}\n\r".to_owned() + &"界".repeat(2000),"next":u64::MAX}});
    let (address, server) = serve(vec![(
        "/session/status",
        "application/json",
        status.to_string(),
    )]);
    let status = OpenCodeClient::new("provider-run-1", address)
        .unwrap()
        .session_status("session-1")
        .unwrap();
    server.join().unwrap();
    let message = status.message.as_ref().unwrap();
    assert_eq!(message.chars().count(), 500);
    assert!(!message.chars().any(char::is_control));
    let rendered = super::transcript::format_session_status(&status);
    assert!(!rendered.contains("Next retry:"));
}
