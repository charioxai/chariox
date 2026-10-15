//! MP-08 / MP-10 / MP-11: one durable delivery queue per worker binding/run.
//! The cursor and queue commit together. Push and poll both read its head;
//! only an authenticated, exact acknowledgement removes that head.
use super::{operational_history_error, OperationalHistoryStore};
use crate::error::DaemonError;
use crate::transport::relay_peer::RelayPeerEvent;
use rusqlite::{params, OptionalExtension};

pub(super) fn stage(
    transaction: &rusqlite::Transaction<'_>,
    projection_id: &str,
    events: &[RelayPeerEvent],
) -> Result<(), DaemonError> {
    for event in events {
        let RelayPeerEvent::LeasedRuntimeProjection { delivery, .. } = event;
        let receipt = delivery
            .as_ref()
            .expect("retained projection has a receipt");
        let json = serde_json::to_string(event)
            .map_err(|e| operational_history_error("encode projection delivery", e))?;
        transaction.execute(
            "INSERT INTO leased_projection_deliveries(projection_id, sequence, event_json) VALUES (?1, ?2, ?3)",
            params![projection_id, receipt.sequence as i64, json],
        ).map_err(|e| operational_history_error("retain projection delivery", e))?;
    }
    Ok(())
}

impl OperationalHistoryStore {
    pub(crate) fn pending_leased_projection_delivery(
        &self,
        projection_id: &str,
    ) -> Result<Option<RelayPeerEvent>, DaemonError> {
        self.ensure_leased_projection_schema()?;
        let connection = self.lock_read_connection(None)?;
        let json: Option<String> = connection.query_row(
            "SELECT event_json FROM leased_projection_deliveries WHERE projection_id = ?1 ORDER BY sequence LIMIT 1",
            [projection_id], |row| row.get(0),
        ).optional().map_err(|e| operational_history_error("read projection delivery", e))?;
        json.map(|json| {
            serde_json::from_str(&json)
                .map_err(|e| operational_history_error("decode projection delivery", e))
        })
        .transpose()
    }

    pub(crate) fn leased_projection_delivery_sequence(
        &self,
        projection_id: &str,
    ) -> Result<u64, DaemonError> {
        self.ensure_leased_projection_schema()?;
        self.lock_read_connection(None)?
            .query_row(
                "SELECT sequence FROM leased_projection_delivery_receipts WHERE projection_id = ?1",
                [projection_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|value| value.unwrap_or(0) as u64)
            .map_err(|e| operational_history_error("read projection receipt", e))
    }

    pub(crate) fn acknowledge_leased_projection_delivery(
        &self,
        projection_id: &str,
        sequence: u64,
    ) -> Result<bool, DaemonError> {
        self.ensure_leased_projection_schema()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|e| operational_history_error("lock projection acknowledgement", e))?;
        let transaction = connection
            .transaction()
            .map_err(|e| operational_history_error("begin projection acknowledgement", e))?;
        let acknowledged: i64 = transaction
            .query_row(
                "SELECT sequence FROM leased_projection_delivery_receipts WHERE projection_id = ?1",
                [projection_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| operational_history_error("read acknowledged projection", e))?
            .unwrap_or(0);
        if sequence == 0 {
            return Ok(false);
        }
        if sequence <= acknowledged as u64 {
            return Ok(true);
        }
        let head: Option<i64> = transaction.query_row(
            "SELECT sequence FROM leased_projection_deliveries WHERE projection_id = ?1 ORDER BY sequence LIMIT 1",
            [projection_id], |row| row.get(0),
        ).optional().map_err(|e| operational_history_error("read projection acknowledgement head", e))?;
        if head.map(|head| head as u64) != Some(sequence) {
            return Ok(false);
        }
        transaction.execute(
            "DELETE FROM leased_projection_deliveries WHERE projection_id = ?1 AND sequence = ?2",
            params![projection_id, sequence as i64],
        ).map_err(|e| operational_history_error("retire projection delivery", e))?;
        record_receipt(&transaction, projection_id, sequence)?;
        transaction
            .commit()
            .map_err(|e| operational_history_error("commit projection acknowledgement", e))?;
        Ok(true)
    }

    pub(crate) fn record_leased_projection_delivery_receipt(
        &self,
        projection_id: &str,
        sequence: u64,
    ) -> Result<(), DaemonError> {
        self.ensure_leased_projection_schema()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|e| operational_history_error("lock applied projection receipt", e))?;
        let transaction = connection
            .transaction()
            .map_err(|e| operational_history_error("begin applied projection receipt", e))?;
        record_receipt(&transaction, projection_id, sequence)?;
        transaction
            .commit()
            .map_err(|e| operational_history_error("commit applied projection receipt", e))
    }
}

fn record_receipt(
    transaction: &rusqlite::Transaction<'_>,
    key: &str,
    sequence: u64,
) -> Result<(), DaemonError> {
    transaction.execute(
        "INSERT INTO leased_projection_delivery_receipts VALUES (?1, ?2) ON CONFLICT(projection_id) DO UPDATE SET sequence = MAX(sequence, excluded.sequence)",
        params![key, sequence as i64],
    ).map_err(|e| operational_history_error("save projection receipt", e))?;
    Ok(())
}
