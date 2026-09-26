//! Protocol 354 file picks and exports: each pending App file request or file
//! offer is shown to its owner as a trusted kernel prompt. The owner answers
//! from a terminal with `GrantAppFile` (the chosen files' bytes) or
//! `SaveAppFileExport` (taking the offered copy), or declines; App code, an
//! App view or an agent cannot answer.
use super::KernelRuntimeState;
use crate::durable_state::app_file_exports::{FileExport, FileExportCommand, FileExportReply};
use crate::durable_state::app_file_grants::{
    FileGrantCommand, FilePick, GrantedFile, MAX_FILES, MAX_FILE_BYTES,
};
use crate::local::{
    AppRequestErrorCode, GrantAppFileRequest, LocalDaemonResponse, SaveAppFileExportRequest,
};
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};
use base64::{engine::general_purpose::STANDARD, Engine};

const PAGE: usize = 64;

pub(crate) fn interaction_id(operation_id: &str) -> String {
    format!("app_file_pick_{operation_id}")
}

fn export_interaction_id(operation_id: &str) -> String {
    format!("app_file_export_{operation_id}")
}

impl KernelRuntimeState {
    /// Runs with each validation pass: same throttle, same prompt slots.
    pub(super) async fn app_file_pick_pass(&self, now_ms: u64) {
        let store = self.owned.durable_state_store.clone();
        let Ok(Ok(pending)) = tokio::task::spawn_blocking(move || {
            let _ = store.app_file_grant(FileGrantCommand::Expire { now_ms });
            store.pending_app_file_picks(PAGE)
        })
        .await
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
            let remaining_sec = pick.expires_ms.saturating_sub(now_ms) / 1000;
            let interaction = pick_interaction(&pick).with_timeout_sec(remaining_sec.clamp(1, 300));
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
            let runtime = self.clone();
            tokio::spawn(async move {
                let declined = receiver.await.is_ok_and(|resolution| {
                    resolution.status == "answered"
                        && resolution.choice_id.as_deref() == Some("decline")
                });
                if declined {
                    let store = runtime.owned.durable_state_store.clone();
                    let operation_id = pick.operation_id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        store.app_file_grant(FileGrantCommand::Decline {
                            operation_id,
                            now_ms: crate::session::unix_epoch_ms(),
                        })
                    })
                    .await;
                }
                // Granted from a terminal, timed out or abandoned: a still
                // pending pick is shown again on a later pass.
                runtime
                    .app_control()
                    .end_validation_prompt(&pick.operation_id);
            });
        }
    }

    /// Offers wait in the same prompt slots as picks and validations.
    pub(super) async fn app_file_export_pass(&self, now_ms: u64) {
        let store = self.owned.durable_state_store.clone();
        let Ok(Ok(pending)) = tokio::task::spawn_blocking(move || {
            let _ = store.app_file_export(FileExportCommand::Expire { now_ms });
            store.pending_app_file_exports(PAGE)
        })
        .await
        else {
            return;
        };
        for export in pending {
            if !self
                .app_control()
                .begin_validation_prompt(&export.operation_id, &export.owner)
            {
                continue;
            }
            let Some(session) = self.validation_session(&export.owner) else {
                self.app_control()
                    .end_validation_prompt(&export.operation_id);
                continue;
            };
            let remaining_sec = export.expires_ms.saturating_sub(now_ms) / 1000;
            let interaction =
                export_interaction(&export).with_timeout_sec(remaining_sec.clamp(1, 300));
            let receiver = match self
                .create_kernel_operation_interaction(&session, &export.owner, interaction)
                .await
            {
                Ok(receiver) => receiver,
                Err(_) => {
                    self.app_control()
                        .end_validation_prompt(&export.operation_id);
                    continue;
                }
            };
            let runtime = self.clone();
            tokio::spawn(async move {
                let declined = receiver.await.is_ok_and(|resolution| {
                    resolution.status == "answered"
                        && resolution.choice_id.as_deref() == Some("decline")
                });
                if declined {
                    let store = runtime.owned.durable_state_store.clone();
                    let operation_id = export.operation_id.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        store.app_file_export(FileExportCommand::Decline {
                            operation_id,
                            now_ms: crate::session::unix_epoch_ms(),
                        })
                    })
                    .await;
                }
                runtime
                    .app_control()
                    .end_validation_prompt(&export.operation_id);
            });
        }
    }

    /// The owner takes an offered file, once, then the prompt closes.
    pub(crate) async fn save_app_file_export(
        &self,
        owner: String,
        request: SaveAppFileExportRequest,
    ) -> LocalDaemonResponse {
        let failed = |code| LocalDaemonResponse::AppRequestFailed { code };
        let store = self.owned.durable_state_store.clone();
        let operation_id = request.operation_id.clone();
        let Ok(permit) = self.app_control().try_admit() else {
            return failed(AppRequestErrorCode::Busy);
        };
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.app_file_export(FileExportCommand::Save {
                owner,
                operation_id,
                now_ms: crate::session::unix_epoch_ms(),
            })
        })
        .await;
        let (name, contents) = match result {
            Ok(Ok(FileExportReply::Saved { name, contents })) => (name, contents),
            Ok(Err("NOT_FOUND")) => return failed(AppRequestErrorCode::NotFound),
            Ok(Err("CONFLICT")) => return failed(AppRequestErrorCode::Conflict),
            _ => return failed(AppRequestErrorCode::StorageUnavailable),
        };
        let _ = self
            .timeout_runtime_interaction(
                &request.session_id,
                &export_interaction_id(&request.operation_id),
            )
            .await;
        LocalDaemonResponse::AppFileExport {
            operation_id: request.operation_id,
            name,
            contents_base64: STANDARD.encode(contents),
        }
    }

    /// The owner's answer: store the chosen files as grants, then close the
    /// prompt on every terminal.
    pub(crate) async fn grant_app_file(
        &self,
        owner: String,
        request: GrantAppFileRequest,
    ) -> LocalDaemonResponse {
        let failed = |code| LocalDaemonResponse::AppRequestFailed { code };
        if request.files.is_empty() || request.files.len() > MAX_FILES {
            return failed(AppRequestErrorCode::InvalidRequest);
        }
        let mut files = Vec::with_capacity(request.files.len());
        for file in request.files {
            if file.contents_base64.len() > MAX_FILE_BYTES.div_ceil(3) * 4 {
                return failed(AppRequestErrorCode::LimitExceeded);
            }
            let Ok(contents) = STANDARD.decode(&file.contents_base64) else {
                return failed(AppRequestErrorCode::InvalidRequest);
            };
            files.push(GrantedFile {
                name: file.name,
                contents,
            });
        }
        let store = self.owned.durable_state_store.clone();
        let operation_id = request.operation_id.clone();
        let Ok(permit) = self.app_control().try_admit() else {
            return failed(AppRequestErrorCode::Busy);
        };
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.app_file_grant(FileGrantCommand::Grant {
                owner,
                operation_id,
                files,
                now_ms: crate::session::unix_epoch_ms(),
            })
        })
        .await;
        let pick = match result {
            Ok(Ok(Some(pick))) => pick,
            Ok(Err("NOT_FOUND")) => return failed(AppRequestErrorCode::NotFound),
            Ok(Err("CONFLICT")) => return failed(AppRequestErrorCode::Conflict),
            Ok(Err("INVALID_ARGUMENT")) => return failed(AppRequestErrorCode::InvalidRequest),
            _ => return failed(AppRequestErrorCode::StorageUnavailable),
        };
        // The prompt has no grant choice; close it now that it is answered,
        // in the session that shows it (the answer may come from another).
        let session = self
            .app_control()
            .validation_prompt_session(&pick.operation_id)
            .unwrap_or(request.session_id);
        let _ = self
            .timeout_runtime_interaction(&session, &interaction_id(&pick.operation_id))
            .await;
        LocalDaemonResponse::AppFileGranted {
            operation_id: pick.operation_id,
            files: pick.grants.len() as u32,
        }
    }
}

fn pick_interaction(pick: &FilePick) -> RuntimeInteraction {
    let kinds = if pick.accept.is_empty() {
        "any file".to_owned()
    } else {
        pick.accept.join(", ")
    };
    let count = if pick.multiple {
        "one or more files"
    } else {
        "a file"
    };
    RuntimeInteraction::for_kernel_operation(
        interaction_id(&pick.operation_id),
        format!("file_pick:{}", pick.operation_id),
        "Share a file with an App",
        format!(
            "An App asks you to choose {count} to share with it.\n\nInstallation: {}\nAccepted: {kinds}\n\nOnly the contents of the files you choose are shared, as private copies; the App never sees where they are. Only {} (the App's owner) can answer. Choose files in this terminal, or decline. In a text terminal: /app file grant {} \"FILE\"",
            pick.installation, pick.owner, pick.operation_id
        ),
        vec![RuntimeInteractionChoice::new("decline", "Decline", "deny", None)],
    )
}

fn export_interaction(export: &FileExport) -> RuntimeInteraction {
    RuntimeInteraction::for_kernel_operation(
        export_interaction_id(&export.operation_id),
        format!("file_export:{}", export.operation_id),
        "Save a file from an App",
        format!(
            "An App offers you a copy of one of its files.\n\nInstallation: {}\nFile: {} ({} bytes)\n\nSave it from this terminal to choose where it goes, or decline. Only {} (the App's owner) can answer. In a text terminal: /app file save {} \"PATH\"",
            export.installation, export.name, export.size, export.owner, export.operation_id
        ),
        vec![RuntimeInteractionChoice::new("decline", "Decline", "deny", None)],
    )
}
