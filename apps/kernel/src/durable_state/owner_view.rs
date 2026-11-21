use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

use super::DurableKernelStateStore;

const REVOKED: &str = "owner_view.revoked";

#[derive(Deserialize, Serialize)]
struct Revocation {
    realm_id: String,
    owner_id: String,
    agent_id: String,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS owner_view_revocations (
            realm_id TEXT NOT NULL, owner_id TEXT NOT NULL, agent_id TEXT NOT NULL,
            PRIMARY KEY (realm_id, owner_id, agent_id)
        );",
    )
}

pub(super) fn apply_event(tx: &Transaction<'_>, kind: &str, payload: &str) -> rusqlite::Result<()> {
    if kind == REVOKED {
        let entry: Revocation = serde_json::from_str(payload)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        tx.execute(
            "INSERT OR IGNORE INTO owner_view_revocations (realm_id, owner_id, agent_id)
             VALUES (?1, ?2, ?3)",
            params![entry.realm_id, entry.owner_id, entry.agent_id],
        )?;
    }
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn revoke_owner_view(
        &self,
        realm_id: &str,
        owner_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        self.append_event(
            REVOKED,
            Some(agent_id.to_owned()),
            serde_json::json!(Revocation {
                realm_id: realm_id.into(),
                owner_id: owner_id.into(),
                agent_id: agent_id.into(),
            }),
        )?;
        Ok(())
    }

    pub(crate) fn owner_view_revoked(
        &self,
        realm_id: &str,
        owner_id: &str,
        agent_id: &str,
    ) -> Result<bool, DaemonError> {
        self.lock_connection("owner_view.read_revocation")?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM owner_view_revocations
                 WHERE realm_id = ?1 AND owner_id = ?2 AND agent_id = ?3)",
                params![realm_id, owner_id, agent_id],
                |row| row.get(0),
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "owner_view.read_revocation",
                message: error.to_string(),
            })
    }
}
