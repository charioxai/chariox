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

    pub(crate) async fn reconcile_browser_controller_environment(
        &self,
        session_id: &str,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        let viewport = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.reconcile", error))?
            .viewport;
        let processes = self.owned.browser_controller_processes.clone();
        let owned_session_id = session_id.to_string();
        let reconciliation = tokio::task::spawn_blocking(move || {
            processes.reconcile_browser(&owned_session_id, &viewport)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "browser_controller.reconcile",
            message: error.to_string(),
        })?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "browser_controller.reconcile",
            message,
        })?;
        let Some(reconciliation) = reconciliation else {
            return self
                .room_environment_snapshot(session_id)
                .map_err(|error| environment_runtime_error("browser_controller.reconcile", error));
        };
        let focused_target_id = reconciliation.browser.focused_target_id.clone();
        let tabs = reconciliation
            .browser
            .tabs
            .into_iter()
            .map(|tab| crate::session::EnvironmentTabObservation {
                runtime_target_id: tab.target_id,
                document_id: tab.document_id,
                url: tab.url,
                title: tab.title,
            })
            .collect();
        self.reconcile_room_environment_controller_tabs(
            session_id,
            tabs,
            focused_target_id.as_deref(),
        )
        .map_err(|error| environment_runtime_error("browser_controller.reconcile", error))
    }

    pub(crate) async fn capture_browser_environment_snapshot(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<
        crate::runtime::browser_controller_snapshot::RoomBrowserStructuredSnapshot,
        DaemonError,
    > {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        let processes = self.owned.browser_controller_processes.clone();
        let owned_session_id = session_id.to_string();
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let controller_snapshot = tokio::task::spawn_blocking(move || {
            processes.capture_browser_snapshot(&owned_session_id, &target_id, &document_id)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "browser_controller.snapshot",
            message: error.to_string(),
        })?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "browser_controller.snapshot",
            message,
        })?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.snapshot",
            message: "browser controller is not enabled".to_string(),
        })?;
        let references = self
            .register_room_environment_element_references(
                session_id,
                tab_id,
                environment.runtime_generation,
                binding.document_revision,
                controller_snapshot.controller_node_refs(),
            )
            .map_err(|error| environment_runtime_error("browser_controller.snapshot", error))?;
        controller_snapshot
            .into_room_snapshot(
                session_id.to_string(),
                environment.environment_id,
                environment.runtime_generation,
                tab_id.to_string(),
                binding.document_revision,
                &references,
            )
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.snapshot",
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
