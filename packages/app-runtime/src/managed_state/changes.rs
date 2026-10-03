use super::*;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct StateCheck {
    pub key: String,
    /// None means absent, including a key that was deleted earlier.
    pub version: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum StateWrite {
    Put { key: String, value: Value },
    Delete { key: String },
}

/// Prevalidated bounded changes. No occurrence is silently dropped here: this
/// is the structured-state part used by the broker's encompassing transaction.
#[derive(Debug, Clone)]
pub struct StateChanges {
    pub(super) schema_version: u32,
    pub(super) checks: Vec<StateCheck>,
    pub(super) writes: Vec<(String, Option<String>)>,
}

impl StateChanges {
    pub fn new(
        schema_version: u32,
        checks: Vec<StateCheck>,
        writes: Vec<StateWrite>,
    ) -> Result<Self> {
        if checks.len() > MAX_CHECKS || writes.len() > MAX_CHANGES {
            return Err(StateError::Limit);
        }
        let mut seen = BTreeSet::new();
        let mut bytes = 0_usize;
        for check in &checks {
            key(&check.key)?;
            if !seen.insert(&check.key)
                || check
                    .version
                    .is_some_and(|version| version == 0 || version > MAX_REVISION)
            {
                return Err(StateError::Invalid);
            }
            bytes += check.key.len();
        }
        let mut encoded = Vec::with_capacity(writes.len());
        let mut seen = BTreeSet::new();
        for write in writes {
            let (name, value) = match write {
                StateWrite::Put { key, value } => {
                    // Bound traversal before serialization, including values
                    // supplied by internal callers rather than the IPC decoder.
                    value_budget(&value, 0, &mut 0, &mut 0)?;
                    let text = serde_json::to_string(&value).map_err(|_| StateError::Invalid)?;
                    if text.len() > MAX_VALUE_BYTES {
                        return Err(StateError::Limit);
                    }
                    (key, Some(text))
                }
                StateWrite::Delete { key } => (key, None),
            };
            key(&name)?;
            bytes += name.len() + value.as_ref().map_or(0, String::len);
            if bytes > MAX_CHANGE_BYTES {
                return Err(StateError::Limit);
            }
            if !seen.insert(name.clone()) {
                return Err(StateError::Invalid);
            }
            encoded.push((name, value));
        }
        Ok(Self {
            schema_version,
            checks,
            writes: encoded,
        })
    }
}

pub(super) fn key(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_KEY_BYTES || value.chars().any(char::is_control) {
        return Err(StateError::Invalid);
    }
    Ok(())
}

fn value_budget(value: &Value, depth: usize, nodes: &mut usize, bytes: &mut usize) -> Result<()> {
    *nodes += 1;
    if depth > 32 || *nodes > 16384 {
        return Err(StateError::Limit);
    }
    match value {
        Value::String(text) => *bytes = bytes.saturating_add(text.len()),
        Value::Array(items) => {
            for item in items {
                value_budget(item, depth + 1, nodes, bytes)?;
            }
        }
        Value::Object(fields) => {
            for (name, item) in fields {
                *bytes = bytes.saturating_add(name.len());
                if *bytes > MAX_VALUE_BYTES {
                    return Err(StateError::Limit);
                }
                value_budget(item, depth + 1, nodes, bytes)?;
            }
        }
        _ => {}
    }
    if *bytes > MAX_VALUE_BYTES {
        return Err(StateError::Limit);
    }
    Ok(())
}
