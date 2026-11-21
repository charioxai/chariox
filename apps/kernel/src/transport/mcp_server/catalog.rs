//! MCP 2025 Streamable HTTP catalog delivery. Invalidation hints contain no
//! identities or tools; each connection compares its own authenticated catalog.
use std::collections::BTreeSet;
use std::sync::OnceLock;

use futures_util::{stream, StreamExt as _};
use http_body_util::StreamBody;
use hyper::body::Frame;
use tokio::sync::{broadcast, mpsc, watch};

use super::*;

type Listing = Vec<(String, String, Value)>;

/// How often the monitor compares catalogs without a change hint: some
/// changes (a sudo window closing) send none.
const UNHINTED_REFRESH: std::time::Duration = std::time::Duration::from_secs(15);

struct Catalog {
    listing: Listing,
    /// Wakes this run's notification streams when its listing changes; dropped
    /// with the entry when the run ends, which ends its streams.
    listed: watch::Sender<()>,
}

/// The kernel's one view of every running provider's catalog. It computes
/// each catalog once per change, beside the accept loop and off the async
/// runtime: on a busy kernel that takes seconds, and neither a provider's
/// first connection nor its open notification stream may wait for it. A
/// change to a run's listing wakes its streams; a change outside the sudo
/// tools also arms the run's reload or continuation.
pub(super) struct CatalogMonitor {
    catalogs: std::sync::Mutex<std::collections::BTreeMap<String, Catalog>>,
}

impl CatalogMonitor {
    pub(super) fn new() -> Self {
        Self {
            catalogs: Default::default(),
        }
    }

    /// Refreshes once at start, again after every change hint (hints that
    /// arrive during a refresh are folded into the next one) and at least
    /// every `UNHINTED_REFRESH`.
    pub(super) async fn watch(self: Arc<Self>, router: Arc<CommandRouter>) -> Infallible {
        let mut changes = changes().subscribe();
        loop {
            let (monitor, router) = (Arc::clone(&self), Arc::clone(&router));
            let _ = tokio::task::spawn_blocking(move || monitor.refresh(&router)).await;
            tokio::select! {
                _ = changes.recv() => {} // lag also requires comparing current catalogs
                _ = tokio::time::sleep(UNHINTED_REFRESH) => {}
            }
            while changes.try_recv().is_ok() {}
        }
    }

    /// Records a run's catalog before opening its notification stream, so a
    /// change between that listing and the next refresh is not missed.
    pub(super) async fn seed(self: &Arc<Self>, router: &Arc<CommandRouter>, token: &str) {
        if self.lock().contains_key(token) {
            return;
        }
        let (monitor, router, token) = (Arc::clone(self), Arc::clone(router), token.to_owned());
        let _ = tokio::task::spawn_blocking(move || {
            if router.runtime_mcp_catalog_run(&token).is_none() {
                return;
            }
            let listing = router.runtime_catalog_listing(&token);
            monitor.lock().entry(token).or_insert_with(|| Catalog {
                listing,
                listed: watch::channel(()).0,
            });
        })
        .await;
    }

    /// Wakes when this run's listing changes and closes when the run ends;
    /// `None` for a token without a running run or without tools.
    fn subscribe(&self, token: &str) -> Option<watch::Receiver<()>> {
        self.lock()
            .get(token)
            .filter(|catalog| !catalog.listing.is_empty())
            .map(|catalog| catalog.listed.subscribe())
    }

    /// Only this refresh replaces a known run's catalog; a run seeded while it
    /// computed keeps the catalog its stream opened with. Returns
    /// how many runs it armed a reload for.
    pub(super) fn refresh(&self, router: &CommandRouter) -> usize {
        let known = self.lock().keys().cloned().collect::<BTreeSet<_>>();
        let next = router
            .runtime_tool_catalog_auth_tokens()
            .into_iter()
            .map(|token| {
                let listing = router.runtime_catalog_listing(&token);
                (token, listing)
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
            for (token, listing) in next {
                if !running.contains(&token) {
                    continue;
                }
                match catalogs.get_mut(&token) {
                    Some(current) if known.contains(&token) => {
                        if current.listing == listing {
                            continue;
                        }
                        if !reload_view(&current.listing).eq(reload_view(&listing)) {
                            changed.push(token);
                        }
                        current.listing = listing;
                        current.listed.send_replace(());
                    }
                    Some(_) => {}
                    None => {
                        catalogs.insert(
                            token,
                            Catalog {
                                listing,
                                listed: watch::channel(()).0,
                            },
                        );
                    }
                }
            }
        }
        for token in &changed {
            router.runtime_tool_catalog_changed_for_auth_token(token);
        }
        changed.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<String, Catalog>> {
        self.catalogs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn reload_view(listing: &Listing) -> impl Iterator<Item = &(String, String, Value)> {
    listing
        .iter()
        .filter(|tool| crate::runtime::state::arms_catalog_reload(&tool.0))
}

fn changes() -> &'static broadcast::Sender<()> {
    static CHANGES: OnceLock<broadcast::Sender<()>> = OnceLock::new();
    CHANGES.get_or_init(|| broadcast::channel(64).0)
}

pub(super) fn changed() {
    let _ = changes().send(());
}

/// This token's listing, computed off the async runtime.
pub(super) async fn listing(router: &Arc<CommandRouter>, token: &str) -> Option<Listing> {
    let (router, token) = (Arc::clone(router), token.to_owned());
    tokio::task::spawn_blocking(move || router.runtime_catalog_listing(&token))
        .await
        .ok()
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
    catalogs: &CatalogMonitor,
    headers: &hyper::HeaderMap,
) -> Response<HttpBody> {
    let Some(token) = parse_bearer_token(headers) else {
        return text_response(StatusCode::UNAUTHORIZED, "unauthorized".into());
    };
    // The monitor computes catalogs; a stream only forwards its run's changes.
    let Some(mut listed) = catalogs.subscribe(&token) else {
        return text_response(StatusCode::UNAUTHORIZED, "unauthorized".into());
    };
    let (sender, receiver) = mpsc::channel::<Bytes>(2);
    sender
        .try_send(Bytes::from_static(b": connected\n\n"))
        .expect("empty stream");
    tokio::spawn(async move {
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
        heartbeat.tick().await;
        loop {
            let bytes = tokio::select! {
                _ = sender.closed() => break,
                changed = listed.changed() => match changed {
                    Ok(()) => Bytes::from(event(&notification())),
                    Err(_) => break, // the run ended: its stream ends too
                },
                _ = heartbeat.tick() => Bytes::from_static(b": keepalive\n\n"),
            };
            if sender.send(bytes).await.is_err() {
                break;
            }
        }
    });
    let body = StreamBody::new(
        stream::unfold(receiver, |mut receiver| async move {
            receiver
                .recv()
                .await
                .map(|bytes| (Ok::<_, Infallible>(Frame::data(bytes)), receiver))
        })
        .fuse(),
    )
    .boxed_unsync();
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header("Cache-Control", "no-cache")
        .body(body)
        .expect("static MCP stream headers")
}
