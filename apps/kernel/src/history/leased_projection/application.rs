//! MP-08 / MP-10 / MP-11: durable, locally admitted home page application.
//! A pending result survives receipt failure and prompt removal. History items
//! have stable delivery identities; the receipt never substitutes for output.
use super::{operational_history_error, OperationalHistoryStore};
use crate::{
    error::DaemonError, history::HistoryEvent, session::PromptCompletion,
    transport::relay_peer::RelayPeerEvent,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct HomeProjectionResult {
    pub(crate) completions: Vec<PromptCompletion>,
    pub(crate) provider_failed: bool,
    pub(crate) advance_queue: bool,
    pub(crate) queue_head: Option<crate::session::PromptQueueItem>,
    pub(crate) retain_native_binding: bool,
}

impl OperationalHistoryStore {
    pub(crate) fn begin_home_projection_application(
        &self,
        key: &str,
        sequence: u64,
        event: &RelayPeerEvent,
    ) -> Result<(), DaemonError> {
        self.ensure_leased_projection_schema()?;
        let digest = page_digest(event)?;
        let connection = self
            .connection
            .lock()
            .map_err(|e| operational_history_error("lock home projection", e))?;
        connection
            .execute(
                "INSERT OR IGNORE INTO home_projection_applications VALUES (?1, ?2, ?3, NULL)",
                params![key, sequence as i64, digest],
            )
            .map_err(|e| operational_history_error("begin home projection", e))?;
        read_application(&connection, key, sequence, event).map(|_| ())
    }

    pub(crate) fn home_projection_application(
        &self,
        key: &str,
        sequence: u64,
        event: &RelayPeerEvent,
    ) -> Result<Option<Option<HomeProjectionResult>>, DaemonError> {
        self.ensure_leased_projection_schema()?;
        let connection = self.lock_read_connection(None)?;
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM home_projection_applications WHERE projection_id = ?1 AND sequence = ?2)",
            params![key, sequence as i64], |r| r.get(0),
        ).map_err(|e| operational_history_error("find home projection", e))?;
        if !exists {
            return Ok(None);
        }
        read_application(&connection, key, sequence, event).map(Some)
    }

    pub(crate) fn record_home_projection_result(
        &self,
        key: &str,
        sequence: u64,
        result: &HomeProjectionResult,
    ) -> Result<(), DaemonError> {
        let json = serde_json::to_string(result)
            .map_err(|e| operational_history_error("encode home projection result", e))?;
        let connection = self
            .connection
            .lock()
            .map_err(|e| operational_history_error("lock home projection result", e))?;
        let changed = connection.execute(
            "UPDATE home_projection_applications SET result_json = ?3 WHERE projection_id = ?1 AND sequence = ?2",
            params![key, sequence as i64, json],
        ).map_err(|e| operational_history_error("save home projection result", e))?;
        if changed != 1 {
            return Err(operational_history_error(
                "save home projection result",
                "missing admitted page",
            ));
        }
        Ok(())
    }

    pub(crate) fn finish_home_projection_application(
        &self,
        key: &str,
        sequence: u64,
    ) -> Result<(), DaemonError> {
        self.connection.lock()
            .map_err(|e| operational_history_error("lock home projection cleanup", e))?
            .execute("DELETE FROM home_projection_applications WHERE projection_id = ?1 AND sequence = ?2", params![key, sequence as i64])
            .map_err(|e| operational_history_error("finish home projection", e))?;
        Ok(())
    }

    pub(crate) fn append_home_projection_history(
        &self,
        identity: &str,
        mut event: HistoryEvent,
    ) -> Result<(HistoryEvent, bool), DaemonError> {
        use sha2::{Digest, Sha256};
        event.event_id = format!("remote-page:{:x}", Sha256::digest(identity.as_bytes()));
        let previous: Option<String> = self
            .lock_read_connection(None)?
            .query_row(
                "SELECT event_json FROM history_events WHERE event_id = ?1",
                [&event.event_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| operational_history_error("find projected history item", e))?;
        if let Some(previous) = previous {
            return serde_json::from_str(&previous)
                .map(|event| (event, false))
                .map_err(|e| operational_history_error("decode projected history item", e));
        }
        self.append(&event)?;
        Ok((event, true))
    }
}

fn read_application(
    connection: &rusqlite::Connection,
    key: &str,
    sequence: u64,
    event: &RelayPeerEvent,
) -> Result<Option<HomeProjectionResult>, DaemonError> {
    let (saved_sequence, saved_digest, result): (i64, String, Option<String>) = connection.query_row(
        "SELECT sequence, event_digest, result_json FROM home_projection_applications WHERE projection_id = ?1",
        [key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).map_err(|e| operational_history_error("read home projection", e))?;
    if saved_sequence as u64 != sequence || saved_digest != page_digest(event)? {
        return Err(operational_history_error(
            "revalidate home projection",
            "retained page changed before application finished",
        ));
    }
    result
        .map(|json| {
            serde_json::from_str(&json)
                .map_err(|e| operational_history_error("decode home projection result", e))
        })
        .transpose()
}

// Store an exact payload binding, not another copy of provider launch metadata.
fn page_digest(event: &RelayPeerEvent) -> Result<String, DaemonError> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(event)
        .map_err(|e| operational_history_error("bind home projection", e))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
