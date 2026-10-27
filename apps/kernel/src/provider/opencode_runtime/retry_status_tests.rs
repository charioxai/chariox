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

// MP-08 / MP-10 / MP-11: an open SSE socket does not prove HTTP liveness.
#[test]
fn exhausted_status_poll_error_does_not_leave_an_active_turn_running() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // Reserve an owned address, then close its listener so all bounded native
    // HTTP transport retries fail while the event channel remains connected.
    drop(listener);
    let (_sender, receiver) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        format!("http://{address}"),
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(receiver),
    );
    state.note_prompt_submitted("message-user".into());
    let error = drain_opencode_events(&test_run(), &mut state, None)
        .err()
        .expect("exhausted native status errors must reach the run actor");
    assert!(error.to_string().contains("session_status"), "{error}");
}

#[test]
fn status_failure_preserves_drained_text_and_tool_output_through_http_recovery() {
    use crate::provider::opencode_client::OpenCodeEvent;
    use crate::provider::OpenCodeMessage;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let (sender, receiver) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        format!("http://{address}"),
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(receiver),
    );
    state.note_prompt_submitted("message-user".into());
    let message: OpenCodeMessage = serde_json::from_value(serde_json::json!({
        "info": {"id":"assistant-1", "sessionID":"session-1", "role":"assistant", "parentID":"message-user", "finish":"stop", "time":{"completed":1}},
        "parts": [
            {"id":"text-1", "sessionID":"session-1", "messageID":"assistant-1", "type":"text", "text":"already streamed answer"},
            {"id":"tool-1", "sessionID":"session-1", "messageID":"assistant-1", "type":"tool", "tool":"bash", "state":{"status":"completed", "input":{"command":"git status"}, "output":"already streamed tool result"}}
        ]
    })).unwrap();
    sender
        .send(OpenCodeEvent::MessageUpdated {
            info: message.info.clone(),
        })
        .unwrap();
    for part in &message.parts {
        sender
            .send(OpenCodeEvent::MessagePartUpdated {
                part: Box::new(part.clone()),
            })
            .unwrap();
    }
    let mut first = drain_opencode_events(&test_run(), &mut state, None)
        .expect("failed status HTTP must not discard consumed SSE output");
    // Ready output may be split at the bounded drain's render budget.
    for _ in 0..message.parts.len() {
        if first
            .chunks
            .iter()
            .any(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
        {
            break;
        }
        let next = drain_opencode_events(&test_run(), &mut state, None)
            .expect("remaining ready SSE output must be delivered before quiet HTTP polling");
        assert!(!next.prompt_completed);
        first.chunks.extend(next.chunks);
        first.completions.extend(next.completions);
        first.notices.extend(next.notices);
    }
    assert!(!first.prompt_completed);
    assert!(
        first.notices.is_empty(),
        "ready SSE output must not wait for a speculative HTTP poll"
    );
    assert_eq!(
        first
            .chunks
            .iter()
            .filter(|c| c.kind == TerminalOutputKind::ProviderOutput)
            .count(),
        1
    );
    assert_eq!(
        first
            .chunks
            .iter()
            .filter(|c| c.kind == TerminalOutputKind::ProviderTool)
            .count(),
        1
    );
    assert_eq!(
        first.completions.len(),
        1,
        "consumed completion metadata must also reach the actor"
    );
    // MP-08 / MP-10 / MP-11: the following quiet drain exposes the HTTP failure.
    let error = drain_opencode_events(&test_run(), &mut state, None)
        .err()
        .expect("quiet HTTP failure must remain visible after ready output is delivered");
    assert!(error.to_string().contains("session_status"), "{error}");
    let body = serde_json::json!([{
        "info": {"id":"assistant-1", "sessionID":"session-1", "role":"assistant", "parentID":"message-user", "finish":"stop", "time":{"completed":1}},
        "parts": [
            {"id":"text-1", "sessionID":"session-1", "messageID":"assistant-1", "type":"text", "text":"already streamed answer"},
            {"id":"tool-1", "sessionID":"session-1", "messageID":"assistant-1", "type":"tool", "tool":"bash", "state":{"status":"completed", "input":{"command":"git status"}, "output":"already streamed tool result"}}
        ]
    }]).to_string();
    let (recovered, server) = serve(vec![
        (
            "/session/status",
            "application/json",
            r#"{"session-1":{"type":"idle"}}"#.into(),
        ),
        ("/session/session-1/message", "application/json", body),
    ]);
    state.base_url = recovered;
    let next = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    server.join().unwrap();
    assert!(next.prompt_completed);
    assert!(
        next.completions.is_empty(),
        "published completion must not replay after recovery"
    );
    assert!(!next.chunks.iter().any(|c| matches!(
        c.kind,
        TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderTool
    )));
    let replay = super::snapshot::render_snapshot_output_chunks(
        &mut state,
        test_run().remote_extension_manifest(),
        &[message],
    );
    assert!(
        replay.chunks.is_empty(),
        "recovered snapshots must not replay delivered output"
    );
}
