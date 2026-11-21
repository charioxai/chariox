use std::collections::{BTreeMap, BTreeSet};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use jsonschema::JSONSchema;
use serde_json::Value;

use super::types::{
    WorkflowIntermediateOutput, WorkflowRunOutputSubmission, WorkflowTurnSubmissionKind,
};
use super::{
    unix_epoch_ms, AgentTrigger, AgentTriggerBusyPolicy, AgentTriggerDispatch, AgentTriggerOutcome,
    AgentTriggerRepeat, CanonicalViewport, CollaborationLevel, CreateSessionRequest,
    DurableWorkflowHotState, EnvironmentError, EnvironmentReplay, ProjectId, PromptDetachEffect,
    PromptQueueItem, RoomEnvironmentRegistry, RoomEnvironmentSnapshot, RuntimeProject,
    RuntimeProjectKind, RuntimeProjectStatus, RuntimeSession, SessionConfigState, SessionInvite,
    SessionMember, SessionProjectSelection, SessionStatus, SessionStore,
    WorkflowCompletionSnapshot, WorkflowConsole, WorkflowConsoleEntry, WorkflowDefinition,
    WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowFailureEvent, WorkflowFailureKind,
    WorkflowHandoffPayload, WorkflowHandoffValidationPolicy, WorkflowMessage,
    WorkflowNodeDefinition, WorkflowNodeRun, WorkflowNodeRunStatus, WorkflowOutputPayload,
    WorkflowPromptQueueDefinition, WorkflowPublicationDefinition, WorkflowQueuedPrompt,
    WorkflowQueuedPromptSource, WorkflowQueuedPromptStatus, WorkflowRun, WorkflowRunStatus,
    WorkflowRuntimeToolCallEvent, WorkflowScheduleDefinition, WorkflowScheduleOverlapPolicy,
    WorkflowScheduleTrigger, WorkflowSchemaDefinition, WorkflowTurnEnvelope,
    WorkflowTurnRuntimeState, WorkflowWatchdogDefinition, WorkflowWatchdogPolicy,
    WorkspaceLinkAttachment, WorkspaceLinkDefinition, DEFAULT_LOCAL_USER_ID,
};
#[cfg(test)]
use super::{PromptAttachment, PromptSubmissionOutcome};

mod prompt_ids;
pub use prompt_ids::PromptIdAllocator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDispatch {
    pub node_run: WorkflowNodeRun,
    pub messages: Vec<WorkflowMessage>,
    pub endpoint_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCompletionUpdate {
    pub workflow_run: WorkflowRun,
    pub dispatches: Vec<WorkflowDispatch>,
    pub validation_warnings: Vec<WorkflowHandoffValidationWarning>,
    pub handoff_validation_failure: Option<WorkflowHandoffValidationFailure>,
    pub missing_output_failure: Option<WorkflowMissingOutputFailure>,
    pub run_output_validation_failure: Option<WorkflowRunOutputValidationFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowHandoffValidationWarning {
    pub edge_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowHandoffValidationFailure {
    pub edge_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRunOutputValidationFailure {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowMissingOutputFailure {
    pub message: String,
}

#[derive(Debug, Clone)]
struct WorkflowCompletionContext {
    workflow_run: WorkflowRun,
    source_node_run: WorkflowNodeRun,
    workflow: WorkflowDefinition,
}

#[derive(Debug, Clone)]
struct PendingWorkflowTurnOutputs {
    intermediate: Option<WorkflowRunOutputSubmission>,
    final_output: Option<WorkflowRunOutputSubmission>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowWatchdogTickPlan {
    pub scheduled_for_ms: u64,
    pub watchdog_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub endpoint_id: String,
    pub queue_id: Option<String>,
    pub invocation_prompt: String,
    pub enqueue_prompt: bool,
    /// Busy Queue admissions enqueue durably without starting a second run.
    pub enqueue_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowWatchdogCollection {
    pub plans: Vec<WorkflowWatchdogTickPlan>,
    pub changed_session_ids: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct SessionService {
    store: SessionStore,
    room_environments: RoomEnvironmentRegistry,
    durable_state: Option<crate::durable_state::DurableKernelStateStore>,
    projects: BTreeMap<String, RuntimeProject>,
    ephemeral_session_ids: BTreeSet<String>,
    host_machine_id: String,
    host_daemon_id: String,
    prompt_id_allocator: PromptIdAllocator,
    next_workflow_number: u64,
    next_workflow_schema_number: u64,
    next_workflow_endpoint_number: u64,
    next_workflow_node_number: u64,
    next_workflow_edge_number: u64,
    next_workflow_message_number: u64,
    next_workflow_watchdog_number: u64,
    next_workflow_publication_number: u64,
    next_workflow_prompt_queue_number: u64,
    next_workflow_queued_prompt_number: u64,
    next_agent_trigger_number: u64,
    max_workflow_queues_per_workflow: usize,
    session_default_max_agents: i32,
    workflow_default_max_concurrent: u32,
    next_workspace_link_number: u64,
}

mod core;
mod durable_queue;
mod project_creation;
mod retired_sessions;
pub(crate) use durable_queue::{PreparedWorkflowQueueRun, WorkflowQueueRun};
mod agent_triggers;
mod helpers;
mod launches;
mod room_environment_durability;
mod room_environments;
mod sessions;
#[cfg(test)]
mod tests;
mod turn_completion;
mod turns;
mod watchdogs;
mod workflow_code;
mod workflow_defs;
mod workflow_responsible;

pub use helpers::classify_workflow_failure_kind;
use helpers::{
    collect_ready_workflow_dispatches, describe_session_match, new_workflow_node_run_id,
    normalize_session_alias, normalize_workflow_alias, normalize_workflow_endpoint_alias,
    normalize_workflow_publication_alias, normalize_workflow_queue_alias,
    validate_workflow_edge_handoff,
};
