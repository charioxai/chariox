use serde::{Deserialize, Serialize};

use super::action::{EnvironmentActionOutcome, EnvironmentActionState};
use super::model::{EnvironmentLifecycle, RoomEnvironmentSnapshot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentEvent {
    pub event_id: u64,
    pub environment_id: String,
    pub runtime_generation: u64,
    pub kind: EnvironmentEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvironmentEventKind {
    LifecycleChanged {
        lifecycle: EnvironmentLifecycle,
    },
    RuntimeInvalidated,
    HealthChanged,
    TabsChanged,
    ViewportChanged {
        revision: u64,
    },
    ActorsChanged,
    InputOwnershipChanged,
    ActionChanged {
        action_id: String,
        state: EnvironmentActionState,
        #[serde(default)]
        cancellation_requested: bool,
        #[serde(default)]
        submitted_at_ms: u64,
        #[serde(default)]
        started_at_ms: Option<u64>,
        #[serde(default)]
        finished_at_ms: Option<u64>,
        #[serde(default)]
        outcome: Option<EnvironmentActionOutcome>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvironmentReplay {
    Events {
        events: Vec<EnvironmentEvent>,
        next_cursor: u64,
    },
    SnapshotRequired {
        snapshot: Box<RoomEnvironmentSnapshot>,
    },
}
