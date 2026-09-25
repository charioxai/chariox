//! SDK `validation.request` / `validation.status`. Only actions the signed
//! package declares critical can be requested; parameters are checked against
//! the action's own schema and bound, canonically, into a durable operation.
//! The call returns a pending reference at once and holds no worker slot while
//! a person decides through the kernel's trusted interaction.
use crate::durable_state::{
    app_validations::{self, ValidationCommand, ValidationOperation, ValidationState},
    DurableKernelStateStore,
};
use chariox_app_package::{compile_schema, Limits, VerifiedPackage};
use chariox_app_runtime::{wire::RemoteError, worker_peer::BrokerRequest};
use jsonschema::JSONSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Semaphore;

const MAX_PARAMETERS_BYTES: usize = 16 * 1024;

struct CriticalAction {
    schema: JSONSchema,
}

#[derive(Clone)]
pub(crate) struct AppValidationBroker {
    store: DurableKernelStateStore,
    owner: String,
    installation: String,
    generation: u64,
    actions: Arc<BTreeMap<String, CriticalAction>>,
    admission: Arc<Semaphore>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    action: String,
    parameters: Value,
    /// Re-request an existing operation (idempotent) instead of a new one.
    #[serde(default)]
    operation_id: Option<String>,
    #[serde(default)]
    connection_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Status {
    operation_id: String,
}

impl AppValidationBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        installation: String,
        generation: u64,
        package: &VerifiedPackage<'_>,
        admission: Arc<Semaphore>,
    ) -> Option<Self> {
        let limits = Limits::default();
        let mut actions = BTreeMap::new();
        for action in &package.declarations().actions {
            if action.critical_validation.is_none() {
                continue;
            }
            // Signed schemas were validated at install; a failure here is a
            // provenance error, so the broker refuses to start.
            let schema = compile_schema(&action.input_schema, true, &limits).ok()?;
            actions.insert(action.name.clone(), CriticalAction { schema });
        }
        Some(Self {
            store,
            owner,
            installation,
            generation,
            actions: Arc::new(actions),
            admission,
        })
    }

    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| error("APP_BUSY", true))?;
        let service = self.clone();
        let method = request.method.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match method.as_str() {
                "validation.request" => service.request(request.params),
                "validation.status" => service.status(request.params),
                _ => Err(error("METHOD_UNAVAILABLE", false)),
            }
        })
        .await
        .map_err(|_| error("STORAGE_UNAVAILABLE", true))?
    }

    fn request(&self, params: Value) -> Result<Value, RemoteError> {
        let request: Request =
            serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        if request.connection_id.is_some() {
            // Scoped service connections are not part of Phase 1 validation.
            return Err(error("METHOD_UNAVAILABLE", false));
        }
        let action = self
            .actions
            .get(&request.action)
            .ok_or_else(|| error("UNDECLARED_ACTION", false))?;
        if serde_json::to_vec(&request.parameters)
            .map_or(true, |bytes| bytes.len() > MAX_PARAMETERS_BYTES)
            || !action.schema.is_valid(&request.parameters)
        {
            return Err(error("INVALID_ARGUMENT", false));
        }
        let (parameters, digest) = app_validations::canonical(&request.parameters);
        if let Some(operation_id) = request.operation_id {
            // Idempotent re-request: only the identical binding matches.
            let existing = self
                .store
                .app_validation_status(&self.owner, &self.installation, &operation_id)
                .map_err(|code| error(code, true))?
                .filter(|operation| {
                    operation.action == request.action
                        && operation.digest == digest
                        && operation.generation == self.generation
                })
                .ok_or_else(|| error("NOT_FOUND", false))?;
            return Ok(reply(&existing));
        }
        let operation = ValidationOperation {
            operation_id: format!("validation-{:032x}", rand::random::<u128>()),
            owner: self.owner.clone(),
            installation: self.installation.clone(),
            generation: self.generation,
            action: request.action,
            parameters,
            digest,
            state: ValidationState::Pending,
            expires_ms: crate::session::unix_epoch_ms() + app_validations::PENDING_MS,
        };
        let created = self
            .store
            .app_validation(ValidationCommand::Create(operation))
            .map_err(|code| error(code, code == "STORAGE_UNAVAILABLE"))?
            .ok_or_else(|| error("STORAGE_UNAVAILABLE", true))?;
        Ok(reply(&created))
    }

    fn status(&self, params: Value) -> Result<Value, RemoteError> {
        let status: Status =
            serde_json::from_value(params).map_err(|_| error("INVALID_ARGUMENT", false))?;
        let operation = self
            .store
            .app_validation_status(&self.owner, &self.installation, &status.operation_id)
            .map_err(|code| error(code, true))?
            .ok_or_else(|| error("NOT_FOUND", false))?;
        Ok(reply(&operation))
    }
}

/// The SDK's `ValidationOperation`; a consumed approval reads as approved.
fn reply(operation: &ValidationOperation) -> Value {
    let state = match operation.state {
        ValidationState::Consumed => "approved",
        state => state.name(),
    };
    json!({ "operationId": operation.operation_id, "state": state })
}

fn error(code: &str, retryable: bool) -> RemoteError {
    let message = match code {
        "UNDECLARED_ACTION" => "The action is not declared as a critical action by this App",
        "INVALID_ARGUMENT" => "Invalid validation request",
        "NOT_FOUND" => "No such validation operation",
        "LIMIT_EXCEEDED" => "Too many open validation requests",
        "APP_BUSY" => "App operation limit reached",
        _ => "Validation request did not complete",
    };
    RemoteError {
        code: code.into(),
        message: message.into(),
        retryable: Some(retryable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chariox_app_package::{pack, verify, Manifest, TrustedPublisher, VerificationPolicy};
    use ed25519_dalek::SigningKey;

    fn package() -> (Vec<u8>, TrustedPublisher) {
        let key = SigningKey::from_bytes(&[73; 32]);
        let mut manifest: Manifest = serde_json::from_value(json!({
            "schema":"chariox.app.v1","appId":"com.example.pay","version":"1.0.0",
            "publisher":{"id":"com.example","keyId":"pay-key","name":"Developer"},
            "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION,"appContractVersion":1,
            "minKernelProtocol":crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
            "ui":{"entry":"ui/index.html"},"capabilities":{
                "network":[{"origin":"https://pay.example.com","methods":["POST"]}]}
        }))
        .unwrap();
        manifest.actions = Some("schemas/actions.json".into());
        let actions = json!({"actions":[
            {"name":"send_payment","inputSchema":{"type":"object","additionalProperties":false,
                "required":["to","amount"],"properties":{"to":{"type":"string"},"amount":{"type":"integer"}}},
             "criticalValidation":{"reason":"Moves money","userVerification":false},
             "effectRoutes":[{"origin":"https://pay.example.com","method":"POST","path":"/payments","connection":"pay"}]},
            {"name":"refresh","inputSchema":{"type":"object","additionalProperties":false,"properties":{}}}
        ]});
        let files = std::collections::BTreeMap::from([
            (
                "runtime/main.js".into(),
                b"export default function register() {}".to_vec(),
            ),
            (
                "ui/index.html".into(),
                b"<!doctype html><title>Pay</title>".to_vec(),
            ),
            (
                "schemas/actions.json".into(),
                serde_json::to_vec(&actions).unwrap(),
            ),
        ]);
        let bytes = pack(&manifest, &files, &key, &Limits::default()).unwrap();
        let publisher = TrustedPublisher {
            publisher_id: "com.example".into(),
            key_id: "pay-key".into(),
            public_key: key.verifying_key(),
        };
        (bytes, publisher)
    }

    #[test]
    fn only_declared_critical_actions_with_valid_parameters_become_pending_operations() {
        let root = std::env::temp_dir().join(format!(
            "chariox-validation-broker-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let (bytes, publisher) = package();
        let verified = verify(
            &bytes,
            &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
        )
        .unwrap();
        let broker = AppValidationBroker::new(
            store.clone(),
            "alice".into(),
            "pay".into(),
            2,
            &verified,
            Arc::new(Semaphore::new(1)),
        )
        .unwrap();
        let code = |result: Result<Value, RemoteError>| result.unwrap_err().code;
        assert_eq!(
            code(broker.request(json!({"action":"refresh","parameters":{}}))),
            "UNDECLARED_ACTION",
            "not critical"
        );
        assert_eq!(
            code(broker.request(json!({"action":"delete_all","parameters":{}}))),
            "UNDECLARED_ACTION"
        );
        assert_eq!(
            code(broker.request(json!({"action":"send_payment","parameters":{"to":"x"}}))),
            "INVALID_ARGUMENT"
        );
        assert_eq!(code(broker.request(json!({"action":"send_payment","parameters":{"to":"x","amount":5},"connectionId":"c"}))), "METHOD_UNAVAILABLE");
        let pending = broker
            .request(json!({"action":"send_payment","parameters":{"amount":5,"to":"x"}}))
            .unwrap();
        assert_eq!(pending["state"], "pending");
        let id = pending["operationId"].as_str().unwrap().to_owned();
        // The same operation is found again only for the identical binding.
        let again = broker.request(json!({"action":"send_payment","parameters":{"to":"x","amount":5},"operationId":id})).unwrap();
        assert_eq!(again["operationId"], id.as_str());
        assert_eq!(
            code(broker.request(
                json!({"action":"send_payment","parameters":{"to":"x","amount":6},"operationId":id})
            )),
            "NOT_FOUND"
        );
        assert_eq!(
            broker.status(json!({"operationId":id})).unwrap()["state"],
            "pending"
        );
        // Another installation of the same owner cannot read it.
        let other = AppValidationBroker::new(
            store,
            "alice".into(),
            "other".into(),
            2,
            &verified,
            Arc::new(Semaphore::new(1)),
        )
        .unwrap();
        assert_eq!(code(other.status(json!({"operationId":id}))), "NOT_FOUND");
        drop((broker, other));
        let _ = std::fs::remove_dir_all(root);
    }
}
