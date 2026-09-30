//! Protocol 359 SDK calls: an App acts through an event generator connection
//! its owner granted it, only with the actions its signed manifest declares
//! for that generator. The kernel calls the generator's reviewed action
//! endpoint as the owner's pseudonymous event owner; the App never holds a
//! provider credential, and the generator binds any reply context it issued.
//!
//! - `connections.list {}` → `{connections: [{generatorId, connectionId, actions}]}`
//! - `connections.action {connectionId, action, input?, context?, idempotencyKey?}`
//!   → `{accepted, result, idempotencyKey}`
use super::app_lifecycle::EventConfig;
use super::app_operation_budget::AppOperationBudget;
use crate::durable_state::DurableKernelStateStore;
use chariox_app_package::ConnectionAccess;
use chariox_app_runtime::{wire::RemoteError, worker_peer::BrokerRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;

const MAX_ACTION_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Action {
    connection_id: String,
    action: String,
    #[serde(default)]
    input: Value,
    #[serde(default)]
    context: Value,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Clone)]
pub(crate) struct AppConnectionBroker {
    store: DurableKernelStateStore,
    owner: String,
    installation: String,
    declared: Arc<Vec<ConnectionAccess>>,
    admission: Arc<Semaphore>,
    config: EventConfig,
}

impl AppConnectionBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        installation: String,
        declared: Vec<ConnectionAccess>,
        admission: Arc<Semaphore>,
        config: EventConfig,
    ) -> Self {
        Self {
            store,
            owner,
            installation,
            declared: Arc::new(declared),
            admission,
            config,
        }
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        let budget = AppOperationBudget::from_broker(&request);
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("APP_BUSY", true))?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            budget
                .check()
                .map_err(|_| error("APP_OPERATION_STOPPED", false))?;
            match request.method.as_str() {
                "connections.list" => service.list(),
                "connections.action" => service.act(request.params),
                _ => Err(error("METHOD_UNAVAILABLE", false)),
            }
        })
        .await
        .map_err(|_| error("APP_CONNECTION_OUTCOME_UNCERTAIN", false))?
    }

    fn list(&self) -> Result<Value, RemoteError> {
        let grants = self
            .store
            .app_connection_grants(&self.owner, &self.installation)
            .map_err(|_| error("STORAGE_UNAVAILABLE", true))?;
        Ok(json!({
            "connections": grants
                .into_iter()
                .filter_map(|grant| {
                    let declared = self.declared(&grant.generator_id)?;
                    Some(json!({
                        "generatorId": grant.generator_id,
                        "connectionId": grant.connection_id,
                        "actions": declared.actions,
                    }))
                })
                .collect::<Vec<_>>(),
        }))
    }

    fn act(&self, params: Value) -> Result<Value, RemoteError> {
        if serde_json::to_vec(&params).map_or(true, |bytes| bytes.len() > MAX_ACTION_BYTES) {
            return Err(error("LIMIT_EXCEEDED", false));
        }
        let action: Action =
            serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        if let Some(key) = &action.idempotency_key {
            if key.trim().is_empty() || key.len() > 200 {
                return Err(error("INVALID_ARGUMENT", false));
            }
        }
        let grant = self
            .store
            .app_connection_grants(&self.owner, &self.installation)
            .map_err(|_| error("STORAGE_UNAVAILABLE", true))?
            .into_iter()
            .find(|grant| grant.connection_id == action.connection_id)
            .ok_or_else(|| error("CONNECTION_NOT_GRANTED", false))?;
        if !self
            .declared(&grant.generator_id)
            .is_some_and(|declared| declared.actions.contains(&action.action))
        {
            return Err(error("CAPABILITY_REQUIRED", false));
        }
        let config = self
            .config
            .get()
            .ok_or_else(|| error("CONNECTION_UNAVAILABLE", true))?
            .snapshot();
        // The App's key makes its retries act once; without one, every call
        // is a new action.
        let keyed = action.idempotency_key.is_some();
        let idempotency_key = format!(
            "app:{}:{}",
            self.installation,
            action
                .idempotency_key
                .unwrap_or_else(|| format!("{:032x}", rand::random::<u128>()))
        );
        let request = chariox_event_protocol::AegsProviderActionRequest {
            generator_id: grant.generator_id,
            owner_id: crate::runtime::event_catalog_control::event_connection_owner_id(
                &config.daemon_id,
                &self.owner,
            ),
            connection_id: grant.connection_id,
            action_id: action.action,
            input: action.input,
            context: action.context,
            idempotency_key,
        };
        request
            .validate()
            .map_err(|_| error("INVALID_ARGUMENT", false))?;
        let response = crate::runtime::event_catalog_control::invoke_aegs_action(
            &config.event_generator_management_targets,
            &request,
        )
        .map_err(|failure| {
            // The generator may have acted: never replay it, and a retry is
            // safe only under the App's own idempotency key (V1-INT-08).
            if failure.outcome_unknown() {
                error("APP_CONNECTION_OUTCOME_UNCERTAIN", keyed)
            } else {
                error("CONNECTION_ACTION_FAILED", failure.retryable())
            }
        })?;
        Ok(json!({
            "accepted": response.accepted,
            "result": response.result,
            "idempotencyKey": response.idempotency_key,
        }))
    }

    fn declared(&self, generator_id: &str) -> Option<&ConnectionAccess> {
        self.declared
            .iter()
            .find(|declared| declared.generator == generator_id)
    }
}

fn error(code: &str, retryable: bool) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: code.to_ascii_lowercase().replace('_', " "),
        retryable: Some(retryable),
    }
}
