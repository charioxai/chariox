//! MP-08 / MP-10: provider RPC races must settle ordinary workflow controls.
use super::*;
use std::net::{Shutdown, TcpListener};
use std::thread;
use tokio_tungstenite::tungstenite::{accept, connect, Message};

#[derive(Clone, Copy)]
enum StartTiming {
    Interrupt,
    List,
    AfterList,
}

fn interrupt_fixture(
    first_error: &str,
    turns: Value,
    retry_error: Option<&str>,
) -> Result<(), DaemonError> {
    interrupt_fixture_with_event(
        first_error,
        turns,
        retry_error,
        StartTiming::AfterList,
        "actual",
        false,
    )
}

fn interrupt_fixture_with_event(
    first_error: &str,
    turns: Value,
    retry_error: Option<&str>,
    start_timing: StartTiming,
    retry_turn_id: &str,
    assert_waits_for_start: bool,
) -> Result<(), DaemonError> {
    let started_during_interrupt = matches!(start_timing, StartTiming::Interrupt);
    let started_during_read = matches!(start_timing, StartTiming::List);
    let expect_retry = started_during_interrupt
        || started_during_read
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
    let retry_error = retry_error.map(str::to_string);
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
            methods.push(method.to_string());
            let response = match (method, methods.len()) {
                ("turn/interrupt", 1) => {
                    assert_eq!(request["params"]["turnId"], "submitted");
                    if started_during_interrupt {
                        if turns["data"].as_array().is_some_and(|turns| {
                            turns.iter().any(|turn| {
                                turn["id"] == "submitted" && turn["status"] == "interrupted"
                            })
                        }) {
                            socket.send(Message::Text(json!({"method":"turn/completed","params":{"turn":{"id":"submitted","status":"interrupted","items":[]}}}).to_string().into())).unwrap();
                        }
                        socket.send(Message::Text(json!({"method":"turn/started","params":{"turn":{"id":"actual"}}}).to_string().into())).unwrap();
                    }
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
            if method == "thread/turns/list"
                && expect_retry
                && (!(started_during_interrupt || started_during_read) || retry_turn_id != "actual")
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
fn mp08_interrupt_reread_supersedes_a_stale_rejection_identity() {
    interrupt_fixture_with_event(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"actual","status":"interrupted"},{"id":"submitted","status":"inProgress"}]}),
        None,
        StartTiming::AfterList,
        "submitted",
        false,
    ).unwrap();
}

#[test]
fn mp08_interrupt_fresh_start_event_supersedes_a_stale_snapshot() {
    interrupt_fixture_with_event(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"submitted","status":"inProgress"}]}),
        None,
        StartTiming::List,
        "actual",
        false,
    )
    .unwrap();
}

#[test]
fn mp08_interrupt_fresh_start_supersedes_a_completed_submitted_snapshot() {
    for error in [
        "expected active turn id submitted but found actual",
        "no active turn to interrupt",
    ] {
        interrupt_fixture_with_event(
            error,
            json!({"data":[{"id":"submitted","status":"completed"}]}),
            None,
            StartTiming::List,
            "actual",
            false,
        )
        .expect("the fresh actual turn must be interrupted before settling cancellation");
    }
}

#[test]
fn mp08_interrupt_fresh_start_supersedes_an_interrupted_submitted_snapshot() {
    for error in [
        "expected active turn id submitted but found actual",
        "no active turn to interrupt",
    ] {
        // The fixture buffers submitted's completion before actual's fresh start.
        interrupt_fixture_with_event(
            error,
            json!({"data":[{"id":"submitted","status":"interrupted"}]}),
            None,
            StartTiming::List,
            "actual",
            false,
        )
        .expect("submitted's completion must not hide the fresh actual turn");
    }
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

#[test]
fn mp08_interrupt_newest_active_record_supersedes_older_in_progress_history() {
    interrupt_fixture_with_event(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"submitted","status":"inProgress"},{"id":"actual","status":"inProgress"}]}),
        None,
        StartTiming::AfterList,
        "submitted",
        false,
    ).unwrap();
}

#[test]
fn mp08_interrupt_retry_waits_for_provider_start_after_admitted_snapshot() {
    interrupt_fixture_with_event(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"submitted","status":"inProgress"},{"id":"actual","status":"interrupted"}]}),
        None,
        StartTiming::AfterList,
        "submitted",
        true,
    ).unwrap();
}

#[test]
fn mp08_interrupt_late_old_start_does_not_override_the_current_snapshot() {
    interrupt_fixture_with_event(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"submitted","status":"inProgress"},{"id":"actual","status":"inProgress"}]}),
        None,
        StartTiming::List,
        "submitted",
        false,
    ).unwrap();
}

#[test]
fn mp08_interrupt_missing_admitted_record_waits_for_its_real_start() {
    interrupt_fixture_with_event(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"actual","status":"interrupted"}]}),
        None,
        StartTiming::AfterList,
        "submitted",
        true,
    )
    .unwrap();
}

#[test]
fn mp08_interrupt_normalized_interrupted_snapshot_still_cancels_queued_turn() {
    interrupt_fixture_with_event(
        "expected active turn id submitted but found actual",
        json!({"data":[{"id":"submitted","status":"interrupted"},{"id":"actual","status":"interrupted"}]}),
        None, StartTiming::AfterList, "submitted", true,
    ).unwrap();
}

#[test]
fn mp08_interrupt_no_active_does_not_settle_a_normalized_unstarted_turn() {
    interrupt_fixture_with_event(
        "no active turn to interrupt",
        json!({"data":[{"id":"submitted","status":"interrupted"}]}),
        None,
        StartTiming::AfterList,
        "submitted",
        true,
    )
    .unwrap();
}

#[test]
fn mp08_interrupt_start_during_interrupt_supersedes_completed_snapshot() {
    for error in [
        "expected active turn id submitted but found actual",
        "no active turn to interrupt",
    ] {
        interrupt_fixture_with_event(
            error,
            json!({"data":[{"id":"submitted","status":"completed"}]}),
            None,
            StartTiming::Interrupt,
            "actual",
            false,
        )
        .expect("the start buffered by turn/interrupt must prevent terminal settlement");
    }
}

#[test]
fn mp08_interrupt_start_during_interrupt_supersedes_interrupted_snapshot() {
    for error in [
        "expected active turn id submitted but found actual",
        "no active turn to interrupt",
    ] {
        // Both submitted's completion and actual's start precede the interrupt error.
        // The list RPC sends no lifecycle events and omits the actual turn entirely.
        interrupt_fixture_with_event(
            error,
            json!({"data":[{"id":"submitted","status":"interrupted"}]}),
            None,
            StartTiming::Interrupt,
            "actual",
            false,
        )
        .expect("the interrupt's buffered completion must not hide the fresh actual start");
    }
}
