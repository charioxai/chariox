use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;

use super::prompt_runtime::PromptRuntimeState;
pub use super::workflow_publication::{
    WorkflowPublicationDefinition, WorkflowPublicationPairingCode,
    WorkflowPublicationPairingCodeRecord, WorkflowPublicationSenderCredential,
    WorkflowPublicationTrustedSender,
};
pub use super::workspace_links::{WorkspaceLinkAttachment, WorkspaceLinkDefinition};

pub const DEFAULT_SESSION_MAX_AGENTS: i32 = 64;
pub const DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS: u64 = 100;
pub const DEFAULT_WORKFLOW_LAUNCH_POLICY: WorkflowLaunchPolicy = WorkflowLaunchPolicy::Reject;
pub const DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT: usize = 128;
pub const DEFAULT_LOCAL_USER_ID: &str = "local";

fn default_workflow_flush_agent_context_before_run() -> bool {
    true
}

fn default_workflow_node_can_complete_workflow_run() -> bool {
    false
}

fn default_workflow_node_can_emit_intermediate_run_output() -> bool {
    false
}

fn default_session_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

fn default_session_members() -> Vec<SessionMember> {
    vec![SessionMember::local()]
}

fn default_session_invite_max_uses() -> Option<u32> {
    Some(1)
}

fn default_workflow_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

fn default_workflow_node_public_label() -> String {
    "agent".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEndpointDefinition {
    id: String,
    #[serde(default = "default_workflow_owner_user_id")]
    owner_user_id: String,
    alias: Option<String>,
    entry_node_id: String,
}

impl WorkflowEndpointDefinition {
    pub fn new(
        id: impl Into<String>,
        alias: Option<String>,
        entry_node_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            owner_user_id: default_workflow_owner_user_id(),
            alias,
            entry_node_id: entry_node_id.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    pub fn entry_node_id(&self) -> &str {
        &self.entry_node_id
    }

    pub fn set_owner_user_id(&mut self, owner_user_id: impl Into<String>) {
        self.owner_user_id = owner_user_id.into();
    }

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
    }

    pub fn set_entry_node_id(&mut self, entry_node_id: impl Into<String>) {
        self.entry_node_id = entry_node_id.into();
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowWatchdogPolicy {
    Skip,
    Queue,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowLaunchPolicy {
    Reject,
    Queue,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuedWorkflowLaunchSource {
    Manual,
    Watchdog,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedWorkflowLaunch {
    id: String,
    workflow_id: String,
    endpoint_id: String,
    invocation_prompt: Option<String>,
    source: QueuedWorkflowLaunchSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    watchdog_id: Option<String>,
    queued_at_ms: u64,
}

impl QueuedWorkflowLaunch {
    pub fn new(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        invocation_prompt: Option<String>,
        source: QueuedWorkflowLaunchSource,
        watchdog_id: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            invocation_prompt,
            source,
            watchdog_id,
            queued_at_ms: unix_epoch_ms(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }
    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }
    pub fn invocation_prompt(&self) -> Option<&str> {
        self.invocation_prompt.as_deref()
    }
    pub fn source(&self) -> QueuedWorkflowLaunchSource {
        self.source
    }
    pub fn watchdog_id(&self) -> Option<&str> {
        self.watchdog_id.as_deref()
    }
    pub fn queued_at_ms(&self) -> u64 {
        self.queued_at_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowWatchdogDefinition {
    id: String,
    workflow_id: String,
    endpoint_id: String,
    enabled: bool,
    interval_seconds: u64,
    invocation_prompt: String,
    policy: WorkflowWatchdogPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_wakeups: Option<u64>,
    #[serde(default)]
    wakeups_executed: u64,
    next_run_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_run_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_workflow_run_id: Option<String>,
    #[serde(default)]
    pending_run: bool,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl WorkflowWatchdogDefinition {
    pub fn new(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        interval_seconds: u64,
        invocation_prompt: impl Into<String>,
        policy: WorkflowWatchdogPolicy,
        max_wakeups: Option<u64>,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            enabled: true,
            interval_seconds,
            invocation_prompt: invocation_prompt.into(),
            policy,
            max_wakeups,
            wakeups_executed: 0,
            next_run_at_ms: now.saturating_add(interval_seconds.saturating_mul(1000)),
            last_run_at_ms: None,
            last_status: None,
            last_error: None,
            last_workflow_run_id: None,
            pending_run: false,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }
    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn interval_seconds(&self) -> u64 {
        self.interval_seconds
    }
    pub fn invocation_prompt(&self) -> &str {
        &self.invocation_prompt
    }
    pub fn policy(&self) -> WorkflowWatchdogPolicy {
        self.policy
    }
    pub fn max_wakeups(&self) -> Option<u64> {
        self.max_wakeups
    }
    pub fn wakeups_executed(&self) -> u64 {
        self.wakeups_executed
    }
    pub fn next_run_at_ms(&self) -> u64 {
        self.next_run_at_ms
    }
    pub fn last_run_at_ms(&self) -> Option<u64> {
        self.last_run_at_ms
    }
    pub fn last_status(&self) -> Option<&str> {
        self.last_status.as_deref()
    }
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    pub fn last_workflow_run_id(&self) -> Option<&str> {
        self.last_workflow_run_id.as_deref()
    }
    pub fn pending_run(&self) -> bool {
        self.pending_run
    }
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }
    pub fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_next_run_at_ms(&mut self, value: u64) {
        self.next_run_at_ms = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_last_run_at_ms(&mut self, value: Option<u64>) {
        self.last_run_at_ms = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_last_status(&mut self, value: Option<String>) {
        self.last_status = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_last_error(&mut self, value: Option<String>) {
        self.last_error = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_last_workflow_run_id(&mut self, value: Option<String>) {
        self.last_workflow_run_id = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_pending_run(&mut self, value: bool) {
        self.pending_run = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_max_wakeups(&mut self, value: Option<u64>) {
        self.max_wakeups = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_wakeups_executed(&mut self, value: u64) {
        self.wakeups_executed = value;
        self.updated_at_ms = unix_epoch_ms();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeDefinition {
    id: String,
    agent_id: String,
    #[serde(default = "default_workflow_owner_user_id")]
    owner_user_id: String,
    #[serde(default = "default_workflow_node_public_label")]
    public_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(default = "default_workflow_node_can_complete_workflow_run")]
    can_complete_workflow_run: bool,
    #[serde(default = "default_workflow_node_can_emit_intermediate_run_output")]
    can_emit_intermediate_run_output: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intermediate_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_turns: Option<u32>,
}

impl WorkflowNodeDefinition {
    pub fn new(id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        let agent_id = agent_id.into();
        Self {
            id: id.into(),
            public_label: agent_id.clone(),
            agent_id,
            owner_user_id: default_workflow_owner_user_id(),
            instructions: None,
            can_complete_workflow_run: default_workflow_node_can_complete_workflow_run(),
            can_emit_intermediate_run_output:
                default_workflow_node_can_emit_intermediate_run_output(),
            intermediate_output_schema_ref: None,
            max_turns: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    pub fn public_label(&self) -> &str {
        &self.public_label
    }

    pub fn set_owner_user_id(&mut self, owner_user_id: impl Into<String>) {
        self.owner_user_id = owner_user_id.into();
    }

    pub fn set_public_label(&mut self, public_label: impl Into<String>) {
        self.public_label = public_label.into();
    }

    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    pub fn set_instructions(&mut self, instructions: Option<String>) {
        self.instructions = instructions;
    }

    pub fn can_complete_workflow_run(&self) -> bool {
        self.can_complete_workflow_run
    }

    pub fn set_can_complete_workflow_run(&mut self, value: bool) {
        self.can_complete_workflow_run = value;
    }

    pub fn can_emit_intermediate_run_output(&self) -> bool {
        self.can_emit_intermediate_run_output
    }

    pub fn set_can_emit_intermediate_run_output(&mut self, value: bool) {
        self.can_emit_intermediate_run_output = value;
    }

    pub fn intermediate_output_schema_ref(&self) -> Option<&str> {
        self.intermediate_output_schema_ref.as_deref()
    }

    pub fn set_intermediate_output_schema_ref(&mut self, value: Option<String>) {
        self.intermediate_output_schema_ref = value;
    }

    pub fn max_turns(&self) -> Option<u32> {
        self.max_turns
    }

    pub fn set_max_turns(&mut self, value: Option<u32>) {
        self.max_turns = value;
    }

    pub fn redacted_for_user(mut self, user_id: &str) -> Self {
        if self.owner_user_id != user_id {
            self.instructions = None;
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEdgeDefinition {
    id: String,
    from_node_id: String,
    to_node_id: String,
    #[serde(default = "default_workflow_owner_user_id")]
    created_by_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_policy: Option<WorkflowOutputValidationPolicy>,
}

impl WorkflowEdgeDefinition {
    pub fn new(
        id: impl Into<String>,
        from_node_id: impl Into<String>,
        to_node_id: impl Into<String>,
        output_schema_ref: Option<String>,
        validation_policy: Option<WorkflowOutputValidationPolicy>,
    ) -> Self {
        Self {
            id: id.into(),
            from_node_id: from_node_id.into(),
            to_node_id: to_node_id.into(),
            created_by_user_id: default_workflow_owner_user_id(),
            output_schema_ref,
            validation_policy,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn from_node_id(&self) -> &str {
        &self.from_node_id
    }

    pub fn to_node_id(&self) -> &str {
        &self.to_node_id
    }

    pub fn created_by_user_id(&self) -> &str {
        &self.created_by_user_id
    }

    pub fn set_created_by_user_id(&mut self, created_by_user_id: impl Into<String>) {
        self.created_by_user_id = created_by_user_id.into();
    }

    pub fn output_schema_ref(&self) -> Option<&str> {
        self.output_schema_ref.as_deref()
    }

    pub fn validation_policy(&self) -> Option<WorkflowOutputValidationPolicy> {
        self.validation_policy
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOutputValidationPolicy {
    Warn,
    Halt,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    id: String,
    alias: Option<String>,
    #[serde(default = "unix_epoch_ms")]
    created_at_ms: u64,
    #[serde(default)]
    revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canvas_layout: Option<WorkflowCanvasLayout>,
    #[serde(default = "default_workflow_flush_agent_context_before_run")]
    flush_agent_context_before_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intermediate_output_schema_ref: Option<String>,
    nodes: Vec<WorkflowNodeDefinition>,
    edges: Vec<WorkflowEdgeDefinition>,
    endpoints: Vec<WorkflowEndpointDefinition>,
}

impl WorkflowDefinition {
    pub fn new(id: impl Into<String>, alias: Option<String>) -> Self {
        Self {
            id: id.into(),
            alias,
            created_at_ms: unix_epoch_ms(),
            revision: 0,
            canvas_layout: None,
            flush_agent_context_before_run: default_workflow_flush_agent_context_before_run(),
            run_output_schema_ref: None,
            intermediate_output_schema_ref: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            endpoints: Vec::new(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn canvas_layout(&self) -> Option<&WorkflowCanvasLayout> {
        self.canvas_layout.as_ref()
    }

    pub fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn flush_agent_context_before_run(&self) -> bool {
        self.flush_agent_context_before_run
    }

    pub fn nodes(&self) -> &[WorkflowNodeDefinition] {
        &self.nodes
    }

    pub fn run_output_schema_ref(&self) -> Option<&str> {
        self.run_output_schema_ref.as_deref()
    }

    pub fn intermediate_output_schema_ref(&self) -> Option<&str> {
        self.intermediate_output_schema_ref.as_deref()
    }

    pub fn edges(&self) -> &[WorkflowEdgeDefinition] {
        &self.edges
    }

    pub fn endpoints(&self) -> &[WorkflowEndpointDefinition] {
        &self.endpoints
    }

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
        self.bump_revision();
    }

    pub fn set_flush_agent_context_before_run(&mut self, value: bool) {
        self.flush_agent_context_before_run = value;
        self.bump_revision();
    }

    pub fn set_run_output_schema_ref(&mut self, value: Option<String>) {
        self.run_output_schema_ref = value;
        self.bump_revision();
    }

    pub fn set_intermediate_output_schema_ref(&mut self, value: Option<String>) {
        self.intermediate_output_schema_ref = value;
        self.bump_revision();
    }

    pub fn add_node(&mut self, node: WorkflowNodeDefinition) -> WorkflowNodeDefinition {
        self.nodes.push(node.clone());
        self.bump_revision();
        node
    }

    pub fn node(&self, node_id: &str) -> Option<&WorkflowNodeDefinition> {
        self.nodes.iter().find(|node| node.id() == node_id)
    }

    pub fn node_mut(&mut self, node_id: &str) -> Option<&mut WorkflowNodeDefinition> {
        self.nodes.iter_mut().find(|node| node.id() == node_id)
    }

    pub fn remove_node(&mut self, node_id: &str) -> Option<WorkflowNodeDefinition> {
        let index = self.nodes.iter().position(|node| node.id() == node_id)?;
        let removed = self.nodes.remove(index);
        let removed_edge_ids = self
            .edges
            .iter()
            .filter(|edge| edge.from_node_id() == node_id || edge.to_node_id() == node_id)
            .map(|edge| edge.id().to_string())
            .collect::<Vec<_>>();
        let removed_endpoint_ids = self
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.entry_node_id() == node_id)
            .map(|endpoint| endpoint.id().to_string())
            .collect::<Vec<_>>();
        self.edges
            .retain(|edge| edge.from_node_id() != node_id && edge.to_node_id() != node_id);
        self.endpoints
            .retain(|endpoint| endpoint.entry_node_id() != node_id);
        if let Some(layout) = self.canvas_layout.as_mut() {
            layout.nodes.remove(node_id);
            for edge_id in removed_edge_ids {
                layout.edges.remove(&edge_id);
            }
            for endpoint_id in removed_endpoint_ids {
                layout.endpoints.remove(&endpoint_id);
            }
            layout.bump_revision();
        }
        self.bump_revision();
        Some(removed)
    }

    pub fn set_node_position(&mut self, node_id: impl Into<String>, point: WorkflowCanvasPoint) {
        let layout = self
            .canvas_layout
            .get_or_insert_with(WorkflowCanvasLayout::new);
        layout.nodes.insert(node_id.into(), point);
        layout.bump_revision();
        self.bump_revision();
    }

    pub fn add_edge(&mut self, edge: WorkflowEdgeDefinition) -> WorkflowEdgeDefinition {
        self.edges.push(edge.clone());
        self.bump_revision();
        edge
    }

    pub fn edge(&self, edge_id: &str) -> Option<&WorkflowEdgeDefinition> {
        self.edges.iter().find(|edge| edge.id() == edge_id)
    }

    pub fn has_edge(&self, from_node_id: &str, to_node_id: &str) -> bool {
        self.edges
            .iter()
            .any(|edge| edge.from_node_id() == from_node_id && edge.to_node_id() == to_node_id)
    }

    pub fn remove_edge(&mut self, edge_id: &str) -> Option<WorkflowEdgeDefinition> {
        let index = self.edges.iter().position(|edge| edge.id() == edge_id)?;
        let edge = self.edges.remove(index);
        if let Some(layout) = self.canvas_layout.as_mut() {
            layout.edges.remove(edge_id);
            layout.bump_revision();
        }
        self.bump_revision();
        Some(edge)
    }

    pub fn add_endpoint(
        &mut self,
        endpoint: WorkflowEndpointDefinition,
    ) -> WorkflowEndpointDefinition {
        self.endpoints.push(endpoint.clone());
        self.bump_revision();
        endpoint
    }

    pub fn endpoint(&self, endpoint_id: &str) -> Option<&WorkflowEndpointDefinition> {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.id() == endpoint_id)
    }

    pub fn endpoint_mut(&mut self, endpoint_id: &str) -> Option<&mut WorkflowEndpointDefinition> {
        self.endpoints
            .iter_mut()
            .find(|endpoint| endpoint.id() == endpoint_id)
    }

    pub fn remove_endpoint(&mut self, endpoint_id: &str) -> Option<WorkflowEndpointDefinition> {
        let index = self
            .endpoints
            .iter()
            .position(|endpoint| endpoint.id() == endpoint_id)?;
        let endpoint = self.endpoints.remove(index);
        if let Some(layout) = self.canvas_layout.as_mut() {
            layout.endpoints.remove(endpoint_id);
            layout.bump_revision();
        }
        self.bump_revision();
        Some(endpoint)
    }

    pub fn set_endpoint_position(
        &mut self,
        endpoint_id: impl Into<String>,
        point: WorkflowCanvasPoint,
    ) {
        let layout = self
            .canvas_layout
            .get_or_insert_with(WorkflowCanvasLayout::new);
        layout.endpoints.insert(endpoint_id.into(), point);
        layout.bump_revision();
        self.bump_revision();
    }

    pub fn update_canvas_layout(
        &mut self,
        patches: Vec<WorkflowCanvasLayoutPatch>,
    ) -> WorkflowCanvasLayout {
        let node_ids = self
            .nodes
            .iter()
            .map(|node| node.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        let edge_ids = self
            .edges
            .iter()
            .map(|edge| edge.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        let endpoint_ids = self
            .endpoints
            .iter()
            .map(|endpoint| endpoint.id().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        let layout = self
            .canvas_layout
            .get_or_insert_with(WorkflowCanvasLayout::new);
        let mut changed = false;
        for patch in patches {
            match patch {
                WorkflowCanvasLayoutPatch::NodePosition { node_id, x, y } => {
                    if node_ids.contains(&node_id) {
                        changed |= layout
                            .nodes
                            .insert(node_id, WorkflowCanvasPoint { x, y })
                            .as_ref()
                            .is_none_or(|existing| existing.x != x || existing.y != y);
                    }
                }
                WorkflowCanvasLayoutPatch::EndpointPosition { endpoint_id, x, y } => {
                    if endpoint_ids.contains(&endpoint_id) {
                        changed |= layout
                            .endpoints
                            .insert(endpoint_id, WorkflowCanvasPoint { x, y })
                            .as_ref()
                            .is_none_or(|existing| existing.x != x || existing.y != y);
                    }
                }
                WorkflowCanvasLayoutPatch::EdgeWaypoints { edge_id, waypoints } => {
                    if edge_ids.contains(&edge_id) {
                        let next = WorkflowCanvasEdgeLayout { waypoints };
                        let previous = layout.edges.insert(edge_id, next.clone());
                        changed |= previous.as_ref() != Some(&next);
                    }
                }
            }
        }
        layout.nodes.retain(|node_id, _| node_ids.contains(node_id));
        layout.edges.retain(|edge_id, _| edge_ids.contains(edge_id));
        layout
            .endpoints
            .retain(|endpoint_id, _| endpoint_ids.contains(endpoint_id));
        if changed {
            layout.bump_revision();
        }
        layout.clone()
    }

    pub fn redacted_for_user(mut self, user_id: &str) -> Self {
        self.nodes = self
            .nodes
            .into_iter()
            .map(|node| node.redacted_for_user(user_id))
            .collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCanvasPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCanvasEdgeLayout {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waypoints: Vec<WorkflowCanvasPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCanvasLayout {
    pub version: u32,
    pub revision: u64,
    pub coordinate_space: String,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub nodes: std::collections::BTreeMap<String, WorkflowCanvasPoint>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub endpoints: std::collections::BTreeMap<String, WorkflowCanvasPoint>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub edges: std::collections::BTreeMap<String, WorkflowCanvasEdgeLayout>,
}

impl WorkflowCanvasLayout {
    pub fn new() -> Self {
        Self {
            version: 1,
            revision: 0,
            coordinate_space: "workflow-canvas-v1".to_string(),
            nodes: std::collections::BTreeMap::new(),
            endpoints: std::collections::BTreeMap::new(),
            edges: std::collections::BTreeMap::new(),
        }
    }

    pub fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

impl Default for WorkflowCanvasLayout {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowCanvasLayoutPatch {
    NodePosition {
        node_id: String,
        x: i32,
        y: i32,
    },
    EndpointPosition {
        endpoint_id: String,
        x: i32,
        y: i32,
    },
    EdgeWaypoints {
        edge_id: String,
        #[serde(default)]
        waypoints: Vec<WorkflowCanvasPoint>,
    },
}

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
    #[serde(default, skip_serializing_if = "crate::session::is_false")]
    intermediate_released_downstream: bool,
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
            intermediate_released_downstream: false,
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

    pub fn intermediate_released_downstream(&self) -> bool {
        self.intermediate_released_downstream
    }

    pub fn mark_intermediate_released_downstream(&mut self) {
        self.intermediate_released_downstream = true;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    id: String,
    workflow_id: String,
    endpoint_id: String,
    entry_node_id: String,
    status: WorkflowRunStatus,
    invocation_prompt: Option<String>,
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
        node_runs: Vec<WorkflowNodeRun>,
        messages: Vec<WorkflowMessage>,
    ) -> Self {
        let active_node_run_id = node_runs.first().map(|run| run.id().to_string());
        Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            entry_node_id: entry_node_id.into(),
            status: WorkflowRunStatus::Created,
            invocation_prompt,
            active_node_run_id,
            node_runs,
            messages,
            failure_events: Vec::new(),
            intermediate_outputs: Vec::new(),
            final_output: None,
            final_output_valid: None,
            final_output_warning: None,
            completed_by_node_run_id: None,
            created_at_ms: unix_epoch_ms(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAgentDefaults {
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
}

impl Default for SessionAgentDefaults {
    fn default() -> Self {
        Self {
            provider: "default".to_string(),
            model: None,
            effort: None,
            account_profile: None,
            execution_mode: None,
            permission_level: None,
        }
    }
}

impl SessionAgentDefaults {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            ..Self::default()
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = Some(effort.into());
        self
    }

    pub fn with_account_profile(mut self, account_profile: impl Into<String>) -> Self {
        self.account_profile = Some(account_profile.into());
        self
    }

    pub fn with_execution_mode(
        mut self,
        execution_mode: crate::provider::AgentExecutionMode,
    ) -> Self {
        self.execution_mode = Some(execution_mode);
        self
    }

    pub fn with_permission_level(
        mut self,
        permission_level: crate::provider::AgentPermissionLevel,
    ) -> Self {
        self.permission_level = Some(permission_level);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_defaults: Option<SessionAgentDefaults>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_ref: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default = "default_session_owner_user_id")]
    pub owner_user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMember {
    user_id: String,
    joined_at_ms: u64,
    invited_by_user_id: Option<String>,
}

impl SessionMember {
    pub fn new(
        user_id: impl Into<String>,
        joined_at_ms: u64,
        invited_by_user_id: Option<String>,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            joined_at_ms,
            invited_by_user_id,
        }
    }

    pub fn local() -> Self {
        Self::new(DEFAULT_LOCAL_USER_ID, 0, None)
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn joined_at_ms(&self) -> u64 {
        self.joined_at_ms
    }

    pub fn invited_by_user_id(&self) -> Option<&str> {
        self.invited_by_user_id.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInvite {
    invite_id: String,
    session_id: String,
    created_by_user_id: String,
    created_at_ms: u64,
    expires_at_ms: Option<u64>,
    #[serde(default = "default_session_invite_max_uses")]
    max_uses: Option<u32>,
    #[serde(default)]
    used_count: u32,
    revoked_at_ms: Option<u64>,
}

impl SessionInvite {
    pub fn new(
        invite_id: impl Into<String>,
        session_id: impl Into<String>,
        created_by_user_id: impl Into<String>,
        created_at_ms: u64,
        expires_at_ms: Option<u64>,
        max_uses: Option<u32>,
    ) -> Self {
        Self {
            invite_id: invite_id.into(),
            session_id: session_id.into(),
            created_by_user_id: created_by_user_id.into(),
            created_at_ms,
            expires_at_ms,
            max_uses,
            used_count: 0,
            revoked_at_ms: None,
        }
    }

    pub fn invite_id(&self) -> &str {
        &self.invite_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn created_by_user_id(&self) -> &str {
        &self.created_by_user_id
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn expires_at_ms(&self) -> Option<u64> {
        self.expires_at_ms
    }

    pub fn max_uses(&self) -> Option<u32> {
        self.max_uses
    }

    pub fn used_count(&self) -> u32 {
        self.used_count
    }

    pub fn revoked_at_ms(&self) -> Option<u64> {
        self.revoked_at_ms
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at_ms.is_some()
    }

    pub fn is_expired(&self, now_ms: u64) -> bool {
        self.expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
    }

    pub fn is_exhausted(&self) -> bool {
        self.max_uses
            .is_some_and(|max_uses| self.used_count >= max_uses)
    }

    pub fn mark_used(&mut self) {
        self.used_count = self.used_count.saturating_add(1);
    }

    pub fn revoke(&mut self, revoked_at_ms: u64) {
        self.revoked_at_ms = Some(revoked_at_ms);
    }
}

impl CreateSessionRequest {
    pub fn new(workspace_id: impl Into<String>, worktree_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            worktree_id: worktree_id.into(),
            alias: None,
            agent_defaults: None,
            slice_ref: None,
            hidden: false,
            owner_user_id: default_session_owner_user_id(),
        }
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    pub fn with_hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    pub fn with_agent_defaults(mut self, agent_defaults: SessionAgentDefaults) -> Self {
        self.agent_defaults = Some(agent_defaults);
        self
    }

    pub fn with_slice_ref(mut self, slice_ref: impl Into<String>) -> Self {
        self.slice_ref = Some(slice_ref.into());
        self
    }

    pub fn with_owner_user_id(mut self, owner_user_id: impl Into<String>) -> Self {
        self.owner_user_id = owner_user_id.into();
        self
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Created,
    Active,
    Parked,
    Ended,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionExecutionMode {
    SingleAgent,
    MultiAgentWorkflow,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Cancelled,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedulerState {
    Idle,
    Runnable,
    Running,
    Waiting,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KernelRestartReconciliation {
    pub cleared_active_provider_run: bool,
    pub interrupted_prompt_count: usize,
    pub stopped_workflow_run_count: usize,
}

impl KernelRestartReconciliation {
    pub fn changed(&self) -> bool {
        self.cleared_active_provider_run
            || self.interrupted_prompt_count > 0
            || self.stopped_workflow_run_count > 0
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeIsolationMode {
    SharedSession,
    IsolatedBranch,
    IsolatedWorktree,
}

impl fmt::Display for SessionExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::SingleAgent => "single_agent",
            Self::MultiAgentWorkflow => "multi_agent_workflow",
        };

        write!(f, "{value}")
    }
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Created => "created",
            Self::Active => "active",
            Self::Parked => "parked",
            Self::Ended => "ended",
        };

        write!(f, "{value}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptAttachment {
    url: String,
    mime: String,
    filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contents_base64: Option<String>,
}

impl PromptAttachment {
    pub fn new(url: impl Into<String>, mime: impl Into<String>, filename: Option<String>) -> Self {
        Self {
            url: url.into(),
            mime: mime.into(),
            filename,
            contents_base64: None,
        }
    }

    pub fn with_contents_base64(mut self, contents_base64: impl Into<String>) -> Self {
        self.contents_base64 = Some(contents_base64.into());
        self
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn mime(&self) -> &str {
        &self.mime
    }

    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    pub fn contents_base64(&self) -> Option<&str> {
        self.contents_base64.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptQueueItem {
    id: String,
    source_attachment_id: String,
    target_agent_id: String,
    prompt: String,
    attachments: Vec<PromptAttachment>,
    status: PromptStatus,
    workflow_run_id: Option<String>,
    workflow_node_run_id: Option<String>,
}

impl PromptQueueItem {
    pub fn new(
        id: impl Into<String>,
        source_attachment_id: impl Into<String>,
        target_agent_id: impl Into<String>,
        prompt: impl Into<String>,
        status: PromptStatus,
    ) -> Self {
        Self {
            id: id.into(),
            source_attachment_id: source_attachment_id.into(),
            target_agent_id: target_agent_id.into(),
            prompt: prompt.into(),
            attachments: Vec::new(),
            status,
            workflow_run_id: None,
            workflow_node_run_id: None,
        }
    }

    pub fn with_attachments(mut self, attachments: Vec<PromptAttachment>) -> Self {
        self.attachments = attachments;
        self
    }

    pub fn with_workflow_context(
        mut self,
        workflow_run_id: impl Into<String>,
        workflow_node_run_id: impl Into<String>,
    ) -> Self {
        self.workflow_run_id = Some(workflow_run_id.into());
        self.workflow_node_run_id = Some(workflow_node_run_id.into());
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn source_attachment_id(&self) -> &str {
        &self.source_attachment_id
    }

    pub fn target_agent_id(&self) -> &str {
        &self.target_agent_id
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn attachments(&self) -> &[PromptAttachment] {
        &self.attachments
    }

    pub fn status(&self) -> PromptStatus {
        self.status
    }

    pub fn workflow_run_id(&self) -> Option<&str> {
        self.workflow_run_id.as_deref()
    }

    pub fn workflow_node_run_id(&self) -> Option<&str> {
        self.workflow_node_run_id.as_deref()
    }

    pub fn set_status(&mut self, status: PromptStatus) {
        self.status = status;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionConfigState {
    version: u64,
    values: BTreeMap<String, String>,
    updated_by_attachment_id: Option<String>,
}

impl SessionConfigState {
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn values(&self) -> &BTreeMap<String, String> {
        &self.values
    }

    pub fn updated_by_attachment_id(&self) -> Option<&str> {
        self.updated_by_attachment_id.as_deref()
    }

    pub fn apply_changes(
        &mut self,
        values: BTreeMap<String, String>,
        updated_by_attachment_id: impl Into<String>,
    ) {
        for (key, value) in values {
            self.values.insert(key, value);
        }
        self.version += 1;
        self.updated_by_attachment_id = Some(updated_by_attachment_id.into());
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionKind {
    Choice,
    Permission,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionLevel {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInteractionChoiceStyle {
    Primary,
    Secondary,
    Danger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractionChoice {
    id: String,
    label: String,
    reply: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    style: Option<RuntimeInteractionChoiceStyle>,
}

impl RuntimeInteractionChoice {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        reply: impl Into<String>,
        style: Option<RuntimeInteractionChoiceStyle>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            reply: reply.into(),
            style,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn reply(&self) -> &str {
        &self.reply
    }

    pub fn style(&self) -> Option<RuntimeInteractionChoiceStyle> {
        self.style
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteractionCustomChoice {
    id: String,
    label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_length: Option<usize>,
}

impl RuntimeInteractionCustomChoice {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        placeholder: Option<String>,
        min_length: Option<usize>,
        max_length: Option<usize>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            placeholder,
            min_length,
            max_length,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn placeholder(&self) -> Option<&str> {
        self.placeholder.as_deref()
    }

    pub fn min_length(&self) -> usize {
        self.min_length.unwrap_or(1)
    }

    pub fn max_length(&self) -> Option<usize> {
        self.max_length
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInteraction {
    id: String,
    agent_id: String,
    kind: RuntimeInteractionKind,
    level: RuntimeInteractionLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    message: String,
    choices: Vec<RuntimeInteractionChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_choice: Option<RuntimeInteractionCustomChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_on_timeout: Option<String>,
    requested_at_ms: u64,
}

impl RuntimeInteraction {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        agent_id: impl Into<String>,
        kind: RuntimeInteractionKind,
        level: RuntimeInteractionLevel,
        title: Option<String>,
        message: impl Into<String>,
        choices: Vec<RuntimeInteractionChoice>,
        custom_choice: Option<RuntimeInteractionCustomChoice>,
        timeout_sec: Option<u64>,
        default_on_timeout: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            agent_id: agent_id.into(),
            kind,
            level,
            title,
            message: message.into(),
            choices,
            custom_choice,
            timeout_sec,
            default_on_timeout,
            requested_at_ms: unix_epoch_ms(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn kind(&self) -> RuntimeInteractionKind {
        self.kind
    }

    pub fn level(&self) -> RuntimeInteractionLevel {
        self.level
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn choices(&self) -> &[RuntimeInteractionChoice] {
        &self.choices
    }

    pub fn custom_choice(&self) -> Option<&RuntimeInteractionCustomChoice> {
        self.custom_choice.as_ref()
    }

    pub fn timeout_sec(&self) -> Option<u64> {
        self.timeout_sec
    }

    pub fn default_on_timeout(&self) -> Option<&str> {
        self.default_on_timeout.as_deref()
    }

    pub fn requested_at_ms(&self) -> u64 {
        self.requested_at_ms
    }

    pub fn choice(&self, choice_id: &str) -> Option<&RuntimeInteractionChoice> {
        self.choices.iter().find(|choice| choice.id() == choice_id)
    }

    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = agent_id.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSubmissionOutcome {
    Started { prompt: PromptQueueItem },
    Queued { prompt: PromptQueueItem },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AgentPromptState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::session) active_prompt: Option<PromptQueueItem>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub(in crate::session) queued_prompts: VecDeque<PromptQueueItem>,
}

impl AgentPromptState {
    pub(in crate::session) fn from_parts(
        active_prompt: Option<PromptQueueItem>,
        queued_prompts: VecDeque<PromptQueueItem>,
    ) -> Self {
        Self {
            active_prompt,
            queued_prompts,
        }
    }

    pub fn active_prompt(&self) -> Option<&PromptQueueItem> {
        self.active_prompt.as_ref()
    }

    pub fn queued_prompts(&self) -> &VecDeque<PromptQueueItem> {
        &self.queued_prompts
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCompletion {
    pub completed: PromptQueueItem,
    pub started_next: Option<PromptQueueItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCancellation {
    pub prompt: PromptQueueItem,
    pub started_next: Option<PromptQueueItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PromptDetachEffect {
    pub removed_active_prompt: bool,
    pub removed_queued_prompt_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeWorktreeAssignment {
    id: String,
    worktree_id: String,
    branch: String,
    isolation_mode: WorktreeIsolationMode,
}

impl RuntimeWorktreeAssignment {
    pub fn new(
        id: impl Into<String>,
        worktree_id: impl Into<String>,
        branch: impl Into<String>,
        isolation_mode: WorktreeIsolationMode,
    ) -> Self {
        Self {
            id: id.into(),
            worktree_id: worktree_id.into(),
            branch: branch.into(),
            isolation_mode,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn worktree_id(&self) -> &str {
        &self.worktree_id
    }
    pub fn branch(&self) -> &str {
        &self.branch
    }
    pub fn isolation_mode(&self) -> WorktreeIsolationMode {
        self.isolation_mode
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSession {
    id: String,
    alias: Option<String>,
    workspace_id: String,
    worktree_id: String,
    #[serde(default = "default_session_owner_user_id")]
    owner_user_id: String,
    #[serde(default = "default_session_members")]
    members: Vec<SessionMember>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    invites: Vec<SessionInvite>,
    host_machine_id: String,
    host_daemon_id: String,
    created_at_ms: u64,
    last_used_at_ms: Option<u64>,
    execution_mode: SessionExecutionMode,
    status: SessionStatus,
    #[serde(default, skip_serializing_if = "crate::session::is_false")]
    hidden: bool,
    #[serde(default)]
    agent_defaults: SessionAgentDefaults,
    active_provider_run_id: Option<String>,
    focused_agent_id: Option<String>,
    max_agents: i32,
    agents: Vec<AgentInstance>,
    attachment_ids: BTreeSet<String>,
    #[serde(flatten)]
    prompt_runtime: PromptRuntimeState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    active_interactions: Vec<RuntimeInteraction>,
    config_state: SessionConfigState,
    worktree_assignments: Vec<RuntimeWorktreeAssignment>,
    workflows: Vec<WorkflowDefinition>,
    workflow_runs: Vec<WorkflowRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workflow_launch_policy: Option<WorkflowLaunchPolicy>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    queued_workflow_launches: VecDeque<QueuedWorkflowLaunch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workflow_watchdogs: Vec<WorkflowWatchdogDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workflow_consoles: Vec<WorkflowConsole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workflow_publications: Vec<WorkflowPublicationDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workspace_links: Vec<WorkspaceLinkDefinition>,
}

impl RuntimeSession {
    pub fn new(
        id: impl Into<String>,
        alias: Option<String>,
        workspace_id: impl Into<String>,
        worktree_id: impl Into<String>,
        host_machine_id: impl Into<String>,
        host_daemon_id: impl Into<String>,
    ) -> Self {
        let id = id.into();
        let worktree_id = worktree_id.into();
        let now = unix_epoch_ms();

        Self {
            id: id.clone(),
            alias,
            workspace_id: workspace_id.into(),
            worktree_id: worktree_id.clone(),
            owner_user_id: default_session_owner_user_id(),
            members: default_session_members(),
            invites: Vec::new(),
            host_machine_id: host_machine_id.into(),
            host_daemon_id: host_daemon_id.into(),
            created_at_ms: now,
            last_used_at_ms: Some(now),
            execution_mode: SessionExecutionMode::SingleAgent,
            status: SessionStatus::Created,
            hidden: false,
            agent_defaults: SessionAgentDefaults::default(),
            active_provider_run_id: None,
            focused_agent_id: None,
            max_agents: DEFAULT_SESSION_MAX_AGENTS,
            agents: Vec::new(),
            attachment_ids: BTreeSet::new(),
            prompt_runtime: PromptRuntimeState::default(),
            active_interactions: Vec::new(),
            config_state: SessionConfigState::default(),
            worktree_assignments: vec![RuntimeWorktreeAssignment::new(
                format!("worktree-assignment-{}-1", id),
                worktree_id,
                format!("session/{id}"),
                WorktreeIsolationMode::SharedSession,
            )],
            workflows: Vec::new(),
            workflow_runs: Vec::new(),
            workflow_launch_policy: None,
            queued_workflow_launches: VecDeque::new(),
            workflow_watchdogs: Vec::new(),
            workflow_consoles: Vec::new(),
            workflow_publications: Vec::new(),
            workspace_links: Vec::new(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }
    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }
    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
    }
    pub fn worktree_id(&self) -> &str {
        &self.worktree_id
    }
    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }
    pub fn set_owner_user_id(&mut self, owner_user_id: impl Into<String>) {
        let owner_user_id = owner_user_id.into();
        self.owner_user_id = owner_user_id.clone();
        self.members = vec![SessionMember::new(owner_user_id, 0, None)];
    }
    pub fn members(&self) -> &[SessionMember] {
        &self.members
    }
    pub fn invites(&self) -> &[SessionInvite] {
        &self.invites
    }
    pub fn has_member(&self, user_id: &str) -> bool {
        self.members
            .iter()
            .any(|member| member.user_id() == user_id)
    }
    pub fn add_member(
        &mut self,
        user_id: impl Into<String>,
        invited_by_user_id: Option<String>,
    ) -> SessionMember {
        let user_id = user_id.into();
        if let Some(member) = self
            .members
            .iter()
            .find(|member| member.user_id() == user_id)
            .cloned()
        {
            return member;
        }
        let member = SessionMember::new(user_id, unix_epoch_ms(), invited_by_user_id);
        self.members.push(member.clone());
        member
    }
    pub fn add_invite(&mut self, invite: SessionInvite) -> SessionInvite {
        self.invites.push(invite.clone());
        invite
    }
    pub fn invite_mut(&mut self, invite_id: &str) -> Option<&mut SessionInvite> {
        self.invites
            .iter_mut()
            .find(|invite| invite.invite_id() == invite_id)
    }
    pub fn invite(&self, invite_id: &str) -> Option<&SessionInvite> {
        self.invites
            .iter()
            .find(|invite| invite.invite_id() == invite_id)
    }
    pub fn host_machine_id(&self) -> &str {
        &self.host_machine_id
    }
    pub fn host_daemon_id(&self) -> &str {
        &self.host_daemon_id
    }
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }
    pub fn last_used_at_ms(&self) -> Option<u64> {
        self.last_used_at_ms
    }
    pub fn status(&self) -> SessionStatus {
        self.status
    }
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }
    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }
    pub fn agent_defaults(&self) -> &SessionAgentDefaults {
        &self.agent_defaults
    }
    pub fn set_agent_defaults(&mut self, agent_defaults: SessionAgentDefaults) {
        self.agent_defaults = agent_defaults;
    }
    pub fn execution_mode(&self) -> SessionExecutionMode {
        self.execution_mode
    }
    pub fn active_provider_run_id(&self) -> Option<&str> {
        self.active_provider_run_id.as_deref()
    }
    pub fn focused_agent_id(&self) -> Option<&str> {
        self.focused_agent_id.as_deref()
    }
    pub fn max_agents(&self) -> i32 {
        self.max_agents
    }
    pub fn agents(&self) -> &[AgentInstance] {
        &self.agents
    }
    pub fn set_agents(&mut self, agents: Vec<AgentInstance>) {
        self.agents = agents;
    }

    pub fn redacted_for_user(mut self, user_id: &str) -> Self {
        let has_unowned_agents = self
            .agents
            .iter()
            .any(|agent| agent.owner_user_id() != user_id);
        let owned_agent_ids = self
            .agents
            .iter()
            .filter(|agent| agent.owner_user_id() == user_id)
            .map(|agent| agent.id().to_string())
            .collect::<BTreeSet<_>>();
        self.agents.retain(|agent| agent.owner_user_id() == user_id);
        if self
            .focused_agent_id
            .as_ref()
            .is_some_and(|agent_id| !owned_agent_ids.contains(agent_id))
        {
            self.focused_agent_id = None;
        }
        if has_unowned_agents {
            self.active_provider_run_id = None;
        }
        self.prompt_runtime
            .retain_agent_ids(&owned_agent_ids, self.focused_agent_id.as_deref());
        self.workflows = self
            .workflows
            .into_iter()
            .map(|workflow| workflow.redacted_for_user(user_id))
            .collect();
        self.workflow_publications
            .retain(|publication| publication.created_by_user_id() == user_id);
        self
    }
    pub fn attachment_ids(&self) -> &BTreeSet<String> {
        &self.attachment_ids
    }
    pub fn prompt_states(&self) -> &BTreeMap<String, AgentPromptState> {
        self.prompt_runtime.prompt_states()
    }
    pub fn active_prompt(&self) -> Option<&PromptQueueItem> {
        self.prompt_runtime.active_prompt()
    }
    pub fn has_active_prompt(&self) -> bool {
        self.prompt_runtime.active_prompt().is_some()
    }
    pub fn queued_prompts(&self) -> &VecDeque<PromptQueueItem> {
        self.prompt_runtime.queued_prompts()
    }
    pub fn active_prompt_for_agent(&self, agent_id: &str) -> Option<&PromptQueueItem> {
        self.prompt_runtime.active_prompt_for_agent(agent_id)
    }
    pub fn active_prompt_agent_id(&self) -> Option<String> {
        if let Some(focused_agent_id) = self.focused_agent_id() {
            if self.active_prompt_for_agent(focused_agent_id).is_some() {
                return Some(focused_agent_id.to_string());
            }
        }
        let mut active_agents = self
            .prompt_states()
            .iter()
            .filter(|(_, state)| state.active_prompt().is_some())
            .map(|(agent_id, _)| agent_id.clone());
        let agent_id = active_agents.next()?;
        if active_agents.next().is_none() {
            Some(agent_id)
        } else {
            None
        }
    }
    pub fn queued_prompts_for_agent(&self, agent_id: &str) -> Option<&VecDeque<PromptQueueItem>> {
        self.prompt_runtime.queued_prompts_for_agent(agent_id)
    }
    pub(in crate::session) fn mirror_agent_prompt_state(
        &mut self,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
        queued_prompts: VecDeque<PromptQueueItem>,
    ) {
        self.prompt_runtime.mirror_agent_prompt_state(
            agent_id,
            active_prompt,
            queued_prompts,
            self.focused_agent_id.as_deref(),
        );
    }
    pub fn has_any_active_prompt(&self) -> bool {
        self.prompt_runtime.has_any_active_prompt()
    }
    pub fn has_any_prompt_work(&self) -> bool {
        self.prompt_runtime.has_any_prompt_work()
    }
    pub fn scheduler_state(&self) -> SchedulerState {
        self.prompt_runtime.scheduler_state()
    }
    pub fn active_interactions(&self) -> &[RuntimeInteraction] {
        &self.active_interactions
    }
    pub fn active_interaction_for_agent(&self, agent_id: &str) -> Option<&RuntimeInteraction> {
        self.active_interactions
            .iter()
            .find(|interaction| interaction.agent_id() == agent_id)
    }
    pub fn add_active_interaction(&mut self, interaction: RuntimeInteraction) {
        self.active_interactions
            .retain(|existing| existing.agent_id() != interaction.agent_id());
        self.active_interactions.push(interaction);
        self.active_interactions
            .sort_by(|left, right| left.requested_at_ms().cmp(&right.requested_at_ms()));
    }
    pub fn remove_active_interaction(
        &mut self,
        interaction_id: &str,
    ) -> Option<RuntimeInteraction> {
        let index = self
            .active_interactions
            .iter()
            .position(|interaction| interaction.id() == interaction_id)?;
        Some(self.active_interactions.remove(index))
    }
    pub fn config_state(&self) -> &SessionConfigState {
        &self.config_state
    }
    pub fn worktree_assignments(&self) -> &[RuntimeWorktreeAssignment] {
        &self.worktree_assignments
    }
    pub fn workflows(&self) -> &[WorkflowDefinition] {
        &self.workflows
    }
    pub fn workflow_runs(&self) -> &[WorkflowRun] {
        &self.workflow_runs
    }

    pub fn workflow_launch_policy(&self) -> WorkflowLaunchPolicy {
        self.workflow_launch_policy
            .unwrap_or(DEFAULT_WORKFLOW_LAUNCH_POLICY)
    }

    pub fn queued_workflow_launches(&self) -> &VecDeque<QueuedWorkflowLaunch> {
        &self.queued_workflow_launches
    }

    pub fn workflow_watchdogs(&self) -> &[WorkflowWatchdogDefinition] {
        &self.workflow_watchdogs
    }

    pub fn workflow_watchdogs_mut(&mut self) -> &mut [WorkflowWatchdogDefinition] {
        &mut self.workflow_watchdogs
    }

    pub fn workflow_consoles(&self) -> &[WorkflowConsole] {
        &self.workflow_consoles
    }

    pub fn workflow_publications(&self) -> &[WorkflowPublicationDefinition] {
        &self.workflow_publications
    }

    pub fn workspace_links(&self) -> &[WorkspaceLinkDefinition] {
        &self.workspace_links
    }

    pub fn create_workspace_link(
        &mut self,
        link: WorkspaceLinkDefinition,
    ) -> WorkspaceLinkDefinition {
        self.workspace_links.push(link.clone());
        link
    }

    pub fn workspace_link(&self, link_id: &str) -> Option<&WorkspaceLinkDefinition> {
        self.workspace_links
            .iter()
            .find(|link| link.link_id() == link_id)
    }

    pub fn workspace_link_mut(&mut self, link_id: &str) -> Option<&mut WorkspaceLinkDefinition> {
        self.workspace_links
            .iter_mut()
            .find(|link| link.link_id() == link_id)
    }

    pub fn workspace_link_for_repo_root(
        &self,
        repo_root: &Path,
    ) -> Option<&WorkspaceLinkDefinition> {
        self.workspace_links
            .iter()
            .find(|link| link.attachment_for_repo_root(repo_root).is_some())
    }

    pub fn has_attachment(&self, attachment_id: &str) -> bool {
        self.attachment_ids.contains(attachment_id)
    }

    pub fn add_attachment(&mut self, attachment_id: impl Into<String>) {
        self.attachment_ids.insert(attachment_id.into());
    }

    pub fn remove_attachment(&mut self, attachment_id: &str) -> bool {
        self.attachment_ids.remove(attachment_id)
    }

    pub fn set_active_provider_run(&mut self, provider_run_id: Option<String>) {
        self.active_provider_run_id = provider_run_id;
    }

    pub fn set_focused_agent(&mut self, agent_id: Option<String>) {
        self.focused_agent_id = agent_id;
        self.prompt_runtime
            .refresh_after_focus_change(self.focused_agent_id.as_deref());
    }

    pub fn touch(&mut self) {
        self.last_used_at_ms = Some(unix_epoch_ms());
    }

    pub fn create_workflow(&mut self, workflow: WorkflowDefinition) -> WorkflowDefinition {
        self.workflows.push(workflow.clone());
        workflow
    }

    pub fn remove_workflow(&mut self, workflow_id: &str) -> Option<WorkflowDefinition> {
        let index = self
            .workflows
            .iter()
            .position(|workflow| workflow.id() == workflow_id)?;
        Some(self.workflows.remove(index))
    }

    pub fn workflow(&self, workflow_id: &str) -> Option<&WorkflowDefinition> {
        self.workflows
            .iter()
            .find(|workflow| workflow.id() == workflow_id)
    }

    pub fn workflow_mut(&mut self, workflow_id: &str) -> Option<&mut WorkflowDefinition> {
        self.workflows
            .iter_mut()
            .find(|workflow| workflow.id() == workflow_id)
    }

    pub fn create_workflow_publication(
        &mut self,
        publication: WorkflowPublicationDefinition,
    ) -> WorkflowPublicationDefinition {
        self.workflow_publications.push(publication.clone());
        publication
    }

    pub fn workflow_publication(
        &self,
        publication_id: &str,
    ) -> Option<&WorkflowPublicationDefinition> {
        self.workflow_publications
            .iter()
            .find(|publication| publication.id() == publication_id)
    }

    pub fn workflow_publication_mut(
        &mut self,
        publication_id: &str,
    ) -> Option<&mut WorkflowPublicationDefinition> {
        self.workflow_publications
            .iter_mut()
            .find(|publication| publication.id() == publication_id)
    }

    pub fn create_workflow_run(&mut self, workflow_run: WorkflowRun) -> WorkflowRun {
        self.workflow_runs.push(workflow_run.clone());
        workflow_run
    }

    pub fn has_active_workflow_run(&self) -> bool {
        self.workflow_runs.iter().any(|workflow_run| {
            matches!(
                workflow_run.status(),
                WorkflowRunStatus::Created
                    | WorkflowRunStatus::Running
                    | WorkflowRunStatus::Waiting
            )
        })
    }

    pub fn reconcile_after_kernel_restart(&mut self) -> KernelRestartReconciliation {
        let mut reconciliation = KernelRestartReconciliation::default();
        if self.active_provider_run_id.take().is_some() {
            reconciliation.cleared_active_provider_run = true;
        }

        reconciliation.interrupted_prompt_count = self
            .prompt_runtime
            .interrupt_active_prompts(self.focused_agent_id.as_deref())
            .len();

        for workflow_run in &mut self.workflow_runs {
            let should_stop = matches!(
                workflow_run.status(),
                WorkflowRunStatus::Running | WorkflowRunStatus::Completing
            ) || workflow_run
                .active_node_run_id()
                .and_then(|active_node_run_id| {
                    workflow_run
                        .node_runs()
                        .iter()
                        .find(|node_run| node_run.id() == active_node_run_id)
                })
                .is_some_and(|node_run| {
                    matches!(
                        node_run.status(),
                        WorkflowNodeRunStatus::Running | WorkflowNodeRunStatus::Waiting
                    )
                });
            if !should_stop {
                continue;
            }

            let source_node_run_id = workflow_run
                .active_node_run_id()
                .map(str::to_string)
                .or_else(|| {
                    workflow_run
                        .node_runs()
                        .iter()
                        .find(|node_run| {
                            !matches!(
                                node_run.status(),
                                WorkflowNodeRunStatus::Completed
                                    | WorkflowNodeRunStatus::Failed
                                    | WorkflowNodeRunStatus::Stopped
                            )
                        })
                        .map(|node_run| node_run.id().to_string())
                })
                .unwrap_or_else(|| workflow_run.id().to_string());

            for node_run in workflow_run.node_runs_mut() {
                if !matches!(
                    node_run.status(),
                    WorkflowNodeRunStatus::Completed
                        | WorkflowNodeRunStatus::Failed
                        | WorkflowNodeRunStatus::Stopped
                ) {
                    node_run.set_status(WorkflowNodeRunStatus::Stopped);
                    if let Some(envelope) = node_run.turn_envelope_mut() {
                        envelope.mark_cancelled();
                    }
                }
            }
            workflow_run.clear_active_node_run();
            workflow_run.add_failure_event(WorkflowFailureEvent::new(
                WorkflowFailureKind::RunStopped,
                source_node_run_id,
                Vec::new(),
                "workflow run was interrupted by kernel restart; relaunch or resume it explicitly",
            ));
            workflow_run.set_status(WorkflowRunStatus::Stopped);
            reconciliation.stopped_workflow_run_count += 1;
        }

        reconciliation
    }

    pub fn set_workflow_launch_policy(&mut self, policy: WorkflowLaunchPolicy) {
        self.workflow_launch_policy = Some(policy);
    }

    pub fn enqueue_workflow_launch(
        &mut self,
        queued_launch: QueuedWorkflowLaunch,
    ) -> QueuedWorkflowLaunch {
        self.queued_workflow_launches
            .push_back(queued_launch.clone());
        queued_launch
    }

    pub fn dequeue_workflow_launch(&mut self) -> Option<QueuedWorkflowLaunch> {
        self.queued_workflow_launches.pop_front()
    }

    pub fn remove_queued_workflow_launch(
        &mut self,
        queue_item_id: &str,
    ) -> Option<QueuedWorkflowLaunch> {
        let index = self
            .queued_workflow_launches
            .iter()
            .position(|queued_launch| queued_launch.id() == queue_item_id)?;
        self.queued_workflow_launches.remove(index)
    }

    pub fn clear_queued_workflow_launches(&mut self) -> Vec<QueuedWorkflowLaunch> {
        self.queued_workflow_launches.drain(..).collect()
    }

    pub fn workflow_run(&self, workflow_run_id: &str) -> Option<&WorkflowRun> {
        self.workflow_runs
            .iter()
            .find(|workflow_run| workflow_run.id() == workflow_run_id)
    }

    pub fn workflow_run_mut(&mut self, workflow_run_id: &str) -> Option<&mut WorkflowRun> {
        self.workflow_runs
            .iter_mut()
            .find(|workflow_run| workflow_run.id() == workflow_run_id)
    }

    pub fn add_workflow_watchdog(
        &mut self,
        watchdog: WorkflowWatchdogDefinition,
    ) -> WorkflowWatchdogDefinition {
        self.workflow_watchdogs.push(watchdog.clone());
        watchdog
    }

    pub fn workflow_watchdog(&self, watchdog_id: &str) -> Option<&WorkflowWatchdogDefinition> {
        self.workflow_watchdogs
            .iter()
            .find(|watchdog| watchdog.id() == watchdog_id)
    }

    pub fn workflow_watchdog_mut(
        &mut self,
        watchdog_id: &str,
    ) -> Option<&mut WorkflowWatchdogDefinition> {
        self.workflow_watchdogs
            .iter_mut()
            .find(|watchdog| watchdog.id() == watchdog_id)
    }

    pub fn remove_workflow_watchdog(
        &mut self,
        watchdog_id: &str,
    ) -> Option<WorkflowWatchdogDefinition> {
        let index = self
            .workflow_watchdogs
            .iter()
            .position(|watchdog| watchdog.id() == watchdog_id)?;
        Some(self.workflow_watchdogs.remove(index))
    }

    pub fn workflow_node_run_mut(
        &mut self,
        workflow_node_run_id: &str,
    ) -> Option<&mut WorkflowNodeRun> {
        self.workflow_runs
            .iter_mut()
            .find_map(|workflow_run| workflow_run.node_run_mut(workflow_node_run_id))
    }

    pub fn workflow_console(&self, workflow_id: &str) -> Option<&WorkflowConsole> {
        self.workflow_consoles
            .iter()
            .find(|console| console.workflow_id() == workflow_id)
    }

    pub fn workflow_console_mut(&mut self, workflow_id: &str) -> Option<&mut WorkflowConsole> {
        self.workflow_consoles
            .iter_mut()
            .find(|console| console.workflow_id() == workflow_id)
    }

    pub fn ensure_workflow_console(
        &mut self,
        workflow_id: impl Into<String>,
    ) -> &mut WorkflowConsole {
        let workflow_id = workflow_id.into();
        if let Some(index) = self
            .workflow_consoles
            .iter()
            .position(|console| console.workflow_id() == workflow_id)
        {
            return &mut self.workflow_consoles[index];
        }
        self.workflow_consoles
            .push(WorkflowConsole::new(workflow_id));
        let index = self.workflow_consoles.len() - 1;
        &mut self.workflow_consoles[index]
    }

    #[cfg(test)]
    pub(in crate::session) fn submit_prompt(
        &mut self,
        prompt: PromptQueueItem,
    ) -> PromptSubmissionOutcome {
        self.prompt_runtime
            .submit_prompt(prompt, self.focused_agent_id.as_deref())
    }

    #[cfg(test)]
    pub(in crate::session) fn complete_active_prompt_only(
        &mut self,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.prompt_runtime
            .complete_active_prompt_only(agent_id, self.focused_agent_id.as_deref())
    }

    #[cfg(test)]
    pub(in crate::session) fn cancel_active_prompt_only(
        &mut self,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.prompt_runtime
            .cancel_active_prompt_only(agent_id, self.focused_agent_id.as_deref())
    }

    pub(in crate::session) fn remove_queued_prompts_by_attachment(
        &mut self,
        attachment_id: &str,
    ) -> usize {
        self.prompt_runtime
            .remove_queued_prompts_by_attachment(attachment_id, self.focused_agent_id.as_deref())
    }

    pub(in crate::session) fn remove_queued_prompts_by_workflow_run(
        &mut self,
        workflow_run_id: &str,
    ) -> usize {
        self.prompt_runtime.remove_queued_prompts_by_workflow_run(
            workflow_run_id,
            self.focused_agent_id.as_deref(),
        )
    }

    #[cfg(test)]
    pub(in crate::session) fn peek_next_queued_prompt(
        &self,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.prompt_runtime.peek_next_queued_prompt(agent_id)
    }

    #[cfg(test)]
    pub(in crate::session) fn pop_next_queued_prompt(
        &mut self,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.prompt_runtime
            .pop_next_queued_prompt(agent_id, self.focused_agent_id.as_deref())
    }

    #[cfg(test)]
    pub(in crate::session) fn activate_prompt(
        &mut self,
        prompt: PromptQueueItem,
    ) -> PromptQueueItem {
        self.prompt_runtime
            .activate_prompt(prompt, self.focused_agent_id.as_deref())
    }

    pub fn apply_config_changes(
        &mut self,
        values: BTreeMap<String, String>,
        updated_by_attachment_id: impl Into<String>,
    ) {
        self.config_state
            .apply_changes(values, updated_by_attachment_id);
    }

    pub fn transition_to(&mut self, next: SessionStatus) -> bool {
        let allowed = match (self.status, next) {
            (current, next) if current == next => true,
            (SessionStatus::Created, SessionStatus::Active) => true,
            (SessionStatus::Active, SessionStatus::Parked) => true,
            (SessionStatus::Parked, SessionStatus::Active) => true,
            (SessionStatus::Created, SessionStatus::Ended) => true,
            (SessionStatus::Active, SessionStatus::Ended) => true,
            (SessionStatus::Parked, SessionStatus::Ended) => true,
            (SessionStatus::Ended, SessionStatus::Parked) => true,
            _ => false,
        };

        if !allowed {
            return false;
        }

        self.status = next;

        if next == SessionStatus::Ended {
            self.active_provider_run_id = None;
            self.focused_agent_id = None;
            self.attachment_ids.clear();
            self.prompt_runtime.clear();
        }

        true
    }
}

pub fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
