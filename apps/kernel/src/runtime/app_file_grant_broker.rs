//! SDK `host.pick_file`, `host.pick_file_status` and `files.import`, for Apps
//! whose signed manifest declares `externalFiles: ["user_selected"]`. A pick
//! returns a pending reference at once; the owner answers through a trusted
//! kernel prompt. An import copies one granted file into private data, once.
use super::app_operation_budget::AppOperationBudget;
use crate::durable_state::{
    app_file_grants::{FileGrantCommand, FilePick, PickState, MAX_FILES, PICK_MS},
    DurableKernelStateStore,
};
use chariox_app_package::{ExternalFileAccess, VerifiedPackage};
use chariox_app_runtime::{
    app_outbox::EventCatalog, wire::RemoteError, worker_peer::BrokerRequest,
    worker_process::PrivateData,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;

const MAX_ACCEPT: usize = 16;

#[derive(Clone)]
pub(crate) struct AppFileGrantBroker {
    store: DurableKernelStateStore,
    owner: String,
    catalog: Arc<EventCatalog>,
    admission: Arc<Semaphore>,
    data: PrivateData,
    user_selected: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pick {
    #[serde(default)]
    multiple: bool,
    #[serde(default)]
    accept: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Status {
    operation_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Import {
    grant_id: String,
    destination: String,
}

impl AppFileGrantBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        catalog: Arc<EventCatalog>,
        admission: Arc<Semaphore>,
        data: PrivateData,
        package: &VerifiedPackage<'_>,
    ) -> Self {
        let user_selected = package
            .manifest()
            .capabilities
            .external_files
            .contains(&ExternalFileAccess::UserSelected);
        Self {
            store,
            owner,
            catalog,
            admission,
            data,
            user_selected,
        }
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        if !self.user_selected {
            return Err(error("CAPABILITY_REQUIRED", false));
        }
        let budget = AppOperationBudget::from_broker(&request);
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("APP_BUSY", true))?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match request.method.as_str() {
                "host.pick_file" => service.pick(request.params),
                "host.pick_file_status" => service.status(request.params),
                "files.import" => service.import(request.params, budget),
                _ => Err(error("METHOD_UNAVAILABLE", false)),
            }
        })
        .await
        .map_err(|_| error("STORAGE_UNAVAILABLE", true))?
    }

    fn installation(&self) -> &str {
        self.catalog.installation_id()
    }

    fn pick(&self, params: Value) -> Result<Value, RemoteError> {
        let request: Pick =
            serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        if request.accept.len() > MAX_ACCEPT
            || request.accept.iter().any(|suffix| {
                !suffix.starts_with('.')
                    || suffix.len() < 2
                    || suffix.len() > 16
                    || !suffix[1..]
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
            })
        {
            return Err(error("INVALID_ARGUMENT", false));
        }
        let pick = FilePick {
            operation_id: format!("file-pick-{:032x}", rand::random::<u128>()),
            owner: self.owner.clone(),
            installation: self.installation().to_owned(),
            generation: self.catalog.generation(),
            accept: request.accept,
            multiple: request.multiple,
            state: PickState::Pending,
            expires_ms: crate::session::unix_epoch_ms() + PICK_MS,
            grants: Vec::new(),
        };
        let pick = self
            .store
            .app_file_grant(FileGrantCommand::Create(pick))
            .map_err(|code| error(code, code == "STORAGE_UNAVAILABLE"))?
            .ok_or_else(|| error("STORAGE_UNAVAILABLE", true))?;
        Ok(reply(&pick))
    }

    fn status(&self, params: Value) -> Result<Value, RemoteError> {
        let request: Status =
            serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        let pick = self
            .store
            .app_file_pick(&self.owner, self.installation(), &request.operation_id)
            .map_err(|code| error(code, true))?
            .filter(|pick| pick.generation == self.catalog.generation())
            .ok_or_else(|| error("NOT_FOUND", false))?;
        Ok(reply(&pick))
    }

    fn import(&self, params: Value, budget: AppOperationBudget) -> Result<Value, RemoteError> {
        let request: Import =
            serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        let stopped = |_| error("APP_OPERATION_STOPPED", false);
        budget.check().map_err(stopped)?;
        let file = self
            .store
            .app_file_grant_contents(
                &self.owner,
                self.installation(),
                &request.grant_id,
                crate::session::unix_epoch_ms(),
            )
            .map_err(|code| error(code, true))?
            .ok_or_else(|| error("NOT_FOUND", false))?;
        let staged = self
            .data
            .prepare_replace(&request.destination, &file.contents)
            .map_err(|_| error("INVALID_ARGUMENT", false))?;
        budget.check().map_err(stopped)?;
        self.store
            .publish_app_file(&self.owner, self.catalog.clone(), staged, budget)
            .map_err(|_| error("APP_FILE_UNAVAILABLE", false))?;
        // Published: the grant is spent. If recording that fails, the same
        // bytes may be imported once more, which rewrites the same file.
        let _ = self.store.app_file_grant(FileGrantCommand::Imported {
            owner: self.owner.clone(),
            installation: self.installation().to_owned(),
            grant_id: request.grant_id,
        });
        Ok(json!({"bytesWritten": file.contents.len(), "name": file.name}))
    }
}

fn reply(pick: &FilePick) -> Value {
    debug_assert!(pick.grants.len() <= MAX_FILES);
    json!({
        "operationId": pick.operation_id,
        "state": pick.state.name(),
        "grantIds": pick.grants,
        "expiresAtMs": pick.expires_ms,
    })
}

fn error(code: &str, retryable: bool) -> RemoteError {
    let message = match code {
        "CAPABILITY_REQUIRED" => "The App does not declare user-selected external files",
        "APP_BUSY" => "App operation limit reached",
        "LIMIT_EXCEEDED" => "Too many file requests are waiting for an answer",
        "NOT_FOUND" => "No such file request or grant",
        "INVALID_ARGUMENT" => "Invalid file request",
        _ => "File operation did not complete",
    };
    RemoteError {
        code: code.into(),
        message: message.into(),
        retryable: Some(retryable),
    }
}

#[cfg(test)]
impl AppFileGrantBroker {
    pub(super) fn fixture(
        store: DurableKernelStateStore,
        owner: String,
        catalog: Arc<EventCatalog>,
        admission: Arc<Semaphore>,
        data: PrivateData,
        user_selected: bool,
    ) -> Self {
        Self {
            store,
            owner,
            catalog,
            admission,
            data,
            user_selected,
        }
    }
}
