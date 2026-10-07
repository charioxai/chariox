//! MP-08 / MP-09 / MP-10 / MP-11 A03: durable timers and watched processes.
//! Each wake is a task obligation with a live registration. Firing commits
//! the inbox occurrence and the wake's receipts in one transaction.
use super::*;
use types::AgentWake;

/// Per-agent bound on armed wakes; an agent cancels before arming more.
pub(crate) const MAX_ACTIVE_WAKES: i64 = 32;

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_wakes(id TEXT PRIMARY KEY,task_id TEXT NOT NULL,room_id TEXT NOT NULL,agent_id TEXT NOT NULL,payload TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS agent_wakes_recipient ON agent_wakes(room_id,agent_id);")
        .map_err(sql)
}

impl DurableKernelStateStore {
    pub(crate) fn agent_wakes(
        &self,
        room: Option<&str>,
        agent: Option<&str>,
    ) -> Result<Vec<AgentWake>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.wakes")?;
        let mut q = db.prepare("SELECT payload FROM agent_wakes WHERE (?1 IS NULL OR room_id=?1) AND (?2 IS NULL OR agent_id=?2) ORDER BY rowid").map_err(sql)?;
        let rows = q
            .query_map(params![room, agent], |r| r.get::<_, String>(0))
            .map_err(sql)?;
        rows.map(|row| decode(&row.map_err(sql)?)).collect()
    }
    pub(crate) fn agent_armed_timers(&self) -> Result<Vec<AgentWake>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.timers")?;
        let mut q = db.prepare("SELECT payload FROM agent_wakes WHERE json_extract(payload,'$.kind')='timer' AND json_extract(payload,'$.state')='scheduled'").map_err(sql)?;
        let rows = q.query_map([], |r| r.get::<_, String>(0)).map_err(sql)?;
        rows.map(|row| decode(&row.map_err(sql)?)).collect()
    }
}

fn load_wake(tx: &Transaction<'_>, id: &str) -> Result<AgentWake, DaemonError> {
    let payload: String = tx
        .query_row("SELECT payload FROM agent_wakes WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(sql)?
        .ok_or_else(|| error("wake unavailable"))?;
    let wake: AgentWake = decode(&payload)?;
    if wake.id != id {
        return Err(error("wake identity corrupt; quarantine required"));
    }
    Ok(wake)
}

fn save_wake(tx: &Transaction<'_>, wake: &AgentWake) -> Result<(), DaemonError> {
    tx.execute("INSERT INTO agent_wakes VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![wake.id,wake.task_id,wake.room_id,wake.agent_id,encode(wake)?]).map_err(sql)?;
    Ok(())
}

fn wakes_in(tx: &Transaction<'_>, states: &[&str]) -> Result<Vec<AgentWake>, DaemonError> {
    let mut q = tx
        .prepare("SELECT payload FROM agent_wakes ORDER BY rowid")
        .map_err(sql)?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    let mut result = vec![];
    for payload in rows {
        let wake: AgentWake = decode(&payload)?;
        if states.contains(&wake.state.as_str()) {
            result.push(wake);
        }
    }
    Ok(result)
}

fn terminal(wake: &AgentWake) -> bool {
    matches!(
        wake.state.as_str(),
        "fired" | "exited" | "lost" | "cancelled"
    )
}

/// Records `event` as the latest occurrence of `wake`; receipts restart.
fn record_fire(
    tx: &Transaction<'_>,
    wake: &mut AgentWake,
    event: &InboxEvent,
    now: u64,
) -> Result<(), DaemonError> {
    wake.fire_count += 1;
    wake.last_fired_at_ms = Some(now);
    wake.last_sequence = Some(event.sequence);
    wake.last_delivery = Some(event.state.clone());
    wake.last_delivered_at_ms = None;
    wake.last_acknowledged_at_ms = None;
    save_wake(tx, wake)
}

/// A terminal source outcome reuses the A02 obligation/registration path.
fn settle_source(
    tx: &Transaction<'_>,
    wake: &mut AgentWake,
    occurrence: String,
    success: bool,
    answer: serde_json::Value,
    now: u64,
) -> Result<(), DaemonError> {
    super::supervision::apply(
        tx,
        Operation::SourceOutcome {
            public_answer: Some(answer),
            room: wake.room_id.clone(),
            source: wake.id.clone(),
            occurrence: occurrence.clone(),
            success,
            now,
        },
    )?;
    let row:Option<(i64,String,String)>=tx.query_row("SELECT sequence,source_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND source_id=?3 AND occurrence_id=?4",params![wake.room_id,wake.agent_id,wake.id,occurrence],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
    match row {
        Some((seq, source, payload)) => {
            let event = decode_inbox(
                seq,
                &wake.room_id,
                &wake.agent_id,
                &source,
                &occurrence,
                &payload,
            )?;
            record_fire(tx, wake, &event, now)
        }
        // No live registration remained; the outcome stays recoverable by cursor.
        None => save_wake(tx, wake),
    }
}

fn cancel_settle(tx: &Transaction<'_>, wake: &mut AgentWake, now: u64) -> Result<(), DaemonError> {
    wake.state = "cancelled".into();
    wake.next_due_ms = None;
    let answer = serde_json::json!({"wake_id":wake.id,"label":wake.label,"cancelled":true});
    settle_source(
        tx,
        wake,
        format!("{}-cancelled", wake.id),
        false,
        answer,
        now,
    )
}

pub(super) fn apply(tx: &Transaction<'_>, op: Operation) -> Result<Outcome, DaemonError> {
    match op {
        Operation::CreateWake {
            task,
            prompt,
            mut wake,
        } => {
            let mut t = load(tx, &task)?;
            current(&t, &prompt)?;
            if wake.task_id != t.task_id
                || wake.room_id != t.room_id
                || wake.agent_id != t.agent_id
                || wake.registration_id != format!("completion-{}", wake.id)
                || !matches!(wake.kind.as_str(), "timer" | "process")
            {
                return Err(error("wake binding invalid"));
            }
            if wake.kind == "timer"
                && wake
                    .next_due_ms
                    .is_none_or(|due| due <= wake.created_at_ms || due > i64::MAX as u64)
            {
                return Err(error("timer requires a finite future due time"));
            }
            let active: i64 = tx.query_row("SELECT count(*) FROM agent_wakes WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state') IN ('scheduled','starting','running')",params![t.room_id,t.agent_id],|r|r.get(0)).map_err(sql)?;
            if active >= MAX_ACTIVE_WAKES {
                return Err(error("wake limit reached; cancel an armed wake first"));
            }
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_wakes WHERE id=?1)",
                    [&wake.id],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            if exists {
                return Err(error("wake identity conflict"));
            }
            wake.state = if wake.kind == "timer" {
                "scheduled"
            } else {
                "starting"
            }
            .into();
            wake.verified_at_ms = None;
            t.obligations.push(AgentObligation {
                id: wake.id.clone(),
                kind: wake.kind.clone(),
                resource_id: Some(wake.id.clone()),
                completion_task_id: None,
                status: "open".into(),
                // A process launch is an intent until the supervisor's receipt.
                dispatch_state: if wake.kind == "timer" {
                    "accepted"
                } else {
                    "intent"
                }
                .into(),
            });
            let registration = Registration {
                id: wake.registration_id.clone(),
                task_id: t.task_id.clone(),
                source_id: wake.id.clone(),
                obligation_id: Some(wake.id.clone()),
                source_cursor: 0,
                live: true,
            };
            tx.execute(
                "INSERT INTO agent_registrations VALUES(?1,?2,?3)",
                params![
                    registration.id,
                    registration.task_id,
                    encode(&registration)?
                ],
            )
            .map_err(sql)?;
            t.revision += 1;
            save(tx, &t)?;
            save_wake(tx, &wake)?;
            Ok(Outcome::Wakes(vec![wake]))
        }
        Operation::VerifyWakes { now } => {
            let mut verified = vec![];
            for mut wake in wakes_in(tx, &["scheduled"])? {
                if wake.verified_at_ms.is_none() {
                    wake.verified_at_ms = Some(now);
                    save_wake(tx, &wake)?;
                    verified.push(wake);
                }
            }
            Ok(Outcome::Wakes(verified))
        }
        Operation::FireWakes { now } => {
            let mut fired = vec![];
            for mut wake in wakes_in(tx, &["scheduled"])? {
                let Some(due) = wake.next_due_ms.filter(|due| *due <= now) else {
                    continue;
                };
                let t = load(tx, &wake.task_id)?;
                if matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled) {
                    cancel_settle(tx, &mut wake, now)?;
                    continue;
                }
                // Missed intervals (kernel down, host asleep, stalled scheduler)
                // coalesce into this one occurrence with an explicit count.
                let missed = wake.interval_ms.map_or(0, |i| (now - due) / i.max(1));
                let occurrence_id = format!("{}-fire-{}", wake.id, wake.fire_count + 1);
                let payload = serde_json::json!({"task_id":t.task_id,"wake_id":wake.id,"kind":"timer","label":wake.label,"due_at_ms":due,"fired_at_ms":now,"late_ms":now-due,"missed_fires":missed,"fire":wake.fire_count+1});
                wake.missed_fires += missed;
                if let Some(interval) = wake.interval_ms {
                    wake.next_due_ms = Some(due + (missed + 1) * interval.max(1));
                    let e = event(
                        tx,
                        occurrence(
                            &t.room_id,
                            &t.agent_id,
                            &wake.id,
                            &occurrence_id,
                            "timer_fired",
                            payload,
                        ),
                    )?;
                    record_fire(tx, &mut wake, &e, now)?;
                } else {
                    wake.state = "fired".into();
                    wake.next_due_ms = None;
                    settle_source(tx, &mut wake, occurrence_id, true, payload, now)?;
                }
                fired.push(wake);
            }
            Ok(Outcome::Wakes(fired))
        }
        Operation::ProcessStarted { id, pid, now } => {
            let mut wake = load_wake(tx, &id)?;
            if wake.state != "starting" || pid <= 1 {
                return Err(error("process start receipt does not match its intent"));
            }
            let mut t = load(tx, &wake.task_id)?;
            if let Some(o) = t.obligations.iter_mut().find(|o| o.id == id) {
                o.dispatch_state = "accepted".into();
            }
            t.revision += 1;
            save(tx, &t)?;
            wake.state = "running".into();
            wake.pid = Some(pid);
            wake.verified_at_ms = Some(now);
            save_wake(tx, &wake)?;
            Ok(Outcome::Wakes(vec![wake]))
        }
        Operation::ProcessMatched { id, line, now } => {
            let mut wake = load_wake(tx, &id)?;
            // One occurrence per armed condition; rearming is explicit.
            if wake.state != "running" || wake.matched_at_ms.is_some() {
                return Ok(Outcome::Wakes(vec![]));
            }
            wake.matched_at_ms = Some(now);
            let e = event(
                tx,
                occurrence(
                    &wake.room_id,
                    &wake.agent_id,
                    &wake.id,
                    &format!("{}-match", wake.id),
                    "process_output_matched",
                    serde_json::json!({"task_id":wake.task_id,"wake_id":wake.id,"kind":"process","label":wake.label,"match_text":wake.match_text,"line":line}),
                ),
            )?;
            record_fire(tx, &mut wake, &e, now)?;
            Ok(Outcome::Wakes(vec![wake]))
        }
        Operation::ProcessExited {
            id,
            exit_code,
            tail,
            now,
        } => {
            let mut wake = load_wake(tx, &id)?;
            if terminal(&wake) {
                return Ok(Outcome::Wakes(vec![]));
            }
            wake.state = if exit_code.is_some() {
                "exited"
            } else {
                "lost"
            }
            .into();
            wake.exit_code = exit_code;
            let answer = serde_json::json!({"wake_id":wake.id,"kind":"process","label":wake.label,"exit_code":exit_code,"process_lost":exit_code.is_none(),"output_tail":tail});
            settle_source(
                tx,
                &mut wake,
                format!("{id}-exit"),
                exit_code == Some(0),
                answer,
                now,
            )?;
            Ok(Outcome::Wakes(vec![wake]))
        }
        Operation::CancelWake { id, task, prompt } => {
            let mut wake = load_wake(tx, &id)?;
            if wake.task_id != task {
                return Err(error("wake belongs to another task"));
            }
            if terminal(&wake) {
                return Ok(Outcome::Wakes(vec![wake]));
            }
            let mut t = load(tx, &task)?;
            wake.state = "cancelled".into();
            wake.next_due_ms = None;
            match prompt {
                Some(prompt) => {
                    current(&t, &prompt)?;
                    if let Some(o) = t.obligations.iter_mut().find(|o| o.id == id) {
                        o.status = "cancelled".into();
                    }
                    if let Some(mut reg) = registrations(tx, &task)?
                        .into_iter()
                        .find(|r| r.id == wake.registration_id)
                    {
                        reg.live = false;
                        tx.execute(
                            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                            params![reg.id, encode(&reg)?],
                        )
                        .map_err(sql)?;
                    }
                    t.revision += 1;
                    save(tx, &t)?;
                    save_wake(tx, &wake)?;
                }
                None => {
                    if t.state != ExecutionState::Cancelled {
                        return Err(error(
                            "only the owning turn or task cancellation cancels a wake",
                        ));
                    }
                    cancel_settle(tx, &mut wake, crate::session::unix_epoch_ms())?;
                }
            }
            Ok(Outcome::Wakes(vec![wake]))
        }
        Operation::WakeAlerted { id, sequence } => {
            let mut wake = load_wake(tx, &id)?;
            wake.alerted_sequence = Some(sequence);
            save_wake(tx, &wake)?;
            Ok(Outcome::Wakes(vec![wake]))
        }
        _ => Err(error("invalid wake operation")),
    }
}

/// Mirrors the inbox receipt of a wake's latest occurrence onto the wake.
pub(super) fn record_delivery(tx: &Transaction<'_>, e: &InboxEvent) -> Result<(), DaemonError> {
    let payload:Option<String>=tx.query_row("SELECT payload FROM agent_wakes WHERE room_id=?1 AND agent_id=?2 AND id=?3 AND json_extract(payload,'$.last_sequence')=?4",params![e.room_id,e.agent_id,e.source_id,sql_integer(e.sequence)?],|r|r.get(0)).optional().map_err(sql)?;
    let Some(payload) = payload else {
        return Ok(());
    };
    let mut wake: AgentWake = decode(&payload)?;
    let now = crate::session::unix_epoch_ms();
    wake.last_delivery = Some(e.state.clone());
    if matches!(e.state.as_str(), "accepted" | "acknowledged" | "handled") {
        wake.last_delivered_at_ms.get_or_insert(now);
    }
    if matches!(e.state.as_str(), "acknowledged" | "handled") {
        wake.last_acknowledged_at_ms.get_or_insert(now);
    }
    save_wake(tx, &wake)
}
