mod action;
mod action_ledger;
pub(crate) use action_ledger::EnvironmentActionLedger;
mod durability;
mod elements;
mod event;
mod event_log;
mod model;
mod ownership;
mod registry;
mod state;
mod tabs;
pub(crate) use tabs::TabRegistry;

pub use action::{
    ActionAdmission, ActionCancellationOutcome, EnvironmentAction, EnvironmentActionArguments,
    EnvironmentActionCancellationReason, EnvironmentActionFailureCode,
    EnvironmentActionHistoryPage, EnvironmentActionOutcome, EnvironmentActionRequest,
    EnvironmentActionState, EnvironmentActionTerminal, EnvironmentMode, EnvironmentPointerButton,
    InputTarget,
};
pub(crate) use elements::EnvironmentElementTarget;
pub use event::{EnvironmentEvent, EnvironmentEventKind, EnvironmentReplay};
pub use model::{
    agent_environment_actor_id, human_environment_actor_id, human_environment_actor_label,
    AppPanelLayout, AppPanelPlacement, CanonicalViewport, EnvironmentActor, EnvironmentActorColor,
    EnvironmentActorKind, EnvironmentActorPresence, EnvironmentAppPanel, EnvironmentComponent,
    EnvironmentComponentHealth, EnvironmentComponentHealthState, EnvironmentError,
    EnvironmentLifecycle, EnvironmentPointer, EnvironmentPointerPosition, EnvironmentTab,
    EnvironmentTabApp, RoomEnvironmentSnapshot,
};
pub(crate) use model::{EnvironmentTabObservation, EnvironmentTabRuntimeBinding};
pub use ownership::{InputOwnership, PendingInputTakeover, TakeoverOutcome};
pub(crate) use registry::RoomEnvironmentRegistry;
pub use state::RoomEnvironment;

#[cfg(test)]
mod tests;
