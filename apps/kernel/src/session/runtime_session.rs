use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;

use super::prompt_queue::{AgentPromptState, PromptQueueItem};
use super::prompt_runtime::PromptRuntimeState;
use super::runtime_interactions::RuntimeInteraction;
use super::runtime_worktrees::{RuntimeWorktreeAssignment, WorktreeIsolationMode};
use super::session_config::SessionConfigState;
use super::session_identity::{
    default_session_members, default_session_owner_user_id, SessionAgentDefaults, SessionInvite,
    SessionMember,
};
use super::session_lifecycle::{
    KernelRestartReconciliation, SchedulerState, SessionExecutionMode, SessionStatus,
};
use super::types::{unix_epoch_ms, DEFAULT_SESSION_MAX_AGENTS};
use super::workflow_definition::WorkflowDefinition;
use super::workflow_diagnostics::{WorkflowConsole, WorkflowFailureEvent, WorkflowFailureKind};
use super::workflow_publication::WorkflowPublicationDefinition;
use super::workflow_run_records::WorkflowNodeRun;
use super::workflow_runs::WorkflowRun;
use super::workflow_scheduling::{
    WorkflowPromptQueueDefinition, WorkflowQueuedPrompt, WorkflowQueuedPromptStatus,
    WorkflowWatchdogDefinition,
};
use super::workflow_turns::{WorkflowNodeRunStatus, WorkflowRunStatus};
use super::workspace_links::WorkspaceLinkDefinition;

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workflow_prompt_queues: Vec<WorkflowPromptQueueDefinition>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    workflow_queued_prompts: VecDeque<WorkflowQueuedPrompt>,
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
            workflow_prompt_queues: vec![WorkflowPromptQueueDefinition::default_queue()],
            workflow_queued_prompts: VecDeque::new(),
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

    pub fn workflow_prompt_queues(&self) -> &[WorkflowPromptQueueDefinition] {
        &self.workflow_prompt_queues
    }

    pub fn workflow_queued_prompts(&self) -> &VecDeque<WorkflowQueuedPrompt> {
        &self.workflow_queued_prompts
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

    pub fn add_workflow_prompt_queue(
        &mut self,
        queue: WorkflowPromptQueueDefinition,
    ) -> WorkflowPromptQueueDefinition {
        self.workflow_prompt_queues.push(queue.clone());
        queue
    }

    pub fn workflow_prompt_queue(&self, queue_id: &str) -> Option<&WorkflowPromptQueueDefinition> {
        self.workflow_prompt_queues
            .iter()
            .find(|queue| queue.id() == queue_id || queue.alias() == queue_id)
    }

    pub fn workflow_prompt_queue_mut(
        &mut self,
        queue_id: &str,
    ) -> Option<&mut WorkflowPromptQueueDefinition> {
        self.workflow_prompt_queues
            .iter_mut()
            .find(|queue| queue.id() == queue_id || queue.alias() == queue_id)
    }

    pub fn remove_workflow_prompt_queue(
        &mut self,
        queue_id: &str,
    ) -> Option<WorkflowPromptQueueDefinition> {
        let index = self
            .workflow_prompt_queues
            .iter()
            .position(|queue| queue.id() == queue_id || queue.alias() == queue_id)?;
        Some(self.workflow_prompt_queues.remove(index))
    }

    pub fn enqueue_workflow_prompt(
        &mut self,
        queued_prompt: WorkflowQueuedPrompt,
    ) -> WorkflowQueuedPrompt {
        self.workflow_queued_prompts
            .push_back(queued_prompt.clone());
        queued_prompt
    }

    pub fn update_queued_workflow_prompt(
        &mut self,
        queue_item_id: &str,
        prompt: Option<String>,
        queue_id: Option<String>,
    ) -> Option<WorkflowQueuedPrompt> {
        let queued_prompt = self
            .workflow_queued_prompts
            .iter_mut()
            .find(|item| item.id() == queue_item_id)?;
        if queued_prompt.status() != WorkflowQueuedPromptStatus::Queued {
            return None;
        }
        if let Some(queue_id) = queue_id {
            queued_prompt.set_queue_id(queue_id);
        }
        queued_prompt.set_prompt(prompt);
        Some(queued_prompt.clone())
    }

    pub fn remove_queued_workflow_prompt(
        &mut self,
        queue_item_id: &str,
    ) -> Option<WorkflowQueuedPrompt> {
        let index = self
            .workflow_queued_prompts
            .iter()
            .position(|queued_prompt| queued_prompt.id() == queue_item_id)?;
        if self.workflow_queued_prompts[index].status() != WorkflowQueuedPromptStatus::Queued {
            return None;
        }
        self.workflow_queued_prompts.remove(index)
    }

    pub fn clear_workflow_queue(&mut self, queue_id: &str) -> Vec<WorkflowQueuedPrompt> {
        let mut removed = Vec::new();
        let mut kept = VecDeque::new();
        while let Some(item) = self.workflow_queued_prompts.pop_front() {
            if item.queue_id() == queue_id && item.status() == WorkflowQueuedPromptStatus::Queued {
                removed.push(item);
            } else {
                kept.push_back(item);
            }
        }
        self.workflow_queued_prompts = kept;
        removed
    }

    pub fn pop_next_workflow_queued_prompt(&mut self) -> Option<WorkflowQueuedPrompt> {
        let best = self
            .workflow_queued_prompts
            .iter()
            .enumerate()
            .filter(|(_, item)| item.status() == WorkflowQueuedPromptStatus::Queued)
            .filter_map(|(index, item)| {
                let queue = self.workflow_prompt_queue(item.queue_id())?;
                if !queue.enabled() {
                    return None;
                }
                Some((index, queue.priority(), item.created_at_ms()))
            })
            .min_by_key(|(_, priority, created_at_ms)| (*priority, *created_at_ms))
            .map(|(index, _, _)| index)?;
        let mut item = self.workflow_queued_prompts.remove(best)?;
        item.mark_dispatching();
        Some(item)
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
