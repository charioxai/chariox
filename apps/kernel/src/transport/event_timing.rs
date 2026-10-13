//! MP-08 / MP-10 / MP-11: opt-in, payload-free relay delivery timestamps.
use std::sync::OnceLock;

use super::kernel_protocol::{event_session_id, kernel_event_name, KernelEvent};

pub(crate) fn record_relay_event(subscription_id: &str, event_id: u64, event: &KernelEvent) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| {
        std::env::var("CHARIOX_PERF_DIAGNOSTICS")
            .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"))
    }) {
        return;
    }
    crate::logging::info_with_fields(
        "daemon.event_timing",
        "relay event ready for transport",
        serde_json::json!({"subscription_id":subscription_id, "event_id":event_id,
            "event":kernel_event_name(event), "session_id":event_session_id(event),
            "queued_at_ms":crate::session::unix_epoch_ms()}),
    );
}
