//! MP-08 / MP-10 / MP-11: retire executable workflow instruction references.
//! Names come only from old product records/copies, never arbitrary dollar text.
use serde_json::Value;
use std::collections::BTreeSet;

fn remember(name: &str, names: &mut BTreeSet<String>) {
    if let Ok(key) = crate::skill::catalog::collision_key(name) {
        names.insert(key);
    }
}

pub(crate) fn collect_names(value: &Value, names: &mut BTreeSet<String>) {
    match value {
        Value::Array(values) => values.iter().for_each(|value| collect_names(value, names)),
        Value::Object(fields) => {
            if super::legacy_grant(value) {
                if let Some(name) = fields.get("name").and_then(Value::as_str) {
                    remember(name, names);
                }
            }
            for (key, field) in fields {
                match key.as_str() {
                    "skill_grants" | "required_skills" => {
                        if let Some(values) = field.as_array() {
                            for value in values {
                                if let Some(name) = value
                                    .as_str()
                                    .or_else(|| value.get("name").and_then(Value::as_str))
                                {
                                    remember(name, names);
                                }
                            }
                        }
                    }
                    "workflow_publication_snapshots" => {
                        if let Some(snapshots) = field.as_object() {
                            snapshots
                                .values()
                                .for_each(|snapshot| collect_names(snapshot, names));
                        }
                    }
                    "agent" | "default_agent" | "agents" | "session" | "sessions" | "snapshot"
                    | "requirements" | "workflow" | "workflows" | "nodes" | "definition"
                    | "extensions" | "extension_grants" => collect_names(field, names),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Learn every old name before changing any row, irrespective of replay order.
/// The names checkpoint is cleanup policy, never a catalog or grant decoder.
pub(super) fn capture_replay_names(
    tx: &rusqlite::Transaction<'_>,
    names: &mut BTreeSet<String>,
) -> rusqlite::Result<()> {
    for &(table, kind_column) in super::REPLAY_TABLES {
        let mut cursor = i64::MIN;
        loop {
            let rows = tx.prepare(&format!("SELECT rowid,{kind_column},payload_json FROM {table} WHERE rowid>?1 ORDER BY rowid LIMIT 128"))?
                .query_map([cursor],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if rows.is_empty() {
                break;
            }
            for (id, kind, encoded) in rows {
                cursor = id;
                if super::is_replay_authority(table, &kind) {
                    let value: Value = serde_json::from_str(&encoded)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?;
                    collect_names(&value, names);
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn load_names(connection: &rusqlite::Connection) -> rusqlite::Result<BTreeSet<String>> {
    use rusqlite::OptionalExtension;
    let encoded: Option<String> = connection.query_row(
        "SELECT metadata_value FROM durable_state_metadata WHERE owner_id='skills-v2' AND metadata_key='retired_names'",
        [], |row| row.get(0),
    ).optional()?;
    let names: BTreeSet<String> = encoded
        .map(|encoded| serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()?
        .unwrap_or_default();
    if names
        .iter()
        .any(|name| crate::skill::catalog::collision_key(name).map_or(true, |key| key != *name))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(names)
}

/// The definition stays usable after removing instructions; a binding to old
/// generated launch code cannot attest the rewritten definition. Source files
/// are regenerated separately by the normal workflow-code publisher.
pub(crate) fn retire_definition(value: &mut Value, names: &BTreeSet<String>) {
    if let Some(values) = value.as_array_mut() {
        values
            .iter_mut()
            .for_each(|value| retire_definition(value, names));
        return;
    }
    let before = value.clone();
    super::retire_with_names(value, names);
    retire_body_fields(value, names);
    if *value != before {
        if let Some(fields) = value.as_object_mut() {
            if fields.contains_key("code_source") {
                fields.insert("code_source".into(), Value::Null);
            }
            if let Some(revision) = fields
                .get("revision")
                .and_then(Value::as_u64)
                .and_then(|revision| revision.checked_add(1))
            {
                fields.insert("revision".into(), Value::from(revision));
            }
        }
    }
}

fn retire_body_fields(value: &mut Value, names: &BTreeSet<String>) {
    match value {
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| retire_body_fields(value, names)),
        Value::Object(fields) => {
            for (key, value) in fields {
                match key.as_str() {
                    "prompt" | "instructions" => {
                        if let Some(text) = value.as_str() {
                            *value = Value::String(retire_text(text, names));
                        }
                    }
                    "workflow" | "definition" | "nodes" => retire_body_fields(value, names),
                    // Schemas, examples, arbitrary user JSON and transcript text
                    // are not executable workflow instruction authority.
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn retire_text(text: &str, names: &BTreeSet<String>) -> String {
    let text = super::retire_hidden_context(text);
    let tokens = crate::skill::selection::explicit_skill_tokens(&text);
    let mut result = String::new();
    let mut start = 0;
    for token in tokens {
        if !names.contains(&token.name.to_ascii_lowercase()) {
            continue;
        }
        result.push_str(&text[start..token.range.start]);
        start = token.range.end;
    }
    result.push_str(&text[start..]);
    result
        .split_inclusive('\n')
        .map(|line| {
            let trimmed = line.trim_start();
            let Some(command) = trimmed.strip_prefix("/skill ") else {
                return line.to_owned();
            };
            let name_end = command.find(char::is_whitespace).unwrap_or(command.len());
            if !names.contains(&command[..name_end].to_ascii_lowercase()) {
                return line.to_owned();
            }
            let indent = &line[..line.len() - trimmed.len()];
            let task =
                command[name_end..].trim_start_matches(|c: char| c.is_whitespace() && c != '\n');
            format!("{indent}{task}")
        })
        .collect()
}
