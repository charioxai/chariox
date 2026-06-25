use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use crate::config::DaemonConfig;
use crate::error::DaemonError;
use jsonschema::JSONSchema;
use serde_json::Value;

use super::types::{
    WorkflowIntermediateOutput, WorkflowRunOutputSubmission, WorkflowTurnSubmissionKind,
};
use super::{
    unix_epoch_ms, CollaborationLevel, CreateSessionRequest, PromptDetachEffect, PromptQueueItem,
    RuntimeSession, SessionConfigState, SessionInvite, SessionMember, SessionStatus, SessionStore,
    WorkflowCompletionSnapshot, WorkflowConsole, WorkflowConsoleEntry, WorkflowDefinition,
    WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowFailureEvent, WorkflowFailureKind,
    WorkflowHandoffPayload, WorkflowHandoffValidationPolicy, WorkflowMessage,
    WorkflowNodeDefinition, WorkflowNodeRun, WorkflowNodeRunStatus, WorkflowOutputPayload,
    WorkflowPromptQueueDefinition, WorkflowPublicationDefinition, WorkflowQueuedPrompt,
    WorkflowQueuedPromptSource, WorkflowRun, WorkflowRunStatus, WorkflowRuntimeToolCallEvent,
    WorkflowTurnEnvelope, WorkflowTurnRuntimeState, WorkflowWatchdogDefinition,
    WorkflowWatchdogPolicy, WorkspaceLinkAttachment, WorkspaceLinkDefinition,
    DEFAULT_LOCAL_USER_ID,
};
#[cfg(test)]
use super::{PromptAttachment, PromptSubmissionOutcome};

#[derive(Debug, Clone, Default)]
pub struct PromptIdAllocator {
    next_prompt_number: Arc<AtomicU64>,
}

impl PromptIdAllocator {
    pub(crate) fn next_prompt_id(&self) -> String {
        let next = self.next_prompt_number.fetch_add(1, Ordering::SeqCst) + 1;
        format!("prompt-{next}")
    }

    pub(crate) fn observe_prompt_id(&self, prompt_id: &str) {
        if let Some(number) = prompt_id_number(prompt_id) {
            self.advance_to_at_least(number);
        }
    }

    pub(crate) fn advance_to_at_least(&self, number: u64) {
        let mut current = self.next_prompt_number.load(Ordering::SeqCst);
        while current < number {
            match self.next_prompt_number.compare_exchange(
                current,
                number,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }
}

pub(crate) fn prompt_id_number(prompt_id: &str) -> Option<u64> {
    prompt_id.strip_prefix("prompt-")?.parse::<u64>().ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDispatch {
    pub node_run: WorkflowNodeRun,
    pub messages: Vec<WorkflowMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCompletionUpdate {
    pub workflow_run: WorkflowRun,
    pub dispatches: Vec<WorkflowDispatch>,
    pub validation_warnings: Vec<WorkflowHandoffValidationWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowHandoffValidationWarning {
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
    pub queue_id: Option<String>,
    pub invocation_prompt: String,
}

#[derive(Debug, Clone)]
pub struct SessionService {
    store: SessionStore,
    host_machine_id: String,
    host_daemon_id: String,
    prompt_id_allocator: PromptIdAllocator,
    next_workflow_number: u64,
    next_workflow_endpoint_number: u64,
    next_workflow_node_number: u64,
    next_workflow_edge_number: u64,
    next_workflow_run_number: u64,
    next_workflow_node_run_number: u64,
    next_workflow_message_number: u64,
    next_workflow_watchdog_number: u64,
    next_workflow_publication_number: u64,
    next_workflow_prompt_queue_number: u64,
    next_workflow_queued_prompt_number: u64,
    max_workflow_queues_per_workflow: Option<usize>,
    session_default_max_agents: i32,
    next_workspace_link_number: u64,
}

mod core;
mod helpers;
mod launches;
mod sessions;
#[cfg(test)]
mod tests;
mod turns;
mod watchdogs;
mod workflow_code;
mod workflow_defs;

pub use helpers::classify_workflow_failure_kind;
use helpers::{
    collect_ready_workflow_dispatches, describe_session_match, normalize_session_alias,
    normalize_workflow_alias, normalize_workflow_endpoint_alias,
    normalize_workflow_publication_alias, normalize_workflow_queue_alias,
    validate_workflow_edge_handoff,
};
