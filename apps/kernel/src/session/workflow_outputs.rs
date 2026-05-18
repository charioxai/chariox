use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRunOutputSubmission {
    output: WorkflowOutputPayload,
    valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    submitted_at_ms: u64,
}

impl WorkflowRunOutputSubmission {
    pub fn new(output: WorkflowOutputPayload, valid: bool, warning: Option<String>) -> Self {
        Self {
            output,
            valid,
            warning,
            submitted_at_ms: unix_epoch_ms(),
        }
    }

    pub fn output(&self) -> &WorkflowOutputPayload {
        &self.output
    }

    pub fn valid(&self) -> bool {
        self.valid
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub fn submitted_at_ms(&self) -> u64 {
        self.submitted_at_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTurnSubmissionKind {
    Intermediate,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTurnOutputSubmissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intermediate: Option<WorkflowRunOutputSubmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    final_output: Option<WorkflowRunOutputSubmission>,
}

impl WorkflowTurnOutputSubmissions {
    pub fn new() -> Self {
        Self {
            intermediate: None,
            final_output: None,
        }
    }

    pub fn intermediate(&self) -> Option<&WorkflowRunOutputSubmission> {
        self.intermediate.as_ref()
    }

    pub fn final_output(&self) -> Option<&WorkflowRunOutputSubmission> {
        self.final_output.as_ref()
    }

    pub fn set(
        &mut self,
        kind: WorkflowTurnSubmissionKind,
        submission: Option<WorkflowRunOutputSubmission>,
    ) {
        match kind {
            WorkflowTurnSubmissionKind::Intermediate => self.intermediate = submission,
            WorkflowTurnSubmissionKind::Final => self.final_output = submission,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowIntermediateOutput {
    id: String,
    source_node_run_id: String,
    output: WorkflowOutputPayload,
    valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    timestamp_ms: u64,
}

impl WorkflowIntermediateOutput {
    pub fn new(
        id: impl Into<String>,
        source_node_run_id: impl Into<String>,
        output: WorkflowOutputPayload,
        valid: bool,
        warning: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            source_node_run_id: source_node_run_id.into(),
            output,
            valid,
            warning,
            timestamp_ms: unix_epoch_ms(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn source_node_run_id(&self) -> &str {
        &self.source_node_run_id
    }

    pub fn output(&self) -> &WorkflowOutputPayload {
        &self.output
    }

    pub fn valid(&self) -> bool {
        self.valid
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub fn timestamp_ms(&self) -> u64 {
        self.timestamp_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowArtifactRef {
    id: String,
    kind: String,
    path: String,
    display_name: String,
}

impl WorkflowArtifactRef {
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        path: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            path: path.into(),
            display_name: display_name.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowOutputPayload {
    message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    artifacts: Vec<WorkflowArtifactRef>,
}

impl WorkflowOutputPayload {
    pub fn new(message: impl Into<String>, artifacts: Vec<WorkflowArtifactRef>) -> Self {
        Self {
            message: message.into(),
            artifacts,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn artifacts(&self) -> &[WorkflowArtifactRef] {
        &self.artifacts
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCompletionSnapshot {
    summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<WorkflowOutputPayload>,
}

impl WorkflowCompletionSnapshot {
    pub fn new(summary: impl Into<String>, output: Option<WorkflowOutputPayload>) -> Self {
        Self {
            summary: summary.into(),
            output,
        }
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn output(&self) -> Option<&WorkflowOutputPayload> {
        self.output.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowHandoffPayload {
    workflow_run_id: String,
    workflow_id: String,
    source_node_run_id: String,
    source_node_id: String,
    source_agent_id: String,
    target_node_id: String,
    invocation_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion: Option<WorkflowCompletionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_warning: Option<String>,
}

impl WorkflowHandoffPayload {
    pub fn new(
        workflow_run_id: impl Into<String>,
        workflow_id: impl Into<String>,
        source_node_run_id: impl Into<String>,
        source_node_id: impl Into<String>,
        source_agent_id: impl Into<String>,
        target_node_id: impl Into<String>,
        invocation_prompt: Option<String>,
        completion: Option<WorkflowCompletionSnapshot>,
        output_schema_ref: Option<String>,
        validation_warning: Option<String>,
    ) -> Self {
        Self {
            workflow_run_id: workflow_run_id.into(),
            workflow_id: workflow_id.into(),
            source_node_run_id: source_node_run_id.into(),
            source_node_id: source_node_id.into(),
            source_agent_id: source_agent_id.into(),
            target_node_id: target_node_id.into(),
            invocation_prompt,
            completion,
            output_schema_ref,
            validation_warning,
        }
    }

    pub fn workflow_run_id(&self) -> &str {
        &self.workflow_run_id
    }

    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    pub fn source_node_run_id(&self) -> &str {
        &self.source_node_run_id
    }

    pub fn source_node_id(&self) -> &str {
        &self.source_node_id
    }

    pub fn source_agent_id(&self) -> &str {
        &self.source_agent_id
    }

    pub fn target_node_id(&self) -> &str {
        &self.target_node_id
    }

    pub fn invocation_prompt(&self) -> Option<&str> {
        self.invocation_prompt.as_deref()
    }

    pub fn completion(&self) -> Option<&WorkflowCompletionSnapshot> {
        self.completion.as_ref()
    }
}
