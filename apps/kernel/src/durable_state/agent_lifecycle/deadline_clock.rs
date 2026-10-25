//! MP-08 / MP-10 / MP-11: lane budgets count executor uptime, including after a crash.
use super::*;
use crate::local::AgentDeadline;
use std::sync::OnceLock;

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_deadline_clock(id INTEGER PRIMARY KEY CHECK(id=1),instance_id TEXT NOT NULL,last_seen_ms INTEGER NOT NULL);").map_err(sql)
}

pub(super) fn instance_id() -> &'static str {
    static INSTANCE: OnceLock<String> = OnceLock::new();
    INSTANCE.get_or_init(|| format!("{:032x}", rand::random::<u128>()))
}

// The caller owns a transaction: shifting budgets and advancing the heartbeat must commit together.
// A fresh process pauses only the gap since the preceding heartbeat; a stalled live process does not.
pub(super) fn advance(
    db: &Connection,
    now: u64,
    instance: &str,
    arming: bool,
) -> Result<(), DaemonError> {
    let active: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_deadlines WHERE json_extract(payload,'$.state')='armed')", [], |row| row.get(0)).map_err(sql)?;
    if !active && !arming {
        return Ok(());
    }
    let previous: Option<(String, u64)> = db
        .query_row(
            "SELECT instance_id,last_seen_ms FROM agent_deadline_clock WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(sql)?;
    if let Some((old_instance, last_seen)) = &previous {
        if old_instance != instance && active {
            let down_ms = now.saturating_sub(*last_seen);
            let payloads = db.prepare("SELECT payload FROM agent_deadlines WHERE json_extract(payload,'$.state')='armed'").map_err(sql)?
                .query_map([], |row| row.get::<_, String>(0)).map_err(sql)?
                .collect::<Result<Vec<_>, _>>().map_err(sql)?;
            for payload in payloads {
                let mut deadline: AgentDeadline = decode(&payload)?;
                deadline.deadline_ms = deadline
                    .deadline_ms
                    .checked_add(down_ms)
                    .filter(|value| *value <= i64::MAX as u64)
                    .ok_or_else(|| {
                        error("lane deadline overflow while restoring its uptime budget")
                    })?;
                db.execute(
                    "UPDATE agent_deadlines SET payload=?3 WHERE room_id=?1 AND agent_id=?2",
                    params![deadline.session_id, deadline.agent_id, encode(&deadline)?],
                )
                .map_err(sql)?;
            }
        }
    }
    if arming
        || previous
            .as_ref()
            .is_none_or(|(old, last)| old != instance || now.saturating_sub(*last) >= 1_000)
    {
        db.execute("INSERT INTO agent_deadline_clock(id,instance_id,last_seen_ms) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET instance_id=excluded.instance_id,last_seen_ms=excluded.last_seen_ms", params![instance, sql_integer(now)?]).map_err(sql)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_state::agent_lifecycle::deadlines;

    fn fixture() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        deadlines::initialize(&db).unwrap();
        deadlines::set(&db, "room".into(), "agent".into(), 100_000).unwrap();
        db.execute(
            "UPDATE agent_deadline_clock SET instance_id='boot-a',last_seen_ms=1000",
            [],
        )
        .unwrap();
        db
    }
    fn deadline(db: &Connection) -> AgentDeadline {
        deadlines::get(db, "room", "agent").unwrap().unwrap()
    }

    #[test]
    fn uptime_deadline_pauses_only_when_the_executor_instance_changes() {
        let db = fixture();
        advance(&db, 2_000, "boot-a", false).unwrap();
        assert_eq!(deadline(&db).deadline_ms, 100_000);
        advance(&db, 12_000, "boot-b", false).unwrap();
        assert_eq!(deadline(&db).deadline_ms, 110_000);
        advance(&db, 13_000, "boot-b", false).unwrap();
        assert_eq!(
            deadline(&db).deadline_ms,
            110_000,
            "live/stalled time still counts"
        );
        advance(&db, 23_000, "boot-c", false).unwrap();
        assert_eq!(
            deadline(&db).deadline_ms,
            120_000,
            "every later reboot pauses too"
        );
    }

    #[test]
    fn reached_deadlines_are_never_revived_by_a_reboot() {
        let db = fixture();
        deadlines::reached(&db, "room".into(), "agent".into(), 100_001, true).unwrap();
        advance(&db, 200_000, "boot-b", false).unwrap();
        assert_eq!(deadline(&db).deadline_ms, 100_000);
        assert!(deadline(&db).reached_at_ms.is_some());
    }

    #[test]
    fn an_upgrade_without_a_prior_heartbeat_does_not_invent_downtime() {
        let db = fixture();
        db.execute("DELETE FROM agent_deadline_clock", []).unwrap();
        advance(&db, 20_000, "boot-b", false).unwrap();
        assert_eq!(deadline(&db).deadline_ms, 100_000);
    }

    #[test]
    fn no_clock_is_written_without_an_armed_deadline() {
        let db = Connection::open_in_memory().unwrap();
        deadlines::initialize(&db).unwrap();
        advance(&db, 20_000, "boot-b", false).unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM agent_deadline_clock", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn a_failed_heartbeat_rolls_back_the_deadline_shift_before_retry() {
        let db = fixture();
        db.execute_batch("CREATE TRIGGER clock_failure BEFORE UPDATE ON agent_deadline_clock BEGIN SELECT RAISE(ABORT,'clock unavailable'); END;").unwrap();
        {
            let tx = db.unchecked_transaction().unwrap();
            assert!(advance(&tx, 12_000, "boot-b", false).is_err());
        }
        assert_eq!(deadline(&db).deadline_ms, 100_000);
        db.execute_batch("DROP TRIGGER clock_failure").unwrap();
        let tx = db.unchecked_transaction().unwrap();
        advance(&tx, 12_000, "boot-b", false).unwrap();
        tx.commit().unwrap();
        assert_eq!(deadline(&db).deadline_ms, 111_000);
    }

    #[test]
    fn downtime_restoration_does_not_allow_manual_deadline_extensions() {
        let db = fixture();
        advance(&db, 12_000, instance_id(), false).unwrap();
        assert_eq!(deadline(&db).deadline_ms, 111_000);
        assert!(
            matches!(deadlines::set(&db, "room".into(), "agent".into(), 111_001), Err(DaemonError::MachineRequest { code, .. }) if code == "agent_deadline_extension_refused")
        );
    }
}
