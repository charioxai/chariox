//! App control receipts sit below every transport and after owner authorization.
use std::{future::Future, path::PathBuf, sync::Arc};

use crate::{
    local::{AppRequestErrorCode, LocalDaemonRequest, LocalDaemonResponse},
    runtime::command::KernelCommand,
    runtime_transport::command_cache::{
        CommandFingerprint, CommandReservation, CommandResultCache,
    },
    transport::kernel_protocol::KernelOutgoingFrame,
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
                    cache
                        .complete(
                            key,
                            fingerprint,
                            &KernelOutgoingFrame::Response {
                                request_id: String::new(),
                                response: Box::new(Some(serde_json::to_value(&response).unwrap())),
                                error: None,
                            },
                        )
                        .await;
                    response
                }
                Ok(CommandReservation::Wait(wait)) => wait
                    .await
                    .ok()
                    .and_then(|cached| {
                        (*cached.response).and_then(|value| serde_json::from_value(value).ok())
                    })
                    .unwrap_or_else(failed),
                Ok(CommandReservation::Conflict) => LocalDaemonResponse::AppRequestFailed {
                    code: AppRequestErrorCode::Conflict,
                },
                Err(error) if error.kind() == std::io::ErrorKind::OutOfMemory => {
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
}

#[cfg(test)]
mod tests;
