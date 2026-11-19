//! A deadline measures kernel uptime: the time a kernel is stopped is not counted.
//!
//! The deadline tick leaves a heartbeat while any deadline is armed. A starting kernel
//! moves every armed deadline later by the time since that heartbeat.
use super::*;
use crate::local::AgentDeadlineState;

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_deadline_clock(id INTEGER PRIMARY KEY CHECK(id=1),last_seen_ms INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS agent_deadline_worker_held(room_id TEXT NOT NULL,agent_id TEXT NOT NULL,held_ms INTEGER NOT NULL,PRIMARY KEY(room_id,agent_id));")
        .map_err(sql)?;
    // The uptime clock keeps the old heartbeat but no longer records an instance ID.
    let has_instance_id: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('agent_deadline_clock') WHERE name='instance_id')",
            [],
            |row| row.get(0),
        )
        .map_err(sql)?;
    if has_instance_id {
        db.execute_batch("ALTER TABLE agent_deadline_clock DROP COLUMN instance_id")
            .map_err(sql)?;
    }
    Ok(())
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
            // A worker that enforces this deadline still holds the unshifted one; after a second
            // start before it confirmed, it still holds the first.
            db.execute(
                "INSERT OR IGNORE INTO agent_deadline_worker_held(room_id,agent_id,held_ms) VALUES(?1,?2,?3)",
                params![deadline.session_id, deadline.agent_id, sql_integer(deadline.deadline_ms)?],
            )
            .map_err(sql)?;
            deadline.deadline_ms = shifted(deadline.deadline_ms, down_ms)?;
            deadlines::save(db, &deadline)?;
            deadlines::move_intent(db, &deadline.session_id, &deadline.agent_id, |intent| {
                shifted(intent, down_ms)
            })?;
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

/// A deadline a kernel start moved later, and the value the worker that may enforce it still holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerHeldDeadline {
    pub(crate) room_id: String,
    pub(crate) agent_id: String,
    pub(crate) held_ms: u64,
}

/// Agents whose worker must be told the shifted deadline, until it confirms or refuses.
pub(super) fn awaiting_rearm(db: &Connection) -> Result<Vec<WorkerHeldDeadline>, DaemonError> {
    let mut query = db
        .prepare("SELECT room_id,agent_id,held_ms FROM agent_deadline_worker_held ORDER BY room_id,agent_id")
        .map_err(sql)?;
    let rows = query
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(sql)?;
    rows.map(|row| {
        let (room_id, agent_id, held) = row.map_err(sql)?;
        Ok(WorkerHeldDeadline {
            room_id,
            agent_id,
            held_ms: u64::try_from(held).map_err(|_| error("invalid held deadline"))?,
        })
    })
    .collect()
}

/// The worker confirmed the deadline, or an edit armed it afresh, or no worker holds it.
pub(super) fn rearmed(db: &Connection, room: &str, agent: &str) -> Result<(), DaemonError> {
    db.execute(
        "DELETE FROM agent_deadline_worker_held WHERE room_id=?1 AND agent_id=?2",
        params![room, agent],
    )
    .map_err(sql)?;
    Ok(())
}

/// The worker refused the shifted deadline for good, so the value it holds is the one that
/// stops the agent: the deadline, and the owner's limit behind it, go back to that value. A
/// worker that already reached it is reached here by the next tick.
pub(super) fn worker_kept_held(
    db: &Connection,
    room: &str,
    agent: &str,
) -> Result<(), DaemonError> {
    let held: Option<i64> = db
        .query_row(
            "SELECT held_ms FROM agent_deadline_worker_held WHERE room_id=?1 AND agent_id=?2",
            params![room, agent],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    rearmed(db, room, agent)?;
    let Some(held) = held else {
        return Ok(());
    };
    let held = u64::try_from(held).map_err(|_| error("invalid held deadline"))?;
    if let Some(mut deadline) = deadlines::get(db, room, agent)?
        .filter(|deadline| deadline.state == AgentDeadlineState::Armed)
    {
        let given_back = deadline.deadline_ms.saturating_sub(held);
        deadline.deadline_ms = held;
        deadlines::save(db, &deadline)?;
        deadlines::move_intent(db, room, agent, |intent| {
            Ok(intent.saturating_sub(given_back))
        })?;
    }
    Ok(())
}
