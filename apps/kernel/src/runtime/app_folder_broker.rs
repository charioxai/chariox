//! SDK folder access carries only an opaque grant and a relative path. The
//! worker's signed package and actual process identity supply authority.
use crate::{
    durable_state::{app_folder_grants::FolderCommand, DurableKernelStateStore},
    runtime::app_operation_budget::AppOperationBudget,
};
use chariox_app_package::VerifiedApp;
use chariox_app_runtime::{
    app_outbox::EventCatalog, wire::RemoteError, worker_peer::BrokerRequest,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(crate) struct AppFolderBroker {
    store: DurableKernelStateStore,
    owner: String,
    catalog: Arc<EventCatalog>,
    admission: Arc<Semaphore>,
    allowed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Status {
    operation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Read {
    grant_id: String,
    #[serde(default)]
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Write {
    grant_id: String,
    path: String,
    contents_base64: String,
}

impl AppFolderBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        catalog: Arc<EventCatalog>,
        admission: Arc<Semaphore>,
        package: &impl VerifiedApp,
    ) -> Self {
        // Kept as a signed capability, never a caller claim or agent discovery
        // grant. Manifest parsing rejects every unknown access mode.
        let allowed = package
            .manifest()
            .capabilities
            .external_files
            .contains(&chariox_app_package::ExternalFileAccess::UserFolder);
        Self {
            store,
            owner,
            catalog,
            admission,
            allowed,
        }
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        if !self.allowed {
            return Err(error("CAPABILITY_REQUIRED"));
        }
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("APP_BUSY"))?;
        let service = self.clone();
        let budget = AppOperationBudget::from_broker(&request);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            service.call(request, budget).map_err(error)
        })
        .await
        .map_err(|_| error("STORAGE_UNAVAILABLE"))?
    }

    fn call(
        &self,
        request: BrokerRequest,
        budget: AppOperationBudget,
    ) -> Result<Value, &'static str> {
        budget.check().map_err(|_| "CANCELLED")?;
        let now_ms = crate::session::unix_epoch_ms();
        let decode = |value| serde_json::from_value::<Read>(value).map_err(|_| "INVALID_ARGUMENT");
        match request.method.as_str() {
            "host.pick_folder" => {
                if request.params != json!({}) {
                    return Err("INVALID_ARGUMENT");
                }
                self.store.app_folder_command(FolderCommand::Pick {
                    owner: self.owner.clone(),
                    catalog: self.catalog.clone(),
                    now_ms,
                })
            }
            "host.pick_folder_status" => {
                let request: Status =
                    serde_json::from_value(request.params).map_err(|_| "INVALID_ARGUMENT")?;
                let pick = self
                    .store
                    .app_folder_pick(
                        &self.owner,
                        self.catalog.installation_id(),
                        &request.operation_id,
                    )?
                    .ok_or("NOT_FOUND")?;
                let state = if pick.state
                    == crate::durable_state::app_file_grants::PickState::Pending
                    && pick.expires_ms <= now_ms
                {
                    "expired"
                } else {
                    pick.state.name()
                };
                Ok(json!({"operationId":pick.operation_id,"state":state,"grantId":pick.grant_id}))
            }
            "folders.list" | "folders.read" => {
                let input = decode(request.params)?;
                self.store.app_folder_command(FolderCommand::Read {
                    owner: self.owner.clone(),
                    catalog: self.catalog.clone(),
                    grant_id: input.grant_id,
                    relative: input.path,
                    list: request.method == "folders.list",
                    budget,
                    now_ms,
                })
            }
            "folders.write" => {
                use base64::Engine;
                let input: Write =
                    serde_json::from_value(request.params).map_err(|_| "INVALID_ARGUMENT")?;
                if input.contents_base64.len()
                    > chariox_app_runtime::external_folder::MAX_FILE_BYTES.div_ceil(3) * 4
                {
                    return Err("LIMIT_EXCEEDED");
                }
                let contents = base64::engine::general_purpose::STANDARD
                    .decode(&input.contents_base64)
                    .map_err(|_| "INVALID_ARGUMENT")?;
                self.store.prepare_app_folder_write(
                    self.owner.clone(),
                    self.catalog.clone(),
                    input.grant_id,
                    &input.path,
                    &contents,
                    budget,
                )
            }
            _ => Err("METHOD_UNAVAILABLE"),
        }
    }
}

fn error(code: &str) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: code.into(),
        retryable: Some(matches!(code, "APP_BUSY" | "RATE_LIMITED")),
    }
}
