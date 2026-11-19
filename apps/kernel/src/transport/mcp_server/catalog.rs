//! MCP 2025 Streamable HTTP catalog delivery. Invalidation hints contain no
//! identities or tools; each connection compares its own authenticated catalog.
use std::collections::BTreeSet;
use std::sync::OnceLock;

use futures_util::stream;
use http_body_util::StreamBody;
use hyper::body::Frame;
use tokio::sync::{broadcast, mpsc};

use super::*;

type Signature = Vec<(String, String, Value)>;

/// Arms continuations and reloads for real catalog changes; notification
/// streams compare the full listed `snapshot` instead. It runs beside the
/// accept loop: computing every running provider's catalog can take seconds on
/// a busy kernel, and a provider whose first connection waits behind it starts
/// its turn without its tools.
pub(super) struct CatalogMonitor {
    catalogs: std::sync::Mutex<std::collections::BTreeMap<String, Signature>>,
}

impl CatalogMonitor {
    pub(super) fn new() -> Self {
        Self {
            catalogs: Default::default(),
        }
    }

    /// Refreshes once at start and again after every change hint; hints that
    /// arrive during a refresh are folded into the next one.
    pub(super) async fn watch(self: Arc<Self>, router: Arc<CommandRouter>) -> Infallible {
        let mut changes = changes().subscribe();
        loop {
            let (monitor, router) = (Arc::clone(&self), Arc::clone(&router));
            let _ = tokio::task::spawn_blocking(move || monitor.refresh(&router)).await;
            let _ = changes.recv().await; // lag also requires comparing current catalogs
            while changes.try_recv().is_ok() {}
        }
    }

    /// Records a new run's catalog before its first MCP request is served, so
    /// a change between that listing and the next refresh is not missed.
    pub(super) async fn seed(self: &Arc<Self>, router: &Arc<CommandRouter>, token: &str) {
        if self.lock().contains_key(token) {
            return;
        }
        let (monitor, router, token) = (Arc::clone(self), Arc::clone(router), token.to_owned());
        let _ = tokio::task::spawn_blocking(move || {
            if router.runtime_mcp_catalog_run(&token).is_none() {
                return;
            }
            let signature = router.runtime_catalog_signature(&token);
            monitor.lock().entry(token).or_insert(signature);
        })
        .await;
    }

    /// Only this refresh replaces a known run's catalog; a run seeded while it
    /// computed keeps the catalog its first request was served after.
    pub(super) fn refresh(&self, router: &CommandRouter) -> usize {
        let known = self.lock().keys().cloned().collect::<BTreeSet<_>>();
        let next = router
            .runtime_tool_catalog_auth_tokens()
            .into_iter()
            .map(|token| {
                let signature = router.runtime_catalog_signature(&token);
                (token, signature)
            })
            .collect::<Vec<_>>();
        // Read after computing, so a run seeded meanwhile is not dropped.
        let running = router
            .runtime_tool_catalog_auth_tokens()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut changed = Vec::new();
        {
            let mut catalogs = self.lock();
            catalogs.retain(|token, _| running.contains(token));
            for (token, signature) in next {
                if !running.contains(&token) {
                    continue;
                }
                match catalogs.get_mut(&token) {
                    Some(current) if known.contains(&token) => {
                        if *current != signature {
                            *current = signature;
                            changed.push(token);
                        }
                    }
                    Some(_) => {}
                    None => {
                        catalogs.insert(token, signature);
                    }
                }
            }
        }
        for token in &changed {
            router.runtime_tool_catalog_changed_for_auth_token(token);
        }
        changed.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<String, Signature>> {
        self.catalogs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn changes() -> &'static broadcast::Sender<()> {
    static CHANGES: OnceLock<broadcast::Sender<()>> = OnceLock::new();
    CHANGES.get_or_init(|| broadcast::channel(64).0)
}

pub(super) fn changed() {
    let _ = changes().send(());
}

pub(super) fn snapshot(router: &CommandRouter, token: &str) -> Value {
    let mut tools = router.runtime_tool_specs_for_auth_token(token);
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    Value::Array(tools.into_iter().map(|tool| serde_json::json!({
        "name": tool.name, "description": tool.description, "inputSchema": tool.input_schema,
    })).collect())
}

fn notification() -> Value {
    serde_json::json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"})
}

fn event(value: &Value) -> String {
    format!("event: message\ndata: {value}\n\n")
}

pub(super) fn tool_response(changed: bool, value: Value) -> Response<HttpBody> {
    if !changed {
        return json_response(StatusCode::OK, value);
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header("Cache-Control", "no-cache")
        .body(Full::new(Bytes::from(event(&notification()) + &event(&value))).boxed_unsync())
        .expect("static MCP response headers")
}

pub(super) fn stream_response(
    router: Arc<CommandRouter>,
    headers: &hyper::HeaderMap,
) -> Response<HttpBody> {
    let Some(token) = parse_bearer_token(headers) else {
        return text_response(StatusCode::UNAUTHORIZED, "unauthorized".into());
    };
    // Subscribe before reading the catalog so a concurrent mutation cannot be
    // lost between the initial snapshot and the first recv.
    let mut changes = changes().subscribe();
    let mut catalog = snapshot(&router, &token);
    if catalog.as_array().is_none_or(Vec::is_empty) {
        return text_response(StatusCode::UNAUTHORIZED, "unauthorized".into());
    }
    let (sender, receiver) = mpsc::channel::<Bytes>(2);
    sender
        .try_send(Bytes::from_static(b": connected\n\n"))
        .expect("empty stream");
    tokio::spawn(async move {
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
        heartbeat.tick().await;
        loop {
            let heartbeat_due = tokio::select! {
                _ = sender.closed() => break,
                _ = changes.recv() => false, // includes lag: compare the current catalog
                _ = heartbeat.tick() => true,
            };
            let next = snapshot(&router, &token);
            if next.as_array().is_none_or(Vec::is_empty) {
                break; // ended/replaced provider token loses its stream too
            }
            let bytes = if next != catalog {
                catalog = next;
                Bytes::from(event(&notification()))
            } else if heartbeat_due {
                Bytes::from_static(b": keepalive\n\n")
            } else {
                continue;
            };
            if sender.send(bytes).await.is_err() {
                break;
            }
        }
    });
    let body = StreamBody::new(stream::unfold(receiver, |mut receiver| async move {
        receiver
            .recv()
            .await
            .map(|bytes| (Ok::<_, Infallible>(Frame::data(bytes)), receiver))
    }))
    .boxed_unsync();
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header("Cache-Control", "no-cache")
        .body(body)
        .expect("static MCP stream headers")
}
