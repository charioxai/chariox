//! Protocol 345 owner-scoped App worker control and automation configuration.
//! The owner comes from the authenticated caller; requests name only an
//! installation. Automation targets are resolved under workflow ownership.
use super::{app_automation_owned_state::ConfigureAppAutomation, KernelRuntimeState};
use crate::{
    durable_state::app_worker_lifecycle::{StartGate, WorkerPhase, WorkerStatus},
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
        if let LocalDaemonRequest::AcceptAppHostAction(request) = request {
            use crate::runtime::command::{KernelCallerKind, KernelCommandSource};
            if !matches!(
                command.source,
                KernelCommandSource::LocalCli
                    | KernelCommandSource::LocalIpc
                    | KernelCommandSource::RelayClient
            ) || !matches!(
                command.caller.caller_kind,
                KernelCallerKind::LocalClient | KernelCallerKind::RemoteClient
            ) {
                return Some(failed(AppRequestErrorCode::Unauthorized));
            }
            return Some(match crate::runtime::app_control::owner(command) {
                Ok(owner) => self.accept_app_host_action(owner, request.clone()).await,
                Err(code) => failed(code),
            });
        }
        if let LocalDaemonRequest::RestoreAppDataSnapshot(request) = request {
            let owner = match crate::runtime::app_control::owner(command) {
                Ok(owner) => owner,
                Err(code) => return Some(failed(code)),
            };
            let lifecycle = self.app_control().lifecycle().clone();
            let request = request.clone();
            let response = LocalDaemonResponse::AppDataSnapshotRestored {
                installation_id: request.installation_id.clone(),
                generation: request.expected_generation.clone(),
                snapshot_id: request.snapshot_id.clone(),
            };
            return Some(
                match tokio::task::spawn_blocking(move || {
                    lifecycle.restore_snapshot_blocking(&owner, &request)
                })
                .await
                {
                    Ok(Ok(())) => response,
                    Ok(Err(code)) => failed(code),
                    Err(_) => failed(AppRequestErrorCode::StorageUnavailable),
                },
            );
        }
        if let LocalDaemonRequest::GrantAppFile(request) = request {
            return Some(match crate::runtime::app_control::owner(command) {
                Ok(owner) => self.grant_app_file(owner, request.clone()).await,
                Err(code) => failed(code),
            });
        }
        if let LocalDaemonRequest::PreviewDeploymentApps(request) = request {
            return Some(self.preview_deployment_apps(command, request).await);
        }
        if let LocalDaemonRequest::PrepareDeploymentApps(request) = request {
            return Some(self.prepare_deployment_apps(command, request).await);
        }
        if let LocalDaemonRequest::GetAppSet(_) = request {
            // Boxed: the App set is read through this same dispatch.
            return Some(Box::pin(self.app_set(command)).await);
        }
        if let LocalDaemonRequest::SaveAppFileExport(request) = request {
            return Some(match crate::runtime::app_control::owner(command) {
                Ok(owner) => self.save_app_file_export(owner, request.clone()).await,
                Err(code) => failed(code),
            });
        }
        let installation = match request {
            LocalDaemonRequest::GetAppWorker(request) => &request.installation_id,
            LocalDaemonRequest::ControlAppWorker(request) => &request.installation_id,
            LocalDaemonRequest::ListAppAutomations(request) => &request.installation_id,
            LocalDaemonRequest::ConfigureAppAutomation(request) => &request.installation_id,
            LocalDaemonRequest::DisableAppAutomation(request) => &request.installation_id,
            LocalDaemonRequest::UninstallApp(request) => &request.installation_id,
            LocalDaemonRequest::GetAppLogs(request) => &request.installation_id,
            LocalDaemonRequest::CreateAppInboxRoute(request) => &request.installation_id,
            LocalDaemonRequest::RemoveAppInboxRoute(request) => &request.installation_id,
            LocalDaemonRequest::ListAppInboxRoutes(request) => &request.installation_id,
            LocalDaemonRequest::TestAppInboxRoute(request) => &request.installation_id,
            LocalDaemonRequest::GrantAppConnection(request) => &request.installation_id,
            LocalDaemonRequest::RevokeAppConnection(request) => &request.installation_id,
            LocalDaemonRequest::ListAppConnections(request) => &request.installation_id,
            LocalDaemonRequest::RevokeAppFileGrants(request) => &request.installation_id,
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
        if matches!(
            request,
            LocalDaemonRequest::ControlAppWorker(_) | LocalDaemonRequest::UninstallApp(_)
        ) {
            if let Err(code) = self
                .app_control()
                .require_owned_installation(&owner, &installation, &command.command_id)
                .await
            {
                return Some(failed(code));
            }
            let state = self.clone();
            let input = request.clone();
            let execute_owner = owner.clone();
            return Some(
                self.app_control()
                    .execute_once(&owner, command, request, move || async move {
                        state
                            .app_control_response(execute_owner, installation, input)
                            .await
                            .unwrap_or_else(failed)
                    })
                    .await,
            );
        }
        Some(
            self.app_control_response(owner, installation, request.clone())
                .await
                .unwrap_or_else(failed),
        )
    }

    pub(super) async fn app_control_response(
        &self,
        owner: String,
        installation: String,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        let store = self.owned.durable_state_store.clone();
        let (check_owner, check_installation) = (owner.clone(), installation.clone());
        let permit = self.app_control().try_admit()?;
        let current = tokio::task::spawn_blocking(move || {
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
                            delivery_mode: request.delivery_mode,
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
            LocalDaemonRequest::GetAppLogs(request) => {
                let after = match request.after_sequence.as_deref() {
                    Some(value) => value
                        .parse::<u64>()
                        .map_err(|_| AppRequestErrorCode::InvalidRequest)?,
                    None => 0,
                };
                let limit = usize::from(request.limit.unwrap_or(100));
                let store = self.owned.durable_state_store.clone();
                let (log_owner, log_installation) = (owner.clone(), installation.clone());
                let permit = self.app_control().try_admit()?;
                let entries = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    store.app_logs(&log_owner, &log_installation, after, limit)
                })
                .await
                .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
                .map_err(|_| AppRequestErrorCode::StorageUnavailable)?;
                return Ok(LocalDaemonResponse::AppLogs {
                    installation_id: installation,
                    entries: entries
                        .into_iter()
                        .map(|entry| crate::local::AppLogEntrySummary {
                            sequence: entry.sequence.to_string(),
                            at_ms: entry.at_ms,
                            level: entry.level,
                            message: entry.message,
                            fields: entry.fields,
                        })
                        .collect(),
                });
            }
            request @ (LocalDaemonRequest::CreateAppInboxRoute(_)
            | LocalDaemonRequest::RemoveAppInboxRoute(_)
            | LocalDaemonRequest::ListAppInboxRoutes(_)
            | LocalDaemonRequest::TestAppInboxRoute(_)) => {
                return self.app_inbox_request(owner, installation, request).await;
            }
            request @ (LocalDaemonRequest::GrantAppConnection(_)
            | LocalDaemonRequest::RevokeAppConnection(_)
            | LocalDaemonRequest::ListAppConnections(_)) => {
                return self
                    .app_connection_request(owner, installation, request)
                    .await;
            }
            LocalDaemonRequest::RevokeAppFileGrants(request) => {
                return self
                    .revoke_app_file_grants(owner, installation, request)
                    .await;
            }
            LocalDaemonRequest::UninstallApp(request) => {
                return self
                    .uninstall_app(
                        owner,
                        installation,
                        current.generation,
                        &request.expected_generation,
                        request.delete_data,
                    )
                    .await;
            }
            _ => return Err(AppRequestErrorCode::InvalidRequest),
        }
        self.app_worker_summary(owner, installation).await
    }

    /// The generation is checked before any side effect, so a stale request
    /// changes nothing. Then a user stop (which also withdraws the dormant
    /// catalog, so agents stop seeing its tools) runs before deactivation, so
    /// no App code runs once the installation is inactive. An update committed
    /// between that stop and the deactivation makes the final fence fail with
    /// `Conflict` and leaves the App stopped until an explicit start.
    async fn uninstall_app(
        &self,
        owner: String,
        installation: String,
        current_generation: u64,
        expected_generation: &str,
        delete_data: bool,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        let expected: u64 = expected_generation
            .parse()
            .map_err(|_| AppRequestErrorCode::InvalidRequest)?;
        if expected != current_generation {
            return Err(AppRequestErrorCode::Conflict);
        }
        let lifecycle = self.app_control().lifecycle().clone();
        let (stop_owner, stop_installation) = (owner.clone(), installation.clone());
        let _operation = tokio::task::spawn_blocking(move || {
            lifecycle.begin_uninstall_blocking(
                &stop_owner,
                &stop_installation,
                expected,
                delete_data,
            )
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)??;
        let store = self.owned.durable_state_store.clone();
        let permit = self.app_control().try_admit()?;
        let (view_owner, view_installation) = (owner.clone(), installation.clone());
        let outcome = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.mutate_app_installation(
                &owner,
                crate::durable_state::apps::AppRegistryMutation::Uninstall {
                    installation_id: installation,
                    expected_generation: expected,
                    now_ms: crate::session::unix_epoch_ms(),
                },
            )
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(|error| match error {
            crate::durable_state::apps::AppRegistryError::Registry(
                chariox_app_runtime::installation::InstallationError::Conflict,
            ) => AppRequestErrorCode::Conflict,
            error => crate::runtime::app_control::registry_error(error),
        })?;
        self.refresh_uninstalled_app_bindings(&view_owner, &view_installation)
            .await;
        let crate::durable_state::apps::AppRegistryOutcome::Installation(mut installation) =
            outcome
        else {
            return Err(AppRequestErrorCode::StorageUnavailable);
        };
        // A failure here leaves the App uninstalled with its data; uninstalling
        // again with delete_data finishes the deletion.
        if delete_data {
            self.delete_app_storage(&view_owner, &view_installation)
                .await?;
            let store = self.owned.durable_state_store.clone();
            let permit = self.app_control().try_admit()?;
            let generation = installation.generation;
            installation = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                store.mutate_app_installation(
                    &view_owner,
                    crate::durable_state::apps::AppRegistryMutation::ForgetData {
                        installation_id: view_installation,
                        expected_generation: generation,
                    },
                )
            })
            .await
            .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
            .map_err(crate::runtime::app_control::registry_error)
            .and_then(|outcome| match outcome {
                crate::durable_state::apps::AppRegistryOutcome::Installation(value) => Ok(value),
                _ => Err(AppRequestErrorCode::StorageUnavailable),
            })?;
        }
        let cleanup_store = self.owned.durable_state_store.clone();
        let (cleanup_owner, cleanup_installation, cleanup_generation) = (
            installation.owner_id.clone(),
            installation.installation_id.clone(),
            installation.generation,
        );
        tokio::task::spawn_blocking(move || {
            crate::runtime::app_snapshot_restore::retire_uninstalled(
                &cleanup_store,
                &cleanup_owner,
                &cleanup_installation,
                cleanup_generation,
            )
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)??;
        Ok(LocalDaemonResponse::AppInstallation {
            installation: crate::runtime::app_control::installation_summary(installation),
        })
    }

    /// The stopped worker releases its storage as it is reaped, so a busy
    /// storage is retried briefly; then the request fails and deleting again
    /// finishes it.
    #[cfg(target_os = "macos")]
    async fn delete_macos_app_storage(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<(), AppRequestErrorCode> {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        tokio::task::spawn_blocking(move || {
            let root = crate::runtime::app_lifecycle::macos_storage_root(&store)
                .map_err(|_| AppRequestErrorCode::StorageUnavailable)?;
            for _ in 0..20 {
                match chariox_app_runtime::worker_process::PreparedWorker::delete_macos_storage(
                    &root,
                    &owner,
                    &installation,
                ) {
                    Ok(()) => return Ok(()),
                    Err("app_storage_busy") => {
                        std::thread::sleep(std::time::Duration::from_millis(250))
                    }
                    Err(_) => return Err(AppRequestErrorCode::StorageUnavailable),
                }
            }
            Err(AppRequestErrorCode::Busy)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
    }

    /// Linux App storage is root-owned: the storage helper deletes it. The
    /// stopped worker releases its lease as it is reaped, so a busy storage is
    /// retried briefly; then the request fails and deleting again finishes it.
    #[cfg(target_os = "linux")]
    async fn delete_linux_app_storage(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<(), AppRequestErrorCode> {
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        tokio::task::spawn_blocking(move || {
            for _ in 0..20 {
                match chariox_app_runtime::worker_process::delete_linux_storage(
                    &owner,
                    &installation,
                ) {
                    Ok(()) => return Ok(()),
                    Err(code) if code == "app_storage_busy" => {
                        std::thread::sleep(std::time::Duration::from_millis(250))
                    }
                    Err(_) => return Err(AppRequestErrorCode::StorageUnavailable),
                }
            }
            Err(AppRequestErrorCode::Busy)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
    }

    /// Deletes the stopped App's storage. A test kernel given fixture storage
    /// deletes from it instead (`AppControlService::fixture_app_storage`).
    async fn delete_app_storage(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<(), AppRequestErrorCode> {
        #[cfg(test)]
        if let Some(storage) = self.app_control().fixture_storage() {
            storage.delete(owner, installation);
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        self.delete_linux_app_storage(owner, installation).await?;
        #[cfg(target_os = "macos")]
        self.delete_macos_app_storage(owner, installation).await?;
        Ok(())
    }

    /// Returns whether a new start was accepted (an owner thread was spawned).
    /// A start that finds every live worker slot taken stops the
    /// least-recently-used idle worker (it stays dormant and restarts on use)
    /// and tries once more, as an on-demand start does.
    async fn control_app_worker(
        &self,
        owner: &str,
        installation: &str,
        action: AppWorkerAction,
    ) -> Result<bool, AppRequestErrorCode> {
        use crate::runtime::app_lifecycle::LifecycleError;
        let result = start_evicting_on_live_limit(
            action,
            |action| self.control_app_worker_once(owner, installation, action),
            || self.evict_idle_app(owner, installation),
        )
        .await;
        result.map_err(|error| match error {
            LifecycleError::Busy | LifecycleError::LiveLimit => AppRequestErrorCode::Busy,
            LifecycleError::Storage | LifecycleError::Supervisor => {
                AppRequestErrorCode::StorageUnavailable
            }
            _ => AppRequestErrorCode::Conflict,
        })
    }

    async fn control_app_worker_once(
        &self,
        owner: &str,
        installation: &str,
        action: AppWorkerAction,
    ) -> Result<bool, crate::runtime::app_lifecycle::LifecycleError> {
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
            Ok(false)
        })
        .await
        .map_err(|_| crate::runtime::app_lifecycle::LifecycleError::Supervisor)?
    }

    /// The active release's verified catalog. Automations are durable
    /// configuration, so a stopped, failed or never-started App needs no worker.
    async fn app_catalog(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<std::sync::Arc<chariox_app_runtime::app_outbox::EventCatalog>, AppRequestErrorCode>
    {
        let store = self.owned.durable_state_store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        let permit = self.app_control().try_admit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.active_app_event_catalog(&owner, &installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map_err(AppRequestErrorCode::from)
    }

    async fn app_worker_summary(
        &self,
        owner: String,
        installation: String,
    ) -> Result<LocalDaemonResponse, AppRequestErrorCode> {
        let dormant = self.app_control().is_app_dormant(&owner, &installation);
        let store = self.owned.durable_state_store.clone();
        let id = installation.clone();
        let (status, dormant) = tokio::task::spawn_blocking(move || {
            let status = store.app_worker_status(&owner, &id)?;
            let dormant = if status.as_ref().is_some_and(|status| {
                status.phase == WorkerPhase::Stopped && (dormant || status.dormant)
            }) {
                store.app_worker_start_gate(&owner, &id)? == StartGate::Allowed
            } else {
                false
            };
            Ok::<_, crate::durable_state::app_worker_lifecycle::LifecycleStoreError>((
                status, dormant,
            ))
        })
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
                phase: worker_phase(&status, dormant),
                enabled: status.desired_running,
                failure: status.failure,
                updated_at_ms: Some(status.updated_ms),
            },
        };
        Ok(LocalDaemonResponse::AppWorker { worker })
    }
}

/// An idle-stopped worker restarts on use (Dormant); a user stop does not.
fn worker_phase(status: &WorkerStatus, dormant: bool) -> AppWorkerPhase {
    match status.phase {
        WorkerPhase::Starting => AppWorkerPhase::Starting,
        WorkerPhase::Running => AppWorkerPhase::Running,
        WorkerPhase::Stopped if dormant => AppWorkerPhase::Dormant,
        WorkerPhase::Stopped => AppWorkerPhase::Stopped,
        WorkerPhase::Failed if status.is_quarantined() => AppWorkerPhase::Quarantined,
        WorkerPhase::Failed => AppWorkerPhase::Failed,
    }
}

fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}

fn summary(value: &AutomationConfiguration) -> AppAutomationSummary {
    AppAutomationSummary {
        delivery_mode: value.delivery_mode,
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
        E::Outbox(O::Limit | O::Full) => AppRequestErrorCode::LimitExceeded,
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

/// A start or restart that finds every live worker slot taken evicts one idle
/// worker and tries once more as a plain start: a restart's stop already ran.
async fn start_evicting_on_live_limit<Control, ControlFuture, Evict, EvictFuture>(
    action: AppWorkerAction,
    mut control: Control,
    evict: Evict,
) -> Result<bool, crate::runtime::app_lifecycle::LifecycleError>
where
    Control: FnMut(AppWorkerAction) -> ControlFuture,
    ControlFuture:
        std::future::Future<Output = Result<bool, crate::runtime::app_lifecycle::LifecycleError>>,
    Evict: FnOnce() -> EvictFuture,
    EvictFuture: std::future::Future<Output = ()>,
{
    use crate::runtime::app_lifecycle::LifecycleError;
    match control(action).await {
        Err(LifecycleError::LiveLimit)
            if matches!(action, AppWorkerAction::Start | AppWorkerAction::Restart) =>
        {
            evict().await;
            control(AppWorkerAction::Start).await
        }
        result => result,
    }
}

fn failed(code: AppRequestErrorCode) -> LocalDaemonResponse {
    LocalDaemonResponse::AppRequestFailed { code }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::app_lifecycle::LifecycleError;

    fn status(phase: WorkerPhase, failures: u32) -> WorkerStatus {
        WorkerStatus {
            generation: 1,
            attempt: "attempt".into(),
            phase,
            desired_running: true,
            dormant: false,
            failure: None,
            updated_ms: 1,
            failures,
        }
    }

    #[test]
    fn only_an_idle_stop_reports_dormant() {
        assert_eq!(
            worker_phase(&status(WorkerPhase::Stopped, 0), true),
            AppWorkerPhase::Dormant
        );
        assert_eq!(
            worker_phase(&status(WorkerPhase::Stopped, 0), false),
            AppWorkerPhase::Stopped
        );
        let mut restored = status(WorkerPhase::Stopped, 0);
        restored.dormant = true;
        assert_eq!(worker_phase(&restored, true), AppWorkerPhase::Dormant);
        // A retained durable flag alone cannot override refused authority.
        assert_eq!(worker_phase(&restored, false), AppWorkerPhase::Stopped);
        assert_eq!(
            worker_phase(&status(WorkerPhase::Running, 0), true),
            AppWorkerPhase::Running
        );
        assert_eq!(
            worker_phase(&status(WorkerPhase::Failed, 1), true),
            AppWorkerPhase::Failed
        );
    }

    /// Drives the live-limit retry with scripted start results; returns the
    /// outcome, the actions tried and whether an eviction ran.
    fn drive(
        action: AppWorkerAction,
        results: Vec<Result<bool, LifecycleError>>,
    ) -> (Result<bool, LifecycleError>, Vec<AppWorkerAction>, bool) {
        let results = std::cell::RefCell::new(results.into_iter());
        let tried = std::cell::RefCell::new(Vec::new());
        let evicted = std::cell::Cell::new(false);
        let result = futures_util::FutureExt::now_or_never(start_evicting_on_live_limit(
            action,
            |action| {
                tried.borrow_mut().push(action);
                std::future::ready(results.borrow_mut().next().unwrap())
            },
            || {
                evicted.set(true);
                std::future::ready(())
            },
        ))
        .unwrap();
        (result, tried.into_inner(), evicted.get())
    }

    #[test]
    fn a_start_at_the_live_limit_evicts_an_idle_worker_and_starts() {
        let (result, tried, evicted) = drive(
            AppWorkerAction::Start,
            vec![Err(LifecycleError::LiveLimit), Ok(true)],
        );
        assert!(matches!(result, Ok(true)));
        assert!(evicted);
        assert_eq!(tried, [AppWorkerAction::Start, AppWorkerAction::Start]);
    }

    #[test]
    fn a_start_with_nothing_evictable_stays_at_the_live_limit() {
        let (result, tried, evicted) = drive(
            AppWorkerAction::Start,
            vec![
                Err(LifecycleError::LiveLimit),
                Err(LifecycleError::LiveLimit),
            ],
        );
        assert!(matches!(result, Err(LifecycleError::LiveLimit)));
        assert!(evicted);
        assert_eq!(tried.len(), 2, "one retry only");
    }

    #[test]
    fn a_restart_at_the_live_limit_retries_as_a_plain_start() {
        let (result, tried, _) = drive(
            AppWorkerAction::Restart,
            vec![Err(LifecycleError::LiveLimit), Ok(true)],
        );
        assert!(matches!(result, Ok(true)));
        assert_eq!(tried, [AppWorkerAction::Restart, AppWorkerAction::Start]);
    }

    #[test]
    fn only_the_live_limit_triggers_an_eviction() {
        for (action, error) in [
            (AppWorkerAction::Start, LifecycleError::Busy),
            (AppWorkerAction::Stop, LifecycleError::LiveLimit),
        ] {
            let (_, tried, evicted) = drive(action, vec![Err(error)]);
            assert!(!evicted);
            assert_eq!(tried.len(), 1);
        }
    }

    #[test]
    fn quarantine_is_distinct_from_backoff_and_only_applies_to_failed_workers() {
        for dormant in [false, true] {
            for failures in [0, 1, 2, 3] {
                assert_eq!(
                    worker_phase(&status(WorkerPhase::Failed, failures), dormant),
                    AppWorkerPhase::Failed
                );
            }
            for failures in [4, 5, u32::MAX] {
                assert_eq!(
                    worker_phase(&status(WorkerPhase::Failed, failures), dormant),
                    AppWorkerPhase::Quarantined
                );
                assert_eq!(
                    worker_phase(&status(WorkerPhase::Running, failures), dormant),
                    AppWorkerPhase::Running
                );
                assert_eq!(
                    worker_phase(&status(WorkerPhase::Starting, failures), dormant),
                    AppWorkerPhase::Starting
                );
            }
        }
        assert_eq!(
            worker_phase(&status(WorkerPhase::Stopped, 4), false),
            AppWorkerPhase::Stopped
        );
    }
}
