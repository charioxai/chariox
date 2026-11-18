//! A deadline measures kernel uptime: the time a kernel is stopped is not counted.
//!
//! The deadline tick leaves a heartbeat while any deadline is armed. A starting kernel
//! moves every armed deadline later by the time since that heartbeat.
use super::*;

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_deadline_clock(id INTEGER PRIMARY KEY CHECK(id=1),last_seen_ms INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS agent_deadline_rearm(room_id TEXT NOT NULL,agent_id TEXT NOT NULL,PRIMARY KEY(room_id,agent_id));")
        .map_err(sql)
}

pub(super) fn beat(db: &Connection, now: u64) -> Result<(), DaemonError> {
    db.execute(
        "INSERT INTO agent_deadline_clock(id,last_seen_ms) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET last_seen_ms=excluded.last_seen_ms",
        [sql_integer(now)?],
    )
    .map_err(sql)?;
    Ok(())
}

/// Shifts every armed deadline, and the owner's explicit limit behind it, by the downtime.
/// Reached deadlines stay reached.
pub(super) fn resume_after_kernel_start(db: &Connection, now: u64) -> Result<(), DaemonError> {
    let last_seen: Option<i64> = db
        .query_row(
            "SELECT last_seen_ms FROM agent_deadline_clock WHERE id=1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    let armed = deadlines::armed(db)?;
    // No heartbeat means no armed deadline has ever been counted: there is no downtime to give back.
    if let Some(last_seen) = last_seen.filter(|_| !armed.is_empty()) {
        let down_ms = now.saturating_sub(
            u64::try_from(last_seen).map_err(|_| error("negative deadline heartbeat"))?,
        );
        for mut deadline in armed {
            deadline.deadline_ms = shifted(deadline.deadline_ms, down_ms)?;
            deadlines::save(db, &deadline)?;
            deadlines::shift_intent(db, &deadline.session_id, &deadline.agent_id, down_ms)?;
            // A worker that enforces this deadline still holds the unshifted one.
            db.execute(
                "INSERT OR IGNORE INTO agent_deadline_rearm(room_id,agent_id) VALUES(?1,?2)",
                params![deadline.session_id, deadline.agent_id],
            )
            .map_err(sql)?;
        }
    }
    beat(db, now)
}

pub(super) fn shifted(deadline_ms: u64, down_ms: u64) -> Result<u64, DaemonError> {
    deadline_ms
        .checked_add(down_ms)
        .filter(|time| *time <= i64::MAX as u64)
        .ok_or_else(|| error("an agent deadline cannot move later than the largest supported time"))
}

/// Agents whose worker must be told the shifted deadline, until it confirms.
pub(super) fn awaiting_rearm(db: &Connection) -> Result<Vec<(String, String)>, DaemonError> {
    let mut query = db
        .prepare("SELECT room_id,agent_id FROM agent_deadline_rearm ORDER BY room_id,agent_id")
        .map_err(sql)?;
    let rows = query
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(sql)?;
    rows.collect::<Result<_, _>>().map_err(sql)
}

/// The worker confirmed the deadline, or an edit armed it afresh, or no worker holds it.
pub(super) fn rearmed(db: &Connection, room: &str, agent: &str) -> Result<(), DaemonError> {
    db.execute(
        "DELETE FROM agent_deadline_rearm WHERE room_id=?1 AND agent_id=?2",
        params![room, agent],
    )
    .map_err(sql)?;
    Ok(())
}
