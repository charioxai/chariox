use std::collections::{BTreeMap, BTreeSet};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use jsonschema::JSONSchema;
use serde_json::Value;

use super::types::{
    WorkflowIntermediateOutput, WorkflowRunOutputSubmission, WorkflowTurnSubmissionKind,
};
use super::{
    unix_epoch_ms, CreateSessionRequest, PromptDetachEffect, PromptQueueItem, QueuedWorkflowLaunch,
    QueuedWorkflowLaunchSource, RuntimeSession, SessionConfigState, SessionStatus, SessionStore,
    WorkflowCompletionSnapshot, WorkflowConsole, WorkflowConsoleEntry, WorkflowDefinition,
    WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowFailureEvent, WorkflowFailureKind,
    WorkflowHandoffPayload, WorkflowLaunchPolicy, WorkflowMessage, WorkflowNodeDefinition,
    WorkflowNodeRun, WorkflowNodeRunStatus, WorkflowOutputPayload, WorkflowOutputValidationPolicy,
    WorkflowRun, WorkflowRunStatus, WorkflowRuntimeToolCallEvent, WorkflowTurnEnvelope,
    WorkflowTurnRuntimeState, WorkflowWatchdogDefinition, WorkflowWatchdogPolicy,
};
#[cfg(test)]
use super::{PromptAttachment, PromptSubmissionOutcome};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDispatch {
    pub node_run: WorkflowNodeRun,
    pub messages: Vec<WorkflowMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCompletionUpdate {
    pub workflow_run: WorkflowRun,
    pub dispatches: Vec<WorkflowDispatch>,
    pub validation_warnings: Vec<WorkflowOutputValidationWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowOutputValidationWarning {
    pub edge_id: String,
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
    pub watchdog_id: String,
    pub session_id: String,
    pub workflow_id: String,
    pub endpoint_id: String,
    pub invocation_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowLaunchAdmission {
    StartNow,
    Queued(QueuedWorkflowLaunch),
}

#[derive(Debug, Clone)]
pub struct SessionService {
    store: SessionStore,
    host_machine_id: String,
    host_daemon_id: String,
    next_prompt_number: u64,
    next_workflow_number: u64,
    next_workflow_endpoint_number: u64,
    next_workflow_node_number: u64,
    next_workflow_edge_number: u64,
    next_workflow_run_number: u64,
    next_workflow_node_run_number: u64,
    next_workflow_message_number: u64,
    next_workflow_watchdog_number: u64,
    next_queued_workflow_launch_number: u64,
}

mod core;
mod helpers;
mod launches;
mod sessions;
#[cfg(test)]
mod tests;
mod turns;
mod watchdogs;
mod workflow_defs;

pub use helpers::classify_workflow_failure_kind;
use helpers::{
    collect_ready_workflow_dispatches, describe_session_match, normalize_session_alias,
    normalize_workflow_alias, normalize_workflow_endpoint_alias, validate_workflow_edge_output,
};
