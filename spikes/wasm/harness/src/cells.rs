//! Cells for the async world: many callers on one instance, streaming,
//! subscriptions, cancellation, and what a trap does to the other calls.
use crate::broker::ALLOWED;
use crate::process::Variant;
use crate::{ms, percentile, start, Worker, LONG};
use chariox_app_runtime::wire::{Message, Outcome};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn summary(samples: &mut [f64]) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    json!({ "p50": percentile(samples, 0.5), "p95": percentile(samples, 0.95), "p99": percentile(samples, 0.99), "max": percentile(samples, 1.0) })
}

fn outcome(result: Result<Value, Value>) -> Value {
    result.map_or_else(|e| json!({ "error": e }), |v| v)
}

async fn stats(worker: &Worker) -> Value {
    worker.call("__stats", json!({}), LONG).await.unwrap_or(Value::Null)
}

/// N callers at once on ONE instance, all touching the same in-memory
/// counter, cache and "connection"; each waits on the HTTP broker.
pub async fn shared(variant: &Variant, origin: &str, calls: usize) -> Value {
    let worker = Arc::new(start(variant, origin).await);
    // Warm the instance and the connection object first.
    let _ = worker.call("shared", json!({ "url": format!("{ALLOWED}hello") }), LONG).await;
    let at = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..calls {
        let worker = worker.clone();
        tasks.push(tokio::spawn(async move {
            let at = Instant::now();
            let result = worker.call("shared", json!({ "url": format!("{ALLOWED}hello") }), LONG).await;
            (ms(at), result)
        }));
    }
    let (mut latency, mut numbers, mut sequences, mut connections, mut errors, mut cached) = (Vec::new(), BTreeSet::new(), BTreeSet::new(), BTreeSet::new(), Vec::new(), 0);
    for task in tasks {
        let (elapsed, result) = task.await.unwrap();
        latency.push(elapsed);
        match result {
            Ok(value) => {
                numbers.insert(value["n"].as_u64().unwrap_or(0));
                sequences.insert(value["sequence"].as_u64().unwrap_or(0));
                connections.insert(value["connection"].as_str().unwrap_or_default().to_string());
                cached = cached.max(value["cached"].as_u64().unwrap_or(0));
            }
            Err(error) => errors.push(error),
        }
    }
    let total = ms(at);
    // The warm-up call was n = 1; the burst must hold n = 2..=calls+1 exactly once.
    let expected: BTreeSet<u64> = (2..=calls as u64 + 1).collect();
    let after = worker.call("shared", json!({ "url": format!("{ALLOWED}hello") }), LONG).await.unwrap_or(Value::Null);
    let report = json!({
        "variant": variant.name, "sandbox": variant.sandbox, "mode": "shared", "calls": calls,
        "ok": calls - errors.len(), "totalMs": total, "latency": summary(&mut latency),
        "noLostUpdates": numbers == expected && after["n"] == json!(calls as u64 + 2),
        "sequencesDistinct": sequences.len() == calls, "oneConnection": connections.len() == 1,
        "cacheEntries": cached, "errors": errors.into_iter().take(3).collect::<Vec<_>>(),
        "memory": worker.spawned.memory(),
    });
    worker.peer.close();
    worker.spawned.stop();
    report
}

/// A body of `mb` MiB streamed through the broker; `work` makes a slow reader.
pub async fn stream(variant: &Variant, origin: &str, mb: u64, work: u64) -> Value {
    let worker = start(variant, origin).await;
    let before = worker.spawned.memory();
    let at = Instant::now();
    let result = worker.call("stream", json!({ "url": format!("{ALLOWED}big/{mb}"), "work": work }), LONG).await;
    let elapsed = ms(at);
    let report = json!({
        "variant": variant.name, "sandbox": variant.sandbox, "mode": "stream", "mb": mb, "work": work,
        "result": outcome(result), "ms": elapsed, "mbPerS": mb as f64 / (elapsed / 1e3),
        "memoryBefore": before, "memoryAfter": worker.spawned.memory(), "stats": stats(&worker).await,
        "brokerStreamsOpen": worker.broker.open_streams(),
    });
    worker.peer.close();
    worker.spawned.stop();
    report
}

async fn deliver(worker: &Worker, seq: u64) -> Result<(), String> {
    let slot = worker.peer.reserve(LONG).map_err(|e| format!("{e:?}"))?;
    match slot.request("events.deliver", json!({ "name": "tick", "data": { "seq": seq } }), None).await {
        Ok(Message::Response { outcome: Outcome::Success(_), .. }) => Ok(()),
        Ok(Message::Response { outcome: Outcome::Failure(failure), .. }) => Err(failure.error.code),
        other => Err(format!("{other:?}")),
    }
}

/// A long-lived subscription: the kernel delivers `count` events one after
/// another; each delivery is acknowledged once it sits in the App's stream.
pub async fn subscribe(variant: &Variant, origin: &str, count: u64, work: u64) -> Value {
    let worker = Arc::new(start(variant, origin).await);
    let reader = {
        let worker = worker.clone();
        tokio::spawn(async move { worker.call("subscribe", json!({ "name": "tick", "count": count, "work": work }), LONG).await })
    };
    // The first delivery waits until the subscription is open.
    let opened = Instant::now();
    while deliver(&worker, 1).await.is_err() {
        if opened.elapsed() > Duration::from_secs(10) { break; }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let at = Instant::now();
    let mut acks = Vec::new();
    let mut failures = 0;
    for seq in 2..=count {
        let sent = Instant::now();
        if deliver(&worker, seq).await.is_err() { failures += 1; }
        acks.push(ms(sent));
    }
    let delivered_ms = ms(at);
    let result = reader.await.unwrap();
    let report = json!({
        "variant": variant.name, "sandbox": variant.sandbox, "mode": "subscribe", "count": count, "work": work,
        "result": outcome(result), "deliverMs": delivered_ms, "eventsPerS": (count - 1) as f64 / (delivered_ms / 1e3),
        "ack": summary(&mut acks), "failedDeliveries": failures, "memory": worker.spawned.memory(),
    });
    worker.peer.close();
    worker.spawned.stop();
    report
}

/// Cancels one waiting call and one streaming call while other callers run.
pub async fn cancel(variant: &Variant, origin: &str) -> Value {
    let worker = Arc::new(start(variant, origin).await);
    let spawn = |name: &'static str, input: Value| {
        let worker = worker.clone();
        tokio::spawn(async move {
            let at = Instant::now();
            let result = worker.call(name, input, LONG).await;
            (ms(at), result)
        })
    };
    let others: Vec<_> = (0..8).map(|_| spawn("shared", json!({ "url": format!("{ALLOWED}hello") }))).collect();
    let waiting = spawn("wait", json!({ "url": format!("{ALLOWED}slow") }));
    let streaming = spawn("stream", json!({ "url": format!("{ALLOWED}big/2000"), "work": 0 }));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mid = stats(&worker).await;
    // Dropping the kernel-side call sends the worker a Cancel frame.
    let cancelled_at = Instant::now();
    waiting.abort();
    streaming.abort();
    let mut released_ms = Value::Null;
    let mut last = Value::Null;
    while cancelled_at.elapsed() < Duration::from_secs(5) {
        last = stats(&worker).await;
        if last["inFlight"] == json!(0) && worker.broker.open_streams() == 0 {
            released_ms = json!(ms(cancelled_at));
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let mut ok = 0;
    for task in others {
        if task.await.unwrap().1.is_ok() { ok += 1; }
    }
    let at = Instant::now();
    let next = worker.call("echo", json!({ "after": "cancel" }), LONG).await.is_ok();
    let next_ms = ms(at);
    let open_after = worker.broker.open_streams();
    // A guest that ignores the cancel leaves its broker stream open until its
    // own work ends (the slow origin answers after 5 s).
    let mut late_ms = Value::Null;
    while cancelled_at.elapsed() < Duration::from_secs(9) {
        if worker.broker.open_streams() == 0 { late_ms = json!(ms(cancelled_at)); break; }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let report = json!({
        "variant": variant.name, "sandbox": variant.sandbox, "mode": "cancel",
        "othersOk": ok, "othersTotal": 8, "statsBeforeCancel": mid, "statsAfter": last,
        "allReleasedMs": released_ms, "brokerStreamsOpen": open_after, "releasedAfterMs": late_ms,
        "statsEnd": stats(&worker).await,
        "nextCallOk": next, "nextCallMs": next_ms, "memory": worker.spawned.memory(),
    });
    worker.peer.close();
    worker.spawned.stop();
    report
}

/// What one runaway or over-budget call does to the other calls on the same
/// instance, and how long a CPU-heavy call holds the others up.
pub async fn blast(variant: &Variant, origin: &str, culprit: &str) -> Value {
    let worker = Arc::new(start(variant, origin).await);
    let before = worker.call("shared", json!({ "url": format!("{ALLOWED}hello") }), LONG).await.unwrap_or(Value::Null);
    let spawn = |name: &'static str, input: Value| {
        let worker = worker.clone();
        tokio::spawn(async move {
            let at = Instant::now();
            let result = worker.call(name, input, LONG).await;
            (ms(at), result)
        })
    };
    let others: Vec<_> = (0..8).map(|_| spawn("shared", json!({ "url": format!("{ALLOWED}hello") }))).collect();
    tokio::time::sleep(Duration::from_millis(5)).await;
    let input = match culprit { "spin" => json!({}), _ => json!({ "mb": 256, "kind": culprit }) };
    let culprit_name: &'static str = if culprit == "spin" { "spin" } else { "hog" };
    let (culprit_ms, culprit_result) = spawn(culprit_name, input).await.unwrap();
    let mut outcomes = Vec::new();
    for task in others {
        let (elapsed, result) = task.await.unwrap();
        outcomes.push(json!({ "ms": elapsed, "outcome": result.map_or_else(|e| e["code"].clone(), |_| json!("ok")) }));
    }
    let at = Instant::now();
    let after = worker.call("shared", json!({ "url": format!("{ALLOWED}hello") }), LONG).await;
    let report = json!({
        "variant": variant.name, "sandbox": variant.sandbox, "mode": "blast", "culprit": culprit,
        "culprit": { "ms": culprit_ms, "outcome": outcome(culprit_result) }, "others": outcomes,
        "sharedCounterBefore": before["n"], "nextCallMs": ms(at), "sharedCounterAfter": outcome(after)["n"],
        "memory": worker.spawned.memory(), "exit": worker.spawned.exit_status(),
    });
    worker.peer.close();
    worker.spawned.stop();
    report
}

/// Echo latency while one CPU-heavy call runs on the same instance.
pub async fn head_of_line(variant: &Variant, origin: &str) -> Value {
    let worker = Arc::new(start(variant, origin).await);
    let heavy = {
        let worker = worker.clone();
        tokio::spawn(async move {
            let at = Instant::now();
            let _ = worker.call("compute", json!({ "n": 3000000 }), LONG).await;
            ms(at)
        })
    };
    tokio::time::sleep(Duration::from_millis(2)).await;
    let mut echoes = Vec::new();
    for n in 0..5 {
        let at = Instant::now();
        let _ = worker.call("echo", json!({ "n": n }), LONG).await;
        echoes.push(ms(at));
    }
    let heavy_ms = heavy.await.unwrap();
    let report = json!({
        "variant": variant.name, "sandbox": variant.sandbox, "mode": "hol",
        "computeMs": heavy_ms, "echoDuringComputeMs": echoes,
    });
    worker.peer.close();
    worker.spawned.stop();
    report
}
