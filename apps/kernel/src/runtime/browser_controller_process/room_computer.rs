//! MP-08 / MP-10 / MP-11: same Room-owned controller and native AT-SPI backend.
use super::*;
impl BrowserControllerProcessStore {
    pub(crate) fn room_computer_request(
        &self,
        session_id: &str,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let ownership = self
            .ownership
            .as_ref()
            .ok_or("Room controller unavailable")?;
        let (pending, timeout) = {
            let mut ownership = ownership
                .lock()
                .map_err(|_| "Room controller lock unavailable")?;
            self.authorize()?;
            ownership.require_lease(session_id)?;
            let supervisor = &mut ownership.supervisor;
            supervisor.prepare_unlocked_request()?;
            (
                supervisor
                    .backend
                    .begin_observation_request(method, params)?,
                supervisor.backend.timeout,
            )
        };
        pending.wait(timeout)?.into_result(method)
    }
    pub(crate) fn room_computer_action(
        &self,
        session_id: &str,
        params: serde_json::Value,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Result<serde_json::Value, crate::error::HostFailure> {
        use crate::error::HostFailure;
        let dispatch = || -> Result<BrowserControllerRpcResponse, String> {
            let ownership = self
                .ownership
                .as_ref()
                .ok_or("Room controller unavailable")?;
            let signal = cancellation::CancellationSignal::for_authority(
                Arc::new(cancellation::CancellationSignal::default()),
                move || !cancelled(),
            );
            let pending = {
                let mut ownership = ownership
                    .lock()
                    .map_err(|_| "Room controller lock unavailable")?;
                self.authorize()?;
                ownership.require_lease(session_id)?;
                let supervisor = &mut ownership.supervisor;
                supervisor.prepare_unlocked_request()?;
                supervisor.backend.begin_cancellable_mutation(
                    "computer.target_action",
                    &params,
                    &signal,
                )?
            };
            pending.wait(&signal)
        };
        let response = dispatch().map_err(HostFailure::Other)?;
        if !response.ok {
            if let Some(reason) = response
                .error
                .as_ref()
                .and_then(|error| crate::error::UserDomainRefusalReason::from_code(&error.code))
            {
                return Err(HostFailure::Refused(reason));
            }
        }
        response
            .into_result("computer.target_action")
            .map_err(HostFailure::Other)
    }
}
