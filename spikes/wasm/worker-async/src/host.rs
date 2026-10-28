//! wasmtime host for the async chariox:app world. Every async import runs
//! concurrently with the instance's other calls and becomes kernel broker
//! calls on the existing worker IPC.
use crate::ipc::{error, Ipc};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chariox_app_runtime::wire::RemoteError;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Instant;
use tokio::sync::mpsc;
use wasmtime::component::{
    Accessor, Destination, HasSelf, ResourceTable, StreamProducer, StreamReader, StreamResult, VecBuffer,
};
use wasmtime::StoreContextMut;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

// `register` is a sync export; the store has async host functions, so it is
// called with call_async.
wasmtime::component::bindgen!({
    world: "app",
    path: "../wit",
    exports: { "chariox:app/handlers.register": async },
    with: { "wasi": wasmtime_wasi::p3::bindings },
});

use chariox::app::types::{AppError, Event};
use chariox::app::{events, http, log, registry, state};

/// Senders of the open event subscriptions, by event name. The IPC side
/// delivers `events.deliver` into them.
pub type Subscriptions = Arc<Mutex<HashMap<String, mpsc::Sender<Event>>>>;

pub struct Host {
    pub ipc: Ipc,
    pub wasi: WasiCtx,
    pub table: ResourceTable,
    pub limits: MemoryCap,
    pub subscriptions: Subscriptions,
    pub stats: Arc<Stats>,
    /// When the guest last got the CPU from the event loop; `None` while the
    /// instance waits. The epoch callback stops a slice that runs too long.
    pub slice: Option<Instant>,
    pub tools: Vec<String>,
    pub events: Vec<String>,
    pub lifecycle: Vec<String>,
}

/// Counters the spike reports: broker streams opened and released.
#[derive(Default)]
pub struct Stats {
    pub streams_opened: AtomicU64,
    pub streams_released: AtomicU64,
    pub chunks_buffered_max: AtomicU64,
}

/// A refused grow returns -1 to the guest instead of trapping, so a garbage
/// collected guest can collect and retry. The refusal is remembered so a trap
/// that follows is reported as the memory limit.
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
    // componentize-qjs components hold 5+ core instances.
    fn instances(&self) -> usize { 16 }
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

fn ipc<T>(accessor: &Accessor<T, HasSelf<Host>>) -> Ipc {
    accessor.with(|mut access| access.get().ipc.clone())
}

impl chariox::app::types::Host for Host {}

impl registry::Host for Host {
    fn tool(&mut self, name: String) {
        self.tools.push(name);
    }
    fn event(&mut self, name: String) {
        self.events.push(name);
    }
    fn lifecycle(&mut self, event: String) {
        self.lifecycle.push(event);
    }
}

impl log::Host for Host {
    /// Fire-and-forget: a log line never holds the instance.
    fn write(&mut self, level: log::Level, message: String, fields: String) {
        let level = match level { log::Level::Debug => "debug", log::Level::Info => "info", log::Level::Warn => "warn", log::Level::Error => "error" };
        let Ok(fields) = json_value(&fields, "log fields") else { return };
        let ipc = self.ipc.clone();
        tokio::spawn(async move {
            let _ = ipc.call("log.write", json!({ "level": level, "message": message, "fields": fields })).await;
        });
    }
}

impl state::Host for Host {}
impl<T: Send> state::HostWithStore<T> for HasSelf<Host> {
    async fn get(accessor: &Accessor<T, Self>, key: String) -> Result<Option<state::StateRecord>, AppError> {
        let result = ipc(accessor).call("state.get", json!({ "key": key })).await.map_err(app_error)?;
        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(state::StateRecord {
            value: result["value"].to_string(),
            version: result["version"].as_u64().unwrap_or(0),
        }))
    }

    async fn transact(accessor: &Accessor<T, Self>, tx: state::Transaction) -> Result<state::Committed, AppError> {
        let checks: Vec<Value> = tx.checks.iter().map(|c| json!({ "key": c.key, "version": c.version })).collect();
        let mut writes = Vec::new();
        for write in tx.writes {
            writes.push(match write {
                state::Write::Put((key, value)) => json!({ "key": key, "value": json_value(&value, "state value")? }),
                state::Write::Delete(key) => json!({ "key": key, "delete": true }),
            });
        }
        let params = json!({ "schemaVersion": tx.schema_version, "checks": checks, "writes": writes });
        let result = ipc(accessor).call("state.transaction", params).await.map_err(app_error)?;
        Ok(state::Committed { revision: result["revision"].as_u64().unwrap_or(0) })
    }
}

const BUFFERED: usize = 512 * 1024;
/// Streamed bodies: at most this many 64 KiB broker chunks wait for the guest.
const READ_AHEAD: usize = 4;

/// One open broker stream. Dropping it (call finished, guest cancelled the
/// import or dropped the body) releases the kernel stream.
struct BrokerStream {
    ipc: Ipc,
    id: String,
    stats: Arc<Stats>,
}

impl Drop for BrokerStream {
    fn drop(&mut self) {
        let (ipc, id) = (self.ipc.clone(), std::mem::take(&mut self.id));
        self.stats.streams_released.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let _ = ipc.call("http.cancel", json!({ "streamId": id })).await;
        });
    }
}

async fn open(ipc: &Ipc, stats: &Arc<Stats>, req: http::Request) -> Result<(BrokerStream, http::ResponseHead), AppError> {
    let mut params = json!({ "url": req.url, "method": req.method, "headers": req.headers, "hasBody": req.body.is_some() });
    if let Some(id) = req.connection_id { params["connectionId"] = id.into(); }
    if let Some(id) = req.operation_id { params["operationId"] = id.into(); }
    let opened = ipc.call("http.open", params).await.map_err(app_error)?;
    stats.streams_opened.fetch_add(1, Ordering::Relaxed);
    let stream = BrokerStream { ipc: ipc.clone(), id: opened["streamId"].as_str().unwrap_or_default().to_string(), stats: stats.clone() };
    if let Some(body) = req.body {
        let parts: Vec<&[u8]> = if body.is_empty() { vec![&body[..]] } else { body.chunks(64 * 1024).collect() };
        let last = parts.len() - 1;
        for (index, part) in parts.into_iter().enumerate() {
            let params = json!({ "streamId": stream.id, "bodyBase64": B64.encode(part), "end": index == last });
            ipc.call("http.write", params).await.map_err(app_error)?;
        }
    }
    let head = loop {
        let head = ipc.call("http.headers", json!({ "streamId": stream.id })).await.map_err(app_error)?;
        if head["pending"] != json!(true) { break head; }
    };
    let head = http::ResponseHead {
        status: head["status"].as_u64().unwrap_or(0) as u16,
        headers: serde_json::from_value(head["headers"].clone()).unwrap_or_default(),
        url: head["url"].as_str().unwrap_or_default().into(),
    };
    Ok((stream, head))
}

/// Next body chunk, `None` at the end.
async fn read(stream: &BrokerStream) -> Result<Option<Vec<u8>>, AppError> {
    loop {
        let part = stream.ipc.call("http.read", json!({ "streamId": stream.id })).await.map_err(app_error)?;
        if part["pending"] == json!(true) { continue; }
        if part["done"] == json!(true) { return Ok(None); }
        return B64.decode(part["chunkBase64"].as_str().unwrap_or_default())
            .map(Some)
            .map_err(|_| app_error(error("PROTOCOL_ERROR", "Invalid HTTP response encoding")));
    }
}

fn stats<T>(accessor: &Accessor<T, HasSelf<Host>>) -> Arc<Stats> {
    accessor.with(|mut access| access.get().stats.clone())
}

impl http::Host for Host {}
impl<T: Send> http::HostWithStore<T> for HasSelf<Host> {
    async fn send(accessor: &Accessor<T, Self>, req: http::Request) -> Result<http::Response, AppError> {
        let (ipc, stats) = (ipc(accessor), stats(accessor));
        let (stream, head) = open(&ipc, &stats, req).await?;
        let mut body = Vec::new();
        while let Some(chunk) = read(&stream).await? {
            body.extend_from_slice(&chunk);
            if body.len() > BUFFERED {
                return Err(app_error(error("APP_HTTP_LIMIT", "Buffered HTTP response exceeds 512 KiB; use http.fetch")));
            }
        }
        Ok(http::Response { head, body })
    }

    async fn fetch(accessor: &Accessor<T, Self>, req: http::Request) -> Result<(http::ResponseHead, StreamReader<u8>), AppError> {
        let (ipc, stats) = (ipc(accessor), stats(accessor));
        let (stream, head) = open(&ipc, &stats, req).await?;
        let (chunks, receiver) = mpsc::channel::<Vec<u8>>(READ_AHEAD);
        let watch = stats.clone();
        // The reader stops when the guest drops the body (send fails), which
        // drops `stream` and cancels the broker stream.
        tokio::spawn(async move {
            while let Ok(Some(chunk)) = read(&stream).await {
                if chunks.send(chunk).await.is_err() { break; }
                let queued = (READ_AHEAD - chunks.capacity()) as u64;
                watch.chunks_buffered_max.fetch_max(queued, Ordering::Relaxed);
            }
        });
        let body = accessor.with(|mut access| StreamReader::new(&mut access, Bytes(receiver)))
            .map_err(|_| app_error(error("APP_UNAVAILABLE", "stream table full")))?;
        Ok((head, body))
    }
}

/// A host stream fed from a bounded channel: the guest's reads pull, the
/// sender waits when the guest is slow.
struct Channel<I>(mpsc::Receiver<I>);

impl<D, I: Send + Sync + 'static> StreamProducer<D> for Channel<I> {
    type Item = I;
    type Buffer = VecBuffer<I>;

    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _store: StoreContextMut<'a, D>,
        mut destination: Destination<'a, I, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        match self.get_mut().0.poll_recv(cx) {
            Poll::Ready(Some(item)) => {
                destination.set_buffer(VecBuffer::from(vec![item]));
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Pending if finish => Poll::Ready(Ok(StreamResult::Cancelled)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Byte streams deliver whole chunks, not one byte per item.
struct Bytes(mpsc::Receiver<Vec<u8>>);

impl<D> StreamProducer<D> for Bytes {
    type Item = u8;
    type Buffer = VecBuffer<u8>;

    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        _store: StoreContextMut<'a, D>,
        mut destination: Destination<'a, u8, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        match self.get_mut().0.poll_recv(cx) {
            Poll::Ready(Some(chunk)) => {
                destination.set_buffer(VecBuffer::from(chunk));
                Poll::Ready(Ok(StreamResult::Completed))
            }
            Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Pending if finish => Poll::Ready(Ok(StreamResult::Cancelled)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T: Send> events::HostWithStore<T> for HasSelf<Host> {
    async fn subscribe(accessor: &Accessor<T, Self>, name: String) -> Result<StreamReader<Event>, AppError> {
        let subscriptions = accessor.with(|mut access| access.get().subscriptions.clone());
        let (sender, receiver) = mpsc::channel::<Event>(16);
        subscriptions.lock().unwrap().insert(name, sender);
        accessor.with(|mut access| StreamReader::new(&mut access, Channel(receiver)))
            .map_err(|_| app_error(error("APP_UNAVAILABLE", "stream table full")))
    }
}
impl events::Host for Host {}
