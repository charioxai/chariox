//! MCP 2025 Streamable HTTP catalog delivery. Invalidation hints contain no
//! identities or tools; each connection compares its own authenticated catalog.
use std::sync::OnceLock;

use futures_util::stream;
use http_body_util::StreamBody;
use hyper::body::Frame;
use tokio::sync::{broadcast, mpsc};

use super::*;

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
                router.runtime_tool_catalog_changed_for_auth_token(&token);
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
