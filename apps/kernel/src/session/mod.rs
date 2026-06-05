mod agent_config;
mod owner;
mod prompt_queue;
mod prompt_runtime;
mod runtime_interactions;
mod runtime_session;
mod runtime_worktrees;
mod service;
mod session_config;
mod session_identity;
mod session_lifecycle;
mod store;
mod types;
mod workflow_canvas;
mod workflow_definition;
mod workflow_diagnostics;
mod workflow_graph;
mod workflow_outputs;
mod workflow_publication;
mod workflow_run_records;
mod workflow_runs;
mod workflow_scheduling;
mod workflow_turns;
mod workspace_links;

pub use agent_config::{
    effective_agent_execution_config, effective_agent_execution_mode,
    effective_agent_permission_level, EffectiveAgentExecutionConfig,
};
pub(crate) use owner::{SessionStateOwner, SessionStateReader, SessionStateStore};
pub use service::{
    classify_workflow_failure_kind, WorkflowCompletionUpdate, WorkflowDispatch,
    WorkflowHandoffValidationWarning, WorkflowWatchdogTickPlan,
};
pub use service::{PromptIdAllocator, SessionService};
pub use store::SessionStore;
pub use types::WorkflowHandoffValidationPolicy;
pub use types::{
    unix_epoch_ms, CollaborationLevel, CreateSessionRequest, PromptAttachment, PromptCancellation,
    PromptCompletion, PromptDetachEffect, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
    RuntimeSession, RuntimeWorktreeAssignment, SchedulerState, SessionAgentDefaults,
    SessionCollaborationAgentCounts, SessionConfigState, SessionExecutionMode, SessionInvite,
    SessionMember, SessionStatus, WorkflowArtifactRef, WorkflowCanvasLayout,
    WorkflowCanvasLayoutPatch, WorkflowCanvasPoint, WorkflowCompletionSnapshot, WorkflowConsole,
    WorkflowConsoleEntry, WorkflowDefinition, WorkflowEdgeDefinition, WorkflowEdgeEndpointSide,
    WorkflowEndpointDefinition, WorkflowFailureEvent, WorkflowFailureKind, WorkflowFailurePolicy,
    WorkflowFailurePolicyMode, WorkflowHandoffPayload, WorkflowMessage, WorkflowNodeDefinition,
    WorkflowNodeRun, WorkflowNodeRunStatus, WorkflowOutputPayload, WorkflowPromptQueueDefinition,
    WorkflowPublicationDefinition, WorkflowPublicationInvocationEnvelope, WorkflowQueuedPrompt,
    WorkflowQueuedPromptSource, WorkflowQueuedPromptStatus, WorkflowRun, WorkflowRunStatus,
    WorkflowRuntimeToolCallEvent, WorkflowTurnEnvelope, WorkflowTurnOutputSubmissions,
    WorkflowTurnRuntimeState, WorkflowTurnSubmissionKind, WorkflowWatchdogDefinition,
    WorkflowWatchdogPolicy, WorkspaceLinkAttachment, WorkspaceLinkDefinition,
    WorktreeIsolationMode, DEFAULT_LOCAL_USER_ID, DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT,
    DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS,
};
pub(crate) use workspace_links::normalize_workspace_link_repo_root;

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn is_zero(value: &usize) -> bool {
    *value == 0
}
