//! Typed source normalization happens once at the authoritative kernel boundary.
use crate::durable_state::workflow_gate::error;
use crate::{
    error::DaemonError,
    session::{SessionService, WorkflowDefinition, WorkflowPublicationInvocationEnvelope},
};
use chariox_app_runtime::workflow_gate as gate;
use serde_json::Value;

type HttpSource = (String, gate::JsonObject, Option<gate::DataSchema>);
pub(super) fn http(
    sessions: &SessionService,
    session: &str,
    workflow: &WorkflowDefinition,
    endpoint: &str,
    inv: &WorkflowPublicationInvocationEnvelope,
) -> Result<HttpSource, DaemonError> {
    let publication = sessions.resolve_workflow_publication_ref(session, &inv.publication_id)?;
    if !publication.enabled()
        || publication.workflow_id() != workflow.id()
        || publication.endpoint_id() != endpoint
        || inv.endpoint_id != endpoint
        || inv
            .hook_id
            .as_deref()
            .is_some_and(|id| id != format!("{}-hook", publication.id()))
    {
        return Err(error(
            "gate HTTP publication/hook binding does not match the current endpoint",
        ));
    }
    let mut data = inv
        .input
        .as_object()
        .cloned()
        .ok_or_else(|| error("gate HTTP data must be an object"))?;
    data.remove("prompt");
    let schema = publication.input_schema();
    if inv.http_method.as_deref() == Some("GET") {
        if let Some(schema) = schema {
            for (name, value) in &mut data {
                let text = value
                    .as_str()
                    .ok_or_else(|| error("GET parameters must occur once and be strings"))?;
                let field = schema.get("properties").and_then(|p| p.get(name));
                match field.and_then(|f|f.get("type")).and_then(Value::as_str) {
                    Some("integer")=>{
                        let valid=(text=="0" || !text.starts_with('0')) && text!="-0" && !text.starts_with("-0") && !text.starts_with('+');
                        let number=text.parse::<i64>().map_err(|_|error(format!("GET parameter {name} must be an integer")))?;
                        if !valid || number.unsigned_abs()>gate::MAX_SAFE_INTEGER {return Err(error(format!("GET parameter {name} must be a safe integer")));}
                        *value=number.into();
                    },
                    Some("boolean")=>*value=match text {"true"=>Value::Bool(true),"false"=>Value::Bool(false),_=>return Err(error(format!("GET parameter {name} must be true or false")))},
                    Some("string")|None=>{},
                    _=>return Err(error("GET schemas support string, integer and boolean fields; use POST JSON for other types")),
                }
            }
        } else if data.values().any(|v| !v.is_string()) {
            return Err(error("GET parameters must occur once and be strings"));
        }
    }
    chariox_event_protocol::validate_event_data(Some(&Value::Object(data.clone())))
        .map_err(error)?;
    let identity = if let Some(schema) = schema {
        chariox_event_protocol::validate_payload(Some(schema), Some(&Value::Object(data.clone())))
            .map_err(error)?;
        Some(gate::DataSchema {
            source_kind: "http".into(),
            event_type: publication.id().into(),
            version: 1,
            sha256: chariox_event_protocol::payload_schema_digest(schema).map_err(error)?,
            schema: schema
                .as_object()
                .ok_or_else(|| error("HTTP schema must be an object"))?
                .clone(),
        })
    } else {
        None
    };
    Ok((format!("http:{}", publication.id()), data, identity))
}
