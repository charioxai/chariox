//! Deadline intent and receipts share the lifecycle writer.
use super::*;
use crate::local::{AgentDeadline, AgentDeadlineState};

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_deadlines(room_id TEXT NOT NULL,agent_id TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(room_id,agent_id));
        CREATE TABLE IF NOT EXISTS agent_deadline_intents(room_id TEXT NOT NULL,agent_id TEXT NOT NULL,deadline_ms INTEGER NOT NULL,PRIMARY KEY(room_id,agent_id));").map_err(sql)?;
    deadline_clock::initialize(db)
}

pub(super) fn armed(db: &Connection) -> Result<Vec<AgentDeadline>, DaemonError> {
    let mut query = db
        .prepare("SELECT payload FROM agent_deadlines WHERE json_extract(payload,'$.state')='armed' ORDER BY room_id,agent_id")
        .map_err(sql)?;
    let rows = query
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sql)?;
    rows.map(|row| decode(&row.map_err(sql)?)).collect()
}

/// The owner's explicit limit moves with the deadline it backs, or a later edit would undo the move.
pub(super) fn move_intent(
    db: &Connection,
    room: &str,
    agent: &str,
    to: impl FnOnce(u64) -> Result<u64, DaemonError>,
) -> Result<(), DaemonError> {
    let intent: Option<i64> = db
        .query_row(
            "SELECT deadline_ms FROM agent_deadline_intents WHERE room_id=?1 AND agent_id=?2",
            params![room, agent],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    if let Some(intent) = intent {
        let moved = to(u64::try_from(intent).map_err(|_| error("invalid deadline intent"))?)?;
        db.execute(
            "UPDATE agent_deadline_intents SET deadline_ms=?3 WHERE room_id=?1 AND agent_id=?2",
            params![room, agent, sql_integer(moved)?],
        )
        .map_err(sql)?;
    }
    Ok(())
}

pub(super) fn get(
    db: &Connection,
    room: &str,
    agent: &str,
) -> Result<Option<AgentDeadline>, DaemonError> {
    let value: Option<String> = db
        .query_row(
            "SELECT payload FROM agent_deadlines WHERE room_id=?1 AND agent_id=?2",
            params![room, agent],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    value.map(|value| decode(&value)).transpose()
}

pub(super) fn ensure_before_deadline(
    db: &Connection,
    room: &str,
    agent: &str,
) -> Result<(), DaemonError> {
    if let Some(deadline) = get(db, room, agent)?.filter(|deadline| {
        deadline.state == AgentDeadlineState::Reached
            || deadline.deadline_ms <= crate::session::unix_epoch_ms()
    }) {
        return Err(crate::error::agent_deadline_refusal(deadline.deadline_ms));
    }
    Ok(())
}

pub(super) fn save(db: &Connection, value: &AgentDeadline) -> Result<Outcome, DaemonError> {
    db.execute("INSERT INTO agent_deadlines(room_id,agent_id,payload) VALUES(?1,?2,?3) ON CONFLICT(room_id,agent_id) DO UPDATE SET payload=excluded.payload", params![value.session_id, value.agent_id, encode(value)?]).map_err(sql)?;
    Ok(Outcome::Deadline(Some(value.clone())))
}

fn validate(
    db: &Connection,
    room: &str,
    agent: &str,
    deadline: Option<u64>,
) -> Result<(), DaemonError> {
    if deadline.is_some_and(|time| time == 0 || time > i64::MAX as u64) {
        return Err(DaemonError::MachineRequest {
            code: "agent_deadline_invalid".into(),
            message: "deadline must be a positive bounded epoch millisecond, or null to clear"
                .into(),
        });
    }
    if get(db, room, agent)?.is_some_and(|old| {
        old.state == AgentDeadlineState::Reached && deadline != Some(old.deadline_ms)
    }) {
        return Err(DaemonError::MachineRequest {
            code: "agent_deadline_reached".into(),
            message: "agent deadline reached; start a new agent for new work".into(),
        });
    }
    Ok(())
}

fn set_effective(
    db: &Connection,
    room: &str,
    agent: &str,
    deadline: Option<u64>,
) -> Result<Outcome, DaemonError> {
    // An edit reaches the worker before it commits, so the worker holds this value.
    deadline_clock::rearmed(db, room, agent)?;
    if let Some(mut old) = get(db, room, agent)? {
        // A parent's edit cannot revive a child whose own deadline already ended its work.
        if old.state == AgentDeadlineState::Reached {
            return Ok(Outcome::Deadline(Some(old)));
        }
        if let Some(deadline) = deadline {
            old.deadline_ms = deadline;
            return save(db, &old);
        }
    }
    match deadline {
        Some(deadline_ms) => save(
            db,
            &AgentDeadline {
                session_id: room.into(),
                agent_id: agent.into(),
                deadline_ms,
                state: AgentDeadlineState::Armed,
                reached_at_ms: None,
                stopped_at_ms: None,
            },
        ),
        None => {
            db.execute(
                "DELETE FROM agent_deadlines WHERE room_id=?1 AND agent_id=?2",
                params![room, agent],
            )
            .map_err(sql)?;
            Ok(Outcome::Deadline(None))
        }
    }
}

pub(super) fn set(
    db: &Connection,
    room: String,
    agent: String,
    deadline: Option<u64>,
    inherited: Vec<(String, String, Option<u64>)>,
) -> Result<Outcome, DaemonError> {
    validate(db, &room, &agent, deadline)?;
    if let Some(time) = deadline {
        db.execute("INSERT INTO agent_deadline_intents(room_id,agent_id,deadline_ms) VALUES(?1,?2,?3) ON CONFLICT(room_id,agent_id) DO UPDATE SET deadline_ms=excluded.deadline_ms", params![room,agent,i64::try_from(time).expect("validated deadline")]).map_err(sql)?;
    } else {
        db.execute(
            "DELETE FROM agent_deadline_intents WHERE room_id=?1 AND agent_id=?2",
            params![room, agent],
        )
        .map_err(sql)?;
    }
    let outcome = set_effective(db, &room, &agent, deadline)?;
    for (child_room, child, time) in inherited {
        set_effective(db, &child_room, &child, time)?;
    }
    // The first armed deadline starts the heartbeat, so a kernel stopped right after has one to resume from.
    deadline_clock::beat(db, crate::session::unix_epoch_ms())?;
    Ok(outcome)
}

pub(super) fn reached(
    db: &Transaction<'_>,
    room: String,
    agent: String,
    now: u64,
    stopped: bool,
) -> Result<Outcome, DaemonError> {
    let mut value = get(db, &room, &agent)?.ok_or_else(|| error("deadline not armed"))?;
    if now < value.deadline_ms {
        return Err(error("deadline not reached"));
    }
    if value.reached_at_ms.is_some() && (!stopped || value.stopped_at_ms.is_some()) {
        return Ok(Outcome::Deadline(Some(value)));
    }
    value.state = AgentDeadlineState::Reached;
    value.reached_at_ms.get_or_insert(now);
    if stopped {
        value.stopped_at_ms.get_or_insert(now);
    }
    let outcome = save(db, &value)?;
    super::delivery_failure::expire_deadline_messages(db, &room, &agent, now)?;
    if stopped {
        outside::publish_deadline(db, &value)?;
    }
    Ok(outcome)
}

impl DurableKernelStateStore {
    pub(crate) fn ensure_agent_before_deadline(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        let db = self.lock_connection("agent.deadline")?;
        ensure_before_deadline(&db, room, agent)
    }

    pub(crate) fn agent_deadline(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<Option<AgentDeadline>, DaemonError> {
        let db = self.lock_connection("agent.deadline")?;
        get(&db, room, agent)
    }
    pub(crate) fn validate_agent_deadline_edit(
        &self,
        room: &str,
        agent: &str,
        deadline: Option<u64>,
    ) -> Result<(), DaemonError> {
        let db = self.lock_connection("agent.deadline")?;
        validate(&db, room, agent, deadline)
    }
    /// The owner's explicit limit, separate from an earlier limit inherited from a parent.
    pub(crate) fn agent_deadline_intent(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<Option<u64>, DaemonError> {
        let db = self.lock_connection("agent.deadline")?;
        let value: Option<i64> = db
            .query_row(
                "SELECT deadline_ms FROM agent_deadline_intents WHERE room_id=?1 AND agent_id=?2",
                params![room, agent],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        value
            .map(|time| u64::try_from(time).map_err(|_| error("invalid deadline intent")))
            .transpose()
    }
    /// A starting kernel's one shift of armed deadlines by the time it was stopped.
    pub(crate) fn resume_agent_deadlines_after_kernel_start(&self) -> Result<(), DaemonError> {
        self.agent_lifecycle(Operation::ResumeDeadlines {
            now: crate::session::unix_epoch_ms(),
        })
        .map(|_| ())
    }
    pub(crate) fn agent_deadlines_awaiting_rearm(
        &self,
    ) -> Result<Vec<WorkerHeldDeadline>, DaemonError> {
        let db = self.lock_connection("agent.deadline")?;
        deadline_clock::awaiting_rearm(&db)
    }
    pub(crate) fn stopped_deadline_agents(&self, room: &str) -> Result<Vec<String>, DaemonError> {
        let db = self.lock_connection("agent.deadline")?;
        let mut query = db
            .prepare(
                "SELECT agent_id FROM agent_deadlines
                 WHERE room_id = ?1 AND json_extract(payload, '$.stopped_at_ms') IS NOT NULL",
            )
            .map_err(sql)?;
        let rows = query
            .query_map(params![room], |row| row.get(0))
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)
    }

    pub(crate) fn agent_deadlines(&self) -> Result<Vec<AgentDeadline>, DaemonError> {
        let db = self.lock_connection("agent.deadline")?;
        let mut query = db
            .prepare("SELECT payload FROM agent_deadlines ORDER BY room_id,agent_id")
            .map_err(sql)?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sql)?;
        rows.map(|row| decode(&row.map_err(sql)?)).collect()
    }
}
