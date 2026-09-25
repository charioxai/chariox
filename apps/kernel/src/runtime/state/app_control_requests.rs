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
        let permit = self.app_control().try_admit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.get_app_installation(&check_owner, &check_installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(crate::runtime::app_control::registry_error)?;
        match request {
            LocalDaemonRequest::GetAppWorker(_) => {}
            LocalDaemonRequest::ControlAppWorker(request) => {
                let started = self
                    .control_app_worker(&owner, &installation, request.action)
                    .await?;
                if started {
                    // The durable claim runs on the owner thread after this
                    // returns; report the accepted start, not the stale row.
                    return Ok(LocalDaemonResponse::AppWorker {
                        worker: AppWorkerSummary {
                            installation_id: installation,
                            phase: AppWorkerPhase::Starting,
                            enabled: true,
                            failure: None,
                            updated_at_ms: Some(crate::session::unix_epoch_ms()),
                        },
                    });
                }
            }
            LocalDaemonRequest::ListAppAutomations(_) => {
                let catalog = self.app_catalog(&owner, &installation).await?;
                let owned = self.owned.clone();
                let permit = self.app_control().try_admit()?;
                let automations = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    owned.list_app_automations(&owner, catalog, budget())
                })
                .await
                .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
                .map_err(automation_error)?;
                return Ok(LocalDaemonResponse::AppAutomations {
                    installation_id: installation,
                    automations: automations.iter().map(summary).collect(),
                });
            }
            LocalDaemonRequest::ConfigureAppAutomation(request) => {
                let catalog = self.app_catalog(&owner, &installation).await?;
                let owned = self.owned.clone();
                let permit = self.app_control().try_admit()?;
                let configured = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
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
                .map_err(automation_error)?;
                return Ok(LocalDaemonResponse::AppAutomation {
                    installation_id: installation,
                    automation: summary(&configured),
                });
            }
            LocalDaemonRequest::DisableAppAutomation(request) => {
                let catalog = self.app_catalog(&owner, &installation).await?;
                let owned = self.owned.clone();
                let permit = self.app_control().try_admit()?;
                let disabled = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
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
                .map_err(automation_error)?;
                return Ok(LocalDaemonResponse::AppAutomation {
                    installation_id: installation,
                    automation: summary(&disabled),
                });
            }
            _ => return Err(AppRequestErrorCode::InvalidRequest),
        }
        self.app_worker_summary(owner, installation).await
    }

    /// Returns whether a new start was accepted (an owner thread was spawned).
    async fn control_app_worker(
        &self,
        owner: &str,
        installation: &str,
        action: AppWorkerAction,
    ) -> Result<bool, AppRequestErrorCode> {
        let lifecycle = self.app_control().lifecycle().clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            if matches!(action, AppWorkerAction::Stop | AppWorkerAction::Restart) {
                lifecycle.stop_blocking(&owner, &installation)?;
            }
            if matches!(action, AppWorkerAction::Start | AppWorkerAction::Restart) {
                // An explicit user start re-enables an App after a user stop.
                let disposition = lifecycle.start_active_blocking(&owner, &installation, handle)?;
                return Ok(matches!(
                    disposition,
                    crate::runtime::app_lifecycle::StartDisposition::Starting { .. }
                ));
            }
            Ok::<_, crate::runtime::app_lifecycle::LifecycleError>(false)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|error| {
            use crate::runtime::app_lifecycle::LifecycleError;
            match error {
                LifecycleError::Busy | LifecycleError::LiveLimit => AppRequestErrorCode::Busy,
                LifecycleError::Storage | LifecycleError::Supervisor => {
                    AppRequestErrorCode::StorageUnavailable
                }
                _ => AppRequestErrorCode::Conflict,
            }
        })
    }

    /// The active release's verified catalog. Automations are durable
    /// configuration, so a stopped, failed or never-started App needs no worker.
    async fn app_catalog(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<std::sync::Arc<chariox_app_runtime::app_outbox::EventCatalog>, AppRequestErrorCode>
    {
        use crate::durable_state::app_active_release::ActiveReleaseError;
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        let permit = self.app_control().try_admit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.active_app_event_catalog(&owner, &installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|error| match error {
            ActiveReleaseError::NotActive => AppRequestErrorCode::NotFound,
            ActiveReleaseError::Untrusted | ActiveReleaseError::Invalid => {
                AppRequestErrorCode::Conflict
            }
            ActiveReleaseError::Unavailable | ActiveReleaseError::Storage => {
                AppRequestErrorCode::StorageUnavailable
            }
        })
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
                phase: worker_phase(status.phase, dormant),
                enabled: status.desired_running,
                failure: status.failure,
                updated_at_ms: Some(status.updated_ms),
            },
        };
        Ok(LocalDaemonResponse::AppWorker { worker })
    }
}

/// An idle-stopped worker restarts on use (Dormant); a user stop does not.
fn worker_phase(phase: WorkerPhase, dormant: bool) -> AppWorkerPhase {
    match phase {
        WorkerPhase::Starting => AppWorkerPhase::Starting,
        WorkerPhase::Running => AppWorkerPhase::Running,
        WorkerPhase::Stopped if dormant => AppWorkerPhase::Dormant,
        WorkerPhase::Stopped => AppWorkerPhase::Stopped,
        WorkerPhase::Failed => AppWorkerPhase::Failed,
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

fn automation_error(
    error: crate::durable_state::app_automations::AppAutomationError,
) -> AppRequestErrorCode {
    use crate::durable_state::app_automations::AppAutomationError as E;
    use chariox_app_runtime::app_outbox::OutboxError as O;
    match error {
        E::Stopped(_) => AppRequestErrorCode::Busy,
        E::Outbox(O::Conflict) | E::TargetChanged => AppRequestErrorCode::Conflict,
        E::Outbox(O::NotFound) | E::NotOwner => AppRequestErrorCode::NotFound,
        E::Outbox(O::Limit) => AppRequestErrorCode::LimitExceeded,
        E::Outbox(O::Invalid | O::Schema | O::Catalog(_) | O::Inactive | O::TooOld)
        | E::InvalidTarget => AppRequestErrorCode::InvalidRequest,
        E::Storage(crate::error::DaemonError::SessionNotFound { .. }) => {
            AppRequestErrorCode::NotFound
        }
        E::Outbox(O::Database(_) | O::Corrupt) | E::Storage(_) => {
            AppRequestErrorCode::StorageUnavailable
        }
    }
}

fn failed(code: AppRequestErrorCode) -> LocalDaemonResponse {
    LocalDaemonResponse::AppRequestFailed { code }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_idle_stop_reports_dormant() {
        assert_eq!(
            worker_phase(WorkerPhase::Stopped, true),
            AppWorkerPhase::Dormant
        );
        assert_eq!(
            worker_phase(WorkerPhase::Stopped, false),
            AppWorkerPhase::Stopped
        );
        assert_eq!(
            worker_phase(WorkerPhase::Running, true),
            AppWorkerPhase::Running
        );
        assert_eq!(
            worker_phase(WorkerPhase::Failed, true),
            AppWorkerPhase::Failed
        );
    }
}
