use super::action::EnvironmentActionState;
use super::model::{EnvironmentLifecycle, RoomEnvironmentSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentEvent {
    pub event_id: u64,
    pub environment_id: String,
    pub runtime_generation: u64,
    pub kind: EnvironmentEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentReplay {
    Events {
        events: Vec<EnvironmentEvent>,
        next_cursor: u64,
    },
    SnapshotRequired {
        snapshot: RoomEnvironmentSnapshot,
    },
}
