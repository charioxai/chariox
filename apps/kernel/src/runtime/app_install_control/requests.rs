//! Shared authenticated terminal adapter. Slow work is owned after a durable ACK.
use super::*;
use crate::durable_state::app_installation_operations::InstallInput;
use crate::{local::*, runtime::command::KernelCommand};

fn failed(code: AppRequestErrorCode) -> LocalDaemonResponse {
    LocalDaemonResponse::AppRequestFailed { code }
}
fn code(error: InstallOperationError) -> AppRequestErrorCode {
    match error {
        InstallOperationError::Invalid => AppRequestErrorCode::InvalidRequest,
        InstallOperationError::NotFound => AppRequestErrorCode::NotFound,
        InstallOperationError::Limit => AppRequestErrorCode::LimitExceeded,
        InstallOperationError::Storage | InstallOperationError::CommitUnknown => {
            AppRequestErrorCode::StorageUnavailable
        }
        _ => AppRequestErrorCode::Conflict,
    }
}
pub(super) fn projection(value: InstallOperation) -> LocalDaemonResponse {
    let prepared = value.phase != InstallPhase::Preparing && value.review.is_some();
    LocalDaemonResponse::AppInstallOperationStatus {
        operation: AppInstallOperationSummary {
            request_id: value.request_id,
            phase: match value.phase {
                InstallPhase::Preparing => AppInstallOperationPhase::Preparing,
                InstallPhase::AwaitingApproval => AppInstallOperationPhase::AwaitingApproval,
                InstallPhase::Starting => AppInstallOperationPhase::Starting,
                InstallPhase::Committed => AppInstallOperationPhase::Committed,
                InstallPhase::Cancelled => AppInstallOperationPhase::Cancelled,
                InstallPhase::Failed => AppInstallOperationPhase::Failed,
            },
            installation_id: prepared.then_some(value.token.installation_id),
            generation: prepared.then(|| value.token.generation.to_string()),
            package_digest: value.package_digest,
            interaction_id: value.interaction_id,
            failure: value.failure,
        },
    }
}
fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
impl AppInstallControl {
    pub(crate) async fn execute(
        &self,
        runtime: &KernelRuntimeState,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Option<LocalDaemonResponse> {
        let request_id = match request {
            LocalDaemonRequest::BeginAppInstall(value) => &value.request_id,
            LocalDaemonRequest::GetAppInstallOperation(value)
            | LocalDaemonRequest::CancelAppInstallOperation(value) => &value.request_id,
            _ => return None,
        };
        let owner = match super::super::app_control::owner(command) {
            Ok(owner) => owner,
            Err(code) => return Some(failed(code)),
        };
        if !identity(request_id) {
            return Some(failed(AppRequestErrorCode::InvalidRequest));
        }
        if self.0.stopped.load(Ordering::Acquire) {
            return Some(failed(AppRequestErrorCode::StorageUnavailable));
        }
        if let LocalDaemonRequest::BeginAppInstall(value) = request {
            if !identity(&value.session_id)
                || value.upload_handle.len() != 71
                || value.expected_package_digest.len() != 71
            {
                return Some(failed(AppRequestErrorCode::InvalidRequest));
            }
            if !runtime.app_install_session_member(&value.session_id, &owner) {
                return Some(failed(AppRequestErrorCode::Unauthorized));
            }
        }
        let key = (owner.clone(), request_id.clone());
        if matches!(request, LocalDaemonRequest::CancelAppInstallOperation(_)) {
            self.cancel_admission(&key);
        }
        let permit = match self.0.shared.admission.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return Some(failed(AppRequestErrorCode::Busy)),
        };
        let store = self.0.shared.store.clone();
        let request = request.clone();
        let request_id = request_id.clone();
        let stopped = self.0.stopped.clone();
        // The fixed deadline starts before queueing, and captures no owner cycle.
        // An admitted cancellation must finish its negative durable write during
        // shutdown; otherwise recovery could restart the operation it cancelled.
        let cancelling = matches!(request, LocalDaemonRequest::CancelAppInstallOperation(_));
        let budget = AppOperationBudget::from_supervisor(move || {
            !cancelling && stopped.load(Ordering::Acquire)
        });
        #[cfg(test)]
        let budget = match self.0.request_checkpoint.lock().unwrap().clone() {
            Some(observe) => budget.fixture_observe_checks(observe),
            None => budget,
        };
        let (response, receiver) = oneshot::channel();
        {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.0.stopped.load(Ordering::Acquire) {
                return Some(failed(AppRequestErrorCode::StorageUnavailable));
            }
            while state.requests.try_join_next().is_some() {}
            if state.requests.len() >= JOBS {
                return Some(failed(AppRequestErrorCode::Busy));
            }
            state.requests.spawn_blocking(move || {
                let _permit = permit;
                let value = (|| match request {
                    LocalDaemonRequest::BeginAppInstall(value) => store
                        .reserve_app_install(
                            &owner,
                            &value.request_id,
                            InstallInput {
                                session_id: value.session_id,
                                upload_handle: value.upload_handle,
                            },
                            &value.expected_package_digest,
                            budget,
                        )
                        .map(|value| (value, None)),
                    LocalDaemonRequest::GetAppInstallOperation(_) => store
                        .replay_public_app_install(&owner, &request_id, budget)
                        .map(|value| (value, None)),
                    LocalDaemonRequest::CancelAppInstallOperation(_) => {
                        let current = store.first_app_install_status(&owner, &request_id)?;
                        let close = current
                            .input
                            .as_ref()
                            .zip(current.interaction_id.as_ref())
                            .map(|(input, id)| (input.session_id.clone(), id.clone()));
                        store
                            .cancel_first_app_install(&owner, &request_id, budget)
                            .map(|value| (value, close))
                    }
                    _ => Err(InstallOperationError::Invalid),
                })();
                let _ = response.send(value);
            });
        }
        let result = receiver.await;
        Some(match result {
            Ok(Ok((operation, close))) => {
                if operation.phase == InstallPhase::Cancelled {
                    self.cancelled(key, operation.review.is_some());
                } else if matches!(
                    operation.phase,
                    InstallPhase::Preparing
                        | InstallPhase::AwaitingApproval
                        | InstallPhase::Starting
                ) {
                    self.notify(key);
                }
                if let Some((session, id)) = close {
                    let _ = runtime.timeout_runtime_interaction(&session, &id).await;
                }
                projection(operation)
            }
            Ok(Err(error)) => failed(code(error)),
            Err(_) => failed(AppRequestErrorCode::StorageUnavailable),
        })
    }
}
