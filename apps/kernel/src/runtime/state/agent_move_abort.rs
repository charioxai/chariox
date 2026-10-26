//! Cancellation restores source dispatch only before any target lease can own the move.
use super::*;
use crate::agent_move::peer::{AgentMovePeerRequest as Request, AgentMovePeerResponse as Response};
use crate::agent_move::{error as move_error, AgentMoveIdentity, HomeMovePhase};

impl KernelRuntimeState {
    pub(crate) async fn abort_agent_move(
        &self,
        session_id: &str,
        agent_ref: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.authorize_current_external_command()?;
        let agent =
            self.owned
                .ensure_agent_ref_owner(agent_ref, caller_user_id, "abort agent move")?;
        if agent.session_id() != session_id {
            return Err(move_error("The agent does not belong to this Room"));
        }
        let _operation = self
            .owned
            .agent_moves
            .operation(format!("home:{session_id}:{}", agent.id()))
            .await;
        let mut operation = self
            .owned
            .agent_moves
            .for_agent(session_id, agent.id())
            .ok_or_else(|| move_error("This agent has no Move to abort"))?;
        if matches!(operation.phase, HomeMovePhase::RolledBack { .. }) {
            return self.owned.agent_store.get_agent(agent.id());
        }
        if matches!(operation.phase, HomeMovePhase::Cancelling) {
            return self.finish_agent_move_abort(operation).await;
        }
        if !matches!(
            operation.phase,
            HomeMovePhase::Fenced
                | HomeMovePhase::SourceStopped
                | HomeMovePhase::Captured { .. }
                | HomeMovePhase::Materialized { .. }
        ) {
            return Err(move_error("This Move may already own a target lease. Finish the operation, then Return through a new generation"));
        }
        let current = self.owned.agent_store.get_agent(agent.id())?;
        let source_binding = operation.source_remote.as_ref();
        let same_source = match (source_binding, current.remote_execution()) {
            (None, None) => true,
            (Some(source), Some(current)) => {
                source.execution_lease_id == current.execution_lease_id
                    && source.worker_kernel_id == current.worker_kernel_id
            }
            _ => false,
        };
        if !same_source {
            return Err(move_error(
                "The execution lease changed during Move. Finish that generation before returning",
            ));
        }
        if operation.target_ref.is_some()
            && matches!(operation.phase, HomeMovePhase::Materialized { .. })
        {
            let response = self
                .move_peer_target(
                    &operation,
                    Request::AbortTarget {
                        identity: operation.identity.clone(),
                    },
                )
                .await?;
            if response != Response::TargetAborted {
                return Err(move_error(
                    "Target cancellation is uncertain; reconnect and retry this abort",
                ));
            }
        } else if operation.target_ref.is_none() {
            self.abort_agent_move_target(&operation.identity).await?;
        }
        // Persist cancellation before source dispatch can reopen. Restart must
        // finish cancellation rather than resume the original Move.
        operation.phase = HomeMovePhase::Cancelling;
        let operation = self.owned.agent_moves.save_home(operation)?;
        self.finish_agent_move_abort(operation).await
    }

    pub(super) async fn finish_agent_move_abort(
        &self,
        mut operation: crate::agent_move::HomeMoveOperation,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.release_agent_move_source(&operation, false, true)
            .await?;
        let agent_id = operation.identity.home_agent_id.clone();
        operation.phase = HomeMovePhase::RolledBack {
            reason: "Move cancelled; source execution restored".into(),
        };
        self.owned.agent_moves.save_home(operation)?;
        self.owned.agent_store.get_agent(&agent_id)
    }

    pub(super) async fn abort_agent_move_target(
        &self,
        identity: &AgentMoveIdentity,
    ) -> Result<(), DaemonError> {
        self.with_app_side_effect(|app| app.abort_agent_move_target(identity))
            .await
    }
}
