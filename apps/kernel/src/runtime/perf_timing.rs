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

/// Phase clocks are dormant unless performance diagnostics are enabled.
/// Callers supply only fixed labels and public operation/event identifiers.
pub(crate) struct Phases {
    previous: Option<std::time::Instant>,
    phases: Vec<(&'static str, u128)>,
}

impl Phases {
    pub(crate) fn new() -> Self {
        Self {
            previous: enabled().then(std::time::Instant::now),
            phases: Vec::new(),
        }
    }

    pub(crate) fn mark(&mut self, phase: &'static str) {
        if let Some(previous) = self.previous.as_mut() {
            let now = std::time::Instant::now();
            self.phases
                .push((phase, now.duration_since(*previous).as_micros()));
            *previous = now;
        }
    }

    pub(crate) fn finish(self, scope: &'static str, operation_id: impl std::fmt::Display) {
        if self.previous.is_none() {
            return;
        }
        let phases: std::collections::BTreeMap<_, _> = self.phases.into_iter().collect();
        crate::logging::info_with_fields(
            "daemon.phase_timing",
            "kernel path phases",
            serde_json::json!({"scope": scope, "operation_id": operation_id.to_string(), "phases_us": phases}),
        );
    }
}
