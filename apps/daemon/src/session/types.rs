use std::collections::BTreeSet;
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
    controller_attachment_id: Option<String>,
}

impl RuntimeSession {
    pub fn new(
        id: impl Into<String>,
        workspace_id: impl Into<String>,
        worktree_id: impl Into<String>,
        host_machine_id: impl Into<String>,
        host_daemon_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            workspace_id: workspace_id.into(),
            worktree_id: worktree_id.into(),
            host_machine_id: host_machine_id.into(),
            host_daemon_id: host_daemon_id.into(),
            execution_mode: SessionExecutionMode::SingleAgent,
            status: SessionStatus::Created,
            active_provider_run_id: None,
            attachment_ids: BTreeSet::new(),
            controller_attachment_id: None,
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

    pub fn controller_attachment_id(&self) -> Option<&str> {
        self.controller_attachment_id.as_deref()
    }

    pub fn has_attachment(&self, attachment_id: &str) -> bool {
        self.attachment_ids.contains(attachment_id)
    }

    pub fn add_attachment(&mut self, attachment_id: impl Into<String>) {
        self.attachment_ids.insert(attachment_id.into());
    }

    pub fn remove_attachment(&mut self, attachment_id: &str) -> bool {
        let removed = self.attachment_ids.remove(attachment_id);

        if removed && self.controller_attachment_id.as_deref() == Some(attachment_id) {
            self.controller_attachment_id = None;
        }

        removed
    }

    pub fn assign_controller(&mut self, attachment_id: &str) -> Option<String> {
        let previous = self.controller_attachment_id.clone();
        self.controller_attachment_id = Some(attachment_id.to_owned());
        previous
    }

    pub fn release_controller(&mut self) -> Option<String> {
        self.controller_attachment_id.take()
    }

    pub fn set_active_provider_run(&mut self, provider_run_id: Option<String>) {
        self.active_provider_run_id = provider_run_id;
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
            self.controller_attachment_id = None;
            self.attachment_ids.clear();
        }

        true
    }
}
