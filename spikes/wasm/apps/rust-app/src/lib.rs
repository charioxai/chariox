//! Spike Rust App: the same six tools as the JS and Node Apps.
wit_bindgen::generate!({ world: "app", path: "../../wit-sync" });

use chariox::app::{http, log, registry, state};
use chariox::app::types::{AppError, InvocationContext};
use exports::chariox::app::handlers::Guest;
use serde_json::{json, Value};

struct App;
export!(App);

fn fail(code: &str, message: impl Into<String>) -> AppError {
    AppError { code: code.into(), message: message.into(), retryable: None }
}

fn parse(input: &str) -> Result<Value, AppError> {
    serde_json::from_str(input).map_err(|_| fail("INVALID_ARGUMENT", "input is not JSON"))
}

const TOOLS: [&str; 7] = ["echo", "counter", "fetch", "transform", "compute", "spin", "hog"];

impl Guest for App {
    fn register() -> Result<(), AppError> {
        for name in TOOLS {
            registry::tool(name);
        }
        log::write(log::Level::Info, "registered", "{\"tools\":7}");
        Ok(())
    }

    fn call_tool(name: String, input: String, _context: InvocationContext) -> Result<String, AppError> {
        let input = parse(&input)?;
        let output = match name.as_str() {
            "echo" => input,
            "counter" => counter()?,
            "fetch" => fetch(input["url"].as_str().unwrap_or_default())?,
            "transform" => transform(input["n"].as_u64().unwrap_or(1000), input["rounds"].as_u64().unwrap_or(1)),
            "compute" => compute(input["n"].as_u64().unwrap_or(1_000_000)),
            "spin" => spin(),
            "hog" => hog(input["mb"].as_u64().unwrap_or(64), input["kind"].as_str().unwrap_or("bytes")),
            _ => return Err(fail("TOOL_NOT_FOUND", "unknown tool")),
        };
        Ok(output.to_string())
    }
}

fn counter() -> Result<Value, AppError> {
    let current = state::get("count")?;
    let (count, version) = match &current {
        Some(record) => (parse(&record.value)?.as_u64().unwrap_or(0), Some(record.version)),
        None => (0, None),
    };
    let next = count + 1;
    let committed = state::transact(&state::Transaction {
        schema_version: 0,
        checks: vec![state::Check { key: "count".into(), version }],
        writes: vec![state::Write::Put(("count".into(), next.to_string()))],
    })?;
    Ok(json!({ "count": next, "revision": committed.revision }))
}

fn fetch(url: &str) -> Result<Value, AppError> {
    let response = http::send(&http::Request {
        url: url.into(),
        method: "GET".into(),
        headers: vec![("accept".into(), "text/plain".into())],
        body: None,
        connection_id: None,
        operation_id: None,
    })?;
    Ok(json!({ "status": response.status, "bytes": response.body.len(),
               "head": String::from_utf8_lossy(&response.body[..response.body.len().min(16)]) }))
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
    let mut counts = std::collections::HashMap::new();
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
