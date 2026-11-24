//! Setup transitions commit before they are visible, independently of session checkpoints.
use rusqlite::{params, Transaction};

use super::{DurableKernelStateStore, DurableWriteOperation};
use crate::error::DaemonError;
use crate::session::RoomSetupState;

#[derive(Debug)]
pub(crate) struct RoomSetupWrite {
    owner_id: String,
    session_id: String,
    setup: Option<(String, String)>,
}

pub(crate) struct StoredRoomSetup {
    pub session_id: String,
    pub agent_id: String,
    pub state: RoomSetupState,
    pub storage_failed: bool,
}

impl DurableKernelStateStore {
    pub(crate) fn save_room_setup(
        &self,
        owner_id: &str,
        session_id: &str,
        setup: Option<(&str, &RoomSetupState)>,
    ) -> Result<(), DaemonError> {
        let setup = setup
            .map(|(agent, state)| {
                serde_json::to_string(state)
                    .map(|state| (agent.to_string(), state))
                    .map_err(|_| state_error())
            })
            .transpose()?;
        self.writer
            .execute(DurableWriteOperation::RoomSetup(RoomSetupWrite {
                owner_id: owner_id.into(),
                session_id: session_id.into(),
                setup,
            }))
            .map(|_| ())
    }

    pub(crate) fn mark_room_setup_storage_failure(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        self.writer
            .execute(DurableWriteOperation::RoomSetupStorageFailure {
                owner_id: owner_id.into(),
                session_id: session_id.into(),
            })
            .map(|_| ())
    }

    pub(crate) fn load_room_setups(
        &self,
        owner_id: &str,
    ) -> Result<Vec<StoredRoomSetup>, DaemonError> {
        let connection = self.lock_connection("room_setup.restore")?;
        let mut statement = connection.prepare(
            "SELECT setup.session_id, setup.agent_id, setup.state_json, failure.session_id IS NOT NULL
             FROM durable_room_setups setup
             LEFT JOIN durable_room_setup_storage_failures failure USING (owner_id, session_id)
             WHERE setup.owner_id = ?1"
        ).map_err(|_| state_error())?;
        let rows = statement
            .query_map(params![owner_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            })
            .map_err(|_| state_error())?;
        rows.map(|row| {
            let (session_id, agent_id, state, storage_failed) = row.map_err(|_| state_error())?;
            Ok(StoredRoomSetup {
                session_id,
                agent_id,
                state: serde_json::from_str(&state).map_err(|_| state_error())?,
                storage_failed,
            })
        })
        .collect()
    }
}

pub(crate) fn apply(
    transaction: &Transaction<'_>,
    write: &RoomSetupWrite,
) -> rusqlite::Result<u64> {
    if let Some((agent_id, state_json)) = &write.setup {
        transaction.execute(
            "INSERT INTO durable_room_setups (owner_id, session_id, agent_id, state_json) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_id, session_id) DO UPDATE SET agent_id = excluded.agent_id, state_json = excluded.state_json",
            params![write.owner_id, write.session_id, agent_id, state_json],
        )?;
    } else {
        transaction.execute(
            "DELETE FROM durable_room_setups WHERE owner_id = ?1 AND session_id = ?2",
            params![write.owner_id, write.session_id],
        )?;
    }
    transaction.execute(
        "DELETE FROM durable_room_setup_storage_failures WHERE owner_id = ?1 AND session_id = ?2",
        params![write.owner_id, write.session_id],
    )?;
    Ok(0)
}

pub(crate) fn mark_storage_failure(
    transaction: &Transaction<'_>,
    owner_id: &str,
    session_id: &str,
) -> rusqlite::Result<u64> {
    transaction.execute(
        "INSERT INTO durable_room_setup_storage_failures (owner_id, session_id) VALUES (?1, ?2)
         ON CONFLICT(owner_id, session_id) DO NOTHING",
        params![owner_id, session_id],
    )?;
    Ok(0)
}

fn state_error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "room_setup.durable_state",
        message: "Room setup state is unavailable; repair kernel durable state before retrying"
            .into(),
    }
}
