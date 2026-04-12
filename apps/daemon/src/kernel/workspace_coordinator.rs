use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex};

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceOperationClaimSnapshot {
    pub claim_id: String,
    pub workspace_id: String,
    pub worktree_id: String,
    pub session_id: String,
    pub attachment_id: Option<String>,
    pub operation: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct WorkspaceCoordinator {
    state: Arc<StdMutex<WorkspaceCoordinatorState>>,
}

#[derive(Debug, Default)]
struct WorkspaceCoordinatorState {
    claims: BTreeMap<String, WorkspaceOperationClaimSnapshot>,
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

impl WorkspaceCoordinator {
    pub(crate) fn acquire_worktree_write_claim(
        &self,
        workspace_id: impl Into<String>,
        worktree_id: impl Into<String>,
        session_id: impl Into<String>,
        attachment_id: Option<String>,
        operation: &'static str,
    ) -> Result<WorkspaceClaimGuard, DaemonError> {
        let workspace_id = workspace_id.into();
        let worktree_id = worktree_id.into();
        let session_id = session_id.into();
        let claim_id = format!(
            "{}:{}:{}:{}",
            workspace_id, worktree_id, session_id, operation
        );
        let mut state = self
            .state
            .lock()
            .expect("workspace coordinator lock should not be poisoned");
        if let Some(conflict) = state
            .claims
            .values()
            .find(|claim| claim.workspace_id == workspace_id && claim.worktree_id == worktree_id)
        {
            return Err(DaemonError::WorkspaceClaimConflict {
                workspace_id,
                worktree_id,
                existing_session_id: conflict.session_id.clone(),
                existing_operation: conflict.operation.clone(),
                requested_session_id: session_id,
                requested_operation: operation.to_string(),
            });
        }
        state.claims.insert(
            claim_id.clone(),
            WorkspaceOperationClaimSnapshot {
                claim_id: claim_id.clone(),
                workspace_id,
                worktree_id,
                session_id,
                attachment_id,
                operation: operation.to_string(),
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

    fn release_claim(&self, claim_id: &str) {
        self.state
            .lock()
            .expect("workspace coordinator lock should not be poisoned")
            .claims
            .remove(claim_id);
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceCoordinator;

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
        }

        assert!(coordinator.active_claims().is_empty());
    }
}
