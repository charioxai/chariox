//! Row 305: OpenCode answers the free model's HTTP 429 `FreeUsageLimitError` with a
//! `retry` session status that carries `action.reason = "free_tier_limit"` and a retry
//! hours away, then stays silent (no session error, no idle). The turn must end as a
//! usage limit, with a plain reason, and the OpenCode session must stop retrying.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use crate::provider::opencode_client::{
    OpenCodeEvent, OpenCodeEventSubscription, OpenCodeSessionStatus,
};
use crate::provider::{TurnOutcome, TurnOutcomeKind};

use super::{drain_opencode_events, tests::test_run, OpenCodeRuntimeState};

/// The retry status the real OpenCode 1.19.0 server reports for the free-model 429
/// (recorded against a local 429 stand-in behind the real binary, 2026-11-22).
const FREE_LIMIT_STATUS: &str = r#"{"type":"retry","attempt":1,"message":"Free usage exceeded, subscribe to Go","action":{"reason":"free_tier_limit","provider":"opencode","title":"Free limit reached","message":"Subscribe to OpenCode Go for reliable access to the best open-source models for $10/month.","label":"subscribe","link":"https://opencode.ai/go"},"next":1795392083138}"#;

/// Answers `/session/status` with the given body and records every request line.
fn serve_status(body: String) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 4096];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut bytes) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => request.extend_from_slice(&bytes[..count]),
                }
            }
            let line = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            let reply = if line.starts_with("GET /session/status ") {
                body.clone()
            } else if line.contains("/message") {
                "[]".to_string()
            } else {
                "true".to_string()
            };
            seen.lock().unwrap().push(line);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.len(),
                reply
            );
        }
    });
    (address, requests)
}

fn assert_free_limit_ended_the_turn(
    result: &super::state::OpenCodeEventDrainResult,
    requests: &Mutex<Vec<String>>,
) {
    assert!(result.prompt_completed, "the turn must end");
    let failure = result
        .terminal_failure
        .as_deref()
        .expect("the free-limit refusal is a terminal failure");
    let outcome = TurnOutcome::classify("opencode", failure);
    assert_eq!(outcome.kind, TurnOutcomeKind::UsageLimit);
    assert!(
        !outcome.retryable,
        "the free limit does not clear by waiting a minute"
    );
    assert!(
        outcome
            .reason
            .to_lowercase()
            .contains("free usage limit reached"),
        "plain state, got {:?}",
        outcome.reason
    );
    assert!(
        !outcome.reason.contains("http") && !outcome.reason.contains("429"),
        "no technical text for the person: {:?}",
        outcome.reason
    );
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("POST /session/session-1/abort ")),
        "the OpenCode session must stop retrying: {:?}",
        requests.lock().unwrap()
    );
}

#[test]
fn polling_a_free_tier_limit_retry_ends_the_turn_as_a_usage_limit() {
    let body = format!(r#"{{"session-1":{FREE_LIMIT_STATUS}}}"#);
    let (address, requests) = serve_status(body);
    let (_sender, receiver) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        address,
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(receiver),
    );
    state.note_prompt_submitted("message-user".into());
    let result = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    assert_free_limit_ended_the_turn(&result, &requests);
}

#[test]
fn a_live_free_tier_limit_retry_event_ends_the_turn_as_a_usage_limit() {
    let (address, requests) = serve_status("{}".to_string());
    let (sender, receiver) = mpsc::channel();
    let status: OpenCodeSessionStatus = serde_json::from_str(FREE_LIMIT_STATUS).unwrap();
    sender
        .send(OpenCodeEvent::SessionStatus {
            session_id: "session-1".into(),
            status,
        })
        .unwrap();
    let mut state = OpenCodeRuntimeState::new(
        address,
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(receiver),
    );
    state.note_prompt_submitted("message-user".into());
    let result = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    assert_free_limit_ended_the_turn(&result, &requests);
}

#[test]
fn an_ordinary_retry_status_still_keeps_the_turn_open() {
    let body = r#"{"session-1":{"type":"retry","attempt":1,"message":"Temporarily unavailable.","next":1788528000000}}"#;
    let (address, requests) = serve_status(body.to_string());
    let (_sender, receiver) = mpsc::channel();
    let mut state = OpenCodeRuntimeState::new(
        address,
        "session-1".into(),
        OpenCodeEventSubscription::for_tests(receiver),
    );
    state.note_prompt_submitted("message-user".into());
    let result = drain_opencode_events(&test_run(), &mut state, None).unwrap();
    assert!(!result.prompt_completed);
    assert!(result.terminal_failure.is_none());
    assert!(!requests
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.contains("/abort")));
}
