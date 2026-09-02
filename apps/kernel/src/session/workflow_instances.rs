use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEndpointRuntimeInstanceStatus {
    Idle,
    Busy,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEndpointRuntimeInstance {
    id: String,
    workflow_id: String,
    endpoint_id: String,
    workflow_revision: u64,
    ordinal: u16,
    primary: bool,
    node_agent_ids: BTreeMap<String, String>,
    worktree_id: String,
    status: WorkflowEndpointRuntimeInstanceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_run_id: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl WorkflowEndpointRuntimeInstance {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        workflow_revision: u64,
        ordinal: u16,
        primary: bool,
        node_agent_ids: BTreeMap<String, String>,
        worktree_id: impl Into<String>,
    ) -> Self {
        let now_ms = unix_epoch_ms();
        Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            workflow_revision,
            ordinal,
            primary,
            node_agent_ids,
            worktree_id: worktree_id.into(),
            status: WorkflowEndpointRuntimeInstanceStatus::Idle,
            active_run_id: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
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

    pub fn workflow_revision(&self) -> u64 {
        self.workflow_revision
    }

    pub fn ordinal(&self) -> u16 {
        self.ordinal
    }

    pub fn primary(&self) -> bool {
        self.primary
    }

    pub fn node_agent_ids(&self) -> &BTreeMap<String, String> {
        &self.node_agent_ids
    }

    pub fn agent_id_for_node(&self, node_id: &str) -> Option<&str> {
        self.node_agent_ids.get(node_id).map(String::as_str)
    }

    pub fn worktree_id(&self) -> &str {
        &self.worktree_id
    }

    pub fn status(&self) -> WorkflowEndpointRuntimeInstanceStatus {
        self.status
    }

    pub fn active_run_id(&self) -> Option<&str> {
        self.active_run_id.as_deref()
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    pub fn claim(&mut self, run_id: impl Into<String>) -> bool {
        if self.status != WorkflowEndpointRuntimeInstanceStatus::Idle {
            return false;
        }
        self.status = WorkflowEndpointRuntimeInstanceStatus::Busy;
        self.active_run_id = Some(run_id.into());
        self.updated_at_ms = unix_epoch_ms();
        true
    }

    pub fn release(&mut self, run_id: &str) -> bool {
        if self.active_run_id.as_deref() != Some(run_id) {
            return false;
        }
        self.active_run_id = None;
        self.status = WorkflowEndpointRuntimeInstanceStatus::Idle;
        self.updated_at_ms = unix_epoch_ms();
        true
    }

    pub fn mark_stale(&mut self) {
        if self.status == WorkflowEndpointRuntimeInstanceStatus::Busy {
            return;
        }
        self.status = WorkflowEndpointRuntimeInstanceStatus::Stale;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub(crate) fn retarget_workflow_revision(&mut self, workflow_revision: u64) {
        self.workflow_revision = workflow_revision;
        self.updated_at_ms = unix_epoch_ms();
    }
}
