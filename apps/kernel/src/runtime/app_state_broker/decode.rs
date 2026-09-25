use super::{errors, events, AppStateOperation};
use chariox_app_runtime::{
    managed_state::{
        StateChanges, StateCheck, StateWrite, WakeChange, MAX_CHANGES, MAX_CHECKS, MAX_KEY_BYTES,
        MAX_WAKE_CHANGES,
    },
    wire::RemoteError,
};
use serde_json::{Map, Value};

type Result<T> = std::result::Result<T, RemoteError>;

pub(super) fn operation(method: &str, params: Value) -> Result<AppStateOperation> {
    match method {
        "state.get" => {
            let mut object = fields(params, &["key"])?;
            Ok(AppStateOperation::Get {
                key: key(take(&mut object, "key")?)?,
            })
        }
        "migration.step" => {
            let mut object = fields(params, &["to"])?;
            let to = take(&mut object, "to")?
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(errors::invalid)?;
            Ok(AppStateOperation::MigrationStep { to })
        }
        "state.transaction" => {
            let mut object = fields(
                params,
                &["schemaVersion", "checks", "writes", "occurrences", "wakes"],
            )?;
            let schema = take(&mut object, "schemaVersion")?
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(errors::invalid)?;
            let occurrences = match object.remove("occurrences") {
                Some(value) => events::occurrences(value)?,
                None => Vec::new(),
            };
            let wakes = match object.remove("wakes") {
                Some(value) => wake_changes(value)?,
                None => Vec::new(),
            };
            let checks = array(take(&mut object, "checks")?, MAX_CHECKS)?
                .into_iter()
                .map(|value| {
                    let mut object = fields(value, &["key", "version"])?;
                    let name = key(take(&mut object, "key")?)?;
                    let version = match take(&mut object, "version")? {
                        Value::Null => None,
                        value => Some(value.as_u64().ok_or_else(errors::invalid)?),
                    };
                    Ok(StateCheck { key: name, version })
                })
                .collect::<Result<Vec<_>>>()?;
            let writes = array(take(&mut object, "writes")?, MAX_CHANGES)?
                .into_iter()
                .map(|value| {
                    let mut object = fields(value, &["key", "value", "delete"])?;
                    let name = key(take(&mut object, "key")?)?;
                    match (object.remove("value"), object.remove("delete")) {
                        (Some(value), None) => Ok(StateWrite::Put { key: name, value }),
                        (None, Some(Value::Bool(true))) => Ok(StateWrite::Delete { key: name }),
                        _ => Err(errors::invalid()),
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            let changes = StateChanges::new(schema, checks, writes).map_err(errors::changes)?;
            Ok(AppStateOperation::Transaction {
                changes,
                occurrences,
                wakes,
            })
        }
        "schedule.set" => {
            let object = fields(params, &["id", "dueAtMs", "revision"])?;
            let mut change = Map::new();
            change.insert("op".into(), Value::String("set".into()));
            change.extend(object);
            Ok(AppStateOperation::Schedule(wake_changes(Value::Array(
                vec![Value::Object(change)],
            ))?))
        }
        "schedule.cancel" => {
            let mut object = fields(params, &["id"])?;
            let id = key(take(&mut object, "id")?)?;
            Ok(AppStateOperation::Schedule(vec![WakeChange::Cancel { id }]))
        }
        "schedule.list" => {
            fields(params, &[])?;
            Ok(AppStateOperation::ScheduleList)
        }
        _ => events::operation(method, params),
    }
}
fn wake_changes(value: Value) -> Result<Vec<WakeChange>> {
    array(value, MAX_WAKE_CHANGES)?
        .into_iter()
        .map(|change| serde_json::from_value(change).map_err(|_| errors::invalid()))
        .collect()
}
pub(super) fn fields(value: Value, allowed: &[&str]) -> Result<Map<String, Value>> {
    let Value::Object(object) = value else {
        return Err(errors::invalid());
    };
    if object.len() > allowed.len() || object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(errors::invalid());
    }
    Ok(object)
}
pub(super) fn take(object: &mut Map<String, Value>, name: &str) -> Result<Value> {
    object.remove(name).ok_or_else(errors::invalid)
}
fn array(value: Value, limit: usize) -> Result<Vec<Value>> {
    let Value::Array(array) = value else {
        return Err(errors::invalid());
    };
    if array.len() > limit {
        return Err(errors::limit());
    }
    Ok(array)
}
pub(super) fn key(value: Value) -> Result<String> {
    let Value::String(key) = value else {
        return Err(errors::invalid());
    };
    if key.is_empty()
        || key.len() > MAX_KEY_BYTES
        || key
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace() || ch == '\u{feff}')
    {
        return Err(errors::invalid());
    }
    Ok(key)
}
