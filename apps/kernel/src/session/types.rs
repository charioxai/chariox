use std::time::{SystemTime, UNIX_EPOCH};

pub use super::prompt_queue::{
    AgentPromptState, PromptAttachment, PromptCancellation, PromptCompletion, PromptDetachEffect,
    PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
};
pub use super::runtime_interactions::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};
pub use super::runtime_session::{RuntimeSession, SessionCollaborationAgentCounts};
pub use super::runtime_worktrees::{RuntimeWorktreeAssignment, WorktreeIsolationMode};
pub use super::session_config::SessionConfigState;
pub use super::session_identity::{
    CollaborationLevel, CreateSessionRequest, SessionAgentDefaults, SessionInvite, SessionMember,
    DEFAULT_LOCAL_USER_ID,
};
pub use super::session_lifecycle::{SchedulerState, SessionExecutionMode, SessionStatus};
pub use super::workflow_canvas::{
    WorkflowCanvasLayout, WorkflowCanvasLayoutPatch, WorkflowCanvasPoint,
};
pub use super::workflow_definition::WorkflowDefinition;
pub use super::workflow_diagnostics::{
    WorkflowConsole, WorkflowConsoleEntry, WorkflowFailureEvent, WorkflowFailureKind,
    WorkflowFailurePolicy, WorkflowFailurePolicyMode,
};
pub use super::workflow_graph::{
    WorkflowEdgeDefinition, WorkflowEdgeEndpointSide, WorkflowEndpointDefinition,
    WorkflowHandoffValidationPolicy, WorkflowNodeDefinition,
};
pub use super::workflow_outputs::{
    WorkflowArtifactRef, WorkflowCompletionSnapshot, WorkflowHandoffPayload,
    WorkflowIntermediateOutput, WorkflowOutputPayload, WorkflowRunOutputSubmission,
    WorkflowTurnOutputSubmissions, WorkflowTurnSubmissionKind,
};
pub use super::workflow_publication::{
    WorkflowPublicationDefinition, WorkflowPublicationPairingCode,
    WorkflowPublicationPairingCodeRecord, WorkflowPublicationSenderCredential,
    WorkflowPublicationTrustedSender,
};
pub use super::workflow_run_records::{WorkflowMessage, WorkflowNodeRun};
pub use super::workflow_runs::WorkflowRun;
pub use super::workflow_scheduling::{
    WorkflowPromptQueueDefinition, WorkflowQueuedPrompt, WorkflowQueuedPromptSource,
    WorkflowQueuedPromptStatus, WorkflowWatchdogDefinition, WorkflowWatchdogPolicy,
    DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS,
};
pub use super::workflow_turns::{
    WorkflowNodeRunStatus, WorkflowRunStatus, WorkflowRuntimeToolCallEvent, WorkflowTurnEnvelope,
    WorkflowTurnRuntimeState,
};
pub use super::workspace_links::{WorkspaceLinkAttachment, WorkspaceLinkDefinition};

pub const DEFAULT_SESSION_MAX_AGENTS: i32 = 64;
pub const DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT: usize = 128;

pub fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
