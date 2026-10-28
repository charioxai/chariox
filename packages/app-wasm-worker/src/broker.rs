//! WIT-to-wire conversions. No ambient IO is linked by this module.
use crate::{
    bindings::chariox::app::{
        app_types as types, connections, events, host, schedule, state, validation,
    },
    host::{app_error, json, Host},
    ipc::error,
    scope::Scope,
};
use serde_json::{json, Value};
use wasmtime::component::{Accessor, HasSelf};

pub fn context<T: Send>(
    accessor: &Accessor<T, HasSelf<Host>>,
) -> Result<(crate::ipc::Ipc, Scope), types::AppError> {
    accessor.with(|mut access| {
        let host = access.get();
        Ok((host.ipc.clone(), host.scope()?))
    })
}
fn invalid() -> types::AppError {
    app_error(error("APP_BROKER_PROTOCOL", "invalid broker response"))
}
pub fn text(value: &Value, key: &str) -> Result<String, types::AppError> {
    value[key].as_str().map(String::from).ok_or_else(invalid)
}
pub fn number(value: &Value, key: &str) -> Result<u64, types::AppError> {
    value[key].as_u64().ok_or_else(invalid)
}
fn receipt(value: &Value) -> Result<types::Receipt, types::AppError> {
    Ok(types::Receipt {
        receipt_id: text(value, "receiptId")?,
        state: text(value, "state")?,
    })
}
pub fn wake(value: &types::Wake) -> Value {
    json!({"id": value.id, "dueAtMs": value.due_at_ms, "revision": value.revision})
}
fn wakes(value: Value) -> Result<Vec<types::Wake>, types::AppError> {
    value["wakes"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|value| {
            Ok(types::Wake {
                id: text(value, "id")?,
                due_at_ms: number(value, "dueAtMs")?,
                revision: value["revision"].as_str().map(String::from),
            })
        })
        .collect()
}
fn occurrence(value: types::Occurrence) -> Result<Value, types::AppError> {
    let artifacts: Vec<Value> = value.invocation.artifacts.into_iter().map(|artifact| json!({"name": artifact.name, "mediaType": artifact.media_type, "reference": artifact.reference, "sizeBytes": artifact.size_bytes, "digest": artifact.digest})).collect();
    let mut result = json!({"automationId": value.automation_id, "occurrenceId": value.occurrence_id, "eventVersion": value.event_version, "occurredAtMs": value.occurred_at_ms, "payload": json(&value.payload)?, "invocation": {"prompt": value.invocation.prompt, "artifacts": artifacts}});
    if let Some(revision) = value.schedule_revision {
        result["scheduleRevision"] = revision.into();
    }
    Ok(result)
}
async fn call<T: Send>(
    accessor: &Accessor<T, HasSelf<Host>>,
    method: &str,
    params: Value,
) -> Result<Value, types::AppError> {
    let (ipc, scope) = context(accessor)?;
    ipc.call(&scope, method, params).await.map_err(app_error)
}
impl state::Host for Host {}
impl<T: Send> state::HostWithStore<T> for HasSelf<Host> {
    async fn get(
        accessor: &Accessor<T, Self>,
        key: String,
    ) -> Result<Option<state::StateRecord>, types::AppError> {
        let result = call(accessor, "state.get", json!({"key":key})).await?;
        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(state::StateRecord {
            value: result.get("value").ok_or_else(invalid)?.to_string(),
            version: number(&result, "version")?,
        }))
    }
    async fn transact(
        accessor: &Accessor<T, Self>,
        tx: state::Transaction,
    ) -> Result<state::Committed, types::AppError> {
        let checks: Vec<_> = tx
            .checks
            .into_iter()
            .map(|check| json!({"key":check.key,"version":check.version}))
            .collect();
        let writes = tx
            .writes
            .into_iter()
            .map(|write| match write {
                state::Write::Put((key, value)) => Ok(json!({"key":key,"value":json(&value)?})),
                state::Write::Delete(key) => Ok(json!({"key":key,"delete":true})),
            })
            .collect::<Result<Vec<_>, types::AppError>>()?;
        let occurrences = tx
            .occurrences
            .into_iter()
            .map(occurrence)
            .collect::<Result<Vec<_>, _>>()?;
        let wakes: Vec<_> = tx
            .wakes
            .into_iter()
            .map(|change| match change {
                state::WakeChange::Set(value) => {
                    let mut result = wake(&value);
                    result["op"] = "set".into();
                    result
                }
                state::WakeChange::Cancel(id) => json!({"op":"cancel","id":id}),
            })
            .collect();
        let value = call(accessor,"state.transaction",json!({"schemaVersion":tx.schema_version,"checks":checks,"writes":writes,"occurrences":occurrences,"wakes":wakes})).await?;
        let receipts = value["receipts"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(receipt)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(state::Committed {
            revision: number(&value, "revision")?,
            receipts,
        })
    }
}
impl schedule::Host for Host {}
impl<T: Send> schedule::HostWithStore<T> for HasSelf<Host> {
    async fn set(
        accessor: &Accessor<T, Self>,
        value: types::Wake,
    ) -> Result<Vec<types::Wake>, types::AppError> {
        wakes(call(accessor, "schedule.set", wake(&value)).await?)
    }
    async fn cancel(
        accessor: &Accessor<T, Self>,
        id: String,
    ) -> Result<Vec<types::Wake>, types::AppError> {
        wakes(call(accessor, "schedule.cancel", json!({"id":id})).await?)
    }
    async fn list(accessor: &Accessor<T, Self>) -> Result<Vec<types::Wake>, types::AppError> {
        wakes(call(accessor, "schedule.list", json!({})).await?)
    }
}
impl events::Host for Host {
    fn occurrence_id(
        &mut self,
        source_key: String,
        occurred_at_ms: u64,
    ) -> Result<String, types::AppError> {
        use sha2::{Digest, Sha256};
        if source_key.is_empty()
            || source_key.len() > 4096
            || occurred_at_ms > 9_007_199_254_740_991
        {
            return Err(app_error(error(
                "INVALID_ARGUMENT",
                "invalid occurrence identity",
            )));
        }
        let digest = Sha256::digest(
            json!(["chariox.app-occurrence-id.v1", source_key, occurred_at_ms])
                .to_string()
                .as_bytes(),
        );
        Ok(format!("evt1.{occurred_at_ms}.{digest:x}"))
    }
}
impl<T: Send> events::HostWithStore<T> for HasSelf<Host> {
    async fn emit(
        accessor: &Accessor<T, Self>,
        value: types::Occurrence,
    ) -> Result<types::Receipt, types::AppError> {
        receipt(&call(accessor, "events.emit", occurrence(value)?).await?)
    }
    async fn status(
        accessor: &Accessor<T, Self>,
        receipt_id: String,
    ) -> Result<types::Receipt, types::AppError> {
        receipt(&call(accessor, "events.status", json!({"receiptId":receipt_id})).await?)
    }
    async fn retry(
        accessor: &Accessor<T, Self>,
        receipt_id: String,
    ) -> Result<types::Receipt, types::AppError> {
        receipt(&call(accessor, "events.retry", json!({"receiptId":receipt_id})).await?)
    }
    async fn automations(
        accessor: &Accessor<T, Self>,
    ) -> Result<Vec<events::Automation>, types::AppError> {
        call(accessor, "events.automations", json!({})).await?["automations"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|value| {
                Ok(events::Automation {
                    automation_id: text(value, "automationId")?,
                    event: text(value, "event")?,
                    event_version: number(value, "eventVersion")?
                        .try_into()
                        .map_err(|_| invalid())?,
                    state: text(value, "state")?,
                    last_receipt: if value["lastReceipt"].is_null() {
                        None
                    } else {
                        Some(receipt(&value["lastReceipt"])?)
                    },
                })
            })
            .collect()
    }
    async fn subscribe(
        accessor: &Accessor<T, Self>,
        name: String,
    ) -> Result<wasmtime::component::StreamReader<types::LiveEvent>, types::AppError> {
        crate::streams::subscribe(accessor, name)
    }
}
impl connections::Host for Host {}
impl<T: Send> connections::HostWithStore<T> for HasSelf<Host> {
    async fn list(
        accessor: &Accessor<T, Self>,
    ) -> Result<Vec<connections::Connection>, types::AppError> {
        call(accessor, "connections.list", json!({})).await?["connections"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|value| {
                Ok(connections::Connection {
                    generator_id: text(value, "generatorId")?,
                    connection_id: text(value, "connectionId")?,
                    actions: serde_json::from_value(value["actions"].clone())
                        .map_err(|_| invalid())?,
                })
            })
            .collect()
    }
    async fn action(
        accessor: &Accessor<T, Self>,
        request: connections::ActionRequest,
    ) -> Result<connections::ActionResult, types::AppError> {
        let mut params = json!({"connectionId": request.connection_id,"action":request.action});
        if let Some(value) = request.input {
            params["input"] = json(&value)?;
        }
        if let Some(value) = request.context {
            params["context"] = json(&value)?;
        }
        if let Some(value) = request.idempotency_key {
            params["idempotencyKey"] = value.into();
        }
        let value = call(accessor, "connections.action", params).await?;
        Ok(connections::ActionResult {
            accepted: value["accepted"].as_bool().ok_or_else(invalid)?,
            result: value.get("result").ok_or_else(invalid)?.to_string(),
            idempotency_key: text(&value, "idempotencyKey")?,
        })
    }
}
impl host::Host for Host {}
impl<T: Send> host::HostWithStore<T> for HasSelf<Host> {
    async fn notify(
        accessor: &Accessor<T, Self>,
        title: String,
        body: Option<String>,
    ) -> Result<String, types::AppError> {
        let value = call(accessor, "host.notify", json!({"title":title,"body":body})).await?;
        text(&value, "notificationId")
    }
    async fn open_link(
        accessor: &Accessor<T, Self>,
        url: String,
    ) -> Result<host::Offer, types::AppError> {
        let value = call(accessor, "host.open_link", json!({"url":url})).await?;
        Ok(host::Offer {
            offer_id: text(&value, "operationId")?,
            state: text(&value, "state")?,
        })
    }
    async fn write_clipboard(
        accessor: &Accessor<T, Self>,
        text_value: String,
    ) -> Result<host::Offer, types::AppError> {
        let value = call(accessor, "host.clipboard_write", json!({"text":text_value})).await?;
        Ok(host::Offer {
            offer_id: text(&value, "operationId")?,
            state: text(&value, "state")?,
        })
    }
    async fn pick_file(
        accessor: &Accessor<T, Self>,
        request: host::Pick,
    ) -> Result<Vec<String>, types::AppError> {
        let mut value = call(
            accessor,
            "host.pick_file",
            json!({"multiple":request.multiple,"accept":request.accept}),
        )
        .await?;
        loop {
            match value["state"].as_str() {
                Some("granted") => {
                    return serde_json::from_value(value["grantIds"].clone()).map_err(|_| invalid())
                }
                Some("pending") => {}
                Some("cancelled") => {
                    return Err(app_error(error("CANCELLED", "file selection cancelled")))
                }
                _ => return Err(invalid()),
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            value = call(
                accessor,
                "host.pick_file_status",
                json!({"operationId":text(&value,"operationId")?}),
            )
            .await?;
        }
    }
}
fn operation(value: Value) -> Result<validation::Operation, types::AppError> {
    let state = match value["state"].as_str() {
        Some("pending") => validation::OpState::Pending,
        Some("approved") => validation::OpState::Approved,
        Some("denied") => validation::OpState::Denied,
        Some("expired") => validation::OpState::Expired,
        Some("cancelled") => validation::OpState::Cancelled,
        Some("reconciliation") => validation::OpState::Reconciliation,
        _ => return Err(invalid()),
    };
    Ok(validation::Operation {
        operation_id: text(&value, "operationId")?,
        state,
    })
}
impl validation::Host for Host {}
impl<T: Send> validation::HostWithStore<T> for HasSelf<Host> {
    async fn request(
        accessor: &Accessor<T, Self>,
        request: validation::ApprovalRequest,
    ) -> Result<validation::Operation, types::AppError> {
        let mut params = json!({"action":request.action,"parameters":json(&request.parameters)?});
        if let Some(value) = request.operation_id {
            params["operationId"] = value.into();
        }
        if let Some(value) = request.connection_id {
            params["connectionId"] = value.into();
        }
        operation(call(accessor, "validation.request", params).await?)
    }
    async fn status(
        accessor: &Accessor<T, Self>,
        operation_id: String,
    ) -> Result<validation::Operation, types::AppError> {
        operation(
            call(
                accessor,
                "validation.status",
                json!({"operationId":operation_id}),
            )
            .await?,
        )
    }
}
