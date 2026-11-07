//! Protocol 359: event generator connections an owner lets one installation
//! use. A grant names the owner's connection; which actions the App may take
//! through it comes from the App's signed `capabilities.connections`. The
//! kernel checked the connection with its generator before granting it.
use super::{DurableKernelStateStore, DurableWriterRequest};
use rusqlite::{params, Connection};
use std::sync::mpsc;

/// Connections per installation.
const MAX_GRANTS: i64 = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionGrant {
    pub(crate) generator_id: String,
    pub(crate) connection_id: String,
    /// The owner the event services know the connection under, taken from the
    /// connection's record when granted.
    pub(crate) event_owner_id: String,
    pub(crate) granted_at_ms: u64,
}

pub(crate) enum ConnectionGrantCommand {
    Grant {
        owner: String,
        installation: String,
        generator_id: String,
        connection_id: String,
        event_owner_id: String,
        now_ms: u64,
    },
    Revoke {
        owner: String,
        installation: String,
        connection_id: String,
    },
}

pub(super) struct ConnectionGrantRequest {
    command: ConnectionGrantCommand,
    response: mpsc::Sender<Result<(), &'static str>>,
}
impl std::fmt::Debug for ConnectionGrantRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectionGrantRequest(..)")
    }
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_connection_grants (
            owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, connection_id TEXT NOT NULL,
            generator_id TEXT NOT NULL, event_owner_id TEXT NOT NULL,
            granted_at_ms INTEGER NOT NULL,
            PRIMARY KEY(owner_id, installation_id, connection_id));",
    )?;
    // Grants made before connections carried their owner cannot reach their
    // connection; the owner grants again.
    let has_owner = connection
        .prepare(
            "SELECT 1 FROM pragma_table_info('app_connection_grants') WHERE name='event_owner_id'",
        )?
        .exists([])?;
    if !has_owner {
        connection.execute_batch("DROP TABLE app_connection_grants;")?;
        return initialize(connection);
    }
    Ok(())
}

impl DurableKernelStateStore {
    pub(crate) fn app_connection_grant(
        &self,
        command: ConnectionGrantCommand,
    ) -> Result<(), &'static str> {
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppConnectionGrant(Box::new(
                ConnectionGrantRequest { command, response },
            )))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        crate::durable_state::await_writer_response(receiver).map_err(|_| "STORAGE_UNAVAILABLE")?
    }

    pub(crate) fn app_connection_grants(
        &self,
        owner: &str,
        installation: &str,
    ) -> Result<Vec<ConnectionGrant>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_connection_grants")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut statement = connection
            .prepare(
                "SELECT generator_id, connection_id, event_owner_id, granted_at_ms FROM app_connection_grants
                 WHERE owner_id=?1 AND installation_id=?2 ORDER BY generator_id, connection_id",
            )
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let rows = statement
            .query_map(params![owner, installation], |row| {
                Ok(ConnectionGrant {
                    generator_id: row.get(0)?,
                    connection_id: row.get(1)?,
                    event_owner_id: row.get(2)?,
                    granted_at_ms: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        rows.collect::<Result<_, _>>()
            .map_err(|_| "STORAGE_UNAVAILABLE")
    }

    /// The owner and installation of every grant of one connection id, for
    /// active installations only: a grant that outlived its installation is
    /// not a dependency. The caller picks the grants whose owner it means.
    pub(crate) fn app_connection_grantees(
        &self,
        connection_id: &str,
    ) -> Result<Vec<(String, String)>, &'static str> {
        let connection = self
            .lock_connection("durable_state.app_connection_grantees")
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let mut statement = connection
            .prepare(
                "SELECT g.owner_id, g.installation_id FROM app_connection_grants g
                 JOIN app_installations i ON i.installation_id=g.installation_id
                 WHERE g.connection_id=?1 AND i.active_json IS NOT NULL
                 ORDER BY g.owner_id, g.installation_id",
            )
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        let rows = statement
            .query_map(params![connection_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|_| "STORAGE_UNAVAILABLE")?;
        rows.collect::<Result<_, _>>()
            .map_err(|_| "STORAGE_UNAVAILABLE")
    }
}

/// Grants end with the installation's active release: an uninstalled App, or
/// one installed again under the same id, starts with none.
pub(crate) fn forget_inactive(
    connection: &Connection,
    owner: &str,
    installation: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM app_connection_grants WHERE owner_id=?1 AND installation_id=?2
           AND NOT EXISTS (SELECT 1 FROM app_installations
                           WHERE installation_id=?2 AND active_json IS NOT NULL)",
        params![owner, installation],
    )?;
    Ok(())
}

pub(super) fn execute(connection: &mut Connection, request: ConnectionGrantRequest) {
    let result = apply(connection, request.command);
    let _ = request.response.send(result);
}

fn apply(connection: &mut Connection, command: ConnectionGrantCommand) -> Result<(), &'static str> {
    let storage = |_| "STORAGE_UNAVAILABLE";
    let transaction = connection.transaction().map_err(storage)?;
    match command {
        ConnectionGrantCommand::Grant {
            owner,
            installation,
            generator_id,
            connection_id,
            event_owner_id,
            now_ms,
        } => {
            let count: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM app_connection_grants WHERE owner_id=?1 AND installation_id=?2",
                    params![owner, installation],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if count >= MAX_GRANTS {
                return Err("LIMIT_EXCEEDED");
            }
            // Granting again keeps the first grant.
            transaction
                .execute(
                    "INSERT OR IGNORE INTO app_connection_grants
                       (owner_id, installation_id, connection_id, generator_id,
                        event_owner_id, granted_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        owner,
                        installation,
                        connection_id,
                        generator_id,
                        event_owner_id,
                        now_ms as i64
                    ],
                )
                .map_err(storage)?;
        }
        ConnectionGrantCommand::Revoke {
            owner,
            installation,
            connection_id,
        } => {
            let removed = transaction
                .execute(
                    "DELETE FROM app_connection_grants
                     WHERE owner_id=?1 AND installation_id=?2 AND connection_id=?3",
                    params![owner, installation, connection_id],
                )
                .map_err(storage)?;
            if removed == 0 {
                return Err("NOT_FOUND");
            }
        }
    }
    transaction.commit().map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_are_per_installation_idempotent_bounded_and_revocable() {
        let root = std::env::temp_dir().join(format!(
            "chariox-connection-grants-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let store = DurableKernelStateStore::open_owned(root.join("kernel.sqlite")).unwrap();
        let grant = |installation: &str, connection: &str| {
            store.app_connection_grant(ConnectionGrantCommand::Grant {
                owner: "alice".into(),
                installation: installation.into(),
                generator_id: "dev.chariox.slack".into(),
                connection_id: connection.into(),
                event_owner_id: "event-owner-1".into(),
                now_ms: 5,
            })
        };
        grant("slack", "connection-1").unwrap();
        grant("slack", "connection-1").unwrap();
        assert_eq!(
            store.app_connection_grants("alice", "slack").unwrap(),
            vec![ConnectionGrant {
                generator_id: "dev.chariox.slack".into(),
                connection_id: "connection-1".into(),
                event_owner_id: "event-owner-1".into(),
                granted_at_ms: 5,
            }]
        );
        assert!(store
            .app_connection_grants("alice", "todo")
            .unwrap()
            .is_empty());
        assert!(store
            .app_connection_grants("bob", "slack")
            .unwrap()
            .is_empty());
        for index in 1..MAX_GRANTS {
            grant("slack", &format!("connection-extra-{index}")).unwrap();
        }
        assert_eq!(grant("slack", "one-too-many"), Err("LIMIT_EXCEEDED"));
        let revoke = |connection: &str| {
            store.app_connection_grant(ConnectionGrantCommand::Revoke {
                owner: "alice".into(),
                installation: "slack".into(),
                connection_id: connection.into(),
            })
        };
        revoke("connection-1").unwrap();
        assert_eq!(revoke("connection-1"), Err("NOT_FOUND"));
        // Grants end with the active release.
        let connection = rusqlite::Connection::open(store.path()).unwrap();
        connection
            .execute_batch(
                "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
                 VALUES('slack','dev.chariox.slack','alice',1,1,'{}')",
            )
            .unwrap();
        forget_inactive(&connection, "alice", "slack").unwrap();
        assert!(!store
            .app_connection_grants("alice", "slack")
            .unwrap()
            .is_empty());
        connection
            .execute_batch("UPDATE app_installations SET active_json=NULL")
            .unwrap();
        forget_inactive(&connection, "alice", "slack").unwrap();
        assert!(store
            .app_connection_grants("alice", "slack")
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn grants_from_before_connections_carried_their_owner_are_dropped() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE app_connection_grants (
                    owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, connection_id TEXT NOT NULL,
                    generator_id TEXT NOT NULL, granted_at_ms INTEGER NOT NULL,
                    PRIMARY KEY(owner_id, installation_id, connection_id));
                 INSERT INTO app_connection_grants VALUES('alice','slack','connection-1','dev.chariox.slack',1);",
            )
            .unwrap();
        initialize(&connection).unwrap();
        let remaining: i64 = connection
            .query_row("SELECT count(*) FROM app_connection_grants", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0);
        // Opening again keeps what was granted since.
        connection
            .execute(
                "INSERT INTO app_connection_grants VALUES('alice','slack','c2','dev.chariox.slack','event-owner-1',2)",
                [],
            )
            .unwrap();
        initialize(&connection).unwrap();
        let kept: i64 = connection
            .query_row("SELECT count(*) FROM app_connection_grants", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(kept, 1);
    }
}
