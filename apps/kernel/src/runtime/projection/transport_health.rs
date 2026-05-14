use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportHealthSnapshot {
    pub active_connections: usize,
    pub active_subscriptions: usize,
    pub retained_event_limit: usize,
    pub command_result_cache_limit: usize,
    pub inbound_request_limit: usize,
    pub incoming_requests: u64,
    pub emitted_events: u64,
    pub replay_gaps: u64,
    pub inbound_overload_rejections: u64,
    pub duplicate_command_conflicts: u64,
    pub outgoing_queue_overflows: u64,
    pub slow_consumer_closes: u64,
}

impl Default for TransportHealthSnapshot {
    fn default() -> Self {
        Self {
            active_connections: 0,
            active_subscriptions: 0,
            retained_event_limit: 0,
            command_result_cache_limit: 0,
            inbound_request_limit: 0,
            incoming_requests: 0,
            emitted_events: 0,
            replay_gaps: 0,
            inbound_overload_rejections: 0,
            duplicate_command_conflicts: 0,
            outgoing_queue_overflows: 0,
            slow_consumer_closes: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TransportHealthStore {
    state: Arc<TransportHealthState>,
}

#[derive(Debug, Default)]
struct TransportHealthState {
    active_connections: AtomicUsize,
    active_subscriptions: AtomicUsize,
    incoming_requests: AtomicU64,
    emitted_events: AtomicU64,
    replay_gaps: AtomicU64,
    inbound_overload_rejections: AtomicU64,
    duplicate_command_conflicts: AtomicU64,
    outgoing_queue_overflows: AtomicU64,
    slow_consumer_closes: AtomicU64,
}

impl TransportHealthStore {
    pub(crate) fn record_connection_opened(&self) {
        self.state
            .active_connections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_connection_closed(&self) {
        decrement_saturating(&self.state.active_connections);
    }

    pub(crate) fn record_subscription_opened(&self) {
        self.state
            .active_subscriptions
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_subscription_closed(&self) {
        decrement_saturating(&self.state.active_subscriptions);
    }

    pub(crate) fn record_incoming_request(&self) {
        self.state.incoming_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_emitted_event(&self) {
        self.state.emitted_events.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_replay_gap(&self) {
        self.state.replay_gaps.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_inbound_overload_rejection(&self) {
        self.state
            .inbound_overload_rejections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_duplicate_command_conflict(&self) {
        self.state
            .duplicate_command_conflicts
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_outgoing_queue_overflow(&self) {
        self.state
            .outgoing_queue_overflows
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_slow_consumer_close(&self) {
        self.state
            .slow_consumer_closes
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(
        &self,
        retained_event_limit: usize,
        command_result_cache_limit: usize,
        inbound_request_limit: usize,
    ) -> TransportHealthSnapshot {
        TransportHealthSnapshot {
            active_connections: self.state.active_connections.load(Ordering::Relaxed),
            active_subscriptions: self.state.active_subscriptions.load(Ordering::Relaxed),
            retained_event_limit,
            command_result_cache_limit,
            inbound_request_limit,
            incoming_requests: self.state.incoming_requests.load(Ordering::Relaxed),
            emitted_events: self.state.emitted_events.load(Ordering::Relaxed),
            replay_gaps: self.state.replay_gaps.load(Ordering::Relaxed),
            inbound_overload_rejections: self
                .state
                .inbound_overload_rejections
                .load(Ordering::Relaxed),
            duplicate_command_conflicts: self
                .state
                .duplicate_command_conflicts
                .load(Ordering::Relaxed),
            outgoing_queue_overflows: self.state.outgoing_queue_overflows.load(Ordering::Relaxed),
            slow_consumer_closes: self.state.slow_consumer_closes.load(Ordering::Relaxed),
        }
    }
}

fn decrement_saturating(value: &AtomicUsize) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        current.checked_sub(1)
    });
}
