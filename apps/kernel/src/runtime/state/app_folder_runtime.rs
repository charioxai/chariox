//! Owner-only folder selections and revocations, visible on the shared trusted
//! interaction surfaces. A remote terminal selects on this kernel's machine.
use super::KernelRuntimeState;
use crate::{
    durable_state::app_folder_grants::FolderCommand,
    local::{
        AppRequestErrorCode as Code, LocalDaemonRequest as Request, LocalDaemonResponse as Response,
    },
    session::{RuntimeInteraction, RuntimeInteractionChoice},
};
use chariox_app_runtime::external_folder::ExternalFolder;

fn prompt_id(operation: &str) -> String {
    format!("app_folder_pick_{operation}")
}
fn failure(code: Code) -> Response {
    Response::AppRequestFailed { code }
}
fn storage_code(code: &str) -> Code {
    match code {
        "NOT_FOUND" => Code::NotFound,
        "CONFLICT" | "APP_STALE" => Code::Conflict,
        "LIMIT_EXCEEDED" => Code::LimitExceeded,
        "INVALID_ARGUMENT" | "FOLDER_CHANGED" | "UNSAFE_ENTRY" | "APP_FOLDER_UNAVAILABLE" => {
            Code::InvalidRequest
        }
        _ => Code::StorageUnavailable,
    }
}

impl KernelRuntimeState {
    pub(super) async fn folder_control(&self, owner: String, request: Request) -> Response {
        if let Request::GrantAppFolder(request) = request {
            let Ok(permit) = self.app_control().try_admit() else {
                return failure(Code::Busy);
            };
            let store = self.owned.durable_state_store.clone();
            let operation = request.operation_id.clone();
            let pick_owner = owner.clone();
            let pick = match tokio::task::spawn_blocking(move || {
                let _permit = permit;
                store.owned_app_folder_pick(&pick_owner, &operation)
            })
            .await
            {
                Ok(Ok(Some(pick))) => pick,
                Ok(Ok(None)) => return failure(Code::NotFound),
                Ok(Err(code)) => return failure(storage_code(code)),
                Err(_) => return failure(Code::StorageUnavailable),
            };
            if pick.state != crate::durable_state::app_file_grants::PickState::Pending
                || pick.expires_ms <= crate::session::unix_epoch_ms()
            {
                return failure(Code::Conflict);
            }
            let catalog = match self.app_catalog(&owner, &pick.installation).await {
                Ok(catalog) => catalog,
                Err(code) => return failure(code),
            };
            if request.path.is_empty()
                || request.path.len() > 4096
                || request.path.chars().any(char::is_control)
            {
                return failure(Code::InvalidRequest);
            }
            let Ok(permit) = self.app_control().try_admit() else {
                return failure(Code::Busy);
            };
            let store = self.owned.durable_state_store.clone();
            let operation = request.operation_id.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let selected = ExternalFolder::select(std::path::Path::new(&request.path))
                    .map_err(|_| "INVALID_ARGUMENT")?;
                store.app_folder_command(FolderCommand::Grant {
                    owner,
                    catalog,
                    operation_id: request.operation_id,
                    path: request.path,
                    identity: selected.identity(),
                    access: request.access,
                    now_ms: crate::session::unix_epoch_ms(),
                })
            })
            .await;
            let granted = match result {
                Ok(Ok(granted)) => granted,
                Ok(Err(code)) => return failure(storage_code(code)),
                Err(_) => return failure(Code::StorageUnavailable),
            };
            let Some(id) = granted["grantId"].as_str() else {
                return failure(Code::StorageUnavailable);
            };
            let Some(installation) = granted["installationId"].as_str() else {
                return failure(Code::StorageUnavailable);
            };
            let session = self
                .app_control()
                .validation_prompt_session(&operation)
                .unwrap_or(request.session_id);
            let _ = self
                .timeout_runtime_interaction(&session, &prompt_id(&operation))
                .await;
            return Response::AppFolderGranted {
                operation_id: operation,
                installation_id: installation.into(),
                grant_id: id.into(),
            };
        }
        let Ok(permit) = self.app_control().try_admit() else {
            return failure(Code::Busy);
        };
        let installation = match &request {
            Request::ListAppFolderGrants(request) => request.installation_id.clone(),
            Request::RevokeAppFolderGrant(request) => request.installation_id.clone(),
            _ => return failure(Code::InvalidRequest),
        };
        let store = self.owned.durable_state_store.clone();
        match tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store
                .get_app_installation(&owner, &installation)
                .map_err(
                    |error| match crate::runtime::app_control::registry_error(error) {
                        Code::NotFound => "NOT_FOUND",
                        _ => "STORAGE_UNAVAILABLE",
                    },
                )?;
            match request {
                Request::ListAppFolderGrants(_) => {
                    let grants = store
                        .list_app_folder_grants(&owner, &installation)?
                        .into_iter()
                        .map(|grant| crate::local::AppFolderGrantSummary {
                            grant_id: grant.grant_id,
                            path: grant.path,
                            access: grant.access,
                            written_bytes: grant.written_bytes,
                        })
                        .collect();
                    Ok(Response::AppFolderGrantsListed {
                        installation_id: installation,
                        grants,
                    })
                }
                Request::RevokeAppFolderGrant(request) => {
                    store.app_folder_command(FolderCommand::Revoke {
                        owner,
                        installation: installation.clone(),
                        grant_id: request.grant_id.clone(),
                    })?;
                    Ok(Response::AppFolderGrantRevoked {
                        installation_id: installation,
                        grant_id: request.grant_id,
                    })
                }
                _ => Err("INVALID_ARGUMENT"),
            }
        })
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(code)) => failure(storage_code(code)),
            Err(_) => failure(Code::StorageUnavailable),
        }
    }

    pub(super) async fn app_folder_pick_pass(&self, now_ms: u64) {
        let store = self.owned.durable_state_store.clone();
        let Ok(Ok(pending)) =
            tokio::task::spawn_blocking(move || store.pending_app_folder_picks(now_ms)).await
        else {
            return;
        };
        for pick in pending {
            if !self
                .app_control()
                .begin_validation_prompt(&pick.operation_id, &pick.owner)
            {
                continue;
            }
            let Some(session) = self.validation_session(&pick.owner) else {
                self.app_control().end_validation_prompt(&pick.operation_id);
                continue;
            };
            self.app_control()
                .show_validation_prompt(&pick.operation_id, &session);
            let message = format!(
                "Choose a folder on this machine for this App. The App can use its contents until you revoke access. Choose read-only or read-write access.\n\nInstallation: {}\n\nIn a text terminal: /app folder grant {} \"PATH\" read-only|read-write",
                pick.installation, pick.operation_id,
            );
            let interaction = RuntimeInteraction::for_kernel_operation(
                prompt_id(&pick.operation_id),
                format!("folder_pick:{}", pick.operation_id),
                "Share a folder with an App",
                message,
                vec![RuntimeInteractionChoice::new(
                    "decline", "Decline", "deny", None,
                )],
            )
            .with_timeout_sec((pick.expires_ms.saturating_sub(now_ms) / 1000).clamp(1, 300));
            let receiver = match self
                .create_kernel_operation_interaction(&session, &pick.owner, interaction)
                .await
            {
                Ok(receiver) => receiver,
                Err(_) => {
                    self.app_control().end_validation_prompt(&pick.operation_id);
                    continue;
                }
            };
            let store = self.owned.durable_state_store.clone();
            let check = pick.clone();
            let pending = tokio::task::spawn_blocking(move || {
                store
                    .app_folder_pick(&check.owner, &check.installation, &check.operation_id)
                    .map(|pick| {
                        pick.is_some_and(|pick| {
                            pick.state == crate::durable_state::app_file_grants::PickState::Pending
                                && pick.expires_ms > crate::session::unix_epoch_ms()
                        })
                    })
            })
            .await;
            if !matches!(pending, Ok(Ok(true))) {
                let _ = self
                    .timeout_runtime_interaction(&session, &prompt_id(&pick.operation_id))
                    .await;
                self.app_control().end_validation_prompt(&pick.operation_id);
                continue;
            }
            let runtime = self.clone();
            tokio::spawn(async move {
                if receiver.await.is_ok_and(|resolution| {
                    resolution.status == "answered"
                        && resolution.choice_id.as_deref() == Some("decline")
                }) {
                    let store = runtime.owned.durable_state_store.clone();
                    let owner = pick.owner;
                    let operation_id = pick.operation_id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        store.app_folder_command(FolderCommand::Decline {
                            owner,
                            operation_id,
                        })
                    })
                    .await;
                }
                runtime
                    .app_control()
                    .end_validation_prompt(&pick.operation_id);
            });
        }
    }
}
