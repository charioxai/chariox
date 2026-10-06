//! MP-08 / MP-10: generated lifecycle and RPC cancellation interleavings.
use super::*;
use std::net::{Shutdown, TcpListener};
use std::thread;
use tokio_tungstenite::tungstenite::{accept, connect, Message};

// MP-08 / MP-10: generated socket interleavings, not per-window implementations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    BeforeRpc,
    Interrupt,
    List,
    StartWait,
    Ack,
    AfterAck,
}

#[derive(Clone, Copy, Debug)]
enum RpcResult {
    Stale,
    NoActive,
    Accepted,
}

#[derive(Debug)]
struct RaceCase {
    start: Phase,
    submitted_completion: Option<Phase>,
    actual_completion: Option<Phase>,
    completion_first: bool,
    completion_status: &'static str,
    snapshot_status: &'static str,
    result: RpcResult,
    previous_tracker: bool,
    delayed_start: bool,
    replayed_submitted_start: bool,
}

fn generated_interrupt_fixture(case: RaceCase) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let before_events = race_events(&case, Phase::BeforeRpc);
    let label = format!("MP-08 {case:?}");
    let case_previous_tracker = case.previous_tracker;
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_nodelay(true).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(6)))
            .unwrap();
        let mut socket = accept(stream).unwrap();
        let mut active_actual = false;
        let mut actual_started = false;
        let mut actual_interrupts = 0;
        let mut calls = 0;
        let mut emit = |socket: &mut tokio_tungstenite::tungstenite::WebSocket<
            std::net::TcpStream,
        >,
                        phase| {
            for event in race_events(&case, phase) {
                if event["method"] == "turn/started" && event["params"]["turn"]["id"] == "actual" {
                    active_actual = true;
                    actual_started = true;
                } else if event["method"] == "turn/completed"
                    && event["params"]["turn"]["id"] == "actual"
                {
                    active_actual = false;
                }
                socket
                    .write(Message::Text(event.to_string().into()))
                    .unwrap();
            }
            if phase == Phase::List && case.snapshot_status == "actual-completed" {
                active_actual = false;
            }
        };
        // Buffered events are also replayed on this fixture socket so the
        // provider oracle sees their exact order. Repeated identity observations
        // must be harmless to the runtime reconciliation.
        emit(&mut socket, Phase::BeforeRpc);
        socket.flush().unwrap();
        while let Ok(Message::Text(raw)) = socket.read() {
            let request: Value = serde_json::from_str(&raw).unwrap();
            calls += 1;
            assert!(calls <= 5, "{case:?}: unbounded retry");
            let method = request["method"].as_str().unwrap();
            let id = request["params"]["turnId"].as_str().unwrap_or("");
            let mut response = if method == "thread/turns/list" {
                emit(&mut socket, Phase::List);
                let data = match case.snapshot_status {
                    "missing" => json!([]),
                    "actual" => json!([{"id":"actual","status":"inProgress"}]),
                    "reported-interrupted" => {
                        json!([{"id":"submitted","status":"completed"},{"id":"actual","status":"interrupted"}])
                    }
                    "actual-completed" => json!([{"id":"actual","status":"completed"}]),
                    status => json!([{"id":"submitted","status":status}]),
                };
                json!({"result":{"data":data}})
            } else {
                assert_eq!(method, "turn/interrupt", "{case:?}");
                if calls == 1 {
                    if case.start == Phase::BeforeRpc {
                        assert_eq!(id, "actual", "{case:?}: pre-RPC reconciliation");
                    } else {
                        assert_eq!(id, "submitted", "{case:?}");
                    }
                    // Admission errors race lifecycle events; successful ACKs
                    // cancel the addressed turn before a successor can start.
                    if matches!(case.result, RpcResult::Accepted) && id == "actual" {
                        actual_interrupts += 1;
                    }
                    emit(&mut socket, Phase::Interrupt);
                    match case.result {
                        RpcResult::Accepted => json!({"result":{}}),
                        RpcResult::Stale => {
                            json!({"error":{"code":-32600,"message":"expected active turn id submitted but found actual"}})
                        }
                        RpcResult::NoActive => {
                            json!({"error":{"code":-32600,"message":"no active turn to interrupt"}})
                        }
                    }
                } else {
                    assert!(id == "actual" || id == "submitted", "{case:?}");
                    if id == "actual" {
                        actual_interrupts += 1;
                    }
                    emit(&mut socket, Phase::Ack);
                    json!({"result":{}})
                }
            };
            response["id"] = request["id"].clone();
            socket
                .write(Message::Text(response.to_string().into()))
                .unwrap();
            if method == "thread/turns/list" {
                if case.delayed_start {
                    socket.flush().unwrap();
                    thread::sleep(Duration::from_millis(100));
                }
                emit(&mut socket, Phase::StartWait);
                // For a successor appearing during the retry ACK, allow the
                // snapshot-selected admitted turn to enter the validator first.
                if case.start >= Phase::Ack {
                    socket
                        .write(Message::Text(
                            json!({"method":"turn/started","params":{"turn":{"id":"submitted"}}})
                                .to_string()
                                .into(),
                        ))
                        .unwrap();
                }
            } else if matches!(case.result, RpcResult::Accepted) || calls > 1 {
                emit(&mut socket, Phase::AfterAck);
            }
            socket.flush().unwrap();
            // Track accepted cancellation independently of runtime internals.
            // A successor that started after an ACK for submitted stays active.
            if method == "turn/interrupt"
                && id == "actual"
                && (calls > 1 || matches!(case.result, RpcResult::Accepted))
            {
                // The closure borrows the oracle; evaluate it after the socket loop.
                break;
            }
        }
        drop(emit);
        if actual_interrupts > 0 {
            active_actual = false;
        }
        assert!(
            actual_started,
            "{case:?}: fixture did not reach actual start"
        );
        assert!(
            !active_actual,
            "{case:?}: settlement left the actual turn running"
        );
        if case.snapshot_status == "actual-completed" && case.start < Phase::StartWait {
            assert_eq!(
                actual_interrupts, 0,
                "{case:?}: completed snapshot must settle without another interrupt"
            );
        } else if case.actual_completion.is_none() {
            assert!(
                actual_interrupts > 0,
                "{case:?}: actual active identity was never interrupted"
            );
        }
    });
    let (socket, _) = connect(&endpoint).unwrap();
    let tokio_tungstenite::tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref()
    else {
        unreachable!()
    };
    stream.set_nodelay(true).unwrap();
    let close = stream.try_clone().unwrap();
    let mut state = CodexRuntimeState::new(endpoint, "thread".into(), socket, 1);
    state.active_turn_id = Some("submitted".into());
    if case_previous_tracker {
        state.turn_tracker.provider_active_turn_id = Some("previous".into());
    }
    for event in before_events {
        state.buffered_notifications.push(
            crate::provider::codex_client::fixture_codex_notification(
                event["method"].as_str().unwrap(),
                event["params"].clone(),
            ),
        );
    }
    let result = abort_codex_turn("run", &mut state);
    close.shutdown(Shutdown::Both).unwrap();
    assert!(
        server.join().is_ok(),
        "{label}: provider oracle failed after abort returned {result:?}"
    );
    result.unwrap_or_else(|error| panic!("{label}: {error}"));
    assert!(state.active_turn_id.is_none(), "{label}");
    assert!(state.buffered_notifications.is_empty(), "{label}");
}

fn race_events(case: &RaceCase, phase: Phase) -> Vec<Value> {
    let mut events = Vec::new();
    let submitted_start = json!({"method":"turn/started","params":{"turn":{"id":"submitted"}}});
    let submitted = json!({"method":"turn/completed","params":{"turn":{"id":"submitted","status":case.completion_status,"items":[]}}});
    if case.completion_first && case.submitted_completion == Some(phase) {
        if case.replayed_submitted_start {
            events.push(submitted_start.clone());
        }
        events.push(submitted.clone());
    }
    if case.start == phase {
        events.push(json!({"method":"turn/started","params":{"turn":{"id":"actual"}}}));
    }
    if !case.completion_first && case.submitted_completion == Some(phase) {
        if case.replayed_submitted_start {
            events.push(submitted_start);
        }
        events.push(submitted);
    }
    if case.actual_completion == Some(phase) {
        events.push(json!({"method":"turn/completed","params":{"turn":{"id":"actual","status":case.completion_status,"items":[]}}}));
    }
    events
}

#[test]
fn mp08_interrupt_generated_lifecycle_rpc_interleavings() {
    let phases = [
        Phase::BeforeRpc,
        Phase::Interrupt,
        Phase::List,
        Phase::StartWait,
        Phase::Ack,
        Phase::AfterAck,
    ];
    let mut count = 0;
    for start in phases {
        for submitted_completion in [
            None,
            Some(Phase::BeforeRpc),
            Some(Phase::Interrupt),
            Some(Phase::List),
            Some(Phase::StartWait),
            Some(Phase::Ack),
            Some(Phase::AfterAck),
        ] {
            for actual_completion in std::iter::once(None)
                .chain(phases.into_iter().filter(|phase| *phase >= start).map(Some))
            {
                for completion_status in ["completed", "interrupted"] {
                    for snapshot_status in [
                        "missing",
                        "inProgress",
                        "completed",
                        "interrupted",
                        "actual",
                    ] {
                        for result in [RpcResult::Stale, RpcResult::NoActive, RpcResult::Accepted] {
                            // Accepted first RPC has no list/start-wait phase.
                            if matches!(result, RpcResult::Accepted) {
                                if !matches!(
                                    start,
                                    Phase::BeforeRpc | Phase::Interrupt | Phase::AfterAck
                                ) || submitted_completion.is_some_and(|phase| {
                                    !matches!(
                                        phase,
                                        Phase::BeforeRpc | Phase::Interrupt | Phase::AfterAck
                                    )
                                }) || snapshot_status != "missing"
                                {
                                    continue;
                                }
                            } else if start == Phase::BeforeRpc {
                                continue;
                            }
                            // Terminal admission can legitimately settle before
                            // an as-yet-unobserved successor exists. Those future
                            // starts are a separate prompt, not this cancellation.
                            if start >= Phase::Ack
                                && (submitted_completion.is_some_and(|phase| phase < Phase::Ack)
                                    || !matches!(snapshot_status, "missing" | "inProgress"))
                            {
                                continue;
                            }
                            if start >= Phase::Ack
                                && matches!(result, RpcResult::Stale)
                                && snapshot_status == "missing"
                            {
                                continue;
                            }
                            if actual_completion.is_some() && start == Phase::BeforeRpc {
                                continue;
                            }
                            for completion_first in [false, true] {
                                for replayed_submitted_start in [false, true] {
                                    if replayed_submitted_start && submitted_completion.is_none() {
                                        continue;
                                    }
                                    generated_interrupt_fixture(RaceCase {
                                        start,
                                        submitted_completion,
                                        actual_completion,
                                        completion_first,
                                        completion_status,
                                        snapshot_status,
                                        result,
                                        previous_tracker: false,
                                        delayed_start: false,
                                        replayed_submitted_start,
                                    });
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("MP-08 / MP-10 generated {count} lifecycle/RPC cases");
    assert!(count >= 500);
}

// MP-08 / MP-10: an RPC reply can advance identity again during a retry.
// Exercise ACK, both recoverable errors and a fatal error at that same seam.
#[test]
fn mp08_interrupt_generated_retry_results_reconcile_successor() {
    for ephemeral in [false, true] {
        for reply in ["accepted", "stale", "no-active", "fatal", "repeated-stale"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("ws://{}", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(6)))
                    .unwrap();
                stream.set_nodelay(true).unwrap();
                let mut socket = accept(stream).unwrap();
                let mut interrupts = Vec::new();
                let mut reads = 0;
                while let Ok(Message::Text(raw)) = socket.read() {
                    let request: Value = serde_json::from_str(&raw).unwrap();
                    let mut response = match request["method"].as_str().unwrap() {
                        "thread/turns/list" => {
                            reads += 1;
                            let id = if reads == 1 || reply == "repeated-stale" {
                                "actual"
                            } else {
                                "third"
                            };
                            socket
                                .write(Message::Text(
                                    json!({"method":"turn/started","params":{"turn":{"id":id}}})
                                        .to_string()
                                        .into(),
                                ))
                                .unwrap();
                            json!({"result":{"data":[{"id":id,"status":"inProgress"}]}})
                        }
                        "turn/interrupt" => {
                            let id = request["params"]["turnId"].as_str().unwrap();
                            interrupts.push(id.to_string());
                            if interrupts.len() == 1 {
                                assert_eq!(id, "submitted");
                                if ephemeral {
                                    socket.write(Message::Text(json!({"method":"turn/started","params":{"turn":{"id":"actual"}}}).to_string().into())).unwrap();
                                }
                                json!({"error":{"code":-32600,"message":"expected active turn id submitted but found actual"}})
                            } else if interrupts.len() == 2 {
                                assert_eq!(id, "actual");
                                if reply != "repeated-stale" {
                                    socket.write(Message::Text(json!({"method":"turn/started","params":{"turn":{"id":"third"}}}).to_string().into())).unwrap();
                                }
                                match reply {
                                    "accepted" => json!({"result":{}}),
                                    "no-active" => {
                                        json!({"error":{"code":-32600,"message":"no active turn to interrupt"}})
                                    }
                                    "fatal" => {
                                        json!({"error":{"code":-32600,"message":"permission denied"}})
                                    }
                                    _ => {
                                        json!({"error":{"code":-32600,"message":"expected active turn id actual but found third"}})
                                    }
                                }
                            } else {
                                assert_eq!(interrupts.len(), 3);
                                if reply == "repeated-stale" {
                                    assert_eq!(id, "actual");
                                    json!({"error":{"code":-32600,"message":"expected active turn id actual but found third"}})
                                } else {
                                    assert_eq!(id, "third");
                                    json!({"result":{}})
                                }
                            }
                        }
                        _ => panic!("MP-08 unexpected RPC"),
                    };
                    response["id"] = request["id"].clone();
                    socket
                        .send(Message::Text(response.to_string().into()))
                        .unwrap();
                }
                interrupts
            });
            let (socket, _) = connect(&endpoint).unwrap();
            let tokio_tungstenite::tungstenite::stream::MaybeTlsStream::Plain(stream) =
                socket.get_ref()
            else {
                unreachable!()
            };
            let close = stream.try_clone().unwrap();
            let mut state = CodexRuntimeState::new(endpoint, "thread".into(), socket, 1);
            state.active_turn_id = Some("submitted".into());
            state.ephemeral = ephemeral;
            let result = abort_codex_turn("run", &mut state);
            close.shutdown(Shutdown::Both).unwrap();
            let interrupts = server.join().unwrap();
            if ["fatal", "repeated-stale"].contains(&reply) {
                assert!(result.is_err(), "MP-08 {reply}");
                assert!(
                    state.active_turn_id.is_some(),
                    "MP-08 error must retain cancellation ownership"
                );
            } else {
                result.unwrap();
                assert_eq!(
                    interrupts,
                    ["submitted", "actual", "third"],
                    "MP-08 {reply}: successor must be interrupted before settlement"
                );
                assert!(state.active_turn_id.is_none());
            }
        }
    }
}

// MP-08 / MP-10: retained tracker state is not a fresh start for this prompt.
#[test]
fn mp08_interrupt_retained_previous_tracker_cannot_rebind_admission() {
    for result in [RpcResult::Stale, RpcResult::NoActive, RpcResult::Accepted] {
        generated_interrupt_fixture(RaceCase {
            start: if matches!(result, RpcResult::Accepted) {
                Phase::Interrupt
            } else {
                Phase::StartWait
            },
            submitted_completion: None,
            actual_completion: None,
            completion_first: false,
            completion_status: "interrupted",
            snapshot_status: "inProgress",
            result,
            previous_tracker: true,
            delayed_start: false,
            replayed_submitted_start: false,
        });
    }
}

// MP-08 / MP-10: the validator already named actual; history can omit it
// before the start notification arrives. Old completion cannot close that gap.
#[test]
fn mp08_interrupt_reported_missing_identity_survives_delayed_start() {
    for snapshot_status in [
        "missing",
        "completed",
        "interrupted",
        "reported-interrupted",
    ] {
        generated_interrupt_fixture(RaceCase {
            start: Phase::StartWait,
            submitted_completion: Some(Phase::List),
            actual_completion: None,
            completion_first: true,
            completion_status: "interrupted",
            snapshot_status,
            result: RpcResult::Stale,
            previous_tracker: false,
            delayed_start: true,
            replayed_submitted_start: false,
        });
    }
}

// MP-08 / MP-10: a later completed snapshot also settles a lifecycle identity
// that started before the list; an after-list start still invalidates it.
#[test]
fn mp08_interrupt_generated_terminal_snapshot_lifecycle_order() {
    for result in [RpcResult::Stale, RpcResult::NoActive] {
        for start in [Phase::Interrupt, Phase::List, Phase::StartWait] {
            generated_interrupt_fixture(RaceCase {
                start,
                submitted_completion: None,
                actual_completion: None,
                completion_first: false,
                completion_status: "completed",
                snapshot_status: "actual-completed",
                result,
                previous_tracker: false,
                delayed_start: false,
                replayed_submitted_start: false,
            });
        }
    }
}
