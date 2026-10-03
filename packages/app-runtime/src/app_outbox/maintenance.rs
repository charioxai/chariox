//! Bounded receipt housekeeping. This grants no workflow or App authority.
use super::*;
use rusqlite::params;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Maintenance {
    pub minimum_occurred_at_ms: u64,
    pub expired: usize,
    pub failed: usize,
    pub pruned: usize,
}

pub(super) fn advance_floor(
    tx: &Connection,
    owner: &str,
    installation: &str,
    now: u64,
) -> Result<u64> {
    time(now)?;
    let proposed = now.saturating_sub(MAX_OCCURRENCE_AGE_MS);
    tx.execute(
        "INSERT INTO app_outbox_replay_floors(owner_id,installation_id,minimum_occurred_at_ms)
         VALUES(?1,?2,?3) ON CONFLICT(owner_id,installation_id) DO UPDATE SET
         minimum_occurred_at_ms=max(minimum_occurred_at_ms,excluded.minimum_occurred_at_ms)",
        params![owner, installation, time(proposed)?],
    )?;
    let floor: i64 = tx.query_row(
        "SELECT minimum_occurred_at_ms FROM app_outbox_replay_floors
         WHERE owner_id=?1 AND installation_id=?2",
        params![owner, installation],
        |row| row.get(0),
    )?;
    u64::try_from(floor).map_err(|_| OutboxError::Corrupt)
}

impl AppOutbox {
    /// Trusted kernel housekeeping on its existing committing writer. It works
    /// for paused/revoked/uninstalled Apps, but cannot queue or execute anything.
    /// At most 256 pending classifications and 256 terminal deletions per call.
    /// Returned counts/floor are provisional until the outer transaction commits.
    pub fn maintain_in(
        tx: &mut Transaction<'_>,
        trusted_owner: &str,
        installation_id: &str,
        now: u64,
    ) -> Result<Maintenance> {
        identifier(trusted_owner)?;
        identifier(installation_id)?;
        time(now)?;
        let owns: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM app_installations WHERE owner_id=?1 AND installation_id=?2)",
            params![trusted_owner,installation_id], |row|row.get(0),
        )?;
        if !owns {
            return Err(OutboxError::NotFound);
        }
        let savepoint = tx.savepoint()?;
        let floor = advance_floor(&savepoint, trusted_owner, installation_id, now)?;
        let mut result = Maintenance {
            minimum_occurred_at_ms: floor,
            expired: 0,
            failed: 0,
            pruned: 0,
        };
        // Unsupported old contracts are never reinterpreted. Classifying them
        // releases capacity without allowing an old page to block new deliveries.
        let candidates = {
            let mut statement = savepoint.prepare(
                "SELECT receipt_id,occurrence_id,occurred_at_ms,expires_at_ms,payload_json IS NOT NULL AND invocation_json IS NOT NULL
                 FROM app_outbox WHERE owner_id=?1 AND installation_id=?2 AND state IN ('accepted','retryable')
                 AND (expires_at_ms<=?3 OR payload_json IS NULL OR invocation_json IS NULL
                   OR occurrence_id NOT GLOB 'evt1.*') ORDER BY sequence LIMIT 256"
            )?;
            let rows = statement.query_map(
                params![trusted_owner, installation_id, time(now)?],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                },
            )?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (id, occurrence, original, expires, has_body) in candidates {
            let expired = expires <= time(now)?;
            let valid = has_body
                && u64::try_from(original)
                    .ok()
                    .is_some_and(|time| identity::validate(&occurrence, time).is_ok());
            if !expired && valid {
                continue;
            }
            let state = if expired { "expired" } else { "failed" };
            savepoint.execute(
                "UPDATE app_outbox SET state=?1,payload_json=NULL,invocation_json=NULL,
                 revision=CASE WHEN revision<9223372036854775807 THEN revision+1 ELSE revision END
                 WHERE owner_id=?2 AND installation_id=?3 AND receipt_id=?4 AND state IN ('accepted','retryable')",
                params![state,trusted_owner,installation_id,id],
            )?;
            if expired {
                result.expired += 1;
            } else {
                result.failed += 1;
            }
        }
        if now >= MAX_OCCURRENCE_AGE_MS {
            result.pruned = savepoint.execute(
                "DELETE FROM app_outbox WHERE sequence IN (
                 SELECT sequence FROM app_outbox WHERE owner_id=?1 AND installation_id=?2
                 AND state IN ('delivered','failed','expired') AND occurred_at_ms<?3 AND accepted_at_ms<=?4
                 ORDER BY sequence LIMIT 256)",
                params![trusted_owner,installation_id,time(floor)?,time(now-MAX_OCCURRENCE_AGE_MS)?],
            )?;
        }
        savepoint.commit()?;
        Ok(result)
    }
}
