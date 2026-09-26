//! The structured-state part of an App snapshot: the installation's state
//! values, its state head and migration record, and its pending wakes, read in
//! one transaction. Delivery receipts (outbox, inbox, validations) are never
//! part of it: they cannot be rolled back.
use super::DurableKernelStateStore;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

impl DurableKernelStateStore {
    pub(crate) fn export_app_state(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Value, &'static str> {
        let storage = |_| "STORAGE_UNAVAILABLE";
        let connection = self
            .lock_connection("durable_state.export_app_state")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let tx = connection.unchecked_transaction().map_err(storage)?;
        let mut statement = tx
            .prepare(
                "SELECT key,version,value_json FROM app_state_values WHERE installation_id=?1
                 ORDER BY key",
            )
            .map_err(storage)?;
        let values = statement
            .query_map(params![installation], |row| {
                Ok(json!({
                    "key": row.get::<_, String>(0)?,
                    "version": row.get::<_, i64>(1)?,
                    "value_json": row.get::<_, String>(2)?,
                }))
            })
            .map_err(storage)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage)?;
        let head = tx
            .query_row(
                "SELECT revision,key_count,payload_bytes FROM app_state_heads WHERE installation_id=?1",
                params![installation],
                |row| {
                    Ok(json!({
                        "revision": row.get::<_, i64>(0)?,
                        "key_count": row.get::<_, i64>(1)?,
                        "payload_bytes": row.get::<_, i64>(2)?,
                    }))
                },
            )
            .optional()
            .map_err(storage)?;
        let migration = tx
            .query_row(
                "SELECT generation,head_json,target,migrated FROM app_state_migrations
                 WHERE installation_id=?1",
                params![installation],
                |row| {
                    Ok(json!({
                        "generation": row.get::<_, i64>(0)?,
                        "head_json": row.get::<_, Option<String>>(1)?,
                        "target": row.get::<_, i64>(2)?,
                        "migrated": row.get::<_, i64>(3)?,
                    }))
                },
            )
            .optional()
            .map_err(storage)?;
        let mut statement = tx
            .prepare(
                "SELECT wake_id,due_at_ms,revision FROM app_wakes
                 WHERE owner_id=?1 AND installation_id=?2 ORDER BY wake_id",
            )
            .map_err(storage)?;
        let wakes = statement
            .query_map(params![owner, installation], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "due_at_ms": row.get::<_, i64>(1)?,
                    "revision": row.get::<_, String>(2)?,
                }))
            })
            .map_err(storage)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(storage)?;
        Ok(json!({ "values": values, "head": head, "migration": migration, "wakes": wakes }))
    }

    /// Test fixture: loads an exported state into another installation, as an
    /// isolated restore would. Wakes start with no attempts.
    #[cfg(test)]
    pub(crate) fn fixture_import_app_state(
        &self,
        owner: &str,
        installation: &str,
        state: &Value,
    ) -> rusqlite::Result<()> {
        let connection = rusqlite::Connection::open(self.path())?;
        let tx = connection.unchecked_transaction()?;
        for value in state["values"].as_array().into_iter().flatten() {
            tx.execute(
                "INSERT OR REPLACE INTO app_state_values(installation_id,key,version,value_json)
                 VALUES(?1,?2,?3,?4)",
                params![
                    installation,
                    value["key"].as_str(),
                    value["version"].as_i64(),
                    value["value_json"].as_str()
                ],
            )?;
        }
        if let Some(head) = state["head"].as_object() {
            tx.execute(
                "INSERT OR REPLACE INTO app_state_heads(installation_id,revision,key_count,payload_bytes)
                 VALUES(?1,?2,?3,?4)",
                params![
                    installation,
                    head["revision"].as_i64(),
                    head["key_count"].as_i64(),
                    head["payload_bytes"].as_i64()
                ],
            )?;
        }
        for wake in state["wakes"].as_array().into_iter().flatten() {
            tx.execute(
                "INSERT OR REPLACE INTO app_wakes(owner_id,installation_id,wake_id,due_at_ms,revision,
                   attempts,next_attempt_at_ms) VALUES(?1,?2,?3,?4,?5,0,?4)",
                params![
                    owner,
                    installation,
                    wake["id"].as_str(),
                    wake["due_at_ms"].as_i64(),
                    wake["revision"].as_str()
                ],
            )?;
        }
        tx.commit()
    }
}
