use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowFailureKind {
    MissingAck,
    MissingStructuredOutput,
    OutputValidationFailed,
    WorkflowRunOutputValidationFailed,
    NodeTurnBudgetExhausted,
    RunStopped,
    ProviderFailure,
    TransportFailure,
    TurnStalled,
    /// A step's agent could not go on without someone: it asked for a permission, an answer or
    /// the browser, or its task blocked. The message is the agent's own reason.
    StepNeedsHelp,
}

impl WorkflowFailureKind {
    /// The name a client and a mailbox entry show: the serialized one.
    pub fn name(self) -> &'static str {
        match self {
            Self::MissingAck => "missing_ack",
            Self::MissingStructuredOutput => "missing_structured_output",
            Self::OutputValidationFailed => "output_validation_failed",
            Self::WorkflowRunOutputValidationFailed => "workflow_run_output_validation_failed",
            Self::NodeTurnBudgetExhausted => "node_turn_budget_exhausted",
            Self::RunStopped => "run_stopped",
            Self::ProviderFailure => "provider_failure",
            Self::TransportFailure => "transport_failure",
            Self::TurnStalled => "turn_stalled",
            Self::StepNeedsHelp => "step_needs_help",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowFailurePolicyMode {
    None,
    Notify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFailurePolicy {
    mode: WorkflowFailurePolicyMode,
    notify_source_node: bool,
    notify_sink_nodes: bool,
}

impl Default for WorkflowFailurePolicy {
    fn default() -> Self {
        Self {
            mode: WorkflowFailurePolicyMode::Notify,
            notify_source_node: true,
            notify_sink_nodes: true,
        }
    }
}

impl WorkflowFailurePolicy {
    pub fn mode(&self) -> WorkflowFailurePolicyMode {
        self.mode
    }

    pub fn notify_source_node(&self) -> bool {
        self.notify_source_node
    }

    pub fn notify_sink_nodes(&self) -> bool {
        self.notify_sink_nodes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFailureEvent {
    kind: WorkflowFailureKind,
    source_node_run_id: String,
    edge_ids: Vec<String>,
    message: String,
    timestamp_ms: u64,
}

impl WorkflowFailureEvent {
    pub fn new(
        kind: WorkflowFailureKind,
        source_node_run_id: impl Into<String>,
        edge_ids: Vec<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            source_node_run_id: source_node_run_id.into(),
            edge_ids,
            message: message.into(),
            timestamp_ms: unix_epoch_ms(),
        }
    }

    pub fn kind(&self) -> WorkflowFailureKind {
        self.kind
    }

    pub fn source_node_run_id(&self) -> &str {
        &self.source_node_run_id
    }

    pub fn edge_ids(&self) -> &[String] {
        &self.edge_ids
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn timestamp_ms(&self) -> u64 {
        self.timestamp_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConsoleEntry {
    timestamp_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_node_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_agent_id: Option<String>,
    text: String,
}

impl WorkflowConsoleEntry {
    pub fn new(
        source_node_run_id: Option<String>,
        source_agent_id: Option<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            timestamp_ms: unix_epoch_ms(),
            source_node_run_id,
            source_agent_id,
            text: text.into(),
        }
    }

    pub fn timestamp_ms(&self) -> u64 {
        self.timestamp_ms
    }

    pub fn source_node_run_id(&self) -> Option<&str> {
        self.source_node_run_id.as_deref()
    }

    pub fn source_agent_id(&self) -> Option<&str> {
        self.source_agent_id.as_deref()
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConsole {
    workflow_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    entries: Vec<WorkflowConsoleEntry>,
}

impl WorkflowConsole {
    pub fn new(workflow_id: impl Into<String>) -> Self {
        Self {
            workflow_id: workflow_id.into(),
            entries: Vec::new(),
        }
    }

    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    pub fn entries(&self) -> &[WorkflowConsoleEntry] {
        &self.entries
    }

    pub fn add_entry(&mut self, entry: WorkflowConsoleEntry) -> WorkflowConsoleEntry {
        self.entries.push(entry.clone());
        entry
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
