//! Standard MCP notifications on the existing authenticated HTTP endpoint.
use super::*;
use crate::runtime::runtime_tool_catalog::CatalogWatch;
use std::{sync::Weak, time::Duration};

struct Stream {
    router: Weak<CommandRouter>,
    token: String,
    run_id: String,
    watch: CatalogWatch,
    shutdown: tokio::sync::watch::Receiver<()>,
    last_sent: Option<u64>,
}

pub(super) fn open(
    router: Arc<CommandRouter>,
    headers: &hyper::HeaderMap,
    shutdown: tokio::sync::watch::Receiver<()>,
) -> Response<HttpBody> {
    let Some(token) = parse_bearer_token(headers) else {
        return empty_response(StatusCode::UNAUTHORIZED);
    };
    let Some(run) = router.runtime_mcp_catalog_run(&token) else {
        return empty_response(StatusCode::UNAUTHORIZED);
    };
    let Some(watch) = router.runtime_mcp_catalog_changes().subscribe(run.id()) else {
        return empty_response(StatusCode::SERVICE_UNAVAILABLE);
    };
    let stream = futures_util::stream::unfold(
        Stream {
            router: Arc::downgrade(&router),
            token,
            run_id: run.id().into(),
            watch,
            shutdown,
            last_sent: None,
        },
        |mut stream| async move {
            loop {
                if stream.shutdown.has_changed().is_err() || stream.watch.is_closed() {
                    return None;
                }
                let router = stream.router.upgrade()?;
                if !router
                    .runtime_mcp_catalog_run(&stream.token)
                    .is_some_and(|run| run.id() == stream.run_id)
                {
                    return None;
                }
                drop(router);
                let desired = stream.watch.current().desired;
                if stream.last_sent != Some(desired) {
                    stream.last_sent = Some(desired);
                    // No tool names, transcript, credentials or run identifiers.
                    // A new connection always invalidates once, covering changes
                    // missed while disconnected without an unbounded replay log.
                    let frame = hyper::body::Frame::data(Bytes::from_static(
                    b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n"
                ));
                    return Some((Ok::<_, Infallible>(frame), stream));
                }
                tokio::select! {
                    _ = stream.shutdown.changed() => return None,
                    changed = stream.watch.changed() => if changed.is_err() { return None; },
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {
                        return Some((Ok(hyper::body::Frame::data(Bytes::from_static(b": keep-alive\n\n"))), stream));
                    }
                }
            }
        },
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header("Cache-Control", "no-cache")
        .body(http_body_util::StreamBody::new(stream).boxed_unsync())
        .unwrap_or_else(|_| empty_response(StatusCode::INTERNAL_SERVER_ERROR))
}
