use std::collections::VecDeque;

use super::event::{EnvironmentEvent, EnvironmentEventKind};
use super::model::EnvironmentError;

pub(crate) enum EnvironmentReplayPlan {
    Events {
        events: Vec<EnvironmentEvent>,
        next_cursor: u64,
    },
    SnapshotRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvironmentEventLog {
    events: VecDeque<EnvironmentEvent>,
    capacity: usize,
    next_event_id: u64,
}

impl EnvironmentEventLog {
    pub(crate) fn new(capacity: usize) -> Result<Self, EnvironmentError> {
        if capacity == 0 {
            return Err(EnvironmentError::InvalidEventCapacity);
        }
        Ok(Self {
            events: VecDeque::new(),
            capacity,
            next_event_id: 1,
        })
    }

    pub(crate) fn cursor(&self) -> u64 {
        self.next_event_id - 1
    }

    pub(crate) fn push(
        &mut self,
        environment_id: &str,
        runtime_generation: u64,
        kind: EnvironmentEventKind,
    ) {
        self.events.push_back(EnvironmentEvent {
            event_id: self.next_event_id,
            environment_id: environment_id.to_string(),
            runtime_generation,
            kind,
        });
        self.next_event_id += 1;
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
    }

    pub(crate) fn replay(&self, cursor: u64) -> EnvironmentReplayPlan {
        let current_cursor = self.cursor();
        let oldest_event_id = self
            .events
            .front()
            .map(|event| event.event_id)
            .unwrap_or(self.next_event_id);
        if cursor > current_cursor || cursor.saturating_add(1) < oldest_event_id {
            return EnvironmentReplayPlan::SnapshotRequired;
        }
        EnvironmentReplayPlan::Events {
            events: self
                .events
                .iter()
                .filter(|event| event.event_id > cursor)
                .cloned()
                .collect(),
            next_cursor: current_cursor,
        }
    }
}
