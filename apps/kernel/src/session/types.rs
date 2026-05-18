use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;

use super::prompt_runtime::PromptRuntimeState;
pub use super::runtime_interactions::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionKind, RuntimeInteractionLevel,
};
pub use super::workflow_canvas::{
    WorkflowCanvasLayout, WorkflowCanvasLayoutPatch, WorkflowCanvasPoint,
};
pub use super::workflow_definition::WorkflowDefinition;
pub use super::workflow_diagnostics::{
    WorkflowConsole, WorkflowConsoleEntry, WorkflowFailureEvent, WorkflowFailureKind,
    WorkflowFailurePolicy, WorkflowFailurePolicyMode,
};
pub use super::workflow_graph::{
    WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowNodeDefinition,
    WorkflowOutputValidationPolicy,
};
pub use super::workflow_outputs::{
    WorkflowArtifactRef, WorkflowCompletionSnapshot, WorkflowHandoffPayload,
    WorkflowIntermediateOutput, WorkflowOutputPayload, WorkflowRunOutputSubmission,
    WorkflowTurnOutputSubmissions, WorkflowTurnSubmissionKind,
};
pub use super::workflow_publication::{
    WorkflowPublicationDefinition, WorkflowPublicationPairingCode,
    WorkflowPublicationPairingCodeRecord, WorkflowPublicationSenderCredential,
    WorkflowPublicationTrustedSender,
};
pub use super::workflow_run_records::{WorkflowMessage, WorkflowNodeRun};
pub use super::workflow_runs::WorkflowRun;
pub use super::workflow_scheduling::{
    QueuedWorkflowLaunch, QueuedWorkflowLaunchSource, WorkflowLaunchPolicy,
    WorkflowWatchdogDefinition, WorkflowWatchdogPolicy, DEFAULT_WORKFLOW_LAUNCH_POLICY,
    DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS,
};
pub use super::workflow_turns::{
    WorkflowNodeRunStatus, WorkflowRunStatus, WorkflowRuntimeToolCallEvent, WorkflowTurnEnvelope,
    WorkflowTurnRuntimeState,
};
pub use super::workspace_links::{WorkspaceLinkAttachment, WorkspaceLinkDefinition};

pub const DEFAULT_SESSION_MAX_AGENTS: i32 = 64;
pub const DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT: usize = 128;
pub const DEFAULT_LOCAL_USER_ID: &str = "local";

fn default_session_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

fn default_session_members() -> Vec<SessionMember> {
    vec![SessionMember::local()]
}

fn default_session_invite_max_uses() -> Option<u32> {
    Some(1)
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
