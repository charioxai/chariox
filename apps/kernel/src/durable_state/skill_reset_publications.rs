//! MP-08 / MP-10 / MP-11: retire immutable publication bindings, never edit their bytes.
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub(super) fn affected(snapshot: &Value) -> bool {
    let mut retired = snapshot.clone();
    super::retire(&mut retired);
    &retired != snapshot
}

/// Session snapshots and old session events can carry the same publication
/// projection as the normalized hot store. Drop the pair, preserving unrelated
/// publications and historical text; a normal explicit publish makes a new one.
pub(super) fn retire_session(fields: &mut Map<String, Value>) {
    let Some(snapshots) = fields
        .get_mut("workflow_publication_snapshots")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let removed: BTreeSet<_> = snapshots
        .iter()
        .filter(|(_, snapshot)| affected(snapshot))
        .map(|(id, _)| id.clone())
        .collect();
    snapshots.retain(|id, _| !removed.contains(id));
    if removed.is_empty() {
        return;
    }
    if let Some(publications) = fields
        .get_mut("workflow_publications")
        .and_then(Value::as_array_mut)
    {
        publications.retain(|publication| {
            !publication
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| removed.contains(id))
        });
    }
    if let Some(queued) = fields
        .get_mut("workflow_queued_prompts")
        .and_then(Value::as_array_mut)
    {
        queued.retain(|prompt| !invokes_any(prompt, &removed));
    }
}
fn invokes_any(prompt: &Value, removed: &BTreeSet<String>) -> bool {
    prompt
        .get("publication_invocation")
        .and_then(|invocation| invocation.get("publication_id"))
        .and_then(Value::as_str)
        .is_some_and(|id| removed.contains(id))
}

pub(super) fn retire_hot(tx: &Transaction<'_>, rowid: i64) -> rusqlite::Result<()> {
    let (owner, session, publication): (String, String, String) = tx.query_row(
        "SELECT owner_id,session_id,entity_id FROM durable_workflow_hot_entities WHERE rowid=?1",
        [rowid],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    tx.execute("DELETE FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_id=?3 AND entity_kind IN ('publication','publication_snapshot')", params![owner,session,publication])?;
    let removed = BTreeSet::from([publication]);
    // Queue input is mutable replay authority, not part of the immutable
    // snapshot. Bound every deletion to the exact owner + session + row.
    let mut cursor = i64::MIN;
    loop {
        let rows = tx.prepare("SELECT rowid,payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind='queued_prompt' AND rowid>?3 ORDER BY rowid LIMIT 128")?
            .query_map(params![owner,session,cursor],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            break;
        }
        for (id, encoded) in rows {
            cursor = id;
            let prompt: Value =
                serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
            if invokes_any(&prompt, &removed) {
                tx.execute(
                    "DELETE FROM durable_workflow_hot_entities WHERE rowid=?1",
                    [id],
                )?;
            }
        }
    }
    retire_intents(tx, Some((&owner, &session, &removed)))
}

/// Dispatch intents are a second persisted copy of a pending queue entry.
/// Retire them in the same transaction and terminalize their active runs so a
/// restart cannot replay the entry or leave it consuming workflow capacity.
pub(super) fn retire_intents(
    tx: &Transaction<'_>,
    removed: Option<(&str, &str, &BTreeSet<String>)>,
) -> rusqlite::Result<()> {
    let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='durable_workflow_dispatch_intents')",[],|r|r.get(0))?;
    if !exists {
        return Ok(());
    }
    let mut cursor = i64::MIN;
    loop {
        let rows=tx.prepare("SELECT rowid,owner_id,session_id,run_id,queued_json FROM durable_workflow_dispatch_intents WHERE rowid>?1 ORDER BY rowid LIMIT 128")?
            .query_map([cursor],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            break;
        }
        for (id, owner, session, run, encoded) in rows {
            cursor = id;
            let prompt: Value =
                serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
            let retire = match removed {
                Some((expected_owner, expected_session, publications)) => {
                    owner == expected_owner
                        && session == expected_session
                        && invokes_any(&prompt, publications)
                }
                None => super::queued_selection(&prompt),
            };
            if retire {
                stop_run(tx, &owner, &session, &run)?;
                tx.execute(
                    "DELETE FROM durable_workflow_dispatch_intents WHERE rowid=?1",
                    [id],
                )?;
            }
        }
    }
    Ok(())
}
fn stop_run(tx: &Transaction<'_>, owner: &str, session: &str, run: &str) -> rusqlite::Result<()> {
    let encoded:Option<String>=tx.query_row("SELECT payload_json FROM durable_workflow_runs WHERE owner_id=?1 AND session_id=?2 AND run_id=?3 AND status NOT IN ('Completed','Failed','Stopped')",params![owner,session,run],|r|r.get(0)).optional()?;
    let Some(encoded) = encoded else {
        return Ok(());
    };
    let mut value: Value =
        serde_json::from_str(&encoded).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let fields = value.as_object_mut().ok_or(rusqlite::Error::InvalidQuery)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    fields.insert("status".into(), Value::String("Stopped".into()));
    fields.insert("active_node_run_id".into(), Value::Null);
    fields.insert("completed_at_ms".into(), Value::from(now));
    if let Some(nodes) = fields.get_mut("node_runs").and_then(Value::as_array_mut) {
        for node in nodes {
            if let Some(fields) = node.as_object_mut() {
                if !matches!(
                    fields.get("status").and_then(Value::as_str),
                    Some("Completed" | "Failed" | "Stopped")
                ) {
                    fields.insert("status".into(), Value::String("Stopped".into()));
                    fields.insert("completed_at_ms".into(), Value::from(now));
                }
            }
        }
    }
    let encoded = serde_json::to_string(&value).map_err(|_| rusqlite::Error::InvalidQuery)?;
    tx.execute("UPDATE durable_workflow_runs SET status='Stopped',completed_at_ms=?4,updated_at_ms=?4,payload_json=?5 WHERE owner_id=?1 AND session_id=?2 AND run_id=?3",params![owner,session,run,now,encoded])?;
    Ok(())
}
