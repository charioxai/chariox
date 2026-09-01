mod action;
mod action_ledger;
mod elements;
mod event;
mod event_log;
mod model;
mod ownership;
mod registry;
mod state;
mod tabs;

pub use action::{
    ActionAdmission, ActionCancellationOutcome, EnvironmentAction,
    EnvironmentActionCancellationReason, EnvironmentActionFailureCode,
    EnvironmentActionHistoryPage, EnvironmentActionOutcome, EnvironmentActionRequest,
    EnvironmentActionState, EnvironmentActionTerminal, EnvironmentMode, InputTarget,
};
pub(crate) use elements::EnvironmentElementTarget;
pub use event::{EnvironmentEvent, EnvironmentEventKind, EnvironmentReplay};
pub use model::{
    agent_environment_actor_id, human_environment_actor_id, human_environment_actor_label,
    CanonicalViewport, EnvironmentActor, EnvironmentActorKind, EnvironmentActorPresence,
    EnvironmentComponent, EnvironmentComponentHealth, EnvironmentComponentHealthState,
    EnvironmentError, EnvironmentLifecycle, EnvironmentTab, RoomEnvironmentSnapshot,
};
pub(crate) use model::{EnvironmentTabObservation, EnvironmentTabRuntimeBinding};
pub use ownership::{InputOwnership, PendingInputTakeover, TakeoverOutcome};
pub(crate) use registry::RoomEnvironmentRegistry;
pub use state::RoomEnvironment;

#[cfg(test)]
mod tests;
