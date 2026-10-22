//! Stand-in kernel brokers behind the real WorkerPeer. State follows the
//! kernel state broker's version checks; HTTP follows its open/headers/read
//! stream shape but only reaches the harness's own loopback origin.
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chariox_app_runtime::wire::RemoteError;
use chariox_app_runtime::worker_peer::{Broker, BrokerFuture, BrokerRequest};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Default)]
struct State {
    records: BTreeMap<String, (Value, u64)>,
    revision: u64,
}

pub struct SpikeBroker {
    state: Mutex<State>,
    streams: Mutex<HashMap<String, (u16, Vec<u8>, String)>>,
    pub ready: watch::Sender<Option<Value>>,
    pub logs: Mutex<Vec<Value>>,
    origin: String,
}

/// The only destination the HTTP broker stand-in admits; it is served by the
/// harness's loopback origin (the SDK requires HTTPS URLs).
pub const ALLOWED: &str = "https://spike-origin.example/";

fn failure(code: &str, message: &str) -> RemoteError {
    RemoteError { code: code.into(), message: message.into(), retryable: Some(false) }
}

impl SpikeBroker {
    pub fn new(origin: String) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::default(),
            streams: Mutex::default(),
            ready: watch::channel(None).0,
            logs: Mutex::default(),
            origin,
        })
    }

    fn transaction(&self, params: &Value) -> Result<Value, RemoteError> {
        let mut state = self.state.lock().unwrap();
        for check in params["checks"].as_array().into_iter().flatten() {
            let current = state.records.get(check["key"].as_str().unwrap_or_default()).map(|r| r.1);
            if current != check["version"].as_u64() {
                return Err(failure("STATE_CONFLICT", "State version check failed"));
            }
        }
        state.revision += 1;
        let revision = state.revision;
        for write in params["writes"].as_array().into_iter().flatten() {
            let key = write["key"].as_str().unwrap_or_default().to_string();
            if write["delete"] == json!(true) {
                state.records.remove(&key);
            } else {
                let version = state.records.get(&key).map_or(1, |r| r.1 + 1);
                state.records.insert(key, (write["value"].clone(), version));
            }
        }
        Ok(json!({ "revision": revision, "receipts": [] }))
    }

    fn open(&self, params: &Value) -> Result<Value, RemoteError> {
        let url = params["url"].as_str().unwrap_or_default();
        let Some(path) = url.strip_prefix(ALLOWED) else {
            return Err(failure("APP_HTTP_DENIED", "Destination is not allowed"));
        };
        let mut socket = std::net::TcpStream::connect(self.origin.as_str()).map_err(|_| failure("APP_HTTP_FAILED", "connect"))?;
        let request = format!("GET /{path} HTTP/1.0\r\nhost: origin\r\n\r\n");
        socket.write_all(request.as_bytes()).map_err(|_| failure("APP_HTTP_FAILED", "write"))?;
        let mut raw = Vec::new();
        socket.read_to_end(&mut raw).map_err(|_| failure("APP_HTTP_FAILED", "read"))?;
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| failure("APP_HTTP_FAILED", "head"))?;
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let status = head.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let id = format!("00000000-0000-4000-8000-{:012x}", self.streams.lock().unwrap().len() + 1);
        self.streams.lock().unwrap().insert(id.clone(), (status, raw[split + 4..].to_vec(), url.into()));
        Ok(json!({ "streamId": id }))
    }

    fn handle_sync(&self, request: &BrokerRequest) -> Result<Value, RemoteError> {
        let params = &request.params;
        let stream = || params["streamId"].as_str().unwrap_or_default().to_string();
        match request.method.as_str() {
            "worker.ready" => {
                self.ready.send_replace(Some(params.clone()));
                Ok(Value::Null)
            }
            "state.get" => Ok(self.state.lock().unwrap().records.get(params["key"].as_str().unwrap_or_default())
                .map_or(Value::Null, |(value, version)| json!({ "value": value, "version": version }))),
            "state.transaction" => self.transaction(params),
            "http.open" => self.open(params),
            "http.headers" => {
                let streams = self.streams.lock().unwrap();
                let (status, _, url) = streams.get(&stream()).ok_or_else(|| failure("NOT_FOUND", "stream"))?;
                Ok(json!({ "pending": false, "status": status, "headers": [["content-type", "text/plain"]], "url": url }))
            }
            "http.read" => {
                let mut streams = self.streams.lock().unwrap();
                let (_, body, _) = streams.get_mut(&stream()).ok_or_else(|| failure("NOT_FOUND", "stream"))?;
                if body.is_empty() {
                    return Ok(json!({ "pending": false, "done": true, "chunkBase64": "" }));
                }
                let chunk: Vec<u8> = body.drain(..body.len().min(64 * 1024)).collect();
                Ok(json!({ "pending": false, "done": false, "chunkBase64": B64.encode(chunk) }))
            }
            "http.cancel" => {
                self.streams.lock().unwrap().remove(&stream());
                Ok(Value::Null)
            }
            "log.write" => {
                self.logs.lock().unwrap().push(params.clone());
                Ok(Value::Null)
            }
            _ => Err(failure("METHOD_NOT_FOUND", "Unknown App method")),
        }
    }
}

pub struct Shared(pub Arc<SpikeBroker>);

impl Broker for Shared {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let outcome = self.0.handle_sync(&request);
        Box::pin(async move { outcome })
    }
}

/// Loopback origin the HTTP broker stand-in is allowed to reach.
pub fn origin() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match socket.read(&mut buffer) { Ok(0) | Err(_) => break, Ok(n) => request.extend_from_slice(&buffer[..n]) }
            }
            let body = "hello from the origin through the kernel HTTP broker\n".repeat(64);
            let response = format!("HTTP/1.0 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\n\r\n{body}", body.len());
            let _ = socket.write_all(response.as_bytes());
        }
    });
    format!("{address}")
}
