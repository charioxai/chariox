//! wasmtime host for the chariox:app world: imports become kernel broker calls.
use crate::ipc::{error, Ipc};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chariox_app_runtime::wire::RemoteError;
use serde_json::{json, Value};
use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    world: "app",
    path: "../wit",
    imports: { default: async },
    exports: { default: async },
});

use chariox::app::types::AppError;
use chariox::app::{http, log, registry, state};

pub struct Host {
    pub ipc: Ipc,
    pub wasi: WasiCtx,
    pub table: ResourceTable,
    pub limits: MemoryCap,
    pub tools: Vec<String>,
    pub events: Vec<String>,
    pub lifecycle: Vec<String>,
}

/// A refused grow returns -1 to the guest instead of trapping, so a garbage
/// collected guest (StarlingMonkey) can collect and retry. The refusal is
/// remembered so a trap that follows is reported as the memory limit.
pub struct MemoryCap {
    pub bytes: usize,
    pub refused: bool,
}

impl wasmtime::ResourceLimiter for MemoryCap {
    fn memory_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        if desired > self.bytes {
            self.refused = true;
            return Ok(false);
        }
        Ok(true)
    }
    fn table_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= 1 << 20)
    }
    fn instances(&self) -> usize { 4 }
    fn memories(&self) -> usize { 4 }
}

impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

fn app_error(error: RemoteError) -> AppError {
    AppError { code: error.code, message: error.message, retryable: error.retryable }
}

pub fn remote_error(error: AppError) -> RemoteError {
    RemoteError { code: error.code, message: error.message, retryable: error.retryable }
}

/// Guest JSON text is checked once here, before any byte reaches the wire.
fn json_value(text: &str, what: &str) -> Result<Value, AppError> {
    serde_json::from_str(text).map_err(|_| AppError {
        code: "INVALID_ARGUMENT".into(),
        message: format!("{what} is not JSON"),
        retryable: None,
    })
}

impl chariox::app::types::Host for Host {}

impl registry::Host for Host {
    async fn tool(&mut self, name: String) {
        self.tools.push(name);
    }
    async fn event(&mut self, name: String) {
        self.events.push(name);
    }
    async fn lifecycle(&mut self, event: String) {
        self.lifecycle.push(event);
    }
}

impl state::Host for Host {
    async fn get(&mut self, key: String) -> Result<Option<state::StateRecord>, AppError> {
        let result = self.ipc.call("state.get", json!({ "key": key })).await.map_err(app_error)?;
        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(state::StateRecord {
            value: result["value"].to_string(),
            version: result["version"].as_u64().unwrap_or(0),
        }))
    }

    async fn transact(&mut self, tx: state::Transaction) -> Result<state::Committed, AppError> {
        let checks: Vec<Value> = tx.checks.iter().map(|c| json!({ "key": c.key, "version": c.version })).collect();
        let mut writes = Vec::new();
        for write in tx.writes {
            writes.push(match write {
                state::Write::Put((key, value)) => json!({ "key": key, "value": json_value(&value, "state value")? }),
                state::Write::Delete(key) => json!({ "key": key, "delete": true }),
            });
        }
        let params = json!({ "schemaVersion": tx.schema_version, "checks": checks, "writes": writes });
        let result = self.ipc.call("state.transaction", params).await.map_err(app_error)?;
        Ok(state::Committed { revision: result["revision"].as_u64().unwrap_or(0) })
    }
}

const CHUNK: usize = 64 * 1024;
const BUFFERED: usize = 512 * 1024;

impl http::Host for Host {
    /// Same broker sequence as the JS SDK's buffered `http.request`.
    async fn send(&mut self, req: http::Request) -> Result<http::Response, AppError> {
        let mut open = json!({ "url": req.url, "method": req.method, "headers": req.headers, "hasBody": req.body.is_some() });
        if let Some(id) = req.connection_id { open["connectionId"] = id.into(); }
        if let Some(id) = req.operation_id { open["operationId"] = id.into(); }
        let opened = self.ipc.call("http.open", open).await.map_err(app_error)?;
        let stream = opened["streamId"].as_str().unwrap_or_default().to_string();
        let outcome = self.exchange(&stream, req.body).await;
        let _ = self.ipc.call("http.cancel", json!({ "streamId": stream })).await;
        outcome
    }
}

impl Host {
    async fn exchange(&mut self, stream: &str, body: Option<Vec<u8>>) -> Result<http::Response, AppError> {
        if let Some(body) = body {
            let parts: Vec<&[u8]> = if body.is_empty() { vec![&body[..]] } else { body.chunks(CHUNK).collect() };
            let last = parts.len() - 1;
            for (index, part) in parts.into_iter().enumerate() {
                let params = json!({ "streamId": stream, "bodyBase64": B64.encode(part), "end": index == last });
                self.ipc.call("http.write", params).await.map_err(app_error)?;
            }
        }
        let head = loop {
            let head = self.ipc.call("http.headers", json!({ "streamId": stream })).await.map_err(app_error)?;
            if head["pending"] != json!(true) { break head; }
        };
        let mut body = Vec::new();
        loop {
            let part = self.ipc.call("http.read", json!({ "streamId": stream })).await.map_err(app_error)?;
            if part["pending"] == json!(true) { continue; }
            if part["done"] == json!(true) { break; }
            let chunk = B64.decode(part["chunkBase64"].as_str().unwrap_or_default())
                .map_err(|_| app_error(error("PROTOCOL_ERROR", "Invalid HTTP response encoding")))?;
            body.extend_from_slice(&chunk);
            if body.len() > BUFFERED {
                return Err(app_error(error("APP_HTTP_LIMIT", "Buffered HTTP response exceeds 512 KiB")));
            }
        }
        let headers = serde_json::from_value(head["headers"].clone()).unwrap_or_default();
        Ok(http::Response {
            status: head["status"].as_u64().unwrap_or(0) as u16,
            headers,
            url: head["url"].as_str().unwrap_or_default().into(),
            body,
        })
    }
}

impl log::Host for Host {
    async fn write(&mut self, level: log::Level, message: String, fields: String) {
        let level = match level { log::Level::Debug => "debug", log::Level::Info => "info", log::Level::Warn => "warn", log::Level::Error => "error" };
        let Ok(fields) = json_value(&fields, "log fields") else { return };
        let _ = self.ipc.call("log.write", json!({ "level": level, "message": message, "fields": fields })).await;
    }
}
