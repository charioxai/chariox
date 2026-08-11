use serde::{Deserialize, Serialize};

use super::unix_epoch_ms;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProjectKind {
    Default,
    Named,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProjectStatus {
    Active,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionProjectSelection {
    Default,
    Existing { project_id: String },
    New,
}

impl Default for SessionProjectSelection {
    fn default() -> Self {
        Self::Default
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProject {
    id: String,
    owner_user_id: String,
    workspace_id: String,
    name: String,
    kind: RuntimeProjectKind,
    status: RuntimeProjectStatus,
    created_at_ms: u64,
    updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archived_at_ms: Option<u64>,
}

impl RuntimeProject {
    pub(crate) fn new(
        id: impl Into<String>,
        owner_user_id: impl Into<String>,
        workspace_id: impl Into<String>,
        name: impl Into<String>,
        kind: RuntimeProjectKind,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            owner_user_id: owner_user_id.into(),
            workspace_id: workspace_id.into(),
            name: name.into(),
            kind,
            status: RuntimeProjectStatus::Active,
            created_at_ms: now,
            updated_at_ms: now,
            archived_at_ms: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> RuntimeProjectKind {
        self.kind
    }

    pub fn status(&self) -> RuntimeProjectStatus {
        self.status
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    pub fn archived_at_ms(&self) -> Option<u64> {
        self.archived_at_ms
    }

    pub(crate) fn rename(&mut self, name: String) {
        self.name = name;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub(crate) fn archive(&mut self) {
        let now = unix_epoch_ms();
        self.status = RuntimeProjectStatus::Archived;
        self.updated_at_ms = now;
        self.archived_at_ms = Some(now);
    }

    pub(crate) fn restore(&mut self) {
        self.status = RuntimeProjectStatus::Active;
        self.updated_at_ms = unix_epoch_ms();
        self.archived_at_ms = None;
    }
}
