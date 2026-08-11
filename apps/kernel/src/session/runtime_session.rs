use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;
use crate::provider::ExternalProviderImportMetadata;

use super::agent_prompt_scheduling::AgentPromptSchedule;
use super::metaagent_task::{MetaagentTask, MetaagentTaskStatus};
#[cfg(test)]
use super::prompt_queue::PromptSubmissionOutcome;
use super::prompt_queue::{AgentPromptState, DurablePromptPrivateState, PromptQueueItem};
use super::prompt_runtime::PromptRuntimeState;
use super::queued_metaagent_task::QueuedMetaagentTask;
use super::runtime_interactions::RuntimeInteraction;
use super::runtime_worktrees::{RuntimeWorktreeAssignment, WorktreeIsolationMode};
use super::session_config::SessionConfigState;
use super::session_identity::{
    default_session_members, default_session_owner_user_id, CollaborationLevel,
    SessionAgentDefaults, SessionInvite, SessionMember,
};
use super::session_lifecycle::{
    KernelRestartReconciliation, SchedulerState, SessionExecutionMode, SessionStatus,
};
use super::types::{unix_epoch_ms, DEFAULT_SESSION_MAX_AGENTS};
use super::workflow_definition::WorkflowDefinition;
use super::workflow_diagnostics::{WorkflowConsole, WorkflowFailureEvent, WorkflowFailureKind};
use super::workflow_publication::{
    WorkflowEventBinding, WorkflowEventDeliveryReceipt, WorkflowPublicationDefinition,
    WorkflowPublicationSnapshot,
};
use super::workflow_run_records::WorkflowNodeRun;
use super::workflow_runs::WorkflowRun;
use super::workflow_scheduling::{
    WorkflowPromptQueueDefinition, WorkflowQueuedPrompt, WorkflowQueuedPromptStatus,
    WorkflowScheduleDefinition, WorkflowWatchdogDefinition,
};
use super::workflow_turns::{WorkflowNodeRunStatus, WorkflowRunStatus};
use super::workspace_links::WorkspaceLinkDefinition;

mod projection;
mod workflows;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCollaborationAgentCounts {
    pub owned_agent_count: usize,
    pub other_user_agent_count: usize,
    pub total_agent_count: usize,
    pub collaborator_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentOutputReadState {
    latest_output_sequence: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    seen_sequences_by_user: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct WorkflowPublicationState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workflow_publications: Vec<WorkflowPublicationDefinition>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    workflow_publication_snapshots: BTreeMap<String, WorkflowPublicationSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workflow_event_bindings: Vec<WorkflowEventBinding>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    workflow_event_delivery_receipts: BTreeMap<String, WorkflowEventDeliveryReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSession {
    id: String,
    #[serde(default)]
    project_id: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_prompt_sent_at_ms: Option<u64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    collaboration_agent_counts: Option<SessionCollaborationAgentCounts>,
    attachment_ids: BTreeSet<String>,
    #[serde(flatten)]
    prompt_runtime: PromptRuntimeState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    active_interactions: Vec<RuntimeInteraction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    metaagent_tasks: Vec<MetaagentTask>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    queued_metaagent_tasks: VecDeque<QueuedMetaagentTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    agent_prompt_schedules: Vec<AgentPromptSchedule>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    agent_output_read_state: BTreeMap<String, AgentOutputReadState>,
    config_state: SessionConfigState,
    worktree_assignments: Vec<RuntimeWorktreeAssignment>,
    workflows: Vec<WorkflowDefinition>,
    workflow_runs: Vec<WorkflowRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workflow_prompt_queues: Vec<WorkflowPromptQueueDefinition>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    workflow_queued_prompts: VecDeque<WorkflowQueuedPrompt>,
    #[serde(
        default,
        alias = "workflow_watchdogs",
        skip_serializing_if = "Vec::is_empty"
    )]
    workflow_schedules: Vec<WorkflowScheduleDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workflow_consoles: Vec<WorkflowConsole>,
    #[serde(flatten)]
    workflow_publication_state: Box<WorkflowPublicationState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workspace_links: Vec<WorkspaceLinkDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    external_provider_imports: Vec<ExternalProviderImportMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
}

impl RuntimeSession {
    pub(crate) fn durable_prompt_private_states(&self) -> Vec<DurablePromptPrivateState> {
        self.prompt_runtime.durable_private_states(&self.id)
    }

    pub(crate) fn restore_durable_prompt_private_states(
        &mut self,
        states: &[DurablePromptPrivateState],
    ) {
        self.prompt_runtime.restore_durable_private_states(states);
        self.prompt_runtime
            .refresh_after_focus_change(self.focused_agent_id.as_deref());
    }

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
            project_id: String::new(),
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
            last_prompt_sent_at_ms: None,
            execution_mode: SessionExecutionMode::SingleAgent,
            status: SessionStatus::Created,
            hidden: false,
            agent_defaults: SessionAgentDefaults::default(),
            active_provider_run_id: None,
            focused_agent_id: None,
            max_agents: DEFAULT_SESSION_MAX_AGENTS,
            agents: Vec::new(),
            collaboration_agent_counts: None,
            attachment_ids: BTreeSet::new(),
            prompt_runtime: PromptRuntimeState::default(),
            active_interactions: Vec::new(),
            metaagent_tasks: Vec::new(),
            queued_metaagent_tasks: VecDeque::new(),
            agent_prompt_schedules: Vec::new(),
            agent_output_read_state: BTreeMap::new(),
            config_state: SessionConfigState::default(),
            worktree_assignments: vec![RuntimeWorktreeAssignment::new(
                format!("worktree-assignment-{}-1", id),
                worktree_id,
                format!("session/{id}"),
                WorktreeIsolationMode::SharedSession,
            )],
            workflows: Vec::new(),
            workflow_runs: Vec::new(),
            workflow_prompt_queues: Vec::new(),
            workflow_queued_prompts: VecDeque::new(),
            workflow_schedules: Vec::new(),
            workflow_consoles: Vec::new(),
            workflow_publication_state: Box::default(),
            workspace_links: Vec::new(),
            external_provider_imports: Vec::new(),
            workspace_live_sync_mode: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub(crate) fn assign_project_id(&mut self, project_id: impl Into<String>) -> bool {
        if !self.project_id.is_empty() {
            return false;
        }
        self.project_id = project_id.into();
        true
    }

    pub(crate) fn clear_project_id_for_hidden_restore(&mut self) {
        debug_assert!(self.hidden);
        self.project_id.clear();
    }

    pub(crate) fn migrate_default_project_scope(
        &mut self,
        workspace_id: impl Into<String>,
        project_id: impl Into<String>,
        alias: Option<String>,
    ) {
        debug_assert!(!self.hidden);
        self.workspace_id = workspace_id.into();
        self.project_id = project_id.into();
        self.alias = alias;
    }

    pub fn agent_prompt_schedules(&self) -> &[AgentPromptSchedule] {
        &self.agent_prompt_schedules
    }

    pub(crate) fn agent_prompt_schedules_mut(&mut self) -> &mut Vec<AgentPromptSchedule> {
        &mut self.agent_prompt_schedules
    }

    pub(crate) fn add_agent_prompt_schedule(
        &mut self,
        schedule: AgentPromptSchedule,
    ) -> AgentPromptSchedule {
        self.agent_prompt_schedules.push(schedule.clone());
        schedule
    }

    pub(crate) fn remove_agent_prompt_schedule(
        &mut self,
        schedule_id: &str,
    ) -> Option<AgentPromptSchedule> {
        let index = self
            .agent_prompt_schedules
            .iter()
            .position(|schedule| schedule.id() == schedule_id)?;
        Some(self.agent_prompt_schedules.remove(index))
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
        self.members = vec![SessionMember::new(
            owner_user_id,
            0,
            None,
            CollaborationLevel::Private,
        )];
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

    pub fn collaboration_level_for_user(&self, user_id: &str) -> Option<CollaborationLevel> {
        self.members
            .iter()
            .find(|member| member.user_id() == user_id)
            .map(|member| member.collaboration_level())
    }

    pub fn can_prompt_agent_owned_by(
        &self,
        caller_user_id: &str,
        agent_owner_user_id: &str,
    ) -> bool {
        caller_user_id == agent_owner_user_id
            || self
                .collaboration_level_for_user(caller_user_id)
                .is_some_and(|level| level.can_prompt_agent_directly())
    }

    pub fn add_member(
        &mut self,
        user_id: impl Into<String>,
        invited_by_user_id: Option<String>,
        collaboration_level: CollaborationLevel,
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
        let member = SessionMember::new(
            user_id,
            unix_epoch_ms(),
            invited_by_user_id,
            collaboration_level,
        );
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

    pub fn last_prompt_sent_at_ms(&self) -> Option<u64> {
        self.last_prompt_sent_at_ms
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

    pub fn set_max_agents(&mut self, max_agents: i32) {
        self.max_agents = max_agents.max(1);
    }

    pub fn agents(&self) -> &[AgentInstance] {
        &self.agents
    }

    pub fn set_agents(&mut self, agents: Vec<AgentInstance>) {
        self.agents = agents;
        self.collaboration_agent_counts = None;
    }

    pub fn collaboration_agent_counts(&self) -> Option<&SessionCollaborationAgentCounts> {
        self.collaboration_agent_counts.as_ref()
    }

    pub fn redacted_for_user(self, user_id: &str) -> Self {
        projection::redacted_for_user(self, user_id)
    }

    pub fn note_agent_output_sequence(&mut self, agent_id: &str, sequence: u64) -> bool {
        if sequence == 0 {
            return false;
        }
        let state = self
            .agent_output_read_state
            .entry(agent_id.to_string())
            .or_default();
        if sequence <= state.latest_output_sequence {
            return false;
        }
        state.latest_output_sequence = sequence;
        true
    }

    pub fn acknowledge_agent_output_seen(&mut self, user_id: &str, agent_id: &str) -> bool {
        let Some(state) = self.agent_output_read_state.get_mut(agent_id) else {
            return false;
        };
        let previous = state
            .seen_sequences_by_user
            .insert(user_id.to_string(), state.latest_output_sequence)
            .unwrap_or(0);
        previous != state.latest_output_sequence
    }

    pub fn agent_has_unread_output(&self, user_id: &str, agent_id: &str) -> bool {
        let Some(state) = self.agent_output_read_state.get(agent_id) else {
            return false;
        };
        state.latest_output_sequence
            > state
                .seen_sequences_by_user
                .get(user_id)
                .copied()
                .unwrap_or(0)
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

    pub(crate) fn mirror_agent_prompt_state(
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

    pub fn metaagent_tasks(&self) -> &[MetaagentTask] {
        &self.metaagent_tasks
    }

    pub fn queued_metaagent_tasks(&self) -> &VecDeque<QueuedMetaagentTask> {
        &self.queued_metaagent_tasks
    }

    pub fn enqueue_metaagent_task(&mut self, task: QueuedMetaagentTask) -> QueuedMetaagentTask {
        self.queued_metaagent_tasks.push_back(task.clone());
        task
    }

    pub fn pop_next_queued_metaagent_task(&mut self) -> Option<QueuedMetaagentTask> {
        self.queued_metaagent_tasks.pop_front()
    }

    pub fn requeue_metaagent_task_front(&mut self, task: QueuedMetaagentTask) {
        self.queued_metaagent_tasks.push_front(task);
    }

    pub fn has_active_metaagent_task(&self) -> bool {
        self.metaagent_tasks.iter().any(|task| {
            matches!(
                task.status(),
                MetaagentTaskStatus::Active | MetaagentTaskStatus::Paused
            )
        })
    }

    pub fn has_active_session_task(&self) -> bool {
        self.has_active_workflow_run() || self.has_active_metaagent_task()
    }

    pub fn has_pending_session_task(&self) -> bool {
        !self.queued_metaagent_tasks.is_empty()
            || self.next_workflow_queued_prompt_created_at_ms().is_some()
    }

    pub fn metaagent_task(&self, metaagent_id: &str) -> Option<&MetaagentTask> {
        self.metaagent_tasks
            .iter()
            .find(|task| task.metaagent_id() == metaagent_id)
    }

    pub fn ensure_metaagent_task(
        &mut self,
        metaagent_id: &str,
        task_markdown: impl Into<String>,
    ) -> &MetaagentTask {
        let task_markdown = task_markdown.into();
        if let Some(index) = self
            .metaagent_tasks
            .iter()
            .position(|task| task.metaagent_id() == metaagent_id)
        {
            let task = &mut self.metaagent_tasks[index];
            if !task_markdown.trim().is_empty() {
                if task.status().is_terminal() {
                    let task_id = format!(
                        "metaagent-task-{}-{}-{}",
                        metaagent_id,
                        unix_epoch_ms(),
                        task.revision().saturating_add(1)
                    );
                    task.restart(task_id, task_markdown);
                } else {
                    task.update_task_markdown(task_markdown);
                }
            } else if task.status().is_terminal() {
                task.set_status(MetaagentTaskStatus::Active);
            }
            return &self.metaagent_tasks[index];
        }
        let task_id = format!("metaagent-task-{}-{}", metaagent_id, unix_epoch_ms());
        self.metaagent_tasks
            .push(MetaagentTask::new(task_id, metaagent_id, task_markdown));
        self.metaagent_tasks
            .last()
            .expect("pushed metaagent task should be present")
    }

    pub fn start_metaagent_task_if_needed(
        &mut self,
        metaagent_id: &str,
        task_markdown: impl Into<String>,
    ) -> Option<&MetaagentTask> {
        let task_markdown = task_markdown.into();
        if let Some(index) = self
            .metaagent_tasks
            .iter()
            .position(|task| task.metaagent_id() == metaagent_id)
        {
            let task = &mut self.metaagent_tasks[index];
            if task.status().is_terminal() {
                let task_id = format!(
                    "metaagent-task-{}-{}-{}",
                    metaagent_id,
                    unix_epoch_ms(),
                    task.revision().saturating_add(1)
                );
                task.restart(task_id, task_markdown);
                return Some(&self.metaagent_tasks[index]);
            }
            return None;
        }
        Some(self.ensure_metaagent_task(metaagent_id, task_markdown))
    }

    pub fn start_or_update_metaagent_task(
        &mut self,
        metaagent_id: &str,
        task_markdown: impl Into<String>,
    ) -> &MetaagentTask {
        let task_markdown = task_markdown.into();
        if let Some(index) = self
            .metaagent_tasks
            .iter()
            .position(|task| task.metaagent_id() == metaagent_id)
        {
            let task = &mut self.metaagent_tasks[index];
            if !task_markdown.trim().is_empty() {
                if task.status().is_terminal() {
                    let task_id = format!(
                        "metaagent-task-{}-{}-{}",
                        metaagent_id,
                        unix_epoch_ms(),
                        task.revision().saturating_add(1)
                    );
                    task.restart(task_id, task_markdown);
                    return &self.metaagent_tasks[index];
                }
                task.update_task_markdown(task_markdown);
            }
            if task.status() != MetaagentTaskStatus::Active {
                task.set_status(MetaagentTaskStatus::Active);
            }
            return &self.metaagent_tasks[index];
        }
        self.ensure_metaagent_task(metaagent_id, task_markdown)
    }

    pub fn update_metaagent_task_markdown(
        &mut self,
        metaagent_id: &str,
        task_markdown: impl Into<String>,
    ) -> Option<&MetaagentTask> {
        let task_markdown = task_markdown.into();
        if self.metaagent_task(metaagent_id).is_none() {
            return Some(self.ensure_metaagent_task(metaagent_id, task_markdown));
        }
        let task = self
            .metaagent_tasks
            .iter_mut()
            .find(|task| task.metaagent_id() == metaagent_id)?;
        task.update_task_markdown(task_markdown);
        Some(task)
    }

    pub fn update_metaagent_plan_markdown(
        &mut self,
        metaagent_id: &str,
        plan_markdown: impl Into<String>,
    ) -> Option<&MetaagentTask> {
        if self.metaagent_task(metaagent_id).is_none() {
            self.ensure_metaagent_task(metaagent_id, "");
        }
        let task = self
            .metaagent_tasks
            .iter_mut()
            .find(|task| task.metaagent_id() == metaagent_id)?;
        task.update_plan_markdown(plan_markdown);
        Some(task)
    }

    pub fn set_metaagent_task_status(
        &mut self,
        metaagent_id: &str,
        status: MetaagentTaskStatus,
    ) -> Option<&MetaagentTask> {
        let task = self
            .metaagent_tasks
            .iter_mut()
            .find(|task| task.metaagent_id() == metaagent_id)?;
        task.set_status(status);
        Some(task)
    }

    pub fn complete_metaagent_task(
        &mut self,
        metaagent_id: &str,
        summary: Option<String>,
    ) -> Option<&MetaagentTask> {
        let task = self
            .metaagent_tasks
            .iter_mut()
            .find(|task| task.metaagent_id() == metaagent_id)?;
        task.mark_completed(summary);
        Some(task)
    }

    pub fn block_metaagent_task(
        &mut self,
        metaagent_id: &str,
        reason: impl Into<String>,
    ) -> Option<&MetaagentTask> {
        let task = self
            .metaagent_tasks
            .iter_mut()
            .find(|task| task.metaagent_id() == metaagent_id)?;
        task.mark_blocked(reason);
        Some(task)
    }

    pub fn abort_metaagent_task(
        &mut self,
        metaagent_id: &str,
        reason: Option<String>,
    ) -> Option<&MetaagentTask> {
        let task = self
            .metaagent_tasks
            .iter_mut()
            .find(|task| task.metaagent_id() == metaagent_id)?;
        task.abort(reason);
        Some(task)
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

    pub(crate) fn equivalent_except_workflow_runs(&self, other: &Self) -> bool {
        let mut left = self.clone();
        let mut right = other.clone();
        left.workflow_runs.clear();
        right.workflow_runs.clear();
        left == right
    }

    pub fn workflow_prompt_queues(&self) -> &[WorkflowPromptQueueDefinition] {
        &self.workflow_prompt_queues
    }

    pub fn workflow_queued_prompts(&self) -> &VecDeque<WorkflowQueuedPrompt> {
        &self.workflow_queued_prompts
    }

    pub fn workflow_schedules(&self) -> &[WorkflowScheduleDefinition] {
        &self.workflow_schedules
    }

    pub fn workflow_schedules_mut(&mut self) -> &mut [WorkflowScheduleDefinition] {
        &mut self.workflow_schedules
    }

    pub fn workflow_watchdogs(&self) -> &[WorkflowWatchdogDefinition] {
        self.workflow_schedules()
    }

    pub fn workflow_watchdogs_mut(&mut self) -> &mut [WorkflowWatchdogDefinition] {
        self.workflow_schedules_mut()
    }

    pub fn workflow_consoles(&self) -> &[WorkflowConsole] {
        &self.workflow_consoles
    }

    pub fn workflow_publications(&self) -> &[WorkflowPublicationDefinition] {
        &self.workflow_publication_state.workflow_publications
    }

    pub fn workflow_event_bindings(&self) -> &[WorkflowEventBinding] {
        &self.workflow_publication_state.workflow_event_bindings
    }

    pub fn workflow_event_delivery_receipts(
        &self,
    ) -> &BTreeMap<String, WorkflowEventDeliveryReceipt> {
        &self
            .workflow_publication_state
            .workflow_event_delivery_receipts
    }

    pub(crate) fn workflow_publication_snapshot(
        &self,
        publication_id: &str,
    ) -> Option<&WorkflowPublicationSnapshot> {
        self.workflow_publication_state
            .workflow_publication_snapshots
            .get(publication_id)
    }

    pub fn workspace_links(&self) -> &[WorkspaceLinkDefinition] {
        &self.workspace_links
    }

    pub fn external_provider_imports(&self) -> &[ExternalProviderImportMetadata] {
        &self.external_provider_imports
    }

    pub fn upsert_external_provider_import(&mut self, import: ExternalProviderImportMetadata) {
        self.external_provider_imports
            .retain(|existing| !existing.same_observed_provider_session(&import));
        self.external_provider_imports.push(import);
        self.external_provider_imports
            .sort_by(|left, right| left.import_order_key().cmp(&right.import_order_key()));
    }

    pub fn workspace_live_sync_mode(&self) -> Option<crate::config::WorkspaceLiveSyncMode> {
        self.workspace_live_sync_mode
    }

    pub fn equivalent_except_session_metadata(&self, other: &Self) -> bool {
        let mut left = self.clone();
        let right = other.clone();
        left.alias = right.alias.clone();
        left.last_used_at_ms = right.last_used_at_ms;
        left.last_prompt_sent_at_ms = right.last_prompt_sent_at_ms;
        left.hidden = right.hidden;
        left.focused_agent_id = right.focused_agent_id.clone();
        left.workspace_live_sync_mode = right.workspace_live_sync_mode;
        left == right
    }

    pub fn equivalent_except_prompt_runtime(&self, other: &Self) -> bool {
        let mut left = self.clone();
        let right = other.clone();
        left.prompt_runtime = right.prompt_runtime.clone();
        left == right
    }

    pub fn equivalent_except_runtime_interactions(&self, other: &Self) -> bool {
        let mut left = self.clone();
        let right = other.clone();
        left.active_interactions = right.active_interactions.clone();
        left == right
    }

    pub fn set_workspace_live_sync_mode(
        &mut self,
        mode: Option<crate::config::WorkspaceLiveSyncMode>,
    ) {
        self.workspace_live_sync_mode = mode;
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

    pub fn clear_attachments(&mut self) -> usize {
        let removed_count = self.attachment_ids.len();
        self.attachment_ids.clear();
        removed_count
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

    pub fn note_prompt_sent_at(&mut self, agent_id: &str, timestamp_ms: u64) {
        self.last_prompt_sent_at_ms = Some(timestamp_ms);
        if let Some(agent) = self.agents.iter_mut().find(|agent| agent.id() == agent_id) {
            agent.note_prompt_sent_at(timestamp_ms);
        }
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

#[cfg(test)]
mod tests;
