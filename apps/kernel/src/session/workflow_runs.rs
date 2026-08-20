use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;
use super::workflow_definition::WorkflowDefinition;
use super::workflow_diagnostics::WorkflowFailureEvent;
use super::workflow_graph::{WorkflowEndpointDefinition, WorkflowNodeDefinition};
use super::workflow_outputs::{WorkflowIntermediateOutput, WorkflowOutputPayload};
use super::workflow_run_records::{WorkflowMessage, WorkflowNodeRun};
use super::workflow_scheduling::WorkflowPublicationInvocationEnvelope;
use super::workflow_turns::WorkflowRunStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    id: String,
    workflow_id: String,
    #[serde(default)]
    workflow_revision: u64,
    endpoint_id: String,
    entry_node_id: String,
    status: WorkflowRunStatus,
    invocation_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queue_ref: Option<String>,
    #[serde(default)]
    received_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queued_at_ms: Option<u64>,
    active_node_run_id: Option<String>,
    node_runs: Vec<WorkflowNodeRun>,
    messages: Vec<WorkflowMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    failure_events: Vec<WorkflowFailureEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    intermediate_outputs: Vec<WorkflowIntermediateOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    final_output: Option<WorkflowOutputPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    final_output_valid: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    final_output_warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_by_node_run_id: Option<String>,
    created_at_ms: u64,
    started_at_ms: Option<u64>,
    completed_at_ms: Option<u64>,
}

impl WorkflowRun {
    pub fn new(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        entry_node_id: impl Into<String>,
        invocation_prompt: Option<String>,
        publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
        node_runs: Vec<WorkflowNodeRun>,
        messages: Vec<WorkflowMessage>,
    ) -> Self {
        let active_node_run_id = node_runs.first().map(|run| run.id().to_string());
        let created_at_ms = unix_epoch_ms();
        let queue_ref = publication_invocation
            .as_ref()
            .and_then(|invocation| invocation.queue_ref.clone());
        Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            workflow_revision: 0,
            endpoint_id: endpoint_id.into(),
            entry_node_id: entry_node_id.into(),
            status: WorkflowRunStatus::Created,
            invocation_prompt,
            publication_invocation,
            queue_ref,
            received_at_ms: created_at_ms,
            queued_at_ms: None,
            active_node_run_id,
            node_runs,
            messages,
            failure_events: Vec::new(),
            intermediate_outputs: Vec::new(),
            final_output: None,
            final_output_valid: None,
            final_output_warning: None,
            completed_by_node_run_id: None,
            created_at_ms,
            started_at_ms: None,
            completed_at_ms: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    pub fn workflow_revision(&self) -> u64 {
        self.workflow_revision
    }

    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }

    pub fn entry_node_id(&self) -> &str {
        &self.entry_node_id
    }

    pub fn status(&self) -> WorkflowRunStatus {
        self.status
    }

    pub fn invocation_prompt(&self) -> Option<&str> {
        self.invocation_prompt.as_deref()
    }

    pub fn publication_invocation(&self) -> Option<&WorkflowPublicationInvocationEnvelope> {
        self.publication_invocation.as_ref()
    }

    pub fn queue_ref(&self) -> Option<&str> {
        self.queue_ref.as_deref()
    }

    pub fn received_at_ms(&self) -> u64 {
        self.received_at_ms
    }

    pub fn queued_at_ms(&self) -> Option<u64> {
        self.queued_at_ms
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn started_at_ms(&self) -> Option<u64> {
        self.started_at_ms
    }

    pub fn completed_at_ms(&self) -> Option<u64> {
        self.completed_at_ms
    }

    pub(crate) fn set_invocation_context(
        &mut self,
        workflow_revision: u64,
        queue_ref: Option<String>,
        received_at_ms: u64,
        queued_at_ms: Option<u64>,
    ) {
        self.workflow_revision = workflow_revision;
        self.queue_ref = queue_ref;
        self.received_at_ms = received_at_ms;
        self.queued_at_ms = queued_at_ms;
    }

    pub fn active_node_run_id(&self) -> Option<&str> {
        self.active_node_run_id.as_deref()
    }

    pub fn node_runs(&self) -> &[WorkflowNodeRun] {
        &self.node_runs
    }

    pub fn node_runs_mut(&mut self) -> &mut [WorkflowNodeRun] {
        &mut self.node_runs
    }

    pub fn node_run_mut(&mut self, workflow_node_run_id: &str) -> Option<&mut WorkflowNodeRun> {
        self.node_runs
            .iter_mut()
            .find(|node_run| node_run.id() == workflow_node_run_id)
    }

    pub fn messages(&self) -> &[WorkflowMessage] {
        &self.messages
    }

    pub fn redacted_for_user(
        mut self,
        workflow: Option<&WorkflowDefinition>,
        user_id: &str,
    ) -> Self {
        let endpoint_owner = workflow
            .and_then(|workflow| workflow.endpoint(&self.endpoint_id))
            .map(WorkflowEndpointDefinition::owner_user_id);
        if endpoint_owner != Some(user_id) {
            self.invocation_prompt = None;
            self.publication_invocation = None;
        }
        self.node_runs = self
            .node_runs
            .into_iter()
            .map(|node_run| {
                let node_owner = workflow
                    .and_then(|workflow| workflow.node(node_run.node_id()))
                    .map(WorkflowNodeDefinition::owner_user_id);
                node_run.redacted_for_node_owner(node_owner, user_id)
            })
            .collect();
        self
    }

    pub fn failure_events(&self) -> &[WorkflowFailureEvent] {
        &self.failure_events
    }

    pub fn intermediate_outputs(&self) -> &[WorkflowIntermediateOutput] {
        &self.intermediate_outputs
    }

    pub fn final_output(&self) -> Option<&WorkflowOutputPayload> {
        self.final_output.as_ref()
    }

    pub fn final_output_valid(&self) -> Option<bool> {
        self.final_output_valid
    }

    pub fn final_output_warning(&self) -> Option<&str> {
        self.final_output_warning.as_deref()
    }

    pub fn completed_by_node_run_id(&self) -> Option<&str> {
        self.completed_by_node_run_id.as_deref()
    }

    pub fn messages_mut(&mut self) -> &mut [WorkflowMessage] {
        &mut self.messages
    }

    pub fn set_status(&mut self, status: WorkflowRunStatus) {
        self.status = status;
        if matches!(status, WorkflowRunStatus::Running) && self.started_at_ms.is_none() {
            self.started_at_ms = Some(unix_epoch_ms());
        }
        if matches!(
            status,
            WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Stopped
        ) {
            self.completed_at_ms = Some(unix_epoch_ms());
        }
    }

    pub fn clear_active_node_run(&mut self) {
        self.active_node_run_id = None;
    }

    pub fn set_active_node_run(&mut self, workflow_node_run_id: impl Into<String>) {
        self.active_node_run_id = Some(workflow_node_run_id.into());
    }

    pub fn add_node_run(&mut self, node_run: WorkflowNodeRun) -> WorkflowNodeRun {
        self.node_runs.push(node_run.clone());
        node_run
    }

    pub fn add_message(&mut self, message: WorkflowMessage) -> WorkflowMessage {
        self.messages.push(message.clone());
        message
    }

    pub fn add_failure_event(&mut self, event: WorkflowFailureEvent) -> WorkflowFailureEvent {
        self.failure_events.push(event.clone());
        event
    }

    pub fn retain_failure_events(
        &mut self,
        mut predicate: impl FnMut(&WorkflowFailureEvent) -> bool,
    ) {
        self.failure_events.retain(|event| predicate(event));
    }

    pub fn add_intermediate_output(
        &mut self,
        output: WorkflowIntermediateOutput,
    ) -> WorkflowIntermediateOutput {
        self.intermediate_outputs.push(output.clone());
        output
    }

    pub fn retain_messages(&mut self, mut predicate: impl FnMut(&WorkflowMessage) -> bool) {
        self.messages.retain(|message| predicate(message));
    }

    pub fn discard_unconsumed_messages(&mut self) {
        self.messages
            .retain(|message| message.consumed_by_node_run_id().is_some());
    }

    pub fn resume(&mut self) {
        self.status = WorkflowRunStatus::Waiting;
        self.completed_at_ms = None;
    }

    pub fn set_final_output(
        &mut self,
        output: Option<WorkflowOutputPayload>,
        valid: Option<bool>,
        warning: Option<String>,
        completed_by_node_run_id: Option<String>,
    ) {
        self.final_output = output;
        self.final_output_valid = valid;
        self.final_output_warning = warning;
        self.completed_by_node_run_id = completed_by_node_run_id;
    }
}
