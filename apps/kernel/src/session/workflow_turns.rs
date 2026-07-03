use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;
use super::workflow_outputs::{
    WorkflowRunOutputSubmission, WorkflowTurnOutputSubmissions, WorkflowTurnSubmissionKind,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowRunStatus {
    Created,
    Running,
    Waiting,
    Completing,
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowNodeRunStatus {
    Created,
    Ready,
    BlockedOnWorkspaceClaim,
    Running,
    Waiting,
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTurnRuntimeState {
    Prepared,
    Dispatched,
    Acknowledged,
    ValidatedCompleted,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRuntimeToolCallEvent {
    tool_name: String,
    arguments_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_json: Option<String>,
    ok: bool,
    timestamp_ms: u64,
}

impl WorkflowRuntimeToolCallEvent {
    pub fn new(
        tool_name: impl Into<String>,
        arguments_json: impl Into<String>,
        result_json: Option<String>,
        ok: bool,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            arguments_json: arguments_json.into(),
            result_json,
            ok,
            timestamp_ms: unix_epoch_ms(),
        }
    }

    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub fn arguments_json(&self) -> &str {
        &self.arguments_json
    }

    pub fn result_json(&self) -> Option<&str> {
        self.result_json.as_deref()
    }

    pub fn ok(&self) -> bool {
        self.ok
    }

    pub fn timestamp_ms(&self) -> u64 {
        self.timestamp_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTurnEnvelope {
    delivery_token: String,
    state: WorkflowTurnRuntimeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rendered_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mailbox_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handoff_payloads_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_output_submissions: Option<WorkflowTurnOutputSubmissions>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    runtime_tool_calls: Vec<WorkflowRuntimeToolCallEvent>,
    prepared_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dispatched_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acknowledged_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validated_completed_at_ms: Option<u64>,
}

impl WorkflowTurnEnvelope {
    pub fn new(
        delivery_token: impl Into<String>,
        rendered_prompt: String,
        mailbox_content: Option<String>,
        handoff_payloads_json: Option<String>,
    ) -> Self {
        Self {
            delivery_token: delivery_token.into(),
            state: WorkflowTurnRuntimeState::Prepared,
            rendered_prompt: Some(rendered_prompt),
            mailbox_content,
            handoff_payloads_json,
            pending_output_submissions: None,
            runtime_tool_calls: Vec::new(),
            prepared_at_ms: unix_epoch_ms(),
            dispatched_at_ms: None,
            acknowledged_at_ms: None,
            validated_completed_at_ms: None,
        }
    }

    pub fn delivery_token(&self) -> &str {
        &self.delivery_token
    }

    pub fn state(&self) -> WorkflowTurnRuntimeState {
        self.state
    }

    pub fn rendered_prompt(&self) -> Option<&str> {
        self.rendered_prompt.as_deref()
    }

    pub fn mailbox_content(&self) -> Option<&str> {
        self.mailbox_content.as_deref()
    }

    pub fn handoff_payloads_json(&self) -> Option<&str> {
        self.handoff_payloads_json.as_deref()
    }

    pub fn runtime_tool_calls(&self) -> &[WorkflowRuntimeToolCallEvent] {
        &self.runtime_tool_calls
    }

    pub fn pending_output_submissions(&self) -> Option<&WorkflowTurnOutputSubmissions> {
        self.pending_output_submissions.as_ref()
    }

    pub fn pending_output_submission(
        &self,
        kind: WorkflowTurnSubmissionKind,
    ) -> Option<&WorkflowRunOutputSubmission> {
        self.pending_output_submissions
            .as_ref()
            .and_then(|submissions| match kind {
                WorkflowTurnSubmissionKind::Intermediate => submissions.intermediate(),
                WorkflowTurnSubmissionKind::Final => submissions.final_output(),
            })
    }

    pub fn set_pending_output_submission(
        &mut self,
        kind: WorkflowTurnSubmissionKind,
        value: Option<WorkflowRunOutputSubmission>,
    ) {
        if self.pending_output_submissions.is_none() && value.is_none() {
            return;
        }
        let submissions = self
            .pending_output_submissions
            .get_or_insert_with(WorkflowTurnOutputSubmissions::new);
        submissions.set(kind, value);
        if submissions.intermediate().is_none() && submissions.final_output().is_none() {
            self.pending_output_submissions = None;
        }
    }

    pub fn add_runtime_tool_call(&mut self, event: WorkflowRuntimeToolCallEvent) {
        self.runtime_tool_calls.push(event);
    }

    pub fn mark_dispatched(&mut self) {
        self.state = WorkflowTurnRuntimeState::Dispatched;
        if self.dispatched_at_ms.is_none() {
            self.dispatched_at_ms = Some(unix_epoch_ms());
        }
    }

    pub fn mark_acknowledged(&mut self) {
        self.state = WorkflowTurnRuntimeState::Acknowledged;
        if self.acknowledged_at_ms.is_none() {
            self.acknowledged_at_ms = Some(unix_epoch_ms());
        }
    }

    pub fn mark_validated_completed(&mut self) {
        self.state = WorkflowTurnRuntimeState::ValidatedCompleted;
        if self.validated_completed_at_ms.is_none() {
            self.validated_completed_at_ms = Some(unix_epoch_ms());
        }
    }

    pub fn mark_cancelled(&mut self) {
        self.state = WorkflowTurnRuntimeState::Cancelled;
    }

    pub fn mark_failed(&mut self) {
        self.state = WorkflowTurnRuntimeState::Failed;
    }

    pub fn clear_transient_inputs(&mut self) {
        self.rendered_prompt = None;
        self.mailbox_content = None;
        self.handoff_payloads_json = None;
    }

    pub fn redacted_private_inputs(mut self) -> Self {
        self.clear_transient_inputs();
        self
    }
}
