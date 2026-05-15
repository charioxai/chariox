use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowDesignEventStore {
    inner: Arc<Mutex<WorkflowDesignEventStoreState>>,
}

#[derive(Debug, Default)]
struct WorkflowDesignEventStoreState {
    next_sequence: u64,
    events: VecDeque<crate::local::WorkflowDesignOpForwarded>,
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
}
