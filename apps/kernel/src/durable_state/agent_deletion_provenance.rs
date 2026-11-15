//! MP-11: ownership of retired agents from kernel-owned deletion events.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use rusqlite::{params, OptionalExtension};

impl DurableKernelStateStore {
    pub(crate) fn deleted_agent_is_owned(
        &self,
        session_id: &str,
        agent_id: &str,
        user_id: &str,
    ) -> Result<bool, DaemonError> {
        let operation = "durable_state.deleted_agent_is_owned";
        let connection = self.lock_connection(operation)?;
        // Ordinary deletion uses the agent subject; workflow cleanup uses the
        // session subject. Filter by payload identity before limiting the rows.
        let provenance: Option<(Option<String>, Option<String>)> = connection
            .query_row(
                "SELECT json_extract(payload_json, '$.agent.session_id'),
                        json_extract(payload_json, '$.agent.owner_user_id')
                 FROM durable_state_events
                 WHERE kind = 'agent.deleted' AND subject_id IN (?1, ?2)
                   AND json_valid(payload_json)
                   AND json_extract(payload_json, '$.agent.id') = ?2
                 ORDER BY sequence DESC LIMIT 1",
                params![session_id, agent_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| DaemonError::LocalTransport {
                operation,
                message: error.to_string(),
            })?;
        Ok(provenance.is_some_and(|(session, owner)| {
            session.as_deref() == Some(session_id) && owner.as_deref() == Some(user_id)
        }))
    }
}

/// The completed deletion of a child.
pub(crate) struct DeletedChild {
    pub agent: crate::agent::AgentInstance,
    pub deleted_at_ms: u64,
}

impl DurableKernelStateStore {
    pub(crate) fn deleted_spawned_child(
        &self,
        parent: &crate::agent::AgentInstance,
        reference: &str,
    ) -> Result<Option<DeletedChild>, DaemonError> {
        let operation = "durable_state.deleted_spawned_child";
        let db = self.lock_connection(operation)?;
        let row: Option<(String, i64)> = db
            .query_row(
                "SELECT json_extract(d.payload_json,'$.agent'), d.timestamp_ms
             FROM durable_state_events d WHERE d.kind='agent.deleted'
             AND json_valid(d.payload_json)
             AND json_extract(d.payload_json,'$.agent.session_id')=?1
             AND json_extract(d.payload_json,'$.agent.owner_user_id')=?2
             AND json_extract(d.payload_json,'$.agent.parent_agent_id.agent_id')=?3
             AND json_extract(d.payload_json,'$.agent.spawn_key') IS NOT NULL
             AND (json_extract(d.payload_json,'$.agent.id')=?4
                  OR json_extract(d.payload_json,'$.agent.agent_ref')=?4
                  OR json_extract(d.payload_json,'$.agent.alias')=?4)
             ORDER BY d.sequence DESC LIMIT 1",
                params![
                    parent.session_id(),
                    parent.owner_user_id(),
                    parent.id(),
                    reference
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| DaemonError::LocalTransport {
                operation,
                message: error.to_string(),
            })?;
        row.map(|(payload, deleted_at_ms)| {
            let deleted_at_ms =
                u64::try_from(deleted_at_ms).map_err(|_| DaemonError::LocalTransport {
                    operation,
                    message: "deleted child timestamp is invalid".into(),
                })?;
            let agent =
                serde_json::from_str(&payload).map_err(|error| DaemonError::LocalTransport {
                    operation,
                    message: format!("deleted child record is invalid: {error}"),
                })?;
            Ok(DeletedChild {
                agent,
                deleted_at_ms,
            })
        })
        .transpose()
    }
}
