//! Only declared outgoing occurrence data and opaque receipt references cross
//! this decoder. Workflow targets, owners and binding revisions come from SQL.

use super::{decode, errors, AppStateOperation};
use chariox_app_runtime::{
    app_outbox::{AppOutbox, Occurrence, MAX_BATCH},
    wire::RemoteError,
};
use serde_json::Value;

type Result<T> = std::result::Result<T, RemoteError>;

pub(super) fn operation(method: &str, params: Value) -> Result<AppStateOperation> {
    match method {
        "events.emit" => {
            let occurrence = occurrence(params)?;
            AppOutbox::validate_occurrences(std::slice::from_ref(&occurrence))
                .map_err(errors::outbox)?;
            Ok(AppStateOperation::Emit(occurrence))
        }
        "events.status" | "events.retry" => {
            let mut params = decode::fields(params, &["receiptId"])?;
            let receipt_id = decode::key(decode::take(&mut params, "receiptId")?)?;
            if method == "events.status" {
                Ok(AppStateOperation::Status { receipt_id })
            } else {
                Ok(AppStateOperation::Retry { receipt_id })
            }
        }
        _ => Err(errors::unknown_method()),
    }
}

pub(super) fn occurrences(value: Value) -> Result<Vec<Occurrence>> {
    let Value::Array(values) = value else {
        return Err(errors::invalid());
    };
    if values.len() > MAX_BATCH {
        return Err(errors::limit());
    }
    let occurrences = values
        .into_iter()
        .map(occurrence)
        .collect::<Result<Vec<_>>>()?;
    if !occurrences.is_empty() {
        AppOutbox::validate_occurrences(&occurrences).map_err(errors::outbox)?;
    }
    Ok(occurrences)
}

fn occurrence(value: Value) -> Result<Occurrence> {
    serde_json::from_value(value).map_err(|_| errors::invalid())
}
