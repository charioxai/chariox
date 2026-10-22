//! Spike harness: plays the kernel side of one App worker with the kernel's
//! own IPC code (chariox-app-runtime wire::Channel + WorkerPeer) and measures
//! the four runtime variants with identical calls.
mod broker;
mod process;

use broker::{Shared, SpikeBroker};
use chariox_app_runtime::wire::{Message, Outcome};
use chariox_app_runtime::worker_peer::{PeerLimits, WorkerPeer};
use process::{Spawned, Variant};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Worker {
    spawned: Spawned,
    peer: WorkerPeer,
    broker: Arc<SpikeBroker>,
    ready_ms: f64,
}

async fn start(variant: &Variant, origin: &str) -> Worker {
    let started = Instant::now();
    let spawned = process::spawn(variant);
    let broker = SpikeBroker::new(origin.into());
    let channel = spawned.channel();
    let (peer, _events, _task) = WorkerPeer::start(channel, Arc::new(Shared(broker.clone())), PeerLimits::default()).unwrap();
    let mut ready = broker.ready.subscribe();
    tokio::time::timeout(Duration::from_secs(30), ready.wait_for(|r| r.is_some())).await.expect("worker never became ready").unwrap();
    Worker { spawned, peer, broker, ready_ms: started.elapsed().as_secs_f64() * 1e3 }
}

impl Worker {
    async fn call(&self, name: &str, input: Value, budget: Duration) -> Result<Value, Value> {
        let slot = self.peer.reserve(budget).map_err(|e| json!(format!("{e:?}")))?;
        match slot.request("tools.invoke", json!({ "name": name, "input": input }), None).await {
            Ok(Message::Response { outcome: Outcome::Success(success), .. }) => Ok(success.result),
            Ok(Message::Response { outcome: Outcome::Failure(failure), .. }) => Err(json!({ "code": failure.error.code, "message": failure.error.message })),
            other => Err(json!(format!("{other:?}"))),
        }
    }
}

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[((samples.len() as f64 - 1.0) * p).round() as usize]
}

fn ms(at: Instant) -> f64 {
    at.elapsed().as_secs_f64() * 1e3
}

const LONG: Duration = Duration::from_secs(30);

async fn measure(variant: &Variant, origin: &str) -> Value {
    let worker = start(variant, origin).await;
    let mut report = json!({ "variant": variant.name, "sandbox": variant.sandbox, "coldReadyMs": worker.ready_ms, "ready": worker.broker.ready.borrow().clone() });
    report["rssAfterReady"] = worker.spawned.memory();

    let mut counter = Vec::new();
    for _ in 0..3 {
        counter.push(worker.call("counter", json!({}), LONG).await.unwrap_or_else(|e| e));
    }
    report["counter"] = json!(counter);
    report["fetch"] = worker.call("fetch", json!({ "url": format!("{}hello", broker::ALLOWED) }), LONG).await.unwrap_or_else(|e| e);
    let denied = worker.call("fetch", json!({ "url": "https://example.com/" }), LONG).await;
    report["fetchDenied"] = denied.err().unwrap_or(json!("NOT DENIED"));

    let mut echo = Vec::new();
    for n in 0..1000 {
        let at = Instant::now();
        worker.call("echo", json!({ "text": "hello", "n": n }), LONG).await.unwrap();
        echo.push(ms(at));
    }
    let mean = echo.iter().sum::<f64>() / echo.len() as f64;
    report["echo"] = json!({ "p50": percentile(&mut echo, 0.5), "p95": percentile(&mut echo, 0.95), "p99": percentile(&mut echo, 0.99), "mean": mean });

    let mut transform = Vec::new();
    let mut output = Value::Null;
    for _ in 0..5 {
        let at = Instant::now();
        output = worker.call("transform", json!({ "n": 10000, "rounds": 5 }), LONG).await.unwrap();
        transform.push(ms(at));
    }
    report["transform"] = json!({ "medianMs": percentile(&mut transform, 0.5), "minMs": transform[0], "output": output });

    let mut compute = Vec::new();
    for _ in 0..5 {
        let at = Instant::now();
        output = worker.call("compute", json!({ "n": 1000000 }), LONG).await.unwrap();
        compute.push(ms(at));
    }
    report["compute"] = json!({ "medianMs": percentile(&mut compute, 0.5), "minMs": compute[0], "output": output });

    // Plain Node has no in-process App instance to re-create: its unit is the process.
    report["warmInstantiate"] = match worker.call("__warm", json!({ "rounds": 20 }), LONG).await {
        Ok(warm) => {
            let mut samples: Vec<f64> = serde_json::from_value(warm["samplesMs"].clone()).unwrap();
            json!({ "medianMs": percentile(&mut samples, 0.5), "p95Ms": percentile(&mut samples, 0.95) })
        }
        Err(_) => Value::Null,
    };
    report["rssAfterWork"] = worker.spawned.memory();

    // Memory: byte buffers past the 128 MiB App limit, then small objects.
    for kind in ["bytes", "objects"] {
        let worker = start(variant, origin).await;
        let at = Instant::now();
        let outcome = worker.call("hog", json!({ "mb": 256, "kind": kind }), LONG).await;
        let elapsed = ms(at);
        let memory = worker.spawned.memory();
        let alive = worker.call("echo", json!({ "after": "hog" }), Duration::from_secs(5)).await.is_ok();
        report[format!("hog_{kind}")] = json!({ "outcome": outcome.map_or_else(|e| e, |v| v), "ms": elapsed, "workerAliveAfter": alive, "memory": memory, "exit": worker.spawned.exit_status() });
        worker.peer.close();
        worker.spawned.stop();
    }

    // Runaway loop with a 1 s per-call budget. The wasm worker stops it itself
    // (epoch); otherwise the supervisor kills the process at budget + 500 ms
    // grace and starts it again.
    let at = Instant::now();
    let spin = tokio::time::timeout(Duration::from_millis(1500), worker.call("spin", json!({}), LONG)).await;
    report["spin"] = match spin {
        Ok(outcome) => {
            let stopped = ms(at);
            let next = Instant::now();
            let alive = worker.call("echo", json!({ "after": "spin" }), LONG).await.is_ok();
            json!({ "stoppedBy": "worker (epoch)", "outcome": outcome.map_or_else(|e| e, |v| v), "errorAfterMs": stopped, "nextCallMs": ms(next), "workerAlive": alive })
        }
        Err(_) => {
            let kill = Instant::now();
            let kill_at_ms = ms(at);
            let reaped = worker.spawned.kill_and_reap();
            let reap_ms = ms(kill);
            let restarted = start(variant, origin).await;
            let alive = restarted.call("echo", json!({ "after": "spin" }), LONG).await.is_ok();
            let recovery = ms(kill);
            restarted.peer.close();
            restarted.spawned.stop();
            json!({ "stoppedBy": "supervisor (SIGKILL)", "killAtMs": kill_at_ms, "reaped": reaped, "killToReapMs": reap_ms, "killToReadyAgainMs": recovery, "restartedAlive": alive })
        }
    };
    worker.peer.close();
    worker.spawned.stop();
    report
}

/// N tool calls in flight at once, each waiting on the (delayed) HTTP broker.
async fn concurrency(variant: &Variant, origin: &str, calls: usize) -> Value {
    let worker = Arc::new(start(variant, origin).await);
    let at = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..calls {
        let worker = worker.clone();
        tasks.push(tokio::spawn(async move {
            worker.call("fetch", json!({ "url": format!("{}hello", broker::ALLOWED) }), LONG).await
        }));
    }
    let mut ok = 0;
    let mut errors = Vec::new();
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => ok += 1,
            Err(error) => errors.push(error),
        }
    }
    let total = ms(at);
    let memory = worker.spawned.memory();
    let alive = worker.call("echo", json!({}), Duration::from_secs(5)).await.is_ok();
    worker.peer.close();
    worker.spawned.stop();
    json!({ "variant": variant.name, "sandbox": variant.sandbox, "calls": calls, "ok": ok, "totalMs": total, "errors": errors.into_iter().take(3).collect::<Vec<_>>(), "workerAliveAfter": alive, "memory": memory })
}

/// Standard fetch() inside the component, through the broker only.
async fn fetch_standard(variant: &Variant, origin: &str) -> Value {
    let worker = start(variant, origin).await;
    let allowed = worker.call("fetchstd", json!({ "url": format!("{}hello", broker::ALLOWED) }), LONG).await;
    let denied = worker.call("fetchstd", json!({ "url": "https://example.com/" }), LONG).await;
    let opened: Vec<Value> = worker.broker.logs.lock().unwrap().clone();
    worker.peer.close();
    worker.spawned.stop();
    json!({ "variant": variant.name, "sandbox": variant.sandbox, "allowed": allowed.map_or_else(|e| e, |v| v), "denied": denied.map_or_else(|e| e, |v| v), "brokerOpens": opened.len() })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(3).map(String::as_str) == Some("fetchstd") {
        let variant = Variant::from_args(&args[1], args.get(2).map(String::as_str) == Some("bwrap"));
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let origin = broker::origin();
        println!("{}", runtime.block_on(fetch_standard(&variant, &origin)));
        return;
    }
    if args.get(4).map(String::as_str) == Some("concurrency") {
        let variant = Variant::from_args(&args[1], args.get(2).map(String::as_str) == Some("bwrap"));
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let origin = broker::origin();
        let calls = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(16);
        println!("{}", runtime.block_on(concurrency(&variant, &origin, calls)));
        return;
    }
    let variant = Variant::from_args(&args[1], args.get(2).map(String::as_str) == Some("bwrap"));
    let runs: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(1);
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let origin = broker::origin();
    for run in 0..runs {
        let mut report = runtime.block_on(measure(&variant, &origin));
        report["run"] = json!(run + 1);
        println!("{report}");
    }
}
