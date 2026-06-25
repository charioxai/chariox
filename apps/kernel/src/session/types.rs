use std::time::{SystemTime, UNIX_EPOCH};

pub use super::metaagent_task::{MetaagentTask, MetaagentTaskStatus};
pub use super::prompt_queue::{
    AgentPromptState, PromptAttachment, PromptCancellation, PromptCompletion, PromptDetachEffect,
    PromptOrigin, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
};
pub use super::runtime_interactions::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionInputKind, RuntimeInteractionKind,
    RuntimeInteractionLevel,
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
pub use super::workflow_definition::{WorkflowDefinition, WorkflowSchemaDefinition};
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
pub use super::workflow_publication::WorkflowPublicationDefinition;
pub use super::workflow_run_records::{
    WorkflowMessage, WorkflowNodeRun, WorkflowNodeThinkingTrace,
};
pub use super::workflow_runs::WorkflowRun;
pub use super::workflow_scheduling::{
    WorkflowPromptQueueDefinition, WorkflowPublicationInvocationEnvelope, WorkflowQueuedPrompt,
    WorkflowQueuedPromptSource, WorkflowQueuedPromptStatus, WorkflowWatchdogDefinition,
    WorkflowWatchdogPolicy, DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS,
};
pub use super::workflow_turns::{
    WorkflowNodeRunStatus, WorkflowRunStatus, WorkflowRuntimeToolCallEvent, WorkflowTurnEnvelope,
    WorkflowTurnRuntimeState,
};
pub use super::workspace_links::{WorkspaceLinkAttachment, WorkspaceLinkDefinition};

pub const DEFAULT_SESSION_MAX_AGENTS: i32 = 1024;
pub const DEFAULT_WORKFLOW_CODE_MAX_CONCURRENT: u32 = 32;
pub const DEFAULT_WORKFLOW_CODE_MAX_NODES: u32 = 1024;
pub const DEFAULT_WORKFLOW_CODE_MAX_AGENTS: u32 = 1024;
pub const DEFAULT_WORKFLOW_CODE_MAX_EDGES: u32 = 4096;
pub const DEFAULT_WORKFLOW_CODE_MAX_QUEUES: u32 = 1024;
pub const DEFAULT_WORKFLOW_CODE_MAX_WATCHDOGS: u32 = 1024;
pub const DEFAULT_WORKFLOW_CODE_MAX_SCHEMA_BYTES: u32 = 1_048_576;
pub const DEFAULT_WORKFLOW_CODE_MAX_GENERATED_PROMPT_BYTES: u32 = 4_194_304;
pub const DEFAULT_WORKFLOW_CODE_SCRIPT_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_WORKFLOW_CODE_SCRIPT_MEMORY_BYTES: u64 = 268_435_456;
pub const DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT: usize = 128;

pub fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
