use super::{errors, AppStateOperation};
use chariox_app_runtime::{
    managed_state::{StateChanges, StateCheck, StateWrite, MAX_CHANGES, MAX_CHECKS, MAX_KEY_BYTES},
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
        "state.transaction" => {
            let mut object = fields(
                params,
                &["schemaVersion", "checks", "writes", "occurrences"],
            )?;
            let schema = take(&mut object, "schemaVersion")?
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(errors::invalid)?;
            if let Some(occurrences) = object.remove("occurrences") {
                let Value::Array(occurrences) = occurrences else {
                    return Err(errors::invalid());
                };
                if !occurrences.is_empty() {
                    return Err(errors::unsupported_occurrences());
                }
            }
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
            Ok(AppStateOperation::Transaction(changes))
        }
        _ => Err(errors::unknown_method()),
    }
}
fn fields(value: Value, allowed: &[&str]) -> Result<Map<String, Value>> {
    let Value::Object(object) = value else {
        return Err(errors::invalid());
    };
    if object.len() > allowed.len() || object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(errors::invalid());
    }
    Ok(object)
}
fn take(object: &mut Map<String, Value>, name: &str) -> Result<Value> {
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
fn key(value: Value) -> Result<String> {
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
