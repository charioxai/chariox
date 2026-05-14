//! Workspace coordination health projection from session snapshots.

use std::collections::BTreeMap;

use crate::runtime::workspace_coordinator::WorkspaceOperationClaimSnapshot;
use crate::session::RuntimeSession;

use super::super::{WorkspaceCoordinationHealthSnapshot, WorktreeClaimSnapshot};

pub(super) fn snapshot(
    sessions: Vec<RuntimeSession>,
    active_operation_claims: Vec<WorkspaceOperationClaimSnapshot>,
) -> WorkspaceCoordinationHealthSnapshot {
    let mut claims_by_worktree: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for session in sessions {
        if session.status() == crate::session::SessionStatus::Ended {
            continue;
        }
        claims_by_worktree
            .entry((
                session.workspace_id().to_string(),
                session.worktree_id().to_string(),
            ))
            .or_default()
            .push(session.id().to_string());
    }

    let mut active_worktree_claims = Vec::new();
    let mut worktree_collisions = Vec::new();
    for ((workspace_id, worktree_id), mut session_ids) in claims_by_worktree {
        session_ids.sort();
        let claim = WorktreeClaimSnapshot {
            workspace_id,
            worktree_id,
            session_ids,
        };
        if claim.session_ids.len() > 1 {
            worktree_collisions.push(claim.clone());
        }
        active_worktree_claims.push(claim);
    }

    WorkspaceCoordinationHealthSnapshot {
        active_worktree_claims,
        worktree_collisions,
        active_operation_claims,
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::projection::SessionStateProjectionStore;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn workspace_coordination_snapshot_reports_worktree_collisions() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (first, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "shared-worktree"))
            .expect("first session should be created");
        let (second, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "shared-worktree"))
            .expect("second session should be created");
        let (other_workspace, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-2", "shared-worktree"))
            .expect("other workspace session should be created");

        let store = SessionStateProjectionStore::default();
        store.update_list(vec![first.clone(), second.clone(), other_workspace]);

        let snapshot = store.workspace_coordination_snapshot(Vec::new());
        assert_eq!(snapshot.active_worktree_claims.len(), 2);
        assert_eq!(snapshot.worktree_collisions.len(), 1);
        assert!(snapshot.active_operation_claims.is_empty());
        assert_eq!(
            snapshot.worktree_collisions[0].session_ids,
            vec![first.id().to_string(), second.id().to_string()]
        );
    }
}
