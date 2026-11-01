//! A hard wire budget for diagnostics, including nested collections and text.
use crate::runtime::projection::DaemonHealthProjection;
use serde_json::Value;
use std::collections::BTreeMap;

const MAX_BYTES: usize = 60 * 1024;
const MAX_TEXT_CHARS: usize = 256;

pub(super) fn bound(projection: DaemonHealthProjection) -> DaemonHealthProjection {
    let original = serde_json::to_value(projection).expect("health projection is serializable");
    for limit in [16, 8, 4, 2, 1, 0] {
        let mut value = original.clone();
        let mut counts = BTreeMap::new();
        bound_value(&mut value, "", limit, &mut counts);
        value["sample_counts"] = serde_json::to_value(counts).unwrap();
        if serde_json::to_vec(&value).unwrap().len() <= MAX_BYTES {
            return serde_json::from_value(value).expect("bounded health retains its typed shape");
        }
    }
    unreachable!("fixed health scalars exceed the diagnostic wire budget")
}

fn bound_value(value: &mut Value, path: &str, limit: usize, counts: &mut BTreeMap<String, usize>) {
    match value {
        Value::Array(values) => {
            counts
                .entry(path.to_owned())
                .and_modify(|total| *total += values.len())
                .or_insert(values.len());
            values.truncate(if path == "request_timings" {
                limit * 4
            } else {
                limit
            });
            for value in values {
                bound_value(value, path, limit, counts);
            }
        }
        Value::Object(fields) => {
            for (name, value) in fields {
                let path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                bound_value(value, &path, limit, counts);
            }
        }
        Value::String(text) if text.chars().count() > MAX_TEXT_CHARS => {
            *text = text.chars().take(MAX_TEXT_CHARS - 1).chain(['…']).collect();
        }
        _ => {}
    }
}
