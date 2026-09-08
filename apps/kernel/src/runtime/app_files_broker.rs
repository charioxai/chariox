//! Stage bounded private-file bytes outside the durable writer; publish only
//! inside its current installation/signer fence. No host paths or file grants.
use super::app_operation_budget::AppOperationBudget;
use crate::durable_state::{app_files::AppFileError, DurableKernelStateStore};
use base64::{engine::general_purpose::STANDARD, Engine};
use chariox_app_runtime::{
    app_outbox::EventCatalog, wire::RemoteError, worker_peer::BrokerRequest,
    worker_process::PrivateData,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(crate) struct AppFilesBroker {
    store: DurableKernelStateStore,
    owner: String,
    catalog: Arc<EventCatalog>,
    admission: Arc<Semaphore>,
    data: PrivateData,
    #[cfg(test)]
    budget_observer: Option<Arc<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    publication_fault: Option<Arc<crate::durable_state::app_files::TestPublicationFault>>,
    #[cfg(test)]
    panic_after_publish: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Replace {
    path: String,
    contents_base64: String,
}
impl AppFilesBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        catalog: Arc<EventCatalog>,
        admission: Arc<Semaphore>,
        data: PrivateData,
    ) -> Self {
        Self {
            store,
            owner,
            catalog,
            admission,
            data,
            #[cfg(test)]
            budget_observer: None,
            #[cfg(test)]
            publication_fault: None,
            #[cfg(test)]
            panic_after_publish: false,
        }
    }
    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        let budget = AppOperationBudget::from_broker(&request);
        #[cfg(test)]
        let budget = match &self.budget_observer {
            Some(observe) => budget.fixture_observe_checks(observe.clone()),
            None => budget,
        };
        budget
            .check()
            .map_err(|_| error("APP_OPERATION_STOPPED", false))?;
        if request.method != "files.atomic_replace" {
            return Err(error("METHOD_UNAVAILABLE", false));
        }
        let params: Replace =
            serde_json::from_value(request.params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        if params.contents_base64.len() > ((512 * 1024 + 2) / 3) * 4 {
            return Err(error("INVALID_ARGUMENT", false));
        }
        let bytes = STANDARD
            .decode(params.contents_base64)
            .map_err(|_| error("INVALID_ARGUMENT", false))?;
        if bytes.len() > 512 * 1024 {
            return Err(error("INVALID_ARGUMENT", false));
        }
        let bytes_written = bytes.len();
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("APP_BUSY", true))?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            // Admission/preparation remain owned if the awaiting SDK caller is cancelled.
            let _permit = permit;
            budget.check()?;
            let staged = service.data.prepare_replace(&params.path, &bytes)?;
            budget.check()?;
            #[cfg(test)]
            if let Some(fault) = service.publication_fault {
                let result = service.store.fixture_publish_app_file(
                    &service.owner,
                    service.catalog,
                    staged,
                    budget,
                    fault,
                );
                if result.is_ok() && service.panic_after_publish {
                    panic!("fixture lost blocking-task completion after file publication");
                }
                return result;
            }
            service
                .store
                .publish_app_file(&service.owner, service.catalog, staged, budget)
        })
        .await
        .map_err(|_| error("APP_FILE_OUTCOME_UNCERTAIN", false))?
        .map_err(|failure: AppFileError| {
            use chariox_app_runtime::worker_process::PrivateDataError;
            match failure {
                AppFileError::File(PrivateDataError::OutcomeUncertain) => {
                    error("APP_FILE_OUTCOME_UNCERTAIN", false)
                }
                AppFileError::File(PrivateDataError::Invalid) => error("INVALID_ARGUMENT", false),
                AppFileError::Stopped(_) => error("APP_OPERATION_STOPPED", false),
                AppFileError::Catalog(_) => error("APP_STALE", false),
                _ => error("APP_FILE_UNAVAILABLE", false),
            }
        })?;
        Ok(serde_json::json!({"bytesWritten":bytes_written}))
    }
}
#[cfg(test)]
mod tests;
fn error(code: &str, retryable: bool) -> RemoteError {
    let message = match code {
        "APP_FILE_OUTCOME_UNCERTAIN" => {
            "Private file publication may have completed; inspect it before retrying"
        }
        "APP_BUSY" => "App operation limit reached",
        "INVALID_ARGUMENT" => "Invalid private file replacement",
        "APP_STALE" => "App installation authority changed",
        _ => "Private file operation did not complete",
    };
    RemoteError {
        code: code.into(),
        message: message.into(),
        retryable: Some(retryable),
    }
}
