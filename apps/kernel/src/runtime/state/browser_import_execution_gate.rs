use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::session::{ActionCancellationOutcome, EnvironmentActionTerminal, EnvironmentError};

impl KernelRuntimeState {
    /// Only the dispatch waiter that has not started its executor may call this.
    /// Cancellation retires Queued work; work promoted to Running meanwhile can
    /// also be finished here because this waiter still owns its unstarted executor.
    pub(super) fn cancel_unstarted_import_blocked_action(
        &self,
        session_id: &str,
        action_id: &str,
    ) -> Result<(), EnvironmentError> {
        let environment = self.room_environment_snapshot(session_id)?;
        let action = environment
            .actions
            .iter()
            .find(|action| action.action_id == action_id)
            .ok_or_else(|| EnvironmentError::UnknownAction {
                action_id: action_id.into(),
            })?;
        let actor = environment
            .actors
            .iter()
            .find(|actor| actor.actor_id == action.actor_id)
            .ok_or_else(|| EnvironmentError::UnknownActor {
                actor_id: action.actor_id.clone(),
            })?;
        let (outcome, _) =
            self.cancel_room_environment_action_as_actor(session_id, actor.clone(), action_id)?;
        if outcome == ActionCancellationOutcome::CancellationRequested {
            self.finish_room_environment_action(
                session_id,
                action_id,
                EnvironmentActionTerminal::Cancelled,
            )?;
        }
        Ok(())
    }

    pub(crate) fn ensure_browser_import_execution_allowed(
        &self,
        session_id: &str,
    ) -> Result<(), EnvironmentError> {
        match self
            .owned
            .durable_state_store
            .browser_import_pending_for_room(session_id)
        {
            Ok(false) => Ok(()),
            Ok(true) => Err(EnvironmentError::BrowserImportRecoveryRequired),
            Err(_) => Err(EnvironmentError::BrowserImportRecoveryStateUnavailable),
        }
    }
}

pub(super) fn execution_error(error: EnvironmentError) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "browser_import.execute",
        message: error.code().into(),
    }
}
