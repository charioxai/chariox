//! The current Room authority is durable independently of periodic session checkpoints.
use rusqlite::{params, Transaction};

use super::{DurableKernelStateStore, DurableWriteOperation};
use crate::error::DaemonError;
use crate::session::RoomEnvironment;

#[derive(Debug)]
pub(crate) struct RoomEnvironmentWrite {
    owner_id: String,
    session_id: String,
    payload_json: String,
}

impl DurableKernelStateStore {
    pub(crate) fn save_room_environment(
        &self,
        owner_id: &str,
        environment: &RoomEnvironment,
    ) -> Result<(), DaemonError> {
        let snapshot = environment.snapshot();
        let payload_json = serde_json::to_string(environment).map_err(|_| state_error())?;
        self.writer
            .execute(DurableWriteOperation::RoomEnvironment(
                RoomEnvironmentWrite {
                    owner_id: owner_id.into(),
                    session_id: snapshot.session_id,
                    payload_json,
                },
            ))
            .map(|_| ())
    }

    pub(crate) fn load_room_environments(
        &self,
        owner_id: &str,
    ) -> Result<Vec<RoomEnvironment>, DaemonError> {
        let connection = self.lock_connection("environment.restore")?;
        let mut statement = connection.prepare(
            "SELECT session_id, payload_json FROM durable_room_environments WHERE owner_id = ?1"
        ).map_err(|_| state_error())?;
        let rows = statement
            .query_map(params![owner_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|_| state_error())?;
        rows.map(|row| {
            let (session_id, payload) = row.map_err(|_| state_error())?;
            let environment: RoomEnvironment =
                serde_json::from_str(&payload).map_err(|_| state_error())?;
            if environment.snapshot().session_id != session_id {
                return Err(state_error());
            }
            Ok(environment)
        })
        .collect()
    }
}

pub(crate) fn apply(
    transaction: &Transaction<'_>,
    write: &RoomEnvironmentWrite,
) -> rusqlite::Result<u64> {
    transaction.execute(
        "INSERT INTO durable_room_environments (owner_id, session_id, payload_json) VALUES (?1, ?2, ?3)
         ON CONFLICT(owner_id, session_id) DO UPDATE SET payload_json = excluded.payload_json",
        params![write.owner_id, write.session_id, write.payload_json],
    ).map(|_| 0)
}

fn state_error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "environment.durable_state",
        message:
            "Room Environment state is unavailable; repair kernel durable state before retrying"
                .into(),
    }
}
