use super::*;
use chariox_app_runtime::publisher_trust::{
    TrustDecision, TrustDecisionReceipt, MAX_DECISIONS_PER_OWNER, MAX_TRUST_DECISIONS,
};
use rusqlite::{params, OptionalExtension, Transaction};

pub(super) fn sql<T>(value: rusqlite::Result<T>) -> Result<T> {
    value.map_err(|_| PublisherOperationError::Storage)
}
pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS app_publisher_operations (
        owner_id TEXT NOT NULL, request_id TEXT NOT NULL, session_id TEXT NOT NULL,
        publisher_id TEXT NOT NULL, key_id TEXT NOT NULL, public_key BLOB NOT NULL CHECK(length(public_key)=32),
        expected_revision INTEGER NOT NULL CHECK(expected_revision>=0),
        phase TEXT NOT NULL CHECK(phase IN ('pending','approved','denied','cancelled','failed')),
        interaction_id TEXT, receipt_revision INTEGER, receipt_ms INTEGER, failure TEXT,
        created_ms INTEGER NOT NULL CHECK(created_ms>=0), updated_ms INTEGER NOT NULL CHECK(updated_ms>=0),
        PRIMARY KEY(owner_id,request_id));
        CREATE INDEX IF NOT EXISTS app_publisher_operations_phase ON app_publisher_operations(phase,owner_id,request_id);
        CREATE INDEX IF NOT EXISTS app_publisher_operations_owner_phase ON app_publisher_operations(owner_id,phase);
        CREATE INDEX IF NOT EXISTS app_publisher_operations_interaction ON app_publisher_operations(owner_id,interaction_id);")
}
pub(super) fn load(
    connection: &Connection,
    owner: &str,
    request: &str,
) -> Result<PublisherOperation> {
    model::text(owner)?;
    model::text(request)?;
    type Row = (
        String,
        String,
        String,
        Vec<u8>,
        i64,
        String,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
    );
    let row: Row = sql(connection.query_row("SELECT session_id,publisher_id,key_id,public_key,expected_revision,phase,interaction_id,receipt_revision,receipt_ms,failure FROM app_publisher_operations WHERE owner_id=?1 AND request_id=?2",
        params![owner,request], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional())?.ok_or(PublisherOperationError::NotFound)?;
    let (
        session_id,
        publisher_id,
        key_id,
        bytes,
        revision,
        phase,
        interaction,
        receipt_revision,
        receipt_ms,
        failure,
    ) = row;
    let input = PublisherEnrollmentInput {
        session_id,
        publisher_id,
        key_id,
        public_key: bytes
            .try_into()
            .map_err(|_| PublisherOperationError::Storage)?,
        expected_revision: u64::try_from(revision).map_err(|_| PublisherOperationError::Storage)?,
    };
    input
        .publisher()
        .map_err(|_| PublisherOperationError::Storage)?;
    if let Some(id) = &interaction {
        model::text(id).map_err(|_| PublisherOperationError::Storage)?;
    }
    let phase = match phase.as_str() {
        "pending" => PublisherOperationPhase::Pending,
        "approved" => PublisherOperationPhase::Approved,
        "denied" => PublisherOperationPhase::Denied,
        "cancelled" => PublisherOperationPhase::Cancelled,
        "failed" => PublisherOperationPhase::Failed,
        _ => return Err(PublisherOperationError::Storage),
    };
    let receipt = match (phase, receipt_revision, receipt_ms) {
        (PublisherOperationPhase::Approved, Some(revision), Some(time))
            if revision > 0 && time >= 0 =>
        {
            Some(TrustDecisionReceipt {
                decision: TrustDecision {
                    decision_id: interaction
                        .clone()
                        .ok_or(PublisherOperationError::Storage)?,
                    authority_ref: "kernel_operation_human".into(),
                },
                publisher_id: input.publisher_id.clone(),
                key_id: input.key_id.clone(),
                revision: revision as u64,
                enrolled: true,
                applied_at_ms: time as u64,
            })
        }
        (PublisherOperationPhase::Approved, _, _) => return Err(PublisherOperationError::Storage),
        (_, None, None) => None,
        _ => return Err(PublisherOperationError::Storage),
    };
    Ok(PublisherOperation {
        request_id: request.into(),
        input,
        phase,
        interaction_id: interaction,
        receipt,
        failure,
    })
}
pub(super) fn admit(tx: &Transaction<'_>, owner: &str) -> Result<()> {
    // Retain replay tombstones within the existing publisher-decision limits.
    // SQLite uses the primary/phase indexes; no operation rows enter a Vec.
    let (all, owned, pending, owned_pending): (i64, i64, i64, i64) = sql(tx.query_row(
        "SELECT
        (SELECT count(*) FROM app_publisher_operations),
        (SELECT count(*) FROM app_publisher_operations WHERE owner_id=?1),
        (SELECT count(*) FROM app_publisher_operations WHERE phase='pending'),
        (SELECT count(*) FROM app_publisher_operations WHERE owner_id=?1 AND phase='pending')",
        [owner],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ))?;
    if all >= MAX_TRUST_DECISIONS as i64
        || owned >= MAX_DECISIONS_PER_OWNER as i64
        || pending >= 32
        || owned_pending >= 8
    {
        Err(PublisherOperationError::Limit)
    } else {
        Ok(())
    }
}
pub(super) fn now() -> Result<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .ok_or(PublisherOperationError::Storage)
}

pub(super) fn used_decision(
    connection: &Connection,
    owner: &str,
    interaction: &str,
) -> Result<bool> {
    sql(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM app_publisher_decisions WHERE owner_id=?1 AND decision_id=?2)",
        params![owner, interaction],
        |r| r.get(0),
    ))
}
pub(super) fn fresh_nonce(connection: &Connection, owner: &str, interaction: &str) -> Result<()> {
    let used:bool=sql(connection.query_row("SELECT EXISTS(SELECT 1 FROM app_publisher_operations WHERE owner_id=?1 AND interaction_id=?2)",params![owner,interaction],|r|r.get(0)))?;
    if used || used_decision(connection, owner, interaction)? {
        Err(PublisherOperationError::Conflict)
    } else {
        Ok(())
    }
}
