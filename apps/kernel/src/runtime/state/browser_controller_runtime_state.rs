use crate::error::DaemonError;
use crate::runtime::browser_controller_process::BrowserControllerProcessSnapshot;
use crate::session::{
    EnvironmentComponent, EnvironmentComponentHealthState, EnvironmentError, EnvironmentLifecycle,
    RoomEnvironmentSnapshot,
};

use super::KernelRuntimeState;

impl KernelRuntimeState {
    #[cfg(test)]
    pub(crate) fn set_browser_controller_process_store_for_test(
        &mut self,
        processes: crate::runtime::browser_controller_process::BrowserControllerProcessStore,
    ) {
        self.owned.browser_controller_processes = processes;
    }

    pub(crate) fn browser_controller_process_enabled(&self) -> bool {
        self.owned.browser_controller_processes.is_enabled()
    }

    pub(crate) async fn ensure_browser_controller_process_started(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, DaemonError> {
        let processes = self.owned.browser_controller_processes.clone();
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || processes.acquire(&session_id))
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "browser_controller_process.start",
                message: error.to_string(),
            })?
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller_process.start",
                message,
            })
    }

    pub(crate) async fn stop_browser_controller_process(
        &self,
        session_id: &str,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, DaemonError> {
        let processes = self.owned.browser_controller_processes.clone();
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || processes.release(&session_id))
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "browser_controller_process.stop",
                message: error.to_string(),
            })?
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller_process.stop",
                message,
            })
    }

    pub(crate) async fn stop_managed_room_environment_runtime(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        self.begin_stop_room_environment(session_id)
            .map_err(|error| environment_runtime_error("environment.stop", error))?;
        if let Err(error) = self.stop_browser_controller_process(session_id).await {
            let _ = self.update_room_environment_component_health(
                session_id,
                EnvironmentComponent::BrowserController,
                EnvironmentComponentHealthState::Unavailable,
                Some("controller_stop_failed"),
            );
            let _ = self.transition_room_environment(session_id, EnvironmentLifecycle::Failed);
            return Err(error);
        }
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::BrowserController,
            EnvironmentComponentHealthState::Unavailable,
            None,
        )
        .map_err(|error| environment_runtime_error("environment.stop", error))?;
        self.complete_stop_room_environment(session_id)
            .map_err(|error| environment_runtime_error("environment.stop", error))
    }

    pub(crate) async fn shutdown_browser_controller_process(
        &self,
    ) -> Result<Option<BrowserControllerProcessSnapshot>, DaemonError> {
        let processes = self.owned.browser_controller_processes.clone();
        tokio::task::spawn_blocking(move || processes.shutdown())
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "browser_controller_process.shutdown",
                message: error.to_string(),
            })?
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller_process.shutdown",
                message,
            })
    }
}

fn environment_runtime_error(operation: &'static str, error: EnvironmentError) -> DaemonError {
    match error {
        EnvironmentError::RoomNotFound { session_id } => {
            DaemonError::SessionNotFound { session_id }
        }
        other => DaemonError::LocalTransport {
            operation,
            message: format!("{}: {other:?}", other.code()),
        },
    }
}
