//! MP-08 / MP-10: provider RPC races must settle ordinary workflow controls.
use super::*;
use std::net::{Shutdown, TcpListener};
use std::thread;
use tokio_tungstenite::tungstenite::{accept, connect, Message};

fn interrupt_fixture(
    first_error: &str,
    turns: Value,
    retry_error: Option<&str>,
) -> Result<(), DaemonError> {
    interrupt_fixture_with_event(first_error, turns, retry_error, false)
}

fn interrupt_fixture_with_event(
    first_error: &str,
    turns: Value,
    retry_error: Option<&str>,
    started_during_read: bool,
) -> Result<(), DaemonError> {
    let expect_retry = turns["data"]
        .as_array()
        .is_some_and(|turns| turns.iter().any(|turn| turn["status"] == "inProgress"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let first_error = first_error.to_string();
    let retry_error = retry_error.map(str::to_string);
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut socket = accept(stream).unwrap();
        let mut methods = Vec::new();
        while let Ok(Message::Text(raw)) = socket.read() {
            let request: Value = serde_json::from_str(&raw).unwrap();
            let method = request["method"].as_str().unwrap();
            methods.push(method.to_string());
            let response = match (method, methods.len()) {
                ("turn/interrupt", 1) => {
                    assert_eq!(request["params"]["turnId"], "submitted");
                    json!({"error":{"code":-32600,"message":first_error}})
                }
                ("thread/turns/list", 2) => {
                    if started_during_read {
                        socket
                            .send(Message::Text(
                                json!({"method":"turn/started","params":{"turn":{"id":"actual"}}})
                                    .to_string()
                                    .into(),
                            ))
                            .unwrap();
                    }
                    json!({"result":turns})
                }
                ("turn/interrupt", 3) => {
                    assert_eq!(request["params"]["turnId"], "actual");
                    match &retry_error {
                        Some(error) => json!({"error":{"code":-32600,"message":error}}),
                        None => json!({"result":{}}),
                    }
                }
                _ => panic!("unexpected interrupt race RPC: {methods:?}"),
            };
            let mut response = response;
            response["id"] = request["id"].clone();
            socket
                .send(Message::Text(response.to_string().into()))
                .unwrap();
        }
        methods
    });
    let (socket, _) = connect(&endpoint).unwrap();
    let tokio_tungstenite::tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref()
    else {
        unreachable!()
    };
    let close = stream.try_clone().unwrap();
    let mut state = CodexRuntimeState::new(endpoint, "thread".into(), socket, 1);
    state.active_turn_id = Some("submitted".into());
    let result = abort_codex_turn("run", &mut state);
    close.shutdown(Shutdown::Both).unwrap();
    let methods = server.join().unwrap();
    assert!(methods.len() <= 3, "stale-ID retry must be bounded");
    if result.is_ok() && expect_retry {
        assert_eq!(
            methods.len(),
            3,
            "the actual active turn must be interrupted"
        );
    }
    if result.is_ok() {
        assert!(state.active_turn_id.is_none());
    }
    result
}

#[test]
fn mp08_interrupt_stale_id_reads_actual_turn_and_retries_once() {
    interrupt_fixture(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"submitted","status":"interrupted"},{"id":"actual","status":"inProgress"}]}),
        None,
    ).expect("a recoverable provider turn ID race must not fail the workflow");
}

#[test]
fn mp08_interrupt_no_active_rechecks_new_turn_before_settling_old_turn() {
    interrupt_fixture(
        "no active turn to interrupt",
        json!({"data":[{"id":"submitted","status":"completed"},{"id":"actual","status":"inProgress"}]}),
        None,
    ).unwrap();
}

#[test]
fn mp08_interrupt_already_interrupted_turn_is_settled() {
    interrupt_fixture(
        "no active turn to interrupt",
        json!({"data":[{"id":"submitted","status":"interrupted"}]}),
        None,
    )
    .unwrap();
}

#[test]
fn mp08_interrupt_second_stale_id_remains_an_error() {
    let error = interrupt_fixture(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"actual","status":"inProgress"}]}),
        Some("expected active turn id actual but found third"),
    )
    .expect_err("only one stale-ID retry is allowed");
    assert!(error.to_string().contains("third"));
}

#[test]
fn mp08_interrupt_unrelated_provider_failure_is_not_suppressed() {
    let error = interrupt_fixture("permission denied", json!({"data":[]}), None).unwrap_err();
    assert!(error.to_string().contains("permission denied"));
}

#[test]
fn mp08_interrupt_listener_id_can_lag_the_core_snapshot() {
    interrupt_fixture(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"actual","status":"interrupted"},{"id":"submitted","status":"inProgress"}]}),
        None,
    ).unwrap();
}

#[test]
fn mp08_interrupt_fresh_start_event_supersedes_a_stale_snapshot() {
    interrupt_fixture_with_event(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"submitted","status":"inProgress"}]}),
        None,
        true,
    )
    .unwrap();
}

#[test]
fn mp08_interrupt_uses_the_buffered_provider_start_identity() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut socket = accept(stream).unwrap();
        let raw = socket.read().unwrap();
        let request: Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
        assert_eq!(request["method"], "turn/interrupt");
        assert_eq!(request["params"]["turnId"], "actual");
        socket
            .send(Message::Text(
                json!({"id":request["id"],"result":{}}).to_string().into(),
            ))
            .unwrap();
    });
    let (socket, _) = connect(&endpoint).unwrap();
    let mut state = CodexRuntimeState::new(endpoint, "thread".into(), socket, 1);
    state.active_turn_id = Some("submitted".into());
    state
        .buffered_notifications
        .push(CodexNotification::TurnStarted {
            turn_id: "actual".into(),
        });
    abort_codex_turn("run", &mut state).unwrap();
    assert!(state.active_turn_id.is_none());
    server.join().unwrap();
}
