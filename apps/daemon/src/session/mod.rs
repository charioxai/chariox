mod owner;
mod prompt_runtime;
mod service;
mod store;
mod types;

pub(crate) use owner::{SessionStateOwner, SessionStateReader, SessionStateStore};
pub use service::{
    classify_workflow_failure_kind, WorkflowCompletionUpdate, WorkflowDispatch,
    WorkflowLaunchAdmission, WorkflowOutputValidationWarning, WorkflowWatchdogTickPlan,
};
pub use service::{PromptIdAllocator, SessionService};
pub use store::SessionStore;
pub use types::WorkflowOutputValidationPolicy;
pub use types::{
    unix_epoch_ms, CreateSessionRequest, PromptAttachment, PromptCancellation, PromptCompletion,
    PromptDetachEffect, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    QueuedWorkflowLaunch, QueuedWorkflowLaunchSource, RuntimeSession, RuntimeWorktreeAssignment,
    SchedulerState, SessionConfigState, SessionExecutionMode, SessionStatus, WorkflowArtifactRef,
    WorkflowCompletionSnapshot, WorkflowConsole, WorkflowConsoleEntry, WorkflowDefinition,
    WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowFailureEvent, WorkflowFailureKind,
    WorkflowFailurePolicy, WorkflowFailurePolicyMode, WorkflowHandoffPayload, WorkflowLaunchPolicy,
    WorkflowMessage, WorkflowNodeDefinition, WorkflowNodeRun, WorkflowNodeRunStatus,
    WorkflowOutputPayload, WorkflowRun, WorkflowRunStatus, WorkflowRuntimeToolCallEvent,
    WorkflowTurnEnvelope, WorkflowTurnOutputSubmissions, WorkflowTurnRuntimeState,
    WorkflowTurnSubmissionKind, WorkflowWatchdogDefinition, WorkflowWatchdogPolicy,
    WorktreeIsolationMode, DEFAULT_WORKFLOW_LAUNCH_POLICY,
    DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT, DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS,
};

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}
