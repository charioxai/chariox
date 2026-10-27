//! Shared liveness/fencing gate for requests that wait outside ownership.
use super::*;

impl BrowserControllerProcessSupervisor<BrowserControllerProcessStdioBackend> {
    pub(super) fn prepare_unlocked_request(&mut self) -> Result<(), String> {
        // Health is a controller barrier. While requests are pending, inspect
        // process liveness without queuing health; recovery still requires
        // reconciliation before any fresh request can be dispatched.
        let pending = self
            .backend
            .process
            .as_ref()
            .map(|process| process.pending_responses.is_empty().map(|empty| !empty))
            .transpose()?
            .unwrap_or(false);
        let exited = self.backend.take_exited_process()?.is_some();
        if !pending || exited {
            self.ensure_started_without_transparent_restart()?;
        } else if self.recovery_pending
            || self.snapshot.state != BrowserControllerProcessState::Ready
        {
            return Err(CONTROLLER_RESTARTED_BEFORE_OPERATION.into());
        }
        Ok(())
    }
}
