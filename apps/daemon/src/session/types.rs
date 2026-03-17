use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSessionRequest {
    pub workspace_id: String,
    pub worktree_id: String,
}

impl CreateSessionRequest {
    pub fn new(workspace_id: impl Into<String>, worktree_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            worktree_id: worktree_id.into(),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Created,
    Active,
    Parked,
    Ended,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SessionExecutionMode {
    SingleAgent,
    MultiAgentWorkflow,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PromptStatus {
    Queued,
    Running,
    Completed,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SchedulerState {
    Idle,
    Runnable,
    Running,
    Waiting,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptQueueItem {
    id: String,
    source_attachment_id: String,
    prompt: String,
    status: PromptStatus,
}

impl PromptQueueItem {
    pub fn new(
        id: impl Into<String>,
        source_attachment_id: impl Into<String>,
        prompt: impl Into<String>,
        status: PromptStatus,
    ) -> Self {
        Self {
            id: id.into(),
            source_attachment_id: source_attachment_id.into(),
            prompt: prompt.into(),
            status,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn source_attachment_id(&self) -> &str {
        &self.source_attachment_id
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn status(&self) -> PromptStatus {
        self.status
    }

    pub fn set_status(&mut self, status: PromptStatus) {
        self.status = status;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptSubmissionOutcome {
    Started { prompt: PromptQueueItem },
    Queued { prompt: PromptQueueItem },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptCompletion {
    pub completed: PromptQueueItem,
    pub started_next: Option<PromptQueueItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptDetachEffect {
    pub removed_active_prompt: bool,
    pub removed_queued_prompt_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSession {
    id: String,
    workspace_id: String,
    worktree_id: String,
    host_machine_id: String,
    host_daemon_id: String,
    execution_mode: SessionExecutionMode,
    status: SessionStatus,
    active_provider_run_id: Option<String>,
    attachment_ids: BTreeSet<String>,
    active_prompt: Option<PromptQueueItem>,
    queued_prompts: VecDeque<PromptQueueItem>,
    scheduler_state: SchedulerState,
    config_state: SessionConfigState,
    worktree_assignments: Vec<RuntimeWorktreeAssignment>,
}

impl RuntimeSession {
    pub fn new(
        id: impl Into<String>,
        workspace_id: impl Into<String>,
        worktree_id: impl Into<String>,
        host_machine_id: impl Into<String>,
        host_daemon_id: impl Into<String>,
    ) -> Self {
        let id = id.into();
        let worktree_id = worktree_id.into();

        Self {
            id: id.clone(),
            workspace_id: workspace_id.into(),
            worktree_id: worktree_id.clone(),
            host_machine_id: host_machine_id.into(),
            host_daemon_id: host_daemon_id.into(),
            execution_mode: SessionExecutionMode::SingleAgent,
            status: SessionStatus::Created,
            active_provider_run_id: None,
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
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
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
    pub fn status(&self) -> SessionStatus {
        self.status
    }
    pub fn execution_mode(&self) -> SessionExecutionMode {
        self.execution_mode
    }
    pub fn active_provider_run_id(&self) -> Option<&str> {
        self.active_provider_run_id.as_deref()
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
            _ => false,
        };

        if !allowed {
            return false;
        }

        self.status = next;

        if next == SessionStatus::Ended {
            self.active_provider_run_id = None;
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
