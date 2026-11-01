//! MP-08 / MP-09 / MP-10 / MP-11: deadline intent and receipts share A02's writer.
use super::*;
use crate::local::{AgentDeadline, AgentDeadlineState};

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_deadlines(room_id TEXT NOT NULL,agent_id TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(room_id,agent_id));").map_err(sql)
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
    Ok(Outcome::Deadline(value.clone()))
}

pub(super) fn set(
    db: &Connection,
    room: String,
    agent: String,
    deadline: u64,
) -> Result<Outcome, DaemonError> {
    if deadline == 0 || deadline > i64::MAX as u64 {
        return Err(DaemonError::MachineRequest {
            code: "agent_deadline_invalid".into(),
            message: "deadline must be a positive bounded epoch millisecond".into(),
        });
    }
    if let Some(mut old) = get(db, &room, &agent)? {
        if deadline > old.deadline_ms {
            return Err(DaemonError::MachineRequest {
                code: "agent_deadline_extension_refused".into(),
                message: "an agent deadline cannot be extended; start a new agent".into(),
            });
        }
        old.deadline_ms = deadline;
        return save(db, &old);
    }
    save(
        db,
        &AgentDeadline {
            session_id: room,
            agent_id: agent,
            deadline_ms: deadline,
            state: AgentDeadlineState::Armed,
            reached_at_ms: None,
            stopped_at_ms: None,
        },
    )
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
        return Ok(Outcome::Deadline(value));
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
