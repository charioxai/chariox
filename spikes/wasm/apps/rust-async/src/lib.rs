//! Spike Rust App on the async world: the r1 tools plus shared in-memory
//! state, a streamed HTTP body, an event subscription and a cancellable wait.
wit_bindgen::generate!({ world: "app", path: "../../wit" });

use chariox::app::{events, http, log, registry, state};
use chariox::app::types::{AppError, InvocationContext};
use exports::chariox::app::handlers::Guest;
use futures::future::{select, Either};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use wit_bindgen::{FutureReader, StreamResult};

struct App;
export!(App);

fn fail(code: &str, message: impl Into<String>) -> AppError {
    AppError { code: code.into(), message: message.into(), retryable: None }
}

fn parse(input: &str) -> Result<Value, AppError> {
    serde_json::from_str(input).map_err(|_| fail("INVALID_ARGUMENT", "input is not JSON"))
}

const TOOLS: [&str; 11] = ["echo", "counter", "fetch", "transform", "compute", "spin", "hog", "shared", "stream", "subscribe", "wait"];

/// State every concurrent caller shares: a counter, a cache and one
/// long-lived "connection" (a session id plus its request sequence).
struct Shared { counter: u64, cache: HashMap<u64, usize>, connection: Option<Connection> }
struct Connection { id: String, requests: u64 }

thread_local! {
    static SHARED: RefCell<Shared> = RefCell::new(Shared { counter: 0, cache: HashMap::new(), connection: None });
}

impl Guest for App {
    fn register() -> Result<(), AppError> {
        for name in TOOLS {
            registry::tool(name);
        }
        log::write(log::Level::Info, "registered", "{\"tools\":11}");
        Ok(())
    }

    async fn call_tool(name: String, input: String, _context: InvocationContext, cancel: FutureReader<String>) -> Result<String, AppError> {
        let input = parse(&input)?;
        let work = Box::pin(call(name, input));
        match select(work, std::pin::pin!(std::future::IntoFuture::into_future(cancel))).await {
            Either::Left((output, _)) => Ok(output?.to_string()),
            // Dropping `work` cancels its pending imports; the host releases
            // their broker streams.
            Either::Right((reason, _)) => Err(fail("CANCELLED", reason)),
        }
    }
}

async fn call(name: String, input: Value) -> Result<Value, AppError> {
    Ok(match name.as_str() {
        "echo" => input,
        "counter" => counter().await?,
        "fetch" => fetch(input["url"].as_str().unwrap_or_default()).await?,
        "transform" => transform(input["n"].as_u64().unwrap_or(1000), input["rounds"].as_u64().unwrap_or(1)),
        "compute" => compute(input["n"].as_u64().unwrap_or(1_000_000)),
        "spin" => spin(),
        "hog" => hog(input["mb"].as_u64().unwrap_or(64), input["kind"].as_str().unwrap_or("bytes")),
        "shared" => shared(input["url"].as_str().unwrap_or_default()).await?,
        "stream" => stream(input["url"].as_str().unwrap_or_default(), input["work"].as_u64().unwrap_or(0)).await?,
        "subscribe" => subscribe(input["name"].as_str().unwrap_or("tick"), input["count"].as_u64().unwrap_or(1), input["work"].as_u64().unwrap_or(0)).await?,
        "wait" => fetch(input["url"].as_str().unwrap_or_default()).await?,
        _ => return Err(fail("TOOL_NOT_FOUND", "unknown tool")),
    })
}

fn request(url: &str, connection: Option<String>) -> http::Request {
    http::Request {
        url: url.into(),
        method: "GET".into(),
        headers: vec![("accept".into(), "text/plain".into())],
        body: None,
        connection_id: connection,
        operation_id: None,
    }
}

async fn counter() -> Result<Value, AppError> {
    let current = state::get("count".into()).await?;
    let (count, version) = match &current {
        Some(record) => (parse(&record.value)?.as_u64().unwrap_or(0), Some(record.version)),
        None => (0, None),
    };
    let next = count + 1;
    let committed = state::transact(state::Transaction {
        schema_version: 0,
        checks: vec![state::Check { key: "count".into(), version }],
        writes: vec![state::Write::Put(("count".into(), next.to_string()))],
    }).await?;
    Ok(json!({ "count": next, "revision": committed.revision }))
}

async fn fetch(url: &str) -> Result<Value, AppError> {
    let response = http::send(request(url, None)).await?;
    Ok(json!({ "status": response.head.status, "bytes": response.body.len(),
               "head": String::from_utf8_lossy(&response.body[..response.body.len().min(16)]) }))
}

/// Counter and cache updates happen between awaits, so they are atomic for
/// the single-threaded instance; the broker call in between interleaves.
async fn shared(url: &str) -> Result<Value, AppError> {
    let (n, connection, sequence) = SHARED.with(|shared| {
        let mut shared = shared.borrow_mut();
        shared.counter += 1;
        let n = shared.counter;
        let connection = shared.connection.get_or_insert_with(|| Connection { id: format!("conn-{n}"), requests: 0 });
        connection.requests += 1;
        (n, connection.id.clone(), connection.requests)
    });
    let response = http::send(request(url, Some(connection.clone()))).await?;
    let cached = SHARED.with(|shared| {
        let mut shared = shared.borrow_mut();
        shared.cache.insert(n, response.body.len());
        shared.cache.len()
    });
    Ok(json!({ "n": n, "sequence": sequence, "connection": connection, "cached": cached }))
}

/// Reads a streamed body; `work` rounds of hashing per chunk make a slow reader.
async fn stream(url: &str, work: u64) -> Result<Value, AppError> {
    let (head, mut body) = http::fetch(request(url, None)).await?;
    let (mut bytes, mut chunks, mut hash) = (0usize, 0u64, 0u32);
    loop {
        let (result, chunk) = body.read(Vec::with_capacity(64 * 1024)).await;
        match result {
            StreamResult::Complete(_) => {
                bytes += chunk.len();
                chunks += 1;
                for _ in 0..work {
                    for byte in &chunk { hash = hash.wrapping_mul(31).wrapping_add(*byte as u32); }
                }
            }
            StreamResult::Dropped | StreamResult::Cancelled => break,
        }
    }
    Ok(json!({ "status": head.status, "bytes": bytes, "chunks": chunks, "hash": hash }))
}

async fn subscribe(name: &str, count: u64, work: u64) -> Result<Value, AppError> {
    let mut events = events::subscribe(name.into()).await?;
    let (mut received, mut in_order, mut last, mut hash) = (0u64, true, 0u64, 0u32);
    while received < count {
        let Some(event) = events.next().await else { break };
        let sequence = parse(&event.data)?["seq"].as_u64().unwrap_or(0);
        in_order &= sequence == last + 1;
        last = sequence;
        received += 1;
        for i in 0..work { hash = hash.wrapping_mul(31).wrapping_add(i as u32); }
    }
    Ok(json!({ "received": received, "inOrder": in_order, "last": last, "hash": hash }))
}

fn transform(n: u64, rounds: u64) -> Value {
    let records: Vec<Value> = (0..n)
        .map(|i| json!({ "id": i, "name": format!("item-{i}"), "score": (i * 7919) % 100, "tags": ["a", "b", "c"] }))
        .collect();
    let text = Value::Array(records).to_string();
    let mut sum = 0u64;
    let mut out_bytes = 0usize;
    for _ in 0..rounds {
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let picked: Vec<Value> = parsed.as_array().unwrap().iter()
            .filter(|r| r["score"].as_u64().unwrap() > 50)
            .map(|r| {
                sum += r["score"].as_u64().unwrap();
                json!({ "id": r["id"], "name": r["name"].as_str().unwrap().to_uppercase() })
            })
            .collect();
        out_bytes = Value::Array(picked).to_string().len();
    }
    json!({ "sum": sum, "inBytes": text.len(), "outBytes": out_bytes })
}

fn compute(n: u64) -> Value {
    let mut h: i32 = 0;
    let mut counts = HashMap::new();
    for i in 0..n {
        *counts.entry(format!("k{}", i % 1000)).or_insert(0u64) += 1;
        h = h.wrapping_mul(31).wrapping_add(i as i32);
    }
    json!({ "h": h, "keys": counts.len() })
}

fn spin() -> Value {
    let mut x = 0u64;
    loop {
        x = std::hint::black_box(x.wrapping_add(1));
    }
}

fn hog(mb: u64, kind: &str) -> Value {
    if kind == "objects" {
        let objects: Vec<String> = (0..mb * 10000).map(|i| format!("{}{i}", "x".repeat(64))).collect();
        return json!({ "allocatedMb": mb, "objects": objects.len() });
    }
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    for i in 0..mb {
        chunks.push(vec![(i % 251) as u8; 1 << 20]);
    }
    json!({ "allocatedMb": chunks.len() })
}
