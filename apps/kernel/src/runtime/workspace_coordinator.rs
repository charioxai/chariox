use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceClaimMode {
    Read,
    Write,
}

impl Default for WorkspaceClaimMode {
    fn default() -> Self {
        Self::Write
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceOperationClaimSnapshot {
    pub claim_id: String,
    pub workspace_id: String,
    pub worktree_id: String,
    pub session_id: String,
    pub attachment_id: Option<String>,
    pub operation: String,
    #[serde(default)]
    pub mode: WorkspaceClaimMode,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct WorkspaceCoordinator {
    state: Arc<StdMutex<WorkspaceCoordinatorState>>,
}

#[derive(Debug, Default)]
struct WorkspaceCoordinatorState {
    claims: BTreeMap<String, WorkspaceOperationClaimSnapshot>,
    next_claim_number: u64,
}

#[derive(Debug)]
pub(crate) struct WorkspaceClaimGuard {
    coordinator: WorkspaceCoordinator,
    claim_id: String,
}

impl Drop for WorkspaceClaimGuard {
    fn drop(&mut self) {
        self.coordinator.release_claim(&self.claim_id);
    }
}

impl WorkspaceClaimGuard {
    pub(crate) fn snapshot(&self) -> Option<WorkspaceOperationClaimSnapshot> {
        self.coordinator.claim_snapshot(&self.claim_id)
    }
}

impl WorkspaceCoordinator {
    #[allow(dead_code)]
    pub(crate) fn acquire_worktree_read_claim(
        &self,
        workspace_id: impl Into<String>,
        worktree_id: impl Into<String>,
        session_id: impl Into<String>,
        attachment_id: Option<String>,
        operation: &'static str,
    ) -> Result<WorkspaceClaimGuard, DaemonError> {
        self.acquire_claim(
            workspace_id,
            worktree_id,
            session_id,
            attachment_id,
            operation,
            WorkspaceClaimMode::Read,
        )
    }

    pub(crate) fn acquire_worktree_write_claim(
        &self,
        workspace_id: impl Into<String>,
        worktree_id: impl Into<String>,
        session_id: impl Into<String>,
        attachment_id: Option<String>,
        operation: &'static str,
    ) -> Result<WorkspaceClaimGuard, DaemonError> {
        self.acquire_claim(
            workspace_id,
            worktree_id,
            session_id,
            attachment_id,
            operation,
            WorkspaceClaimMode::Write,
        )
    }

    fn acquire_claim(
        &self,
        workspace_id: impl Into<String>,
        worktree_id: impl Into<String>,
        session_id: impl Into<String>,
        attachment_id: Option<String>,
        operation: &'static str,
        mode: WorkspaceClaimMode,
    ) -> Result<WorkspaceClaimGuard, DaemonError> {
        let workspace_id = workspace_id.into();
        let worktree_id = normalize_worktree_id(worktree_id.into());
        let session_id = session_id.into();
        let mut state = self
            .state
            .lock()
            .expect("workspace coordinator lock should not be poisoned");
        if let Some(conflict) = state.claims.values().find(|claim| {
            claim.workspace_id == workspace_id
                && claim.worktree_id == worktree_id
                && claims_conflict(claim, mode)
        }) {
            return Err(DaemonError::WorkspaceClaimConflict {
                workspace_id,
                worktree_id,
                existing_session_id: conflict.session_id.clone(),
                existing_operation: Box::new(conflict.operation.clone()),
                requested_session_id: session_id,
                requested_operation: Box::new(operation.to_string()),
            });
        }
        state.next_claim_number += 1;
        let claim_id = format!(
            "{}:{}:{}:{}:{}",
            workspace_id, worktree_id, session_id, operation, state.next_claim_number
        );
        state.claims.insert(
            claim_id.clone(),
            WorkspaceOperationClaimSnapshot {
                claim_id: claim_id.clone(),
                workspace_id,
                worktree_id,
                session_id,
                attachment_id,
                operation: operation.to_string(),
                mode,
            },
        );
        Ok(WorkspaceClaimGuard {
            coordinator: self.clone(),
            claim_id,
        })
    }

    pub(crate) fn active_claims(&self) -> Vec<WorkspaceOperationClaimSnapshot> {
        self.state
            .lock()
            .expect("workspace coordinator lock should not be poisoned")
            .claims
            .values()
            .cloned()
            .collect()
    }

    fn claim_snapshot(&self, claim_id: &str) -> Option<WorkspaceOperationClaimSnapshot> {
        self.state
            .lock()
            .expect("workspace coordinator lock should not be poisoned")
            .claims
            .get(claim_id)
            .cloned()
    }

    fn release_claim(&self, claim_id: &str) {
        self.state
            .lock()
            .expect("workspace coordinator lock should not be poisoned")
            .claims
            .remove(claim_id);
    }
}

fn claims_conflict(
    existing: &WorkspaceOperationClaimSnapshot,
    requested_mode: WorkspaceClaimMode,
) -> bool {
    !(existing.mode == WorkspaceClaimMode::Read && requested_mode == WorkspaceClaimMode::Read)
}

fn normalize_worktree_id(worktree_id: String) -> String {
    let path = std::path::Path::new(&worktree_id);
    if path.is_absolute() {
        if let Ok(canonical) = std::fs::canonicalize(path) {
            return canonical.to_string_lossy().to_string();
        }
    }
    worktree_id
}

pub(crate) fn workflow_agent_claim_id(agent_id: &str) -> String {
    format!("workflow-agent:{agent_id}")
}

#[cfg(test)]
mod tests {
    use super::{workflow_agent_claim_id, WorkspaceCoordinator};

    #[test]
    fn workflow_claim_identity_is_scoped_to_the_agent() {
        assert_eq!(workflow_agent_claim_id("agent-1"), "workflow-agent:agent-1");
        assert_ne!(
            workflow_agent_claim_id("agent-1"),
            workflow_agent_claim_id("agent-2")
        );
    }

    #[test]
    fn workflow_claims_queue_the_same_agent_but_allow_different_agents() {
        let coordinator = WorkspaceCoordinator::default();
        let _first = coordinator
            .acquire_worktree_write_claim(
                "workspace",
                workflow_agent_claim_id("agent-1"),
                "session",
                None,
                "workflow_node_dispatch",
            )
            .expect("first agent claim should acquire");
        coordinator
            .acquire_worktree_write_claim(
                "workspace",
                workflow_agent_claim_id("agent-2"),
                "session",
                None,
                "workflow_node_dispatch",
            )
            .expect("another agent should run in parallel");
        coordinator
            .acquire_worktree_write_claim(
                "workspace",
                workflow_agent_claim_id("agent-1"),
                "session",
                None,
                "workflow_node_dispatch",
            )
            .expect_err("the same agent should stay serialized");
    }

    #[test]
    fn rejects_overlapping_worktree_write_claims() {
        let coordinator = WorkspaceCoordinator::default();
        let _claim = coordinator
            .acquire_worktree_write_claim(
                "workspace",
                "worktree",
                "session-1",
                Some("attachment-1".to_string()),
                "file_edit",
            )
            .expect("first claim should acquire");

        let conflict = coordinator
            .acquire_worktree_write_claim(
                "workspace",
                "worktree",
                "session-2",
                Some("attachment-2".to_string()),
                "transfer_store",
            )
            .expect_err("second claim should conflict");
        assert!(conflict.to_string().contains("workspace claim conflict"));
    }

    #[test]
    fn releases_claims_when_guard_drops() {
        let coordinator = WorkspaceCoordinator::default();
        {
            let _claim = coordinator
                .acquire_worktree_write_claim(
                    "workspace",
                    "worktree",
                    "session-1",
                    None,
                    "file_edit",
                )
                .expect("claim should acquire");
            assert_eq!(coordinator.active_claims().len(), 1);
            assert_eq!(
                coordinator.active_claims()[0].mode,
                super::WorkspaceClaimMode::Write
            );
        }

        assert!(coordinator.active_claims().is_empty());
    }

    #[test]
    fn read_claims_share_but_write_claims_conflict() {
        let coordinator = WorkspaceCoordinator::default();
        let _read_1 = coordinator
            .acquire_worktree_read_claim("workspace", "worktree", "session-1", None, "git_status")
            .expect("first read claim should acquire");
        let _read_2 = coordinator
            .acquire_worktree_read_claim("workspace", "worktree", "session-2", None, "tree_read")
            .expect("second read claim should share");

        let conflict = coordinator
            .acquire_worktree_write_claim("workspace", "worktree", "session-3", None, "file_edit")
            .expect_err("write claim should conflict with readers");
        assert!(conflict.to_string().contains("workspace claim conflict"));
    }

    #[test]
    fn write_claims_do_not_block_other_worktrees_or_workspaces() {
        let coordinator = WorkspaceCoordinator::default();
        let _synced_repo_claim = coordinator
            .acquire_worktree_write_claim(
                "workspace",
                "synced-worktree",
                "session-1",
                None,
                "workspace_live_sync_apply",
            )
            .expect("synced worktree claim should acquire");

        coordinator
            .acquire_worktree_write_claim(
                "workspace",
                "other-worktree",
                "session-2",
                None,
                "file_edit",
            )
            .expect("unrelated worktree should not be blocked");
        coordinator
            .acquire_worktree_write_claim(
                "other-workspace",
                "synced-worktree",
                "session-3",
                None,
                "file_edit",
            )
            .expect("same worktree id in another workspace should not be blocked");
    }
}
