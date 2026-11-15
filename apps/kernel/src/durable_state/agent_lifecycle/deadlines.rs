//! Deadline intent and receipts share the lifecycle writer.
use super::*;
use crate::local::{AgentDeadline, AgentDeadlineState};

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_deadlines(room_id TEXT NOT NULL,agent_id TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(room_id,agent_id));
        CREATE TABLE IF NOT EXISTS agent_deadline_intents(room_id TEXT NOT NULL,agent_id TEXT NOT NULL,deadline_ms INTEGER NOT NULL,PRIMARY KEY(room_id,agent_id));").map_err(sql)
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

fn save(db: &Connection, value: &AgentDeadline) -> Result<Outcome, DaemonError> {
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
    inherited: Vec<(String, Option<u64>)>,
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
    for (child, time) in inherited {
        set_effective(db, &room, &child, time)?;
    }
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
    if stopped {
        outside::publish_deadline(db, &value)?;
    }
    Ok(outcome)
}

impl DurableKernelStateStore {
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
