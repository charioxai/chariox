//! MP-08 / MP-10 / MP-11: opt-in outer routing timestamps, payloads stay opaque.
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_tungstenite::tungstenite::Message;

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("CHARIOX_PERF_DIAGNOSTICS").is_ok_and(|v| v == "1"))
}

pub(super) fn record(stage: &str, subscription_id: &str, event_id: u64) {
    if !enabled() {
        return;
    }
    eprintln!(
        "{}",
        serde_json::json!({"component":"relay.event_timing", "stage":stage,
        "subscription_id":subscription_id, "event_id":event_id,
        "at_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()})
    );
}

pub(super) fn record_message(stage: &str, message: &Message) {
    if !enabled() {
        return;
    }
    #[derive(serde::Deserialize)]
    struct RoutingMetadata {
        kind: String,
        subscription_id: Option<String>,
        event_id: Option<u64>,
    }
    if let Message::Text(text) = message {
        if let Ok(meta) = serde_json::from_str::<RoutingMetadata>(text) {
            if meta.kind == "client_event" {
                if let (Some(subscription), Some(event)) = (meta.subscription_id, meta.event_id) {
                    record(stage, &subscription, event);
                }
            }
        }
    }
}
