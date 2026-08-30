use crate::error::DaemonError;
use crate::runtime::browser_controller_event::{RoomBrowserEvent, RoomBrowserEventBatch};
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

    pub(crate) async fn finish_room_environment_controller_start(
        &self,
        session_id: &str,
        operation: &'static str,
    ) -> Result<RoomEnvironmentSnapshot, DaemonError> {
        if !self.browser_controller_process_enabled() {
            return self
                .room_environment_snapshot(session_id)
                .map_err(|error| environment_runtime_error(operation, error));
        }
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::BrowserController,
            EnvironmentComponentHealthState::Starting,
            None,
        )
        .map_err(|error| environment_runtime_error(operation, error))?;
        if let Err(error) = self
            .ensure_browser_controller_process_started(session_id)
            .await
        {
            let _ = self.update_room_environment_component_health(
                session_id,
                EnvironmentComponent::BrowserController,
                EnvironmentComponentHealthState::Unavailable,
                Some("controller_start_failed"),
            );
            let _ = self.transition_room_environment(session_id, EnvironmentLifecycle::Failed);
            return Err(error);
        }
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::BrowserController,
            EnvironmentComponentHealthState::Ready,
            None,
        )
        .map_err(|error| environment_runtime_error(operation, error))?;
        self.update_room_environment_component_health(
            session_id,
            EnvironmentComponent::Browser,
            EnvironmentComponentHealthState::Starting,
            None,
        )
        .map_err(|error| environment_runtime_error(operation, error))?;
        match self
            .reconcile_browser_controller_environment(session_id)
            .await
        {
            Ok(_) => self
                .update_room_environment_component_health(
                    session_id,
                    EnvironmentComponent::Browser,
                    EnvironmentComponentHealthState::Ready,
                    None,
                )
                .map_err(|error| environment_runtime_error(operation, error)),
            Err(error) => {
                let _ = self.update_room_environment_component_health(
                    session_id,
                    EnvironmentComponent::Browser,
                    EnvironmentComponentHealthState::Unavailable,
                    Some("browser_reconcile_failed"),
                );
                let _ = self.transition_room_environment(session_id, EnvironmentLifecycle::Failed);
                Err(error)
            }
        }
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

    pub(crate) async fn perform_browser_environment_locator_action(
        &self,
        session_id: &str,
        element_ref: &str,
        action: crate::runtime::browser_controller_action::BrowserLocatorAction,
        timeout_ms: u64,
    ) -> Result<crate::runtime::browser_controller_action::RoomBrowserActionResult, DaemonError>
    {
        action
            .validate()
            .and_then(|_| {
                crate::runtime::browser_controller_action::validate_browser_action_timeout(
                    timeout_ms,
                )
            })
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.action",
                message,
            })?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.action", error))?;
        let element = self
            .resolve_room_environment_element_reference(session_id, element_ref)
            .map_err(|error| environment_runtime_error("browser_controller.action", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, &element.tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.action", error))?;
        if element.runtime_generation != environment.runtime_generation
            || element.document_revision != binding.document_revision
        {
            return Err(DaemonError::LocalTransport {
                operation: "browser_controller.action",
                message: "browser element reference became stale before dispatch".to_string(),
            });
        }

        let processes = self.owned.browser_controller_processes.clone();
        let owned_session_id = session_id.to_string();
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let controller_node_ref = element.controller_node_ref.clone();
        let controller_action = action.clone();
        let result = tokio::task::spawn_blocking(move || {
            processes.perform_browser_action(
                &owned_session_id,
                &target_id,
                &document_id,
                &controller_node_ref,
                &controller_action,
                timeout_ms,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "browser_controller.action",
            message: error.to_string(),
        })?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "browser_controller.action",
            message,
        })?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.action",
            message: "browser controller is not enabled".to_string(),
        })?;

        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            element.tab_id,
            element.document_revision,
            element_ref.to_string(),
        ))
    }

    pub(crate) async fn handle_browser_environment_dialog(
        &self,
        session_id: &str,
        tab_id: &str,
        action: crate::runtime::browser_controller_action::BrowserDialogAction,
    ) -> Result<crate::runtime::browser_controller_action::RoomBrowserDialogResult, DaemonError>
    {
        action
            .validate()
            .map_err(|message| DaemonError::LocalTransport {
                operation: "browser_controller.dialog",
                message,
            })?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.dialog", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.dialog", error))?;
        let processes = self.owned.browser_controller_processes.clone();
        let owned_session_id = session_id.to_string();
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let controller_action = action.clone();
        let result = tokio::task::spawn_blocking(move || {
            processes.handle_browser_dialog(
                &owned_session_id,
                &target_id,
                &document_id,
                &controller_action,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "browser_controller.dialog",
            message: error.to_string(),
        })?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "browser_controller.dialog",
            message,
        })?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.dialog",
            message: "browser controller is not enabled".to_string(),
        })?;
        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            tab_id.to_string(),
            binding.document_revision,
        ))
    }

    pub(crate) async fn configure_browser_environment_downloads(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<
        crate::runtime::browser_controller_file_transfer::RoomBrowserDownloadsResult,
        DaemonError,
    > {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.downloads", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.downloads", error))?;
        let processes = self.owned.browser_controller_processes.clone();
        let owned_session_id = session_id.to_string();
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            processes.configure_browser_downloads(&owned_session_id, &target_id, &document_id)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "browser_controller.downloads",
            message: error.to_string(),
        })?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "browser_controller.downloads",
            message,
        })?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.downloads",
            message: "browser controller is not enabled".to_string(),
        })?;
        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            tab_id.to_string(),
            binding.document_revision,
        ))
    }

    pub(crate) async fn upload_browser_environment_files(
        &self,
        session_id: &str,
        element_ref: &str,
        paths: Vec<std::path::PathBuf>,
    ) -> Result<
        crate::runtime::browser_controller_file_transfer::RoomBrowserUploadResult,
        DaemonError,
    > {
        let files =
            crate::runtime::browser_controller_file_transfer::BrowserUploadFiles::new(paths)
                .map_err(|message| DaemonError::LocalTransport {
                    operation: "browser_controller.upload",
                    message,
                })?;
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.upload", error))?;
        let element = self
            .resolve_room_environment_element_reference(session_id, element_ref)
            .map_err(|error| environment_runtime_error("browser_controller.upload", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, &element.tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.upload", error))?;
        if element.runtime_generation != environment.runtime_generation
            || element.document_revision != binding.document_revision
        {
            return Err(DaemonError::LocalTransport {
                operation: "browser_controller.upload",
                message: "browser element reference became stale before upload".to_string(),
            });
        }
        let processes = self.owned.browser_controller_processes.clone();
        let owned_session_id = session_id.to_string();
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let controller_node_ref = element.controller_node_ref.clone();
        let result = tokio::task::spawn_blocking(move || {
            processes.upload_browser_files(
                &owned_session_id,
                &target_id,
                &document_id,
                &controller_node_ref,
                &files,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "browser_controller.upload",
            message: error.to_string(),
        })?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "browser_controller.upload",
            message,
        })?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.upload",
            message: "browser controller is not enabled".to_string(),
        })?;
        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            element.tab_id,
            element.document_revision,
            element_ref.to_string(),
        ))
    }

    pub(crate) async fn set_browser_environment_permission(
        &self,
        session_id: &str,
        tab_id: &str,
        permission: crate::runtime::browser_controller_permission::BrowserPermissionName,
        setting: crate::runtime::browser_controller_permission::BrowserPermissionSetting,
    ) -> Result<
        crate::runtime::browser_controller_permission::RoomBrowserPermissionResult,
        DaemonError,
    > {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.permission", error))?;
        let binding = self
            .room_environment_controller_tab_binding(session_id, tab_id)
            .map_err(|error| environment_runtime_error("browser_controller.permission", error))?;
        let processes = self.owned.browser_controller_processes.clone();
        let owned_session_id = session_id.to_string();
        let target_id = binding.runtime_target_id.clone();
        let document_id = binding.document_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            processes.set_browser_permission(
                &owned_session_id,
                &target_id,
                &document_id,
                permission,
                setting,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "browser_controller.permission",
            message: error.to_string(),
        })?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "browser_controller.permission",
            message,
        })?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.permission",
            message: "browser controller is not enabled".to_string(),
        })?;
        Ok(result.into_room_result(
            session_id.to_string(),
            environment.environment_id,
            environment.runtime_generation,
            tab_id.to_string(),
            binding.document_revision,
        ))
    }

    pub(crate) async fn poll_browser_environment_events(
        &self,
        session_id: &str,
        browser_generation: u64,
        cursor: u64,
        limit: u16,
    ) -> Result<RoomBrowserEventBatch, DaemonError> {
        let environment = self
            .room_environment_snapshot(session_id)
            .map_err(|error| environment_runtime_error("browser_controller.events", error))?;
        let mut tab_ids_by_target = std::collections::BTreeMap::new();
        for tab in &environment.tabs {
            let binding = self
                .room_environment_controller_tab_binding(session_id, &tab.tab_id)
                .map_err(|error| environment_runtime_error("browser_controller.events", error))?;
            tab_ids_by_target.insert(binding.runtime_target_id, tab.tab_id.clone());
        }
        let processes = self.owned.browser_controller_processes.clone();
        let owned_session_id = session_id.to_string();
        let batch = tokio::task::spawn_blocking(move || {
            processes.poll_browser_events(&owned_session_id, browser_generation, cursor, limit)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "browser_controller.events",
            message: error.to_string(),
        })?
        .map_err(|message| DaemonError::LocalTransport {
            operation: "browser_controller.events",
            message,
        })?
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "browser_controller.events",
            message: "browser controller is not enabled".to_string(),
        })?;
        let events = batch
            .events
            .into_iter()
            .filter_map(|event| {
                let tab_id = match event.target_id.as_deref() {
                    Some(target_id) => Some(tab_ids_by_target.get(target_id)?.clone()),
                    None if matches!(
                        event.kind.as_str(),
                        "browser_connected" | "browser_disconnected"
                    ) =>
                    {
                        None
                    }
                    None => return None,
                };
                Some(RoomBrowserEvent {
                    event_id: event.event_id,
                    kind: event.kind,
                    tab_id,
                    document_id: event.document_id,
                    data: event.data,
                })
            })
            .collect();
        Ok(RoomBrowserEventBatch {
            browser_generation: batch.browser_generation,
            events,
            next_cursor: batch.next_cursor,
            replay_gap: batch.replay_gap,
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
