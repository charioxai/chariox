//! Protocol 345 owner-scoped App worker control and automation configuration.
//! The owner comes from the authenticated caller; requests name only an
//! installation. Automation targets are resolved under workflow ownership.
use super::{app_automation_owned_state::ConfigureAppAutomation, KernelRuntimeState};
use crate::{
    durable_state::app_worker_lifecycle::WorkerPhase,
    local::{
        AppAutomationStatus, AppAutomationSummary, AppRequestErrorCode, AppWorkerAction,
        AppWorkerPhase, AppWorkerSummary, LocalDaemonRequest, LocalDaemonResponse,
    },
    runtime::{app_operation_budget::AppOperationBudget, command::KernelCommand},
};
use chariox_app_runtime::app_outbox::{AutomationConfiguration, AutomationStatus};

impl KernelRuntimeState {
    pub(crate) async fn execute_app_control_request(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<LocalDaemonResponse> {
        let installation = match request {
            LocalDaemonRequest::GetAppWorker(request) => &request.installation_id,
            LocalDaemonRequest::ControlAppWorker(request) => &request.installation_id,
            LocalDaemonRequest::ListAppAutomations(request) => &request.installation_id,
            LocalDaemonRequest::ConfigureAppAutomation(request) => &request.installation_id,
            LocalDaemonRequest::DisableAppAutomation(request) => &request.installation_id,
            _ => return None,
        };
        let owner = match crate::runtime::app_control::owner(command) {
            Ok(owner) => owner,
            Err(code) => return Some(failed(code)),
        };
        if installation.is_empty()
            || installation.len() > 128
            || installation.chars().any(char::is_control)
        {
            return Some(failed(AppRequestErrorCode::InvalidRequest));
        }
        let installation = installation.clone();
        Some(
            self.app_control_response(owner, installation, request.clone())
                .await
                .unwrap_or_else(failed),
        )
    }

    async fn app_control_response(
        &self,
        owner: String,
        installation: String,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        let store = self.owned.durable_state_store.clone();
        let (check_owner, check_installation) = (owner.clone(), installation.clone());
        tokio::task::spawn_blocking(move || {
            store.get_app_installation(&check_owner, &check_installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|_| AppRequestErrorCode::NotFound)?;
        match request {
            LocalDaemonRequest::GetAppWorker(_) => {}
            LocalDaemonRequest::ControlAppWorker(request) => {
                self.control_app_worker(&owner, &installation, request.action)
                    .await?;
            }
            LocalDaemonRequest::ListAppAutomations(_) => {
                let catalog = self.app_catalog(&owner, &installation).await?;
                let owned = self.owned.clone();
                let automations = tokio::task::spawn_blocking(move || {
                    owned.list_app_automations(&owner, catalog, budget())
                })
                .await
                .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
                .map_err(|_| AppRequestErrorCode::Conflict)?;
                return Ok(LocalDaemonResponse::AppAutomations {
                    installation_id: installation,
                    automations: automations.iter().map(summary).collect(),
                });
            }
            LocalDaemonRequest::ConfigureAppAutomation(request) => {
                let catalog = self.app_catalog(&owner, &installation).await?;
                let owned = self.owned.clone();
                let configured = tokio::task::spawn_blocking(move || {
                    owned.configure_app_automation(
                        &owner,
                        catalog,
                        ConfigureAppAutomation {
                            automation_id: request.automation_id,
                            expected_revision: request.expected_revision,
                            event_name: request.event_name,
                            session_id: request.session_id,
                            publication_ref: request.publication_ref,
                            queue_ref: request.queue_ref,
                            scheduled: request.scheduled,
                        },
                        budget(),
                    )
                })
                .await
                .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
                .map_err(|_| AppRequestErrorCode::Conflict)?;
                return Ok(LocalDaemonResponse::AppAutomation {
                    installation_id: installation,
                    automation: summary(&configured),
                });
            }
            LocalDaemonRequest::DisableAppAutomation(request) => {
                let catalog = self.app_catalog(&owner, &installation).await?;
                let owned = self.owned.clone();
                let disabled = tokio::task::spawn_blocking(move || {
                    owned.deactivate_app_automation(
                        &owner,
                        catalog,
                        request.automation_id,
                        request.expected_revision,
                        AutomationStatus::Disabled,
                        budget(),
                    )
                })
                .await
                .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
                .map_err(|_| AppRequestErrorCode::Conflict)?;
                return Ok(LocalDaemonResponse::AppAutomation {
                    installation_id: installation,
                    automation: summary(&disabled),
                });
            }
            _ => return Err(AppRequestErrorCode::InvalidRequest),
        }
        self.app_worker_summary(owner, installation).await
    }

    async fn control_app_worker(
        &self,
        owner: &str,
        installation: &str,
        action: AppWorkerAction,
    ) -> Result<(), AppRequestErrorCode> {
        let lifecycle = self.app_control().lifecycle().clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            if matches!(action, AppWorkerAction::Stop | AppWorkerAction::Restart) {
                lifecycle.stop_blocking(&owner, &installation)?;
            }
            if matches!(action, AppWorkerAction::Start | AppWorkerAction::Restart) {
                // An explicit user start re-enables an App after a user stop.
                lifecycle.start_active_blocking(&owner, &installation, handle)?;
            }
            Ok::<_, crate::runtime::app_lifecycle::LifecycleError>(())
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|error| match error {
            crate::runtime::app_lifecycle::LifecycleError::Busy => AppRequestErrorCode::Busy,
            _ => AppRequestErrorCode::Conflict,
        })
    }

    /// The verified catalog of a live or dormant App, starting it on demand.
    async fn app_catalog(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<std::sync::Arc<chariox_app_runtime::app_outbox::EventCatalog>, AppRequestErrorCode>
    {
        self.app_lease_on_demand(owner, installation)
            .await
            .map(|lease| lease.catalog().clone())
            .map_err(|_| AppRequestErrorCode::Conflict)
    }

    async fn app_worker_summary(
        &self,
        owner: String,
        installation: String,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        let dormant = self.app_control().is_app_dormant(&owner, &installation);
        let store = self.owned.durable_state_store.clone();
        let id = installation.clone();
        let status = tokio::task::spawn_blocking(move || store.app_worker_status(&owner, &id))
            .await
            .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
            .map_err(|_| AppRequestErrorCode::StorageUnavailable)?;
        let worker = match status {
            None => AppWorkerSummary {
                installation_id: installation,
                phase: AppWorkerPhase::NotStarted,
                enabled: true,
                failure: None,
                updated_at_ms: None,
            },
            Some(status) => AppWorkerSummary {
                installation_id: installation,
                phase: match status.phase {
                    WorkerPhase::Starting => AppWorkerPhase::Starting,
                    WorkerPhase::Running => AppWorkerPhase::Running,
                    WorkerPhase::Stopped if dormant => AppWorkerPhase::Dormant,
                    WorkerPhase::Stopped => AppWorkerPhase::Stopped,
                    WorkerPhase::Failed => AppWorkerPhase::Failed,
                },
                enabled: status.desired_running,
                failure: status.failure,
                updated_at_ms: Some(status.updated_ms),
            },
        };
        Ok(LocalDaemonResponse::AppWorker { worker })
    }
}

fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}

fn summary(value: &AutomationConfiguration) -> AppAutomationSummary {
    AppAutomationSummary {
        automation_id: value.automation_id.clone(),
        revision: value.revision,
        event_name: value.event_name.clone(),
        event_version: value.event_version,
        session_id: value.target.session_id.clone(),
        publication_id: value.target.publication_id.clone(),
        endpoint_id: value.target.endpoint_id.clone(),
        queue_id: value.target.queue_id.clone(),
        scheduled: value.scheduled,
        status: match value.status {
            AutomationStatus::Active => AppAutomationStatus::Active,
            AutomationStatus::Paused => AppAutomationStatus::Paused,
            AutomationStatus::Broken => AppAutomationStatus::Broken,
            AutomationStatus::Disabled => AppAutomationStatus::Disabled,
        },
    }
}

fn failed(code: AppRequestErrorCode) -> LocalDaemonResponse {
    LocalDaemonResponse::AppRequestFailed { code }
}
