//! Dispatch bridge drains/replies under ownership, then wait without its lock.
use super::*;
use crate::runtime::browser_controller_app_view::BrowserAppViewRequest;

impl BrowserControllerProcessStore {
    pub(super) fn app_view_bridge(
        &self,
        session_id: &str,
        request: &BrowserAppViewRequest,
    ) -> Result<Option<serde_json::Value>, String> {
        let Some(ownership) = &self.ownership else {
            return Ok(None);
        };
        let (pending, timeout) = {
            let mut ownership = ownership
                .lock()
                .map_err(|_| "browser controller supervisor lock poisoned")?;
            ownership.require_app_view_lease(session_id)?;
            let supervisor = &mut ownership.supervisor;
            let responses_pending = supervisor
                .backend
                .process
                .as_ref()
                .map(|process| process.pending_responses.is_empty().map(|empty| !empty))
                .transpose()?
                .unwrap_or(false);
            let exited = supervisor.backend.take_exited_process()?.is_some();
            if !responses_pending || exited {
                supervisor.ensure_started_without_transparent_restart()?;
            } else if supervisor.recovery_pending
                || supervisor.snapshot.state != BrowserControllerProcessState::Ready
            {
                return Err(CONTROLLER_RESTARTED_BEFORE_OPERATION.into());
            }
            (
                supervisor
                    .backend
                    .begin_observation_request(request.method(), request.params())?,
                supervisor.backend.timeout,
            )
        };
        pending
            .wait(timeout)?
            .into_result(request.method())
            .map(Some)
    }
}
