use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;

pub const DEFAULT_SESSION_MAX_AGENTS: i32 = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEndpointDefinition {
    id: String,
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

    pub fn entry_node_id(&self) -> &str {
        &self.entry_node_id
    }

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
    }

    pub fn set_entry_node_id(&mut self, entry_node_id: impl Into<String>) {
        self.entry_node_id = entry_node_id.into();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeDefinition {
    id: String,
    agent_id: String,
}

impl WorkflowNodeDefinition {
    pub fn new(id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            agent_id: agent_id.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEdgeDefinition {
    id: String,
    from_node_id: String,
    to_node_id: String,
}

impl WorkflowEdgeDefinition {
    pub fn new(
        id: impl Into<String>,
        from_node_id: impl Into<String>,
        to_node_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            from_node_id: from_node_id.into(),
            to_node_id: to_node_id.into(),
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    id: String,
    alias: Option<String>,
    nodes: Vec<WorkflowNodeDefinition>,
    edges: Vec<WorkflowEdgeDefinition>,
    endpoints: Vec<WorkflowEndpointDefinition>,
}

impl WorkflowDefinition {
    pub fn new(id: impl Into<String>, alias: Option<String>) -> Self {
        Self {
            id: id.into(),
            alias,
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

    pub fn nodes(&self) -> &[WorkflowNodeDefinition] {
        &self.nodes
    }

    pub fn edges(&self) -> &[WorkflowEdgeDefinition] {
        &self.edges
    }

    pub fn endpoints(&self) -> &[WorkflowEndpointDefinition] {
        &self.endpoints
    }

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
    }

    pub fn add_node(&mut self, node: WorkflowNodeDefinition) -> WorkflowNodeDefinition {
        self.nodes.push(node.clone());
        node
    }

    pub fn node(&self, node_id: &str) -> Option<&WorkflowNodeDefinition> {
        self.nodes.iter().find(|node| node.id() == node_id)
    }

    pub fn remove_node(&mut self, node_id: &str) -> Option<WorkflowNodeDefinition> {
        let index = self.nodes.iter().position(|node| node.id() == node_id)?;
        let removed = self.nodes.remove(index);
        self.edges
            .retain(|edge| edge.from_node_id() != node_id && edge.to_node_id() != node_id);
        self.endpoints
            .retain(|endpoint| endpoint.entry_node_id() != node_id);
        Some(removed)
    }

    pub fn add_edge(&mut self, edge: WorkflowEdgeDefinition) -> WorkflowEdgeDefinition {
        self.edges.push(edge.clone());
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
        Some(self.edges.remove(index))
    }

    pub fn add_endpoint(
        &mut self,
        endpoint: WorkflowEndpointDefinition,
    ) -> WorkflowEndpointDefinition {
        self.endpoints.push(endpoint.clone());
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
        Some(self.endpoints.remove(index))
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowRunStatus {
    Created,
    Running,
    Waiting,
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowNodeRunStatus {
    Created,
    Ready,
    Running,
    Waiting,
    Completed,
    Failed,
    Stopped,
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
    ) -> Self {
        Self {
            workflow_run_id: workflow_run_id.into(),
            workflow_id: workflow_id.into(),
            source_node_run_id: source_node_run_id.into(),
            source_node_id: source_node_id.into(),
            source_agent_id: source_agent_id.into(),
            target_node_id: target_node_id.into(),
            invocation_prompt,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowMessage {
    id: String,
    source_node_run_id: Option<String>,
    target_node_id: String,
    message_type: String,
    summary: String,
    handoff_payload: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeRun {
    id: String,
    node_id: String,
    agent_id: String,
    status: WorkflowNodeRunStatus,
    summary: Option<String>,
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

    pub fn messages(&self) -> &[WorkflowMessage] {
        &self.messages
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    pub alias: Option<String>,
}

impl CreateSessionRequest {
    pub fn new(workspace_id: impl Into<String>, worktree_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            worktree_id: worktree_id.into(),
            alias: None,
        }
    }

    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
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
}

impl PromptAttachment {
    pub fn new(url: impl Into<String>, mime: impl Into<String>, filename: Option<String>) -> Self {
        Self {
            url: url.into(),
            mime: mime.into(),
            filename,
        }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptSubmissionOutcome {
    Started { prompt: PromptQueueItem },
    Queued { prompt: PromptQueueItem },
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
    host_machine_id: String,
    host_daemon_id: String,
    created_at_ms: u64,
    execution_mode: SessionExecutionMode,
    status: SessionStatus,
    active_provider_run_id: Option<String>,
    focused_agent_id: Option<String>,
    max_agents: i32,
    agents: Vec<AgentInstance>,
    attachment_ids: BTreeSet<String>,
    active_prompt: Option<PromptQueueItem>,
    queued_prompts: VecDeque<PromptQueueItem>,
    scheduler_state: SchedulerState,
    config_state: SessionConfigState,
    worktree_assignments: Vec<RuntimeWorktreeAssignment>,
    workflows: Vec<WorkflowDefinition>,
    workflow_runs: Vec<WorkflowRun>,
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

        Self {
            id: id.clone(),
            alias,
            workspace_id: workspace_id.into(),
            worktree_id: worktree_id.clone(),
            host_machine_id: host_machine_id.into(),
            host_daemon_id: host_daemon_id.into(),
            created_at_ms: unix_epoch_ms(),
            execution_mode: SessionExecutionMode::SingleAgent,
            status: SessionStatus::Created,
            active_provider_run_id: None,
            focused_agent_id: None,
            max_agents: DEFAULT_SESSION_MAX_AGENTS,
            agents: Vec::new(),
            attachment_ids: BTreeSet::new(),
            active_prompt: None,
            queued_prompts: VecDeque::new(),
            scheduler_state: SchedulerState::Idle,
            config_state: SessionConfigState::default(),
            worktree_assignments: vec![RuntimeWorktreeAssignment::new(
                format!("worktree-assignment-{}-1", id),
                worktree_id,
                format!("session/{id}"),
                WorktreeIsolationMode::SharedSession,
            )],
            workflows: Vec::new(),
            workflow_runs: Vec::new(),
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
    pub fn worktree_id(&self) -> &str {
        &self.worktree_id
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
    pub fn status(&self) -> SessionStatus {
        self.status
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
    pub fn attachment_ids(&self) -> &BTreeSet<String> {
        &self.attachment_ids
    }
    pub fn active_prompt(&self) -> Option<&PromptQueueItem> {
        self.active_prompt.as_ref()
    }
    pub fn has_active_prompt(&self) -> bool {
        self.active_prompt.is_some()
    }
    pub fn queued_prompts(&self) -> &VecDeque<PromptQueueItem> {
        &self.queued_prompts
    }
    pub fn scheduler_state(&self) -> SchedulerState {
        self.scheduler_state
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
    }

    pub fn create_workflow(&mut self, workflow: WorkflowDefinition) -> WorkflowDefinition {
        self.workflows.push(workflow.clone());
        workflow
    }

    pub fn workflow(&self, workflow_id: &str) -> Option<&WorkflowDefinition> {
        self.workflows.iter().find(|workflow| workflow.id() == workflow_id)
    }

    pub fn workflow_mut(&mut self, workflow_id: &str) -> Option<&mut WorkflowDefinition> {
        self.workflows
            .iter_mut()
            .find(|workflow| workflow.id() == workflow_id)
    }

    pub fn create_workflow_run(&mut self, workflow_run: WorkflowRun) -> WorkflowRun {
        self.workflow_runs.push(workflow_run.clone());
        workflow_run
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

    pub fn submit_prompt(&mut self, prompt: PromptQueueItem) -> PromptSubmissionOutcome {
        if self.active_prompt.is_none() {
            let mut running = prompt;
            running.set_status(PromptStatus::Running);
            self.active_prompt = Some(running.clone());
            self.refresh_scheduler_state();
            PromptSubmissionOutcome::Started { prompt: running }
        } else {
            let mut queued = prompt;
            queued.set_status(PromptStatus::Queued);
            self.queued_prompts.push_back(queued.clone());
            self.refresh_scheduler_state();
            PromptSubmissionOutcome::Queued { prompt: queued }
        }
    }

    pub fn complete_active_prompt(&mut self) -> Option<PromptCompletion> {
        let mut completed = self.active_prompt.take()?;
        completed.set_status(PromptStatus::Completed);

        let started_next = self.queued_prompts.pop_front().map(|mut next| {
            next.set_status(PromptStatus::Running);
            self.active_prompt = Some(next.clone());
            next
        });

        self.refresh_scheduler_state();

        Some(PromptCompletion {
            completed,
            started_next,
        })
    }

    pub fn complete_active_prompt_only(&mut self) -> Option<PromptQueueItem> {
        let mut completed = self.active_prompt.take()?;
        completed.set_status(PromptStatus::Completed);
        self.refresh_scheduler_state();
        Some(completed)
    }

    pub fn cancel_active_prompt_only(&mut self) -> Option<PromptQueueItem> {
        let mut cancelled = self.active_prompt.take()?;
        cancelled.set_status(PromptStatus::Cancelled);
        self.refresh_scheduler_state();
        Some(cancelled)
    }

    pub fn begin_cancelling_active_prompt(&mut self) -> Option<PromptQueueItem> {
        self.active_prompt
            .as_mut()?
            .set_status(PromptStatus::Cancelling);
        self.refresh_scheduler_state();
        self.active_prompt.clone()
    }

    pub fn finalize_active_prompt_cancellation(&mut self) -> Option<PromptQueueItem> {
        let active = self.active_prompt.as_ref()?;
        if active.status() != PromptStatus::Cancelling {
            return None;
        }

        let mut cancelled = self.active_prompt.take()?;
        cancelled.set_status(PromptStatus::Cancelled);
        self.refresh_scheduler_state();
        Some(cancelled)
    }

    pub fn peek_next_queued_prompt(&self) -> Option<PromptQueueItem> {
        self.queued_prompts.front().cloned()
    }

    pub fn activate_next_queued_prompt(&mut self) -> Option<PromptQueueItem> {
        let mut next = self.queued_prompts.pop_front()?;
        next.set_status(PromptStatus::Running);
        self.active_prompt = Some(next.clone());
        self.refresh_scheduler_state();
        Some(next)
    }

    pub fn clear_active_prompt_if(&mut self, prompt_id: &str) -> bool {
        if self.active_prompt.as_ref().map(|prompt| prompt.id()) == Some(prompt_id) {
            self.active_prompt = None;
            self.refresh_scheduler_state();
            return true;
        }

        false
    }

    pub fn remove_queued_prompts_by_attachment(&mut self, attachment_id: &str) -> usize {
        let original_len = self.queued_prompts.len();
        self.queued_prompts
            .retain(|prompt| prompt.source_attachment_id() != attachment_id);
        let removed = original_len - self.queued_prompts.len();
        self.refresh_scheduler_state();
        removed
    }

    pub fn pop_next_queued_prompt(&mut self) -> Option<PromptQueueItem> {
        let next = self.queued_prompts.pop_front();
        self.refresh_scheduler_state();
        next
    }

    pub fn activate_prompt(&mut self, mut prompt: PromptQueueItem) -> PromptQueueItem {
        prompt.set_status(PromptStatus::Running);
        self.active_prompt = Some(prompt.clone());
        self.refresh_scheduler_state();
        prompt
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
            self.active_prompt = None;
            self.queued_prompts.clear();
            self.scheduler_state = SchedulerState::Idle;
        }

        true
    }

    fn refresh_scheduler_state(&mut self) {
        self.scheduler_state = if self.active_prompt.is_some() {
            if self.queued_prompts.is_empty() {
                SchedulerState::Running
            } else {
                SchedulerState::Waiting
            }
        } else if self.queued_prompts.is_empty() {
            SchedulerState::Idle
        } else {
            SchedulerState::Runnable
        };
    }
}

pub fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
