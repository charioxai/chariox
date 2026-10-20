//! Opt-in, payload-free kernel event-path timings (`CHARIOX_PERF_DIAGNOSTICS=1`).
use std::sync::OnceLock;

use crate::transport::kernel_protocol::{event_session_id, kernel_event_name, KernelEvent};

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("CHARIOX_PERF_DIAGNOSTICS")
            .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"))
    })
}

pub(crate) fn record_relay_request(request_kind: &str, received_at_ms: u64, response_bytes: usize) {
    if !enabled() {
        return;
    }
    crate::logging::info_with_fields(
        "daemon.request_timing",
        "relay response prepared",
        serde_json::json!({"request_kind": request_kind, "received_at_ms": received_at_ms,
            "prepared_at_ms": crate::session::unix_epoch_ms(), "response_bytes": response_bytes}),
    );
}

pub(crate) fn record_relay_event(
    subscription_id: &str,
    event_id: u64,
    event: &KernelEvent,
    prepared_at_ms: u64,
    append_us: u128,
) {
    if !enabled() {
        return;
    }
    crate::logging::info_with_fields(
        "daemon.event_timing",
        "relay event ready for transport",
        serde_json::json!({"subscription_id": subscription_id, "event_id": event_id,
            "event": kernel_event_name(event), "session_id": event_session_id(event),
            "prepared_at_ms": prepared_at_ms, "append_ms": append_us / 1_000, "append_us": append_us,
            "queued_at_ms": crate::session::unix_epoch_ms()}),
    );
}

pub(crate) fn record_durable_commit(records: usize, batch_wait_us: u128, commit_us: u128) {
    if !enabled() {
        return;
    }
    crate::logging::info_with_fields(
        "daemon.durable_timing",
        "durable batch committed",
        serde_json::json!({"records": records, "batch_wait_us": batch_wait_us, "commit_us": commit_us,
            "committed_at_ms": crate::session::unix_epoch_ms()}),
    );
}
