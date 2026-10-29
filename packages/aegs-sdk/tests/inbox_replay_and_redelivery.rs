//! Real-process drill for the received-event API and the redelivery sweep: an event whose
//! retries ran out is listed as failed with its reason and published by a replay of its owner,
//! a provider redelivery of a stored delivery is acknowledged without a second publication, and
//! a delivery the provider logged but this server never stored is fetched by the sweep.

mod inbox_support;

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chariox_aegs_sdk::{
    AegsProvider, AegsStore, NormalizedEvent, ProviderDelivery, WebhookInput, WebhookRoute,
};
use inbox_support::{
    free_port, post, seed_subscription, spawn_server, wait_until, AedsMode, FakeAeds, CHILD_ENV,
    GENERATOR, MANAGEMENT_TOKEN,
};

const DELIVERY_HEADER: &str = "x-drill-delivery";

struct RedeliveringProvider;

fn event(id: &str) -> NormalizedEvent {
    NormalizedEvent {
        occurrence_id: format!("drill-{id}"),
        event_type: "drill.event".to_string(),
        occurred_at: "2026-10-29T00:00:00Z".to_string(),
        connection_scope: "scope-1".to_string(),
        prompt: "drill prompt".to_string(),
        metadata: serde_json::json!({}),
        reply_context: None,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

impl AegsProvider for RedeliveringProvider {
    fn generator_id(&self) -> &'static str {
        GENERATOR
    }

    fn provider_slug(&self) -> &'static str {
        "drill"
    }

    fn normalize_webhook(
        &self,
        input: WebhookInput<'_>,
        _route: &WebhookRoute,
    ) -> Result<NormalizedEvent, String> {
        Ok(event(String::from_utf8_lossy(input.body).trim()))
    }

    fn webhook_delivery_id(&self, input: WebhookInput<'_>) -> Option<String> {
        input.headers.get(DELIVERY_HEADER).cloned()
    }

    fn recent_deliveries(&self, _since_ms: u64) -> Result<Vec<ProviderDelivery>, String> {
        Ok(vec![ProviderDelivery {
            delivery_id: "missed".to_string(),
            redelivery_ref: "missed".to_string(),
            delivered_at_ms: now_ms() - 10 * 60_000,
        }])
    }

    /// Plays the provider's redelivery: the same signed request arrives at the webhook route.
    fn redeliver(&self, delivery: &ProviderDelivery) -> Result<(), String> {
        let address = std::env::var("CHARIOX_AEGS_ADDR").unwrap();
        let started = Instant::now();
        let mut stream = loop {
            match TcpStream::connect(&address) {
                Ok(stream) => break stream,
                Err(error) if started.elapsed() > Duration::from_secs(10) => {
                    return Err(error.to_string())
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        let body = &delivery.redelivery_ref;
        write!(
            stream,
            "POST /webhooks/drill/conn-1 HTTP/1.1\r\nhost: localhost\r\n{DELIVERY_HEADER}: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            delivery.delivery_id,
            body.len()
        )
        .map_err(|error| error.to_string())?;
        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response)
            .map_err(|error| error.to_string())?;
        response
            .starts_with("HTTP/1.1 202")
            .then_some(())
            .ok_or_else(|| "redelivery was not accepted".to_string())
    }
}

#[test]
fn drill_redelivery_server_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(chariox_aegs_sdk::run_from_environment(|_store| {
            Ok(Arc::new(RedeliveringProvider))
        }))
        .unwrap();
}

/// Stores a webhook and spends its whole attempt budget against a refusing AEDS.
fn seed_failed_event(database: &std::path::Path, delivery_id: &str) {
    let store = AegsStore::open(database).unwrap();
    let received = now_ms() - 60 * 60_000;
    assert!(store
        .record_inbox_event(delivery_id, Some("conn-1"), &event(delivery_id), received)
        .unwrap());
    let claim = store
        .all(GENERATOR)
        .unwrap()
        .into_iter()
        .find(|claim| claim.connection_id == "conn-1")
        .unwrap();
    assert!(store
        .select_inbox_interests(delivery_id, &[(claim, Some(1))], received)
        .unwrap());
    for attempt in 1..=8u64 {
        let at = received + attempt * 3_600_000;
        let claimed: Vec<_> = store
            .due_inbox_bindings(delivery_id, None, at)
            .unwrap()
            .iter()
            .filter_map(|binding| {
                store
                    .claim_inbox_delivery(delivery_id, binding, at)
                    .unwrap()
            })
            .collect();
        assert_eq!(claimed.len(), 1, "attempt {attempt} must be claimable");
        store
            .fail_inbox_delivery(
                &claimed[0],
                "the event service returned HTTP 503",
                received + attempt * 3_600_000,
            )
            .unwrap();
    }
}

fn management(port: u16, path: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
    let reply = post(
        port,
        path,
        &[
            ("authorization", &format!("Bearer {MANAGEMENT_TOKEN}")),
            ("content-type", "application/json"),
        ],
        &body.to_string(),
        Duration::from_secs(20),
    )
    .expect("management request got no answer");
    let value = serde_json::from_str(&reply.body).unwrap_or(serde_json::Value::Null);
    (reply.status, value)
}

fn query(port: u16, owner: &str) -> Vec<serde_json::Value> {
    let (status, body) = management(
        port,
        "/v1/events/query",
        serde_json::json!({"generator_id": GENERATOR, "owner_id": owner, "limit": 50}),
    );
    assert_eq!(status, 200, "{body}");
    body["events"].as_array().unwrap().clone()
}

#[test]
fn failed_event_is_listed_and_replayed_and_missed_delivery_is_swept() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    let directory =
        std::env::temp_dir().join(format!("chariox-aegs-replay-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory.join("aegs.db");
    seed_subscription(&database);
    seed_failed_event(&database, "failed-1");
    let aeds = FakeAeds::start();
    aeds.set_mode(AedsMode::Accept);
    let port = free_port();
    let mut server = spawn_server("drill_redelivery_server_child", &database, port, aeds.port);

    let events = query(port, "owner-1");
    let failed = events
        .iter()
        .find(|event| event["delivery_id"] == "failed-1")
        .expect("the failed event is listed for its owner");
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["attempts"], 8);
    assert_eq!(failed["last_error"], "the event service returned HTTP 503");
    assert!(
        failed.get("prompt").is_none() && failed.get("reply_context").is_none(),
        "the listing never carries the prompt or reply context"
    );
    assert!(
        query(port, "owner-2").is_empty(),
        "another owner sees none of it"
    );
    let (status, _) = management(
        port,
        "/v1/events/replay",
        serde_json::json!({"generator_id": GENERATOR, "owner_id": "owner-2", "delivery_id": "failed-1"}),
    );
    assert_eq!(status, 404, "another owner cannot replay it");
    assert!(aeds
        .accepted
        .lock()
        .unwrap()
        .iter()
        .all(|id| id != "drill-failed-1"));

    let (status, replayed) = management(
        port,
        "/v1/events/replay",
        serde_json::json!({"generator_id": GENERATOR, "owner_id": "owner-1", "delivery_id": "failed-1"}),
    );
    assert_eq!(status, 200, "{replayed}");
    assert_eq!(replayed["event"]["status"], "published");
    assert!(aeds
        .accepted
        .lock()
        .unwrap()
        .iter()
        .any(|id| id == "drill-failed-1"));
    let (status, _) = management(
        port,
        "/v1/events/replay",
        serde_json::json!({"generator_id": GENERATOR, "owner_id": "owner-1", "delivery_id": "unknown"}),
    );
    assert_eq!(status, 404);

    // A provider redelivery of a stored delivery is acknowledged without a second publication.
    let headers = [(DELIVERY_HEADER, "again-1")];
    let first = post(
        port,
        "/webhooks/drill/conn-1",
        &headers,
        "again",
        Duration::from_secs(20),
    )
    .unwrap();
    let seen = aeds.requests_seen.load(Ordering::SeqCst);
    let second = post(
        port,
        "/webhooks/drill/conn-1",
        &headers,
        "again",
        Duration::from_secs(20),
    )
    .unwrap();
    assert_eq!((first.status, second.status), (202, 202));
    assert_eq!(aeds.requests_seen.load(Ordering::SeqCst), seen);

    // The sweep found the delivery the provider logged and this server never stored.
    let swept = wait_until(Duration::from_secs(20), || {
        aeds.accepted
            .lock()
            .unwrap()
            .iter()
            .any(|id| id == "drill-missed")
    });
    let published: HashMap<String, usize> =
        aeds.accepted
            .lock()
            .unwrap()
            .iter()
            .fold(HashMap::new(), |mut counts, id| {
                *counts.entry(id.clone()).or_default() += 1;
                counts
            });
    server.kill().unwrap();
    server.wait().unwrap();
    let _ = std::fs::remove_dir_all(&directory);
    assert!(swept, "the sweep never asked for the missed delivery");
    assert!(published.values().all(|count| *count == 1));
}
