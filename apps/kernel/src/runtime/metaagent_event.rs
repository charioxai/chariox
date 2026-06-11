use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

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

#[derive(Clone, Default)]
pub(crate) struct MetaagentEventStore {
    state: Arc<Mutex<MetaagentEventState>>,
}

impl MetaagentEventStore {
    pub(crate) fn record(&self, event: NewMetaagentEvent) -> MetaagentEventRecord {
        let mut state = self.state.lock().expect("metaagent event store poisoned");
        state.next_sequence += 1;
        let sequence = state.next_sequence;
        let event_id = format!("metaevt-{sequence}");
        let preview = event.summary.chars().take(160).collect::<String>();
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
            detail: event.detail,
            created_at_ms: crate::session::unix_epoch_ms(),
            read_at_ms: None,
            ack_at_ms: None,
            injected_prompt_id: event.injected_prompt_id,
        };
        state.records.insert(event_id, record.clone());
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
        }
        serde_json::json!({
            "total": total,
            "unacked": unacked,
            "unread": unread,
            "by_kind": by_kind,
        })
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
}
