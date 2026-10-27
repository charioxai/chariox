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
        // A Move that still waits for the turn holds no operation yet; ending its wait cancels it.
        if self
            .owned
            .agent_moves
            .waiting
            .cancel(&super::agent_move_runtime::agent_move_wait_key(
                session_id,
                agent.id(),
            ))
        {
            return self.owned.agent_store.get_agent(agent.id());
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
            self.spawn_agent_move_queue_promotion(session_id, agent.id())?;
            return self.owned.agent_store.get_agent(agent.id());
        }
        if matches!(operation.phase, HomeMovePhase::Cancelling) {
            return self.finish_agent_move_abort(operation).await;
        }
        // A bound remote target is still uncommitted, so failed target setup can be cancelled;
        // the target refuses if it already activated.
        let remote_target_bound = operation.target_ref.is_some()
            && matches!(operation.phase, HomeMovePhase::Bound { .. });
        if !remote_target_bound
            && !matches!(
                operation.phase,
                HomeMovePhase::Fenced
                    | HomeMovePhase::SourceStopped
                    | HomeMovePhase::Captured { .. }
                    | HomeMovePhase::Materialized { .. }
            )
        {
            return Err(move_error("This Move may already own a target lease. Finish the operation, then Return through a new generation"));
        }
        let current = self.owned.agent_store.get_agent(agent.id())?;
        let source_binding = operation.source_remote.as_ref();
        let bound_target = match &operation.phase {
            HomeMovePhase::Bound { remote, .. } => remote.as_ref(),
            _ => None,
        };
        let same_binding = |expected: Option<&crate::agent::RemoteAgentBinding>| match (
            expected,
            current.remote_execution(),
        ) {
            (None, None) => true,
            (Some(expected), Some(current)) => {
                expected.execution_lease_id == current.execution_lease_id
                    && expected.worker_kernel_id == current.worker_kernel_id
            }
            _ => false,
        };
        let on_target = bound_target.is_some() && same_binding(bound_target);
        if !on_target && !same_binding(source_binding) {
            return Err(move_error(
                "The execution lease changed during Move. Finish that generation before returning",
            ));
        }
        if operation.target_ref.is_some()
            && matches!(
                operation.phase,
                HomeMovePhase::Materialized { .. } | HomeMovePhase::Bound { .. }
            )
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
        if on_target {
            self.restore_agent_move_source_route(&operation).await?;
        }
        // Persist cancellation before source dispatch can reopen. Restart must
        // finish cancellation rather than resume the original Move.
        operation.phase = HomeMovePhase::Cancelling;
        let operation = self.owned.agent_moves.save_home(operation)?;
        self.finish_agent_move_abort(operation).await
    }

    /// The target is gone, so the agent returns to the route and workspace it had before binding.
    async fn restore_agent_move_source_route(
        &self,
        operation: &crate::agent_move::HomeMoveOperation,
    ) -> Result<(), DaemonError> {
        self.with_app_side_effect(|app| {
            let mut agent = app.agents().get_agent(&operation.identity.home_agent_id)?;
            agent.set_remote_execution(operation.source_remote.clone());
            app.agents().restore_agent(agent);
            let restored = app.agents().update_agent_config(
                &operation.identity.home_agent_id,
                None,
                None,
                None,
                Some(operation.original_agent.worktree_id().map(String::from)),
            )?;
            app.durable_state_store().append_event(
                "agent.updated",
                Some(restored.id().into()),
                serde_json::json!({"agent": restored, "source": "agent_move_abort"}),
            )?;
            Ok::<_, DaemonError>(())
        })
        .await
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
        let session_id = self
            .owned
            .agent_store
            .get_agent(&agent_id)?
            .session_id()
            .to_string();
        self.spawn_agent_move_queue_promotion(&session_id, &agent_id)?;
        self.owned.agent_store.get_agent(&agent_id)
    }

    pub(super) async fn abort_agent_move_target(
        &self,
        identity: &AgentMoveIdentity,
    ) -> Result<(), DaemonError> {
        self.discard_agent_move_native_probe(identity).await;
        self.with_app_side_effect(|app| app.abort_agent_move_target(identity))
            .await
    }

    /// The retry must find the imported thread as a prefix of the source history, which
    /// may advance before it. Cancelling stays possible when this cannot be done: the
    /// retry then reports the divergence itself.
    async fn discard_agent_move_native_probe(&self, identity: &AgentMoveIdentity) {
        let Some(target) = self.owned.agent_moves.target(&identity.operation_id) else {
            return;
        };
        let (true, false, Some(agent_id)) = (
            target.ready,
            target.active || target.retired,
            target.backing_agent_id.clone(),
        ) else {
            return;
        };
        let result = async {
            let Some(checkpoint) = self.owned.agent_moves.native_checkpoint(&target.capture)?
            else {
                return Ok(());
            };
            let agent = self.owned.agent_store.get_agent(&agent_id)?;
            let request = self
                .agent_move_native_launch_request(&target, &agent)
                .await?;
            tokio::task::spawn_blocking(move || checkpoint.discard_probe_appends(&request))
                .await
                .map_err(|_| move_error("Native target cleanup stopped"))?
                .map_err(crate::provider::NativeCheckpointFailure::into_daemon_error)
        }
        .await;
        if let Err(error) = result {
            crate::logging::warn_with_fields(
                "agent_move.abort",
                "Native target import was left as the probe wrote it",
                serde_json::json!({"operation_id": identity.operation_id, "error": error.to_string()}),
            );
        }
    }
}
