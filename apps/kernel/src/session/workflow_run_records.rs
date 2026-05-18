use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;
use super::workflow_outputs::{WorkflowCompletionSnapshot, WorkflowTurnSubmissionKind};
use super::workflow_turns::{WorkflowNodeRunStatus, WorkflowTurnEnvelope};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowMessage {
    id: String,
    source_node_run_id: Option<String>,
    target_node_id: String,
    message_type: String,
    summary: String,
    handoff_payload: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consumed_by_node_run_id: Option<String>,
    created_at_ms: u64,
}

impl WorkflowMessage {
    pub fn new(
        id: impl Into<String>,
        source_node_run_id: Option<String>,
        target_node_id: impl Into<String>,
        message_type: impl Into<String>,
        summary: impl Into<String>,
        handoff_payload: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            source_node_run_id,
            target_node_id: target_node_id.into(),
            message_type: message_type.into(),
            summary: summary.into(),
            handoff_payload: handoff_payload.into(),
            consumed_by_node_run_id: None,
            created_at_ms: unix_epoch_ms(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn source_node_run_id(&self) -> Option<&str> {
        self.source_node_run_id.as_deref()
    }

    pub fn target_node_id(&self) -> &str {
        &self.target_node_id
    }

    pub fn message_type(&self) -> &str {
        &self.message_type
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn handoff_payload(&self) -> &str {
        &self.handoff_payload
    }

    pub fn consumed_by_node_run_id(&self) -> Option<&str> {
        self.consumed_by_node_run_id.as_deref()
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn set_consumed_by_node_run_id(&mut self, workflow_node_run_id: impl Into<String>) {
        self.consumed_by_node_run_id = Some(workflow_node_run_id.into());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeRun {
    id: String,
    node_id: String,
    agent_id: String,
    status: WorkflowNodeRunStatus,
    summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion: Option<WorkflowCompletionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn_envelope: Option<WorkflowTurnEnvelope>,
    created_at_ms: u64,
    started_at_ms: Option<u64>,
    completed_at_ms: Option<u64>,
}

impl WorkflowNodeRun {
    pub fn new(
        id: impl Into<String>,
        node_id: impl Into<String>,
        agent_id: impl Into<String>,
        status: WorkflowNodeRunStatus,
    ) -> Self {
        Self {
            id: id.into(),
            node_id: node_id.into(),
            agent_id: agent_id.into(),
            status,
            summary: None,
            completion: None,
            turn_envelope: None,
            created_at_ms: unix_epoch_ms(),
            started_at_ms: None,
            completed_at_ms: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn status(&self) -> WorkflowNodeRunStatus {
        self.status
    }

    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    pub fn completion(&self) -> Option<&WorkflowCompletionSnapshot> {
        self.completion.as_ref()
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn turn_envelope(&self) -> Option<&WorkflowTurnEnvelope> {
        self.turn_envelope.as_ref()
    }

    pub fn has_valid_pending_final_output(&self) -> bool {
        self.turn_envelope()
            .and_then(|envelope| {
                envelope.pending_output_submission(WorkflowTurnSubmissionKind::Final)
            })
            .is_some_and(|submission| submission.valid() && submission.warning().is_none())
    }

    pub fn turn_envelope_mut(&mut self) -> Option<&mut WorkflowTurnEnvelope> {
        self.turn_envelope.as_mut()
    }

    pub fn started_at_ms(&self) -> Option<u64> {
        self.started_at_ms
    }

    pub fn completed_at_ms(&self) -> Option<u64> {
        self.completed_at_ms
    }

    pub fn set_status(&mut self, status: WorkflowNodeRunStatus) {
        self.status = status;
        if matches!(status, WorkflowNodeRunStatus::Running) && self.started_at_ms.is_none() {
            self.started_at_ms = Some(unix_epoch_ms());
        }
        if matches!(
            status,
            WorkflowNodeRunStatus::Completed
                | WorkflowNodeRunStatus::Failed
                | WorkflowNodeRunStatus::Stopped
        ) {
            self.completed_at_ms = Some(unix_epoch_ms());
        }
    }

    pub fn set_summary(&mut self, summary: Option<String>) {
        self.summary = summary;
    }

    pub fn set_completion(&mut self, completion: Option<WorkflowCompletionSnapshot>) {
        self.completion = completion;
    }

    pub fn set_turn_envelope(&mut self, turn_envelope: Option<WorkflowTurnEnvelope>) {
        self.turn_envelope = turn_envelope;
    }

    pub fn resume(&mut self) {
        self.status = WorkflowNodeRunStatus::Ready;
        self.completed_at_ms = None;
    }

    pub fn redacted_for_node_owner(mut self, owner_user_id: Option<&str>, user_id: &str) -> Self {
        if owner_user_id != Some(user_id) {
            self.turn_envelope = self
                .turn_envelope
                .map(WorkflowTurnEnvelope::redacted_private_inputs);
        }
        self
    }
}
