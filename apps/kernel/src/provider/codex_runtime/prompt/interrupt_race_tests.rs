//! MP-08 / MP-10: provider RPC races must settle ordinary workflow controls.
use super::*;
use std::net::{Shutdown, TcpListener};
use std::thread;
use tokio_tungstenite::tungstenite::{accept, connect, Message};

#[derive(Clone, Copy)]
enum StartTiming {
    List,
    AfterList,
}

fn interrupt_fixture(first_error: &str, turns: Value) -> Result<(), DaemonError> {
    interrupt_fixture_with_event(first_error, turns, StartTiming::AfterList, "actual", false)
}

fn interrupt_fixture_with_event(
    first_error: &str,
    turns: Value,
    start_timing: StartTiming,
    retry_turn_id: &str,
    assert_waits_for_start: bool,
) -> Result<(), DaemonError> {
    let started_during_read = matches!(start_timing, StartTiming::List);
    let expect_retry = started_during_read
        || assert_waits_for_start
        || turns["data"]
            .as_array()
            .is_some_and(|turns| turns.iter().any(|turn| turn["status"] == "inProgress"))
        || (first_error.starts_with("expected active turn id ")
            && turns["data"]
                .as_array()
                .is_some_and(|turns| !turns.iter().any(|turn| turn["id"] == "submitted")));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let first_error = first_error.to_string();
    let retry_turn_id = retry_turn_id.to_string();
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
            if method == "thread/backgroundTerminals/clean" {
                assert_eq!(request["params"], json!({"threadId":"thread"}));
                socket
                    .send(Message::Text(
                        json!({"id":request["id"],"result":{}}).to_string().into(),
                    ))
                    .unwrap();
                continue;
            }
            methods.push(method.to_string());
            let response = match (method, methods.len()) {
                ("turn/interrupt", 1) => {
                    assert_eq!(request["params"]["turnId"], "submitted");
                    json!({"error":{"code":-32600,"message":first_error}})
                }
                ("thread/turns/list", 2) => {
                    if (!expect_retry || started_during_read)
                        && turns["data"].as_array().is_some_and(|turns| {
                            turns.iter().any(|turn| {
                                turn["id"] == "submitted" && turn["status"] == "interrupted"
                            })
                        })
                    {
                        socket.send(Message::Text(json!({"method":"turn/completed","params":{"turn":{"id":"submitted","status":"interrupted","items":[]}}}).to_string().into())).unwrap();
                    }
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
                    assert_eq!(request["params"]["turnId"], retry_turn_id);
                    json!({"result":{}})
                }
                _ => panic!("unexpected interrupt race RPC: {methods:?}"),
            };
            let mut response = response;
            response["id"] = request["id"].clone();
            socket
                .send(Message::Text(response.to_string().into()))
                .unwrap();
            if method == "thread/turns/list"
                && expect_retry
                && (!started_during_read || retry_turn_id != "actual")
            {
                if assert_waits_for_start {
                    socket
                        .get_ref()
                        .set_read_timeout(Some(Duration::from_millis(50)))
                        .unwrap();
                    assert!(socket.read().is_err(), "interrupt retry must wait for provider turn/started, not just the snapshot");
                    socket
                        .get_ref()
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                }
                socket
                    .send(Message::Text(
                        json!({"method":"turn/started","params":{"turn":{"id":retry_turn_id}}})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
            }
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

// MP-08 / MP-10: retain the snapshot ordering/admission contract in a table.
#[test]
fn mp08_interrupt_snapshot_admission_matrix() {
    let stale = "expected active turn id submitted but found actual";
    for (error, records, timing, target, waits) in [
        (
            stale,
            json!([{"id":"submitted","status":"interrupted"},{"id":"actual","status":"inProgress"}]),
            StartTiming::AfterList,
            "actual",
            false,
        ),
        (
            "no active turn to interrupt",
            json!([{"id":"submitted","status":"completed"},{"id":"actual","status":"inProgress"}]),
            StartTiming::AfterList,
            "actual",
            false,
        ),
        (
            "no active turn to interrupt",
            json!([{"id":"submitted","status":"interrupted"}]),
            StartTiming::AfterList,
            "actual",
            false,
        ),
        (
            stale,
            json!([{"id":"actual","status":"interrupted"},{"id":"submitted","status":"inProgress"}]),
            StartTiming::AfterList,
            "submitted",
            false,
        ),
        (
            stale,
            json!([{"id":"submitted","status":"inProgress"},{"id":"actual","status":"inProgress"}]),
            StartTiming::AfterList,
            "submitted",
            false,
        ),
        (
            stale,
            json!([{"id":"submitted","status":"inProgress"},{"id":"actual","status":"inProgress"}]),
            StartTiming::List,
            "submitted",
            false,
        ),
        (
            stale,
            json!([{"id":"submitted","status":"inProgress"},{"id":"actual","status":"interrupted"}]),
            StartTiming::AfterList,
            "submitted",
            true,
        ),
        (
            stale,
            json!([{"id":"actual","status":"interrupted"}]),
            StartTiming::AfterList,
            "submitted",
            true,
        ),
        (
            stale,
            json!([{"id":"submitted","status":"interrupted"},{"id":"actual","status":"interrupted"}]),
            StartTiming::AfterList,
            "submitted",
            true,
        ),
        (
            "no active turn to interrupt",
            json!([{"id":"submitted","status":"interrupted"}]),
            StartTiming::AfterList,
            "submitted",
            true,
        ),
    ] {
        interrupt_fixture_with_event(error, json!({"data": records}), timing, target, waits)
            .unwrap();
    }
}

#[test]
fn mp08_interrupt_unrelated_provider_failure_is_not_suppressed() {
    let error = interrupt_fixture("permission denied", json!({"data":[]})).unwrap_err();
    assert!(error.to_string().contains("permission denied"));
}

// MP-08 / MP-10 round4: the lifecycle arrives through read_notification,
// after the list RPC has returned its stale submitted identity.
#[test]
fn mp08_interrupt_after_list_start_reconciles_before_submitted_completion() {
    for completed in [false, true] {
        round4_after_list_fixture(completed);
    }
}

fn round4_after_list_fixture(completed: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(6)))
            .unwrap();
        let mut socket = accept(stream).unwrap();
        let mut interrupted_actual = false;
        while let Ok(Message::Text(raw)) = socket.read() {
            let request: Value = serde_json::from_str(&raw).unwrap();
            let response = match request["method"].as_str().unwrap() {
                "turn/interrupt" if request["params"]["turnId"] == "submitted" => {
                    json!({"error":{"code":-32600,"message":"expected active turn id submitted but found actual"}})
                }
                "thread/turns/list" => {
                    json!({"result":{"data":[{"id":"submitted","status":"inProgress"}]}})
                }
                "turn/interrupt" => {
                    assert_eq!(request["params"]["turnId"], "actual");
                    interrupted_actual = true;
                    json!({"result":{}})
                }
                "thread/backgroundTerminals/clean" => json!({"result":{}}),
                _ => panic!("unexpected RPC"),
            };
            let mut response = response;
            response["id"] = request["id"].clone();
            socket
                .send(Message::Text(response.to_string().into()))
                .unwrap();
            if request["method"] == "thread/turns/list" {
                socket
                    .send(Message::Text(
                        json!({"method":"turn/started","params":{"turn":{"id":"actual"}}})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
                if completed {
                    socket.send(Message::Text(json!({"method":"turn/completed","params":{"turn":{"id":"submitted","status":"interrupted","items":[]}}}).to_string().into())).unwrap();
                }
            }
        }
        interrupted_actual
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
    assert!(
        server.join().unwrap(),
        "MP-08 actual must be interrupted (submitted completion={completed}, result={result:?})"
    );
    result.unwrap();
}
