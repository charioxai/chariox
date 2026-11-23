use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};

use crate::error::DaemonError;

use super::DurableKernelStateStore;

const REVOKED: &str = "owner_view.revoked";
/// Cloud enrollment re-owns the local owner's revocations: a view the owner took
/// away must stay taken away for the enrolled owner.
const OWNER_ENROLLED: &str = "owner_view.owner_enrolled";

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

#[derive(Deserialize, Serialize)]
struct OwnerEnrollment {
    old: String,
    new: String,
}

pub(super) fn apply_event(tx: &Transaction<'_>, kind: &str, payload: &str) -> rusqlite::Result<()> {
    if kind == OWNER_ENROLLED {
        let entry: OwnerEnrollment = serde_json::from_str(payload)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        tx.execute(
            "INSERT OR IGNORE INTO owner_view_revocations (realm_id, owner_id, agent_id)
             SELECT realm_id, ?2, agent_id FROM owner_view_revocations WHERE owner_id = ?1",
            params![entry.old, entry.new],
        )?;
        tx.execute(
            "DELETE FROM owner_view_revocations WHERE owner_id = ?1",
            params![entry.old],
        )?;
        return Ok(());
    }
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

    pub(crate) fn enroll_owner_view_owner(&self, old: &str, new: &str) -> Result<(), DaemonError> {
        if old == new {
            return Ok(());
        }
        self.append_event(
            OWNER_ENROLLED,
            None,
            serde_json::json!(OwnerEnrollment {
                old: old.into(),
                new: new.into(),
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

#[cfg(test)]
mod tests {
    use super::DurableKernelStateStore;

    #[test]
    fn enrollment_keeps_a_revoked_owner_view_revoked_for_the_enrolled_owner_after_restart() {
        let directory = crate::test_support::TestTempDir::new("owner-view-enrollment");
        let path = directory.path().join("kernel.db");
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            store
                .revoke_owner_view("default", "local", "agent-a")
                .unwrap();
            store
                .enroll_owner_view_owner("local", "enrolled-person")
                .unwrap();
            assert!(store
                .owner_view_revoked("default", "enrolled-person", "agent-a")
                .unwrap());
            assert!(!store
                .owner_view_revoked("default", "local", "agent-a")
                .unwrap());
            assert!(!store
                .owner_view_revoked("default", "enrolled-person", "agent-b")
                .unwrap());
        }
        let store = DurableKernelStateStore::open(path).unwrap();
        assert!(store
            .owner_view_revoked("default", "enrolled-person", "agent-a")
            .unwrap());
    }
}
