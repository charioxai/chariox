use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaagentTraceMode {
    Compact,
    Verbose,
}

impl MetaagentTraceMode {
    pub(crate) fn parse(value: Option<&str>) -> Option<Self> {
        match value.unwrap_or("compact") {
            "compact" => Some(Self::Compact),
            "verbose" => Some(Self::Verbose),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MetaagentTraceSubscription {
    pub(crate) subscription_id: String,
    pub(crate) session_id: String,
    pub(crate) metaagent_id: String,
    pub(crate) target_agent_id: String,
    pub(crate) recipient_attachment_id: String,
    pub(crate) mode: MetaagentTraceMode,
    pub(crate) created_at_ms: u64,
}

#[derive(Default)]
struct MetaagentTraceState {
    next_sequence: u64,
    subscriptions: BTreeMap<String, MetaagentTraceSubscription>,
    target_activity: BTreeMap<(String, String), MetaagentTraceTargetActivity>,
}

struct MetaagentTraceTargetActivity {
    sequence: u64,
    notify: Arc<Notify>,
}

impl Default for MetaagentTraceTargetActivity {
    fn default() -> Self {
        Self {
            sequence: 0,
            notify: Arc::new(Notify::new()),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct MetaagentTraceSubscriptionStore {
    state: Arc<Mutex<MetaagentTraceState>>,
}

impl MetaagentTraceSubscriptionStore {
    pub(crate) fn subscribe(
        &self,
        session_id: &str,
        metaagent_id: &str,
        target_agent_id: &str,
        mode: MetaagentTraceMode,
    ) -> MetaagentTraceSubscription {
        let mut state = self
            .state
            .lock()
            .expect("metaagent trace subscription store poisoned");
        if let Some(subscription) = state.subscriptions.values_mut().find(|subscription| {
            subscription.session_id == session_id
                && subscription.metaagent_id == metaagent_id
                && subscription.target_agent_id == target_agent_id
        }) {
            subscription.mode = mode;
            return subscription.clone();
        }
        state.next_sequence += 1;
        let subscription_id = format!("trace:{metaagent_id}:{}", state.next_sequence);
        let recipient_attachment_id = format!(
            "meta-trace:{metaagent_id}:{target_agent_id}:{}",
            state.next_sequence
        );
        let subscription = MetaagentTraceSubscription {
            subscription_id: subscription_id.clone(),
            session_id: session_id.to_string(),
            metaagent_id: metaagent_id.to_string(),
            target_agent_id: target_agent_id.to_string(),
            recipient_attachment_id,
            mode,
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
    ) -> Option<MetaagentTraceSubscription> {
        let mut state = self
            .state
            .lock()
            .expect("metaagent trace subscription store poisoned");
        if state
            .subscriptions
            .get(subscription_id)
            .is_some_and(|subscription| subscription.metaagent_id == metaagent_id)
        {
            return state.subscriptions.remove(subscription_id);
        }
        None
    }

    pub(crate) fn get_for_metaagent(
        &self,
        metaagent_id: &str,
        subscription_id: &str,
    ) -> Option<MetaagentTraceSubscription> {
        let state = self
            .state
            .lock()
            .expect("metaagent trace subscription store poisoned");
        state
            .subscriptions
            .get(subscription_id)
            .filter(|subscription| subscription.metaagent_id == metaagent_id)
            .cloned()
    }

    pub(crate) fn get_for_target(
        &self,
        metaagent_id: &str,
        session_id: &str,
        target_agent_id: &str,
    ) -> Option<MetaagentTraceSubscription> {
        let state = self
            .state
            .lock()
            .expect("metaagent trace subscription store poisoned");
        state
            .subscriptions
            .values()
            .find(|subscription| {
                subscription.metaagent_id == metaagent_id
                    && subscription.session_id == session_id
                    && subscription.target_agent_id == target_agent_id
            })
            .cloned()
    }

    pub(crate) fn recipient_attachment_ids_for_target(
        &self,
        session_id: &str,
        target_agent_id: &str,
    ) -> Vec<String> {
        let state = self
            .state
            .lock()
            .expect("metaagent trace subscription store poisoned");
        state
            .subscriptions
            .values()
            .filter(|subscription| {
                subscription.session_id == session_id
                    && subscription.target_agent_id == target_agent_id
            })
            .map(|subscription| subscription.recipient_attachment_id.clone())
            .collect()
    }

    pub(crate) fn watch_target_activity(
        &self,
        session_id: &str,
        target_agent_id: &str,
    ) -> (u64, Arc<Notify>) {
        let mut state = self
            .state
            .lock()
            .expect("metaagent trace subscription store poisoned");
        let activity = state
            .target_activity
            .entry((session_id.to_string(), target_agent_id.to_string()))
            .or_default();
        (activity.sequence, Arc::clone(&activity.notify))
    }

    pub(crate) fn target_activity_sequence(&self, session_id: &str, target_agent_id: &str) -> u64 {
        let state = self
            .state
            .lock()
            .expect("metaagent trace subscription store poisoned");
        state
            .target_activity
            .get(&(session_id.to_string(), target_agent_id.to_string()))
            .map(|activity| activity.sequence)
            .unwrap_or(0)
    }

    pub(crate) fn record_target_activity(&self, session_id: &str, target_agent_id: &str) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("metaagent trace subscription store poisoned");
        if !state.subscriptions.values().any(|subscription| {
            subscription.session_id == session_id && subscription.target_agent_id == target_agent_id
        }) {
            return false;
        }
        let activity = state
            .target_activity
            .entry((session_id.to_string(), target_agent_id.to_string()))
            .or_default();
        activity.sequence = activity.sequence.saturating_add(1);
        let notify = Arc::clone(&activity.notify);
        drop(state);
        notify.notify_waiters();
        true
    }
}
