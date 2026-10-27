use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeIsolationMode {
    SharedSession,
    IsolatedBranch,
    IsolatedWorktree,
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
