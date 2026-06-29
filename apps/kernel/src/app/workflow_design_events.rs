use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowDesignEventStore {
    inner: Arc<Mutex<WorkflowDesignEventStoreState>>,
    changes: Arc<WorkflowDesignEventChangeSignal>,
}

#[derive(Debug, Default)]
struct WorkflowDesignEventStoreState {
    next_sequence: u64,
    events: VecDeque<crate::local::WorkflowDesignOpForwarded>,
}

#[derive(Debug, Default)]
struct WorkflowDesignEventChangeSignal {
    sequence: AtomicU64,
    notify: Notify,
}

impl WorkflowDesignEventStore {
    const RETAINED_EVENTS: usize = 1024;

    pub(crate) fn append(
        &self,
        session_id: String,
        origin_client_id: String,
        op_id: String,
        op: crate::local::WorkflowDesignOp,
    ) -> crate::local::WorkflowDesignOpForwarded {
        let mut state = self
            .inner
            .lock()
            .expect("workflow design event store poisoned");
        state.next_sequence = state.next_sequence.saturating_add(1);
        let event = crate::local::WorkflowDesignOpForwarded {
            session_id,
            kernel_sequence: state.next_sequence,
            origin_client_id,
            op_id,
            op,
        };
        state.events.push_back(event.clone());
        while state.events.len() > Self::RETAINED_EVENTS {
            state.events.pop_front();
        }
        self.changes.record_change();
        event
    }

    pub(crate) fn events_since(
        &self,
        session_id: &str,
        after_sequence: u64,
        origin_client_id_to_skip: &str,
    ) -> Vec<crate::local::WorkflowDesignOpForwarded> {
        let state = self
            .inner
            .lock()
            .expect("workflow design event store poisoned");
        state
            .events
            .iter()
            .filter(|event| {
                event.session_id == session_id
                    && event.kernel_sequence > after_sequence
                    && event.origin_client_id != origin_client_id_to_skip
            })
            .cloned()
            .collect()
    }

    pub(crate) fn change_sequence(&self) -> u64 {
        self.changes.sequence()
    }

    pub(crate) async fn wait_for_change_after(&self, sequence: u64) {
        self.changes.wait_for_change_after(sequence).await;
    }
}

impl WorkflowDesignEventChangeSignal {
    fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }

    fn record_change(&self) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    async fn wait_for_change_after(&self, sequence: u64) {
        if self.sequence() != sequence {
            return;
        }
        let notified = self.notify.notified();
        if self.sequence() != sequence {
            return;
        }
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::WorkflowDesignEventStore;
    use std::time::Duration;

    #[tokio::test]
    async fn workflow_design_event_store_wakes_change_waiters() {
        let store = WorkflowDesignEventStore::default();
        let sequence = store.change_sequence();
        let waiter_store = store.clone();
        let waiter = tokio::spawn(async move {
            waiter_store.wait_for_change_after(sequence).await;
        });

        store.append(
            "session-1".to_string(),
            "client-1".to_string(),
            "op-1".to_string(),
            crate::local::WorkflowDesignOp::WorkflowRemove {
                workflow_id: "workflow-1".to_string(),
            },
        );

        tokio::time::timeout(Duration::from_millis(100), waiter)
            .await
            .expect("workflow design waiter should wake")
            .expect("wait task should complete");
    }

    #[tokio::test]
    async fn workflow_design_event_store_wait_returns_after_prior_change() {
        let store = WorkflowDesignEventStore::default();
        let sequence = store.change_sequence();
        store.append(
            "session-1".to_string(),
            "client-1".to_string(),
            "op-1".to_string(),
            crate::local::WorkflowDesignOp::WorkflowRemove {
                workflow_id: "workflow-1".to_string(),
            },
        );

        tokio::time::timeout(
            Duration::from_millis(100),
            store.wait_for_change_after(sequence),
        )
        .await
        .expect("changed workflow design sequence should not wait");
    }
}
