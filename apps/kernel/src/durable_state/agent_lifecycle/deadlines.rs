//! Deadline edits and receipts share the lifecycle writer.
use super::*;
use crate::local::{AgentDeadline, AgentDeadlineState};

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_deadlines(room_id TEXT NOT NULL,agent_id TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(room_id,agent_id));
        DROP TABLE IF EXISTS agent_deadline_intents;").map_err(sql)?;
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
    // A finished agent can be given a new limit, but not while its cleanup is still pending.
    if get(db, room, agent)?.is_some_and(|old| {
        old.state == AgentDeadlineState::Reached
            && old.stopped_at_ms.is_none()
            && deadline != Some(old.deadline_ms)
    }) {
        return Err(DaemonError::MachineRequest {
            code: "agent_deadline_cleanup_pending".into(),
            message: "the agent's deadline was reached and its cleanup is still running; retry the edit in a few seconds".into(),
        });
    }
    Ok(())
}

pub(super) fn set(
    db: &Connection,
    room: String,
    agent: String,
    deadline: Option<u64>,
) -> Result<Outcome, DaemonError> {
    validate(db, &room, &agent, deadline)?;
    // A repeat of the time that ended the run changes nothing: it is not a new limit.
    if let Some(old) = get(db, &room, &agent)?
        .filter(|old| old.state == AgentDeadlineState::Reached && deadline == Some(old.deadline_ms))
    {
        return Ok(Outcome::Deadline(Some(old)));
    }
    // An edit reaches the worker before it commits, so the worker holds this value.
    deadline_clock::rearmed(db, &room, &agent)?;
    // The new limit replaces the one that ended this agent's run.
    let outcome = match deadline {
        Some(deadline_ms) => save(
            db,
            &AgentDeadline {
                session_id: room,
                agent_id: agent,
                deadline_ms,
                state: AgentDeadlineState::Armed,
                reached_at_ms: None,
                stopped_at_ms: None,
            },
        )?,
        None => {
            db.execute(
                "DELETE FROM agent_deadlines WHERE room_id=?1 AND agent_id=?2",
                params![room, agent],
            )
            .map_err(sql)?;
            Outcome::Deadline(None)
        }
    };
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
