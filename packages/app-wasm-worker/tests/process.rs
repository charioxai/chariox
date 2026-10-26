//! Real worker subprocess drills; build the source App with scripts/drill.sh.
#[path = "process/broker.rs"]
mod broker;
#[path = "process/support.rs"]
mod support;
#[path = "process/transactions.rs"]
mod transactions;
use chariox_app_runtime::wire::{Message, Outcome, WIRE_VERSION};
use chariox_app_wasm_worker::ipc::wall_ms;
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};
use support::Worker;

fn code(outcome: Outcome) -> String {
    match outcome {
        Outcome::Failure(failure) => failure.error.code,
        Outcome::Success(success) => panic!("expected failure, got {}", success.result),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn real_worker_process_concurrency_cancel_deadline_trap_oom_restart() {
    let mut worker = Worker::start(vec![]).await;
    let mut gates = Vec::new();
    for index in 0..64 {
        worker
            .call(&format!("shared-{index}"), "shared", 5000)
            .await;
    }
    while gates.len() < 64 {
        let Message::Request {
            id, method, params, ..
        } = worker.next().await
        else {
            panic!("all 64 calls must reach their shared instance")
        };
        assert_eq!(method, "state.get");
        assert_eq!(params["key"], "gate");
        gates.push(id);
    }
    worker.call("overflow", "echo", 2000).await;
    assert_eq!(code(worker.response("overflow").await), "APP_BUSY");
    for id in gates {
        worker.answer(id, Value::Null).await;
    }
    let mut counts = BTreeSet::new();
    for _ in 0..64 {
        let Message::Response {
            outcome: Outcome::Success(success),
            ..
        } = worker.next().await
        else {
            panic!("shared result expected")
        };
        counts.insert(success.result["count"].as_u64().unwrap());
    }
    assert_eq!(counts, (1..=64).collect());
    println!("DRILL 64 concurrent calls share one instance; 65th is busy: PASS");
    worker.call("cancel", "wait", 5000).await;
    let Message::Request {
        id: broker, method, ..
    } = worker.next().await
    else {
        panic!("broker hold expected")
    };
    assert_eq!(method, "state.get");
    worker.cancel("cancel").await;
    let mut cancelled = false;
    let mut answered = false;
    while !cancelled || !answered {
        match worker.next().await {
            Message::Cancel { id, .. } => {
                assert_eq!(id, broker);
                cancelled = true;
            }
            Message::Response { id, outcome, .. } => {
                assert_eq!(id, "cancel");
                assert_eq!(code(outcome), "CANCELLED");
                answered = true;
            }
            message => panic!("unexpected cancellation message {message:?}"),
        }
    }
    worker.echo("after-cancel").await;
    println!("DRILL cancellation releases broker resource and keeps ordinary calls working: PASS");
    worker.call("deadline", "wait", 40).await;
    let Message::Request { method, .. } = worker.next().await else {
        panic!("broker hold expected")
    };
    assert_eq!(method, "state.get");
    assert_eq!(code(worker.response("deadline").await), "DEADLINE_EXCEEDED");
    worker.echo("after-deadline").await;
    println!("DRILL deadline overrun: PASS");
    worker.call("healthy", "wait", 5000).await;
    let Message::Request { method, .. } = worker.next().await else {
        panic!("healthy hold expected")
    };
    assert_eq!(method, "state.get");
    worker.call("culprit", "trap", 2000).await;
    let mut results = HashMap::new();
    while results.len() < 2 {
        match worker.next().await {
            Message::Response { id, outcome, .. } => {
                results.insert(id, outcome);
            }
            Message::Cancel { .. } => {}
            message => panic!("unexpected trap message {message:?}"),
        }
    }
    assert_eq!(code(results.remove("culprit").unwrap()), "APP_TRAPPED");
    let Outcome::Failure(failure) = results.remove("healthy").unwrap() else {
        panic!("instance loss expected")
    };
    assert_eq!(failure.error.code, "APP_INSTANCE_RESTARTED");
    assert!(failure.error.message.contains("culprit"));
    worker.echo("after-trap").await;
    println!("DRILL trap names culprit and preserves explicit instance-loss result: PASS");
    worker.call("oom", "oom", 2000).await;
    assert_eq!(code(worker.response("oom").await), "APP_MEMORY_LIMIT");
    worker.echo("after-oom").await;
    println!("DRILL OOM, restart and ordinary valid call: PASS");
    worker.call("spin", "spin", 2000).await;
    assert_eq!(code(worker.response("spin").await), "APP_CPU_SLICE");
    worker.echo("after-spin").await;
    println!("DRILL CPU slice, restart and ordinary valid call: PASS");
    worker.call("ignore", "ignore", 5000).await;
    let Message::Request { method, .. } = worker.next().await else {
        panic!("ignored hold expected")
    };
    assert_eq!(method, "state.get");
    worker.cancel("ignore").await;
    assert_eq!(code(worker.response("ignore").await), "CANCELLED");
    worker.echo("after-ignore").await;
    println!("DRILL uncooperative cancellation forces restart and frees resources: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn real_worker_retries_only_signed_tools_and_only_once() {
    let mut worker = Worker::start(vec!["wait"]).await;
    worker.call("retry", "wait", 5000).await;
    assert!(matches!(worker.next().await,Message::Request { method,.. } if method=="state.get"));
    worker.call("trap-first", "trap", 2000).await;
    let mut retried = false;
    let mut trapped = false;
    while !retried || !trapped {
        match worker.next().await {
            Message::Request { method, .. } => {
                assert_eq!(method, "state.get");
                assert!(!retried);
                retried = true;
            }
            Message::Response { id, outcome, .. } => {
                assert_eq!(id, "trap-first");
                assert_eq!(code(outcome), "APP_TRAPPED");
                trapped = true;
            }
            Message::Cancel { .. } => {}
            other => panic!("unexpected replay message {other:?}"),
        }
    }
    worker.call("trap-second", "trap", 2000).await;
    let mut answered = 0;
    while answered < 2 {
        match worker.next().await {
            Message::Response { id, outcome, .. } => {
                assert_eq!(
                    code(outcome),
                    if id == "retry" {
                        "APP_INSTANCE_RESTARTED"
                    } else {
                        "APP_TRAPPED"
                    }
                );
                answered += 1;
            }
            Message::Cancel { .. } => {}
            other => panic!("a third attempt is forbidden: {other:?}"),
        }
    }
    worker.echo("after-retry").await;
    println!("DRILL signed idempotent tool retries once across two losses: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn real_worker_streams_have_backpressure_and_cancel_resources() {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    let mut worker = Worker::start(vec![]).await;
    worker.call("slow", "slow_stream", 5000).await;
    let mut reads = 0;
    let mut held = false;
    while reads < 4 || !held {
        match worker.next().await {
            Message::Request { id,method,params,.. }=>match method.as_str() {
                "http.open"=>worker.answer(id,json!({"streamId":"stream-slow"})).await,
                "http.headers"=>worker.answer(id,json!({"pending":false,"status":200,"headers":[],"url":"https://example.com/"})).await,
                "http.read"=>{assert_eq!(params["streamId"],"stream-slow");reads+=1;assert!(reads<=4);worker.answer(id,json!({"pending":false,"done":false,"chunkBase64":B64.encode(vec![7;64*1024])})).await;},
                "state.get"=>held=true,
                other=>panic!("unexpected broker call {other}"),
            },
            other=>panic!("unexpected stream message {other:?}"),
        }
    }
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            worker
                .channel
                .as_mut()
                .unwrap()
                .receive(Duration::from_secs(1))
        )
        .await
        .is_err(),
        "read-ahead exceeded four chunks"
    );
    worker.cancel("slow").await;
    let mut released = false;
    let mut answered = false;
    while !released || !answered {
        match worker.next().await {
            Message::Request {
                id, method, params, ..
            } => {
                assert_eq!(method, "http.cancel");
                assert_eq!(params["streamId"], "stream-slow");
                released = true;
                worker.answer(id, Value::Null).await;
            }
            Message::Cancel { .. } => {}
            Message::Response { id, outcome, .. } => {
                assert_eq!(id, "slow");
                assert_eq!(code(outcome), "CANCELLED");
                answered = true;
            }
            other => panic!("unexpected release message {other:?}"),
        }
    }
    worker.echo("after-stream-cancel").await;
    println!("DRILL four-chunk backpressure and HTTP resource release on cancellation: PASS");
    worker.call("body", "stream", 5000).await;
    let mut chunks = 0;
    let mut completed = false;
    while !completed {
        match worker.next().await {
            Message::Request { id,method,.. }=>match method.as_str() {
                "http.open"=>worker.answer(id,json!({"streamId":"stream-body"})).await,
                "http.headers"=>worker.answer(id,json!({"pending":false,"status":200,"headers":[],"url":"https://example.com/"})).await,
                "http.read"=>{let done=chunks==16;worker.answer(id,json!({"pending":false,"done":done,"chunkBase64":if done {String::new()}else{B64.encode(vec![7;64*1024])}})).await;chunks+=1;},
                "http.cancel"=>worker.answer(id,Value::Null).await,
                other=>panic!("unexpected HTTP call {other}"),
            },
            Message::Response { id,outcome:Outcome::Success(success),.. }=>{assert_eq!(id,"body");assert_eq!(success.result,json!({"status":200,"bytes":1024*1024}));completed=true;},
            other=>panic!("unexpected body message {other:?}"),
        }
    }
    worker.echo("after-body").await;
    println!("DRILL 1 MiB response streams through bounded 64 KiB chunks: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn real_worker_uploads_with_backpressure_and_delivers_live_events() {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    let mut worker = Worker::start(vec![]).await;
    worker.call("upload", "upload", 5000).await;
    let mut bytes = 0;
    let mut ended = false;
    let mut completed = false;
    while !completed {
        match worker.next().await {
            Message::Request { id,method,params,.. }=>match method.as_str() {
                "http.open"=>{assert_eq!(params["hasBody"],true);worker.answer(id,json!({"streamId":"upload"})).await;},
                "http.write"=>{let chunk=B64.decode(params["bodyBase64"].as_str().unwrap()).unwrap();assert!(chunk.len()<=64*1024);assert!(chunk.iter().all(|byte|*byte==9));bytes+=chunk.len();ended=params["end"]==true;worker.answer(id,Value::Null).await;},
                "http.headers"=>worker.answer(id,if ended {json!({"pending":false,"status":200,"headers":[],"url":"https://example.com/"})}else{json!({"pending":true})}).await,
                "http.read"=>worker.answer(id,json!({"pending":false,"done":true,"chunkBase64":""})).await,
                "http.cancel"=>worker.answer(id,Value::Null).await,
                other=>panic!("unexpected upload call {other}"),
            },
            Message::Response { id,outcome:Outcome::Success(_),.. }=>{assert_eq!(id,"upload");completed=true;},
            Message::Cancel { .. }=>{},
            other=>panic!("unexpected upload message {other:?}"),
        }
    }
    assert_eq!(bytes, 160 * 1024);
    assert!(ended);
    println!("DRILL guest request-body stream uses bounded chunks and acknowledges end: PASS");
    worker.call("subscribe", "subscribe", 5000).await;
    loop {
        match worker.next().await {
            Message::Request {
                id, method, params, ..
            } if method == "state.get" => {
                assert_eq!(params["key"], "subscribed");
                worker.answer(id, Value::Null).await;
                break;
            }
            Message::Request { id, method, .. } if method == "http.cancel" => {
                worker.answer(id, Value::Null).await
            }
            other => panic!("unexpected subscription marker {other:?}"),
        }
    }
    worker
        .send(Message::Request {
            version: WIRE_VERSION,
            generation: "1".into(),
            id: "event".into(),
            method: "events.deliver".into(),
            params: json!({"name":"tick","data":{"tick":1}}),
            deadline_ms: wall_ms() + 2000,
            context: None,
        })
        .await;
    let mut results = HashMap::new();
    while results.len() < 2 {
        let Message::Response { id, outcome, .. } = worker.next().await else {
            panic!("event response expected")
        };
        results.insert(id, outcome);
    }
    assert!(matches!(
        results.remove("event").unwrap(),
        Outcome::Success(_)
    ));
    let Outcome::Success(success) = results.remove("subscribe").unwrap() else {
        panic!("subscription failed")
    };
    assert_eq!(success.result, json!({"events":1}));
    worker
        .send(Message::Request {
            version: WIRE_VERSION,
            generation: "1".into(),
            id: "after-subscribe".into(),
            method: "events.deliver".into(),
            params: json!({"name":"tick","data":null}),
            deadline_ms: wall_ms() + 2000,
            context: None,
        })
        .await;
    assert_eq!(
        code(worker.response("after-subscribe").await),
        "NO_SUBSCRIBER"
    );
    worker.echo("after-events").await;
    println!("DRILL live event stream delivers and releases its subscription with the call: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn slow_live_readers_leave_tool_capacity_available() {
    let mut worker = Worker::start(vec![]).await;
    worker
        .call_input("slow-subscribe", "subscribe", json!({"slow":true}), 5000)
        .await;
    let Message::Request { id, params, .. } = worker.next().await else {
        panic!("subscription marker expected")
    };
    assert_eq!(params["key"], "subscribed");
    worker.answer(id, Value::Null).await;
    assert!(matches!(worker.next().await,Message::Request { params,.. } if params["key"]=="hold"));
    for index in 0..32 {
        let id = format!("event-{index}");
        worker
            .send(Message::Request {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: id.clone(),
                method: "events.deliver".into(),
                params: json!({"name":"tick","data":index}),
                deadline_ms: wall_ms() + 5000,
                context: None,
            })
            .await;
        if index < 16 {
            assert!(matches!(worker.response(&id).await, Outcome::Success(_)));
        }
    }
    worker
        .send(Message::Request {
            version: WIRE_VERSION,
            generation: "1".into(),
            id: "event-overflow".into(),
            method: "events.deliver".into(),
            params: json!({"name":"tick","data":32}),
            deadline_ms: wall_ms() + 2000,
            context: None,
        })
        .await;
    assert_eq!(code(worker.response("event-overflow").await), "APP_BUSY");
    worker.echo("during-slow-subscriber").await;
    worker.cancel("slow-subscribe").await;
    let mut answered = BTreeSet::new();
    while answered.len() < 17 {
        match worker.next().await {
            Message::Response { id, outcome, .. } => {
                assert_eq!(code(outcome), "CANCELLED");
                assert!(answered.insert(id));
            }
            Message::Cancel { .. } => {}
            other => panic!("unexpected delivery cleanup {other:?}"),
        }
    }
    worker.echo("after-slow-subscriber").await;
    println!(
        "DRILL slow event readers apply backpressure while ordinary tools stay responsive: PASS"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn broker_failures_are_explicit_and_do_not_masquerade_as_guest_traps() {
    let mut worker = Worker::start(vec![]).await;
    worker.call("malformed", "shared", 5000).await;
    let Message::Request { id, method, .. } = worker.next().await else {
        panic!("state call expected")
    };
    assert_eq!(method, "state.get");
    worker.answer(id, json!({"version":1})).await;
    assert_eq!(
        code(worker.response("malformed").await),
        "APP_BROKER_PROTOCOL"
    );
    worker.echo("after-malformed").await;
    worker.call("body-failure", "stream", 5000).await;
    loop {
        match worker.next().await {
            Message::Request { id,method,.. }=>match method.as_str() {
                "http.open"=>worker.answer(id,json!({"streamId":"failed-body"})).await,
                "http.headers"=>worker.answer(id,json!({"pending":false,"status":200,"headers":[],"url":"https://example.com/"})).await,
                "http.read"=>worker.send(Message::Response { version:WIRE_VERSION,generation:"1".into(),id,outcome:Outcome::Failure(chariox_app_runtime::wire::Failure { error:chariox_app_wasm_worker::ipc::error("UPSTREAM_CLOSED","upstream closed during response body") }) }).await,
                "http.cancel"=>worker.answer(id,Value::Null).await,
                other=>panic!("unexpected failed body call {other}"),
            },
            Message::Response { id,outcome,.. }=>{assert_eq!(id,"body-failure");assert_eq!(code(outcome),"APP_HTTP_STREAM_FAILED");break;},
            other=>panic!("unexpected failed body message {other:?}"),
        }
    }
    worker.echo("after-body-failure").await;
    println!("DRILL malformed broker replies and mid-body failures have explicit reasons: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn dropping_early_response_cancels_pending_upload_while_handler_lives() {
    let mut worker = Worker::start(vec![]).await;
    worker.call("early", "early_response", 5000).await;
    let mut headers = None;
    let mut write = None;
    let mut held = Vec::new();
    let mut cancelled_write = false;
    let mut cancelled_stream = false;
    let mut stopped_upload = false;
    let mut dropped_response = false;
    while !cancelled_write || !cancelled_stream || !stopped_upload || !dropped_response {
        let message = tokio::time::timeout(Duration::from_millis(750), worker.next())
            .await
            .expect("dropping response must promptly stop upload before invocation ends");
        match message {
            Message::Request {
                id, method, params, ..
            } => match method.as_str() {
                "http.open" => worker.answer(id, json!({"streamId":"early"})).await,
                "http.headers" => headers = Some(id),
                "http.write" => {
                    assert!(
                        write.is_none(),
                        "discarded upload must not send another chunk"
                    );
                    assert_eq!(params["end"], false);
                    write = Some(id);
                }
                "http.read" => {} // The read remains pending until the body is dropped.
                "http.cancel" => {
                    assert_eq!(params["streamId"], "early");
                    assert!(!cancelled_stream, "stream must be released exactly once");
                    cancelled_stream = true;
                    worker.answer(id, Value::Null).await;
                }
                "state.get" => {
                    match params["key"].as_str().unwrap() {
                        "upload-stopped" => stopped_upload = true,
                        "response-dropped" => dropped_response = true,
                        other => panic!("unexpected state key {other}"),
                    }
                    held.push(id);
                }
                other => panic!("unexpected broker call {other}"),
            },
            Message::Cancel { id, .. } => {
                if write.as_ref() == Some(&id) {
                    cancelled_write = true;
                }
            }
            other => panic!("handler must stay alive until upload ends: {other:?}"),
        }
        if write.is_some() {
            if let Some(id) = headers.take() {
                worker
                    .answer(
                        id,
                        json!({"status":200,"headers":[],"url":"https://example.com/"}),
                    )
                    .await;
            }
        }
    }
    worker.echo("while-early-alive").await;
    for id in held {
        worker.answer(id, Value::Null).await;
    }
    assert!(matches!(
        worker.response("early").await,
        Outcome::Success(_)
    ));
    worker.echo("after-early").await;
    println!("DRILL early response drop cancels upload and releases stream exactly once while handler lives: PASS");
}
