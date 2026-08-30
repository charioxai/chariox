mod action;
mod action_ledger;
mod event;
mod event_log;
mod model;
mod ownership;
mod registry;
mod state;
mod tabs;

pub use action::{
    ActionAdmission, ActionCancellationOutcome, EnvironmentAction, EnvironmentActionRequest,
    EnvironmentActionState, EnvironmentActionTerminal, EnvironmentMode, InputTarget,
};
pub use event::{EnvironmentEvent, EnvironmentEventKind, EnvironmentReplay};
pub use model::{
    agent_environment_actor_id, human_environment_actor_id, human_environment_actor_label,
    CanonicalViewport, EnvironmentActor, EnvironmentActorKind, EnvironmentActorPresence,
    EnvironmentComponent, EnvironmentComponentHealth, EnvironmentComponentHealthState,
    EnvironmentError, EnvironmentLifecycle, EnvironmentTab, RoomEnvironmentSnapshot,
};
pub use ownership::{InputOwnership, PendingInputTakeover, TakeoverOutcome};
pub(crate) use registry::RoomEnvironmentRegistry;
pub use state::RoomEnvironment;

#[cfg(test)]
mod tests;
