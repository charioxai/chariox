use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

const DEFAULT_MAX_METAAGENT_EVENT_RECORDS_PER_METAAGENT: usize = 1_000;
const DEFAULT_MAX_METAAGENT_EVENT_DETAIL_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MetaagentEventRecord {
    pub sequence: u64,
    pub event_id: String,
    pub session_id: String,
    pub metaagent_id: String,
    pub owner_user_id: String,
    pub kind: String,
    pub source_agent_id: Option<String>,
    pub title: String,
    pub summary: String,
    pub preview: String,
    pub detail: serde_json::Value,
    pub created_at_ms: u64,
    pub read_at_ms: Option<u64>,
    pub ack_at_ms: Option<u64>,
    pub injected_prompt_id: Option<String>,
    #[serde(default = "default_metaagent_event_delivery_status")]
    pub prompt_delivery_status: String,
    #[serde(default)]
    pub prompt_delivery_updated_at_ms: Option<u64>,
    #[serde(default)]
    pub prompt_delivery_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MetaagentEventSubscription {
    pub subscription_id: String,
    pub metaagent_id: String,
    pub kind: String,
    pub filter: Option<serde_json::Value>,
    pub required: bool,
    pub scope: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct NewMetaagentEvent {
    pub session_id: String,
    pub metaagent_id: String,
    pub owner_user_id: String,
    pub kind: String,
    pub source_agent_id: Option<String>,
    pub title: String,
    pub summary: String,
    pub detail: serde_json::Value,
    pub injected_prompt_id: Option<String>,
}

#[derive(Default)]
struct MetaagentEventState {
    next_sequence: u64,
    next_subscription_sequence: u64,
    records: BTreeMap<String, MetaagentEventRecord>,
    subscriptions: BTreeMap<String, MetaagentEventSubscription>,
}

#[derive(Debug, Clone)]
struct MetaagentEventStoreLimits {
    max_records_per_metaagent: usize,
    max_detail_bytes: usize,
}

impl Default for MetaagentEventStoreLimits {
    fn default() -> Self {
        Self {
            max_records_per_metaagent: DEFAULT_MAX_METAAGENT_EVENT_RECORDS_PER_METAAGENT,
            max_detail_bytes: DEFAULT_MAX_METAAGENT_EVENT_DETAIL_BYTES,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MetaagentEventSnapshot {
    pub records: Vec<MetaagentEventRecord>,
    pub subscriptions: Vec<MetaagentEventSubscription>,
}

#[derive(Clone, Default)]
pub(crate) struct MetaagentEventStore {
    state: Arc<Mutex<MetaagentEventState>>,
    limits: MetaagentEventStoreLimits,
}

fn default_metaagent_event_delivery_status() -> String {
    "recorded".to_string()
}

impl MetaagentEventStore {
    #[cfg(test)]
    pub(crate) fn for_tests(max_records_per_metaagent: usize, max_detail_bytes: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(MetaagentEventState::default())),
            limits: MetaagentEventStoreLimits {
                max_records_per_metaagent,
                max_detail_bytes,
            },
        }
    }

    pub(crate) fn record(&self, event: NewMetaagentEvent) -> MetaagentEventRecord {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        state.next_sequence += 1;
        let sequence = state.next_sequence;
        let event_id = format!("metaevt-{sequence}");
        let preview = event.summary.chars().take(160).collect::<String>();
        let created_at_ms = crate::session::unix_epoch_ms();
        let record = MetaagentEventRecord {
            sequence,
            event_id: event_id.clone(),
            session_id: event.session_id,
            metaagent_id: event.metaagent_id,
            owner_user_id: event.owner_user_id,
            kind: event.kind,
            source_agent_id: event.source_agent_id,
            title: event.title,
            summary: event.summary,
            preview,
            detail: compact_metaagent_event_detail(event.detail, self.limits.max_detail_bytes),
            created_at_ms,
            read_at_ms: None,
            ack_at_ms: None,
            injected_prompt_id: event.injected_prompt_id,
            prompt_delivery_status: default_metaagent_event_delivery_status(),
            prompt_delivery_updated_at_ms: Some(created_at_ms),
            prompt_delivery_error: None,
        };
        state.records.insert(event_id, record.clone());
        self.prune_metaagent_records_locked(&mut state, &record.metaagent_id);
        record
    }

    pub(crate) fn list(
        &self,
        metaagent_id: &str,
        kind: Option<&str>,
        status: Option<&str>,
        limit: usize,
    ) -> Vec<MetaagentEventRecord> {
        let state = self.state.lock().expect("metaagent event store poisoned");
        let mut records = state
            .records
            .values()
            .filter(|record| record.metaagent_id == metaagent_id)
            .filter(|record| kind.is_none_or(|kind| record.kind == kind))
            .filter(|record| match status {
                Some("acked") => record.ack_at_ms.is_some(),
                Some("unacked") => record.ack_at_ms.is_none(),
                Some("read") => record.read_at_ms.is_some(),
                Some("unread") => record.read_at_ms.is_none(),
                Some(status) => record.prompt_delivery_status == status,
                _ => true,
            })
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by(|left, right| right.sequence.cmp(&left.sequence));
        records.truncate(limit);
        records
    }

    pub(crate) fn read(&self, metaagent_id: &str, event_id: &str) -> Option<MetaagentEventRecord> {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        let record = state.records.get_mut(event_id)?;
        if record.metaagent_id != metaagent_id {
            return None;
        }
        if record.read_at_ms.is_none() {
            record.read_at_ms = Some(crate::session::unix_epoch_ms());
        }
        Some(record.clone())
    }

    pub(crate) fn ack(
        &self,
        metaagent_id: &str,
        event_ids: &[String],
        up_to_sequence: Option<u64>,
    ) -> Vec<MetaagentEventRecord> {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        let now = crate::session::unix_epoch_ms();
        let mut acked = Vec::new();
        for record in state.records.values_mut() {
            if record.metaagent_id != metaagent_id {
                continue;
            }
            let matches_id = event_ids
                .iter()
                .any(|event_id| event_id == &record.event_id);
            let matches_sequence =
                up_to_sequence.is_some_and(|sequence| record.sequence <= sequence);
            if matches_id || matches_sequence {
                record.ack_at_ms = Some(now);
                acked.push(record.clone());
            }
        }
        acked.sort_by(|left, right| left.sequence.cmp(&right.sequence));
        acked
    }

    pub(crate) fn counts(&self, metaagent_id: &str) -> serde_json::Value {
        let state = self.state.lock().expect("metaagent event store poisoned");
        let mut total = 0_u64;
        let mut unacked = 0_u64;
        let mut unread = 0_u64;
        let mut by_kind = BTreeMap::<String, u64>::new();
        let mut by_prompt_delivery_status = BTreeMap::<String, u64>::new();
        for record in state.records.values() {
            if record.metaagent_id != metaagent_id {
                continue;
            }
            total += 1;
            if record.ack_at_ms.is_none() {
                unacked += 1;
            }
            if record.read_at_ms.is_none() {
                unread += 1;
            }
            *by_kind.entry(record.kind.clone()).or_default() += 1;
            *by_prompt_delivery_status
                .entry(record.prompt_delivery_status.clone())
                .or_default() += 1;
        }
        serde_json::json!({
            "total": total,
            "unacked": unacked,
            "unread": unread,
            "by_kind": by_kind,
            "by_prompt_delivery_status": by_prompt_delivery_status,
        })
    }

    pub(crate) fn update_prompt_delivery_status(
        &self,
        event_id: &str,
        status: &str,
        error: Option<String>,
    ) -> Option<MetaagentEventRecord> {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        let record = state.records.get_mut(event_id)?;
        record.prompt_delivery_status = status.to_string();
        record.prompt_delivery_updated_at_ms = Some(crate::session::unix_epoch_ms());
        record.prompt_delivery_error = error;
        Some(record.clone())
    }

    pub(crate) fn update_prompt_delivery_status_for_prompt(
        &self,
        prompt_id: &str,
        status: &str,
        error: Option<String>,
    ) -> Option<MetaagentEventRecord> {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        let record = state
            .records
            .values_mut()
            .find(|record| record.injected_prompt_id.as_deref() == Some(prompt_id))?;
        record.prompt_delivery_status = status.to_string();
        record.prompt_delivery_updated_at_ms = Some(crate::session::unix_epoch_ms());
        record.prompt_delivery_error = error;
        Some(record.clone())
    }

    pub(crate) fn subscribe(
        &self,
        metaagent_id: &str,
        kind: String,
        filter: Option<serde_json::Value>,
    ) -> MetaagentEventSubscription {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        if let Some(subscription) = state
            .subscriptions
            .values()
            .find(|subscription| {
                subscription.metaagent_id == metaagent_id
                    && subscription.kind == kind
                    && subscription.filter == filter
            })
            .cloned()
        {
            return subscription;
        }
        state.next_subscription_sequence += 1;
        let subscription_id = format!(
            "optional:{metaagent_id}:{}",
            state.next_subscription_sequence
        );
        let subscription = MetaagentEventSubscription {
            subscription_id: subscription_id.clone(),
            metaagent_id: metaagent_id.to_string(),
            kind,
            filter,
            required: false,
            scope: "session".to_string(),
            created_at_ms: crate::session::unix_epoch_ms(),
        };
        state
            .subscriptions
            .insert(subscription_id, subscription.clone());
        subscription
    }

    pub(crate) fn unsubscribe(
        &self,
        metaagent_id: &str,
        subscription_id: &str,
    ) -> Option<MetaagentEventSubscription> {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        if state
            .subscriptions
            .get(subscription_id)
            .is_some_and(|subscription| subscription.metaagent_id == metaagent_id)
        {
            return state.subscriptions.remove(subscription_id);
        }
        None
    }

    pub(crate) fn list_subscriptions(&self, metaagent_id: &str) -> Vec<MetaagentEventSubscription> {
        let state = self.state.lock().expect("metaagent event store poisoned");
        let mut subscriptions = state
            .subscriptions
            .values()
            .filter(|subscription| subscription.metaagent_id == metaagent_id)
            .cloned()
            .collect::<Vec<_>>();
        subscriptions.sort_by(|left, right| left.subscription_id.cmp(&right.subscription_id));
        subscriptions
    }

    pub(crate) fn has_optional_subscription(&self, metaagent_id: &str, kind: &str) -> bool {
        let state = self.state.lock().expect("metaagent event store poisoned");
        state.subscriptions.values().any(|subscription| {
            subscription.metaagent_id == metaagent_id && subscription.kind == kind
        })
    }

    pub(crate) fn snapshot(&self) -> MetaagentEventSnapshot {
        let state = self.state.lock().expect("metaagent event store poisoned");
        MetaagentEventSnapshot {
            records: state.records.values().cloned().collect(),
            subscriptions: state.subscriptions.values().cloned().collect(),
        }
    }

    pub(crate) fn restore_snapshot(&self, snapshot: MetaagentEventSnapshot) {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        state.records = snapshot
            .records
            .into_iter()
            .map(|mut record| {
                record.detail =
                    compact_metaagent_event_detail(record.detail, self.limits.max_detail_bytes);
                (record.event_id.clone(), record)
            })
            .collect();
        state.subscriptions = snapshot
            .subscriptions
            .into_iter()
            .map(|subscription| (subscription.subscription_id.clone(), subscription))
            .collect();
        self.prune_all_metaagent_records_locked(&mut state);
        Self::refresh_sequences(&mut state);
    }

    pub(crate) fn restore_record(&self, mut record: MetaagentEventRecord) {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        record.detail = compact_metaagent_event_detail(record.detail, self.limits.max_detail_bytes);
        let metaagent_id = record.metaagent_id.clone();
        state.records.insert(record.event_id.clone(), record);
        self.prune_metaagent_records_locked(&mut state, &metaagent_id);
        Self::refresh_sequences(&mut state);
    }

    pub(crate) fn restore_subscription(&self, subscription: MetaagentEventSubscription) {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        state
            .subscriptions
            .insert(subscription.subscription_id.clone(), subscription);
        Self::refresh_sequences(&mut state);
    }

    pub(crate) fn remove_restored_subscription(&self, metaagent_id: &str, subscription_id: &str) {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        if state
            .subscriptions
            .get(subscription_id)
            .is_some_and(|subscription| subscription.metaagent_id == metaagent_id)
        {
            state.subscriptions.remove(subscription_id);
        }
        Self::refresh_sequences(&mut state);
    }

    fn refresh_sequences(state: &mut MetaagentEventState) {
        state.next_sequence = state
            .records
            .values()
            .map(|record| record.sequence)
            .max()
            .unwrap_or_default();
        state.next_subscription_sequence = state
            .subscriptions
            .values()
            .filter_map(|subscription| {
                subscription
                    .subscription_id
                    .rsplit(':')
                    .next()
                    .and_then(|suffix| suffix.parse::<u64>().ok())
            })
            .max()
            .unwrap_or_default();
    }

    fn prune_all_metaagent_records_locked(&self, state: &mut MetaagentEventState) {
        let metaagent_ids = state
            .records
            .values()
            .map(|record| record.metaagent_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        for metaagent_id in metaagent_ids {
            self.prune_metaagent_records_locked(state, &metaagent_id);
        }
    }

    fn prune_metaagent_records_locked(&self, state: &mut MetaagentEventState, metaagent_id: &str) {
        let max_records = self.limits.max_records_per_metaagent.max(1);
        let mut record_ids = state
            .records
            .values()
            .filter(|record| record.metaagent_id == metaagent_id)
            .map(|record| (record.sequence, record.event_id.clone()))
            .collect::<Vec<_>>();
        if record_ids.len() <= max_records {
            return;
        }
        record_ids.sort_by_key(|(sequence, _)| *sequence);
        let prune_count = record_ids.len().saturating_sub(max_records);
        for (_, event_id) in record_ids.into_iter().take(prune_count) {
            state.records.remove(&event_id);
        }
    }
}

fn compact_metaagent_event_detail(
    detail: serde_json::Value,
    max_detail_bytes: usize,
) -> serde_json::Value {
    let max_detail_bytes = max_detail_bytes.max(1);
    let Ok(encoded) = serde_json::to_vec(&detail) else {
        return detail;
    };
    if encoded.len() <= max_detail_bytes {
        return detail;
    }
    serde_json::json!({
        "compacted": true,
        "reason": "metaagent_event_detail_retention_limit",
        "original_size_bytes": encoded.len(),
        "max_detail_bytes": max_detail_bytes,
        "message": "event detail exceeded the metaagent event retention limit; use related turn_overview or turn_blob records when available",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_compacts_oversized_detail_payloads() {
        let store = MetaagentEventStore::for_tests(10, 64);
        let record = store.record(new_event(
            "meta-1",
            serde_json::json!({
                "turn_blob": "x".repeat(512),
                "trace": ["large", "payload"],
            }),
        ));

        assert_eq!(
            record
                .detail
                .get("compacted")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            record
                .detail
                .get("reason")
                .and_then(serde_json::Value::as_str),
            Some("metaagent_event_detail_retention_limit")
        );
        assert!(record
            .detail
            .get("original_size_bytes")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|bytes| bytes > 64));
    }

    #[test]
    fn record_prunes_oldest_records_per_metaagent() {
        let store = MetaagentEventStore::for_tests(2, 1024);
        let first = store.record(new_event("meta-1", serde_json::json!({ "n": 1 })));
        let second = store.record(new_event("meta-1", serde_json::json!({ "n": 2 })));
        let third = store.record(new_event("meta-1", serde_json::json!({ "n": 3 })));
        let peer = store.record(new_event("meta-2", serde_json::json!({ "n": 1 })));

        let retained = store.list("meta-1", None, None, 10);
        assert_eq!(
            retained
                .iter()
                .map(|record| record.event_id.as_str())
                .collect::<Vec<_>>(),
            vec![third.event_id.as_str(), second.event_id.as_str()]
        );
        assert!(store.read("meta-1", &first.event_id).is_none());
        assert_eq!(
            store
                .read("meta-2", &peer.event_id)
                .map(|record| record.event_id),
            Some(peer.event_id)
        );
    }

    #[test]
    fn restore_applies_retention_limits_and_refreshes_sequence() {
        let source = MetaagentEventStore::default();
        let first = source.record(new_event("meta-1", serde_json::json!({ "n": 1 })));
        let second = source.record(new_event("meta-1", serde_json::json!({ "n": 2 })));
        let third = source.record(new_event("meta-1", serde_json::json!({ "n": 3 })));
        let snapshot = source.snapshot();
        let restored = MetaagentEventStore::for_tests(2, 64);

        restored.restore_snapshot(snapshot);
        let after_restore = restored.list("meta-1", None, None, 10);
        assert_eq!(
            after_restore
                .iter()
                .map(|record| record.event_id.as_str())
                .collect::<Vec<_>>(),
            vec![third.event_id.as_str(), second.event_id.as_str()]
        );
        assert!(restored.read("meta-1", &first.event_id).is_none());

        let next = restored.record(new_event("meta-1", serde_json::json!({ "n": 4 })));
        assert_eq!(next.sequence, third.sequence + 1);
    }

    fn new_event(metaagent_id: &str, detail: serde_json::Value) -> NewMetaagentEvent {
        NewMetaagentEvent {
            session_id: "session-1".to_string(),
            metaagent_id: metaagent_id.to_string(),
            owner_user_id: "user-1".to_string(),
            kind: "agent.turn.completed".to_string(),
            source_agent_id: Some("agent-1".to_string()),
            title: "Turn completed".to_string(),
            summary: "agent completed a turn".to_string(),
            detail,
            injected_prompt_id: None,
        }
    }
}
