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
    oldest_unrecoverable_cursor: u64,
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
            oldest_unrecoverable_cursor: 0,
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
        if matches!(kind, EnvironmentEventKind::PointersChanged)
            && self.events.back().is_some_and(|event| {
                event.environment_id == environment_id
                    && event.runtime_generation == runtime_generation
                    && matches!(event.kind, EnvironmentEventKind::PointersChanged)
            })
        {
            self.events.pop_back();
        }
        self.events.push_back(EnvironmentEvent {
            event_id: self.next_event_id,
            environment_id: environment_id.to_string(),
            runtime_generation,
            kind,
        });
        self.next_event_id += 1;
        while self.events.len() > self.capacity {
            if let Some(event) = self.events.pop_front() {
                self.oldest_unrecoverable_cursor = event.event_id;
            }
        }
    }

    pub(crate) fn replay(&self, cursor: u64) -> EnvironmentReplayPlan {
        let current_cursor = self.cursor();
        if cursor > current_cursor || cursor < self.oldest_unrecoverable_cursor {
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
