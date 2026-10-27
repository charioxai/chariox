//! App control receipts sit below every transport and after owner authorization.
use std::{future::Future, path::PathBuf, sync::Arc};

use crate::{
    local::{AppRequestErrorCode, LocalDaemonRequest, LocalDaemonResponse},
    runtime::command::KernelCommand,
    runtime_transport::command_cache::{
        is_receipt_capacity_error, is_receipt_expired_error, CommandFingerprint,
        CommandReservation, CommandResultCache,
    },
};

#[derive(Clone)]
pub(super) struct AppRequestReceipts(Option<Arc<CommandResultCache>>);

impl AppRequestReceipts {
    pub(super) fn new(path: PathBuf) -> Self {
        // Fail closed if the durable journal cannot load, including corrupt records.
        let cache = match CommandResultCache::new_at_most_once(path) {
            Ok(cache) => Some(Arc::new(cache)),
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.app_control",
                    "failed to load App request receipts",
                    serde_json::json!({"error": error.to_string()}),
                );
                None
            }
        };
        Self(cache)
    }

    async fn has_reserved(&self, owner: &str, command_id: &str) -> bool {
        let Some(cache) = self.0.as_ref() else {
            return false;
        };
        cache
            .has_reserved(&serde_json::to_string(&(owner, command_id)).unwrap())
            .await
    }

    pub(super) async fn execute<F, Fut>(
        &self,
        owner: &str,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
        execute: F,
    ) -> LocalDaemonResponse
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = LocalDaemonResponse> + Send + 'static,
    {
        let failed = || LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::StorageUnavailable,
        };
        if command.command_id.trim().is_empty()
            || command.command_id.len() > 128
            || command.command_id.chars().any(char::is_control)
        {
            return LocalDaemonResponse::AppRequestFailed {
                code: AppRequestErrorCode::InvalidRequest,
            };
        }
        let Some(cache) = self.0.clone() else {
            return failed();
        };
        // These controls reduce authority and have their own durable fences:
        // manual-stop intent and uninstall's expected generation. Never let
        // receipt capacity prevent them from reaching the lifecycle service.
        let safety_control = matches!(
            request,
            LocalDaemonRequest::ControlAppWorker(crate::local::ControlAppWorkerRequest {
                action: crate::local::AppWorkerAction::Stop,
                ..
            }) | LocalDaemonRequest::UninstallApp(_)
        );
        let key = serde_json::to_string(&(owner, &command.command_id)).unwrap();
        let fingerprint = CommandFingerprint::for_app_control(command, request);
        // Detach acceptance, execution and settlement together. A disconnected or
        // cancelled caller cannot strand the live reservation or drop its result.
        tokio::spawn(async move {
            match cache
                .reserve_at_most_once(&key, &fingerprint, serde_json::to_value(failed()).unwrap())
                .await
            {
                Ok(CommandReservation::Dispatch) => {
                    let response = tokio::spawn(execute()).await.unwrap_or_else(|_| failed());
                    match cache
                        .complete_at_most_once(
                            key,
                            fingerprint,
                            serde_json::to_value(&response).unwrap(),
                            serde_json::to_value(failed()).unwrap(),
                        )
                        .await
                    {
                        Ok(()) => response,
                        Err(error) => {
                            crate::logging::warn_with_fields(
                                "daemon.app_control",
                                "failed to settle App request receipt",
                                serde_json::json!({"error": error.to_string()}),
                            );
                            failed()
                        }
                    }
                }
                Ok(CommandReservation::Wait(wait)) => wait
                    .await
                    .ok()
                    .and_then(|cached| {
                        (*cached.response_value())
                            .and_then(|value| serde_json::from_value(value).ok())
                    })
                    .unwrap_or_else(failed),
                Ok(CommandReservation::Conflict) => LocalDaemonResponse::AppRequestFailed {
                    code: AppRequestErrorCode::Conflict,
                },
                Err(error) if is_receipt_expired_error(&error) => {
                    LocalDaemonResponse::AppRequestFailed {
                        code: AppRequestErrorCode::ReceiptExpired,
                    }
                }
                Err(error) if is_receipt_capacity_error(&error) && safety_control => {
                    // Reservation checks existing identities/conflicts before
                    // capacity. Preserve every accepted receipt; only this new
                    // control executes without a historical response receipt.
                    tokio::spawn(execute()).await.unwrap_or_else(|_| failed())
                }
                Err(error) if is_receipt_capacity_error(&error) => {
                    LocalDaemonResponse::AppRequestFailed {
                        code: AppRequestErrorCode::LimitExceeded,
                    }
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.app_control",
                        "failed to reserve App request receipt",
                        serde_json::json!({"error": error.to_string()}),
                    );
                    failed()
                }
            }
        })
        .await
        .unwrap_or_else(|_| LocalDaemonResponse::AppRequestFailed {
            code: AppRequestErrorCode::StorageUnavailable,
        })
    }
}

impl super::AppControlService {
    pub(crate) async fn execute_once<F, Fut>(
        &self,
        owner: &str,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
        execute: F,
    ) -> LocalDaemonResponse
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = LocalDaemonResponse> + Send + 'static,
    {
        self.request_receipts
            .execute(owner, command, request, execute)
            .await
    }

    pub(crate) async fn require_owned_installation(
        &self,
        owner: &str,
        installation: &str,
        command_id: &str,
    ) -> Result<(), AppRequestErrorCode> {
        // A matching owner-scoped receipt already proves installation authority
        // at acceptance. Replay still checks the authenticated caller/input, but
        // must not repeat a generation fence or wait for fresh I/O admission.
        if self.request_receipts.has_reserved(owner, command_id).await {
            return Ok(());
        }
        let store = self.store.clone();
        let (owner, installation) = (owner.to_owned(), installation.to_owned());
        let permit = self.try_admit()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.get_app_installation(&owner, &installation)
        })
        .await
        .map_err(|_| AppRequestErrorCode::StorageUnavailable)?
        .map(|_| ())
        .map_err(super::registry_error)
    }
}

#[cfg(test)]
mod tests;
