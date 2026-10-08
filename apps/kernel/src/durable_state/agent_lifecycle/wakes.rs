//! MP-08 / MP-09 / MP-10 / MP-11 A03: durable timers and watched processes.
//! Each wake is a task obligation with a live registration. Firing commits
//! the inbox occurrence and the wake's receipts in one transaction.
use super::*;
use types::AgentWake;

/// Per-agent bound on armed wakes; an agent cancels before arming more.
pub(crate) const MAX_ACTIVE_WAKES: i64 = 32;
/// Per-agent bound on retained finished wakes (the projection shows 64).
const FINISHED_WAKES_RETAINED: i64 = 64;
const ARMED: &[&str] = &["scheduled", "starting", "running", "cancelling"];
const FINISHED: &str = "('fired','exited','lost','cancelled')";

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    super::wake_receipts::initialize(db)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS agent_wakes(id TEXT PRIMARY KEY,task_id TEXT NOT NULL,room_id TEXT NOT NULL,agent_id TEXT NOT NULL,payload TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS agent_wakes_recipient ON agent_wakes(room_id,agent_id);
    CREATE INDEX IF NOT EXISTS agent_wakes_state ON agent_wakes(json_extract(payload,'$.state'));")
        .map_err(sql)
}

/// Keeps the newest finished wakes of one agent and drops older ones with
/// their receipts, so wake history cannot grow without bound.
fn retain_finished(tx: &Transaction<'_>, room: &str, agent: &str) -> Result<(), DaemonError> {
    let expired = format!("SELECT id FROM agent_wakes WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state') IN {FINISHED} ORDER BY rowid DESC LIMIT -1 OFFSET ?3");
    tx.execute(
        &format!("DELETE FROM agent_wake_receipts WHERE wake_id IN ({expired})"),
        params![room, agent, FINISHED_WAKES_RETAINED],
    )
    .map_err(sql)?;
    tx.execute(
        &format!("DELETE FROM agent_wakes WHERE id IN ({expired})"),
        params![room, agent, FINISHED_WAKES_RETAINED],
    )
    .map_err(sql)?;
    Ok(())
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
    /// The arm receipt belongs to the exact wake, including after it fires.
    pub(crate) fn agent_wake_verification(&self, id: &str) -> Result<Option<u64>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.verification")?;
        let payload: Option<String> = db
            .query_row("SELECT payload FROM agent_wakes WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sql)?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let wake: AgentWake = decode(&payload)?;
        if wake.id != id {
            return Err(error("wake identity corrupt; quarantine required"));
        }
        Ok(wake.verified_at_ms)
    }

    pub(crate) fn agent_armed_timers(&self) -> Result<Vec<AgentWake>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.timers")?;
        let mut q = db.prepare("SELECT payload FROM agent_wakes WHERE json_extract(payload,'$.state')='scheduled' AND json_extract(payload,'$.kind')='timer'").map_err(sql)?;
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

/// Reads only wakes in `states` through the state index; finished history
/// is never decoded by the scheduler.
fn wakes_in(tx: &Transaction<'_>, states: &[&str]) -> Result<Vec<AgentWake>, DaemonError> {
    let mut q = tx
        .prepare("SELECT payload FROM agent_wakes WHERE json_extract(payload,'$.state') IN (SELECT value FROM json_each(?1)) ORDER BY rowid")
        .map_err(sql)?;
    let rows = q
        .query_map(
            [serde_json::to_string(states).map_err(|e| error(e.to_string()))?],
            |r| r.get::<_, String>(0),
        )
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
    super::wake_receipts::fire(tx, wake, event, now)?;
    save_wake(tx, wake)
}

/// A terminal source outcome reuses the A02 obligation/registration path.
/// Only a real occurrence (`fired`) is recorded as a fire with receipts.
fn settle_source(
    tx: &Transaction<'_>,
    wake: &mut AgentWake,
    occurrence: String,
    success: bool,
    answer: serde_json::Value,
    now: u64,
    fired: bool,
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
        Some((seq, source, payload)) if fired => {
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
        // A cancellation, or no live registration remained; the outcome
        // stays recoverable by cursor.
        _ => save_wake(tx, wake),
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
        false,
    )
}

fn fire_one(
    tx: &Transaction<'_>,
    mut wake: AgentWake,
    due: u64,
    now: u64,
) -> Result<Option<AgentWake>, DaemonError> {
    let t = load(tx, &wake.task_id)?;
    if matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled) {
        cancel_settle(tx, &mut wake, now)?;
        return Ok(None);
    }
    // Timer-only turns exhaust the existing no-progress budget. Wait for
    // explicit owner disposition before emitting another paid check-in.
    if t.state == ExecutionState::Blocked {
        return Ok(None);
    }
    if wake.interval_ms.is_some() {
        let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND source_id=?3 AND json_extract(payload,'$.state') NOT IN ('handled','expired','failed'))", params![wake.room_id,wake.agent_id,wake.id], |r| r.get(0)).map_err(sql)?;
        if pending {
            return Ok(None);
        }
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
        settle_source(tx, &mut wake, occurrence_id, true, payload, now, true)?;
    }
    Ok(Some(wake))
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
            let active: i64 = tx.query_row("SELECT count(*) FROM agent_wakes WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state') IN ('scheduled','starting','running','cancelling')",params![t.room_id,t.agent_id],|r|r.get(0)).map_err(sql)?;
            // Finished-but-unhandled sources still reserve capacity. Counting
            // inbox attribution survives finished-wake/receipt retention.
            let unhandled: i64 = tx.query_row("SELECT count(DISTINCT source_id) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state') NOT IN ('handled','expired','failed') AND json_extract(payload,'$.kind') != 'message' AND (json_extract(payload,'$.payload.wake_id') IS NOT NULL OR json_extract(payload,'$.payload.public_answer.wake_id') IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM agent_wakes w WHERE w.id=agent_inbox.source_id AND json_extract(w.payload,'$.state') IN ('scheduled','starting','running','cancelling'))", params![t.room_id,t.agent_id], |r| r.get(0)).map_err(sql)?;
            if active + unhandled >= MAX_ACTIVE_WAKES {
                return Err(error("wake limit reached; handle outstanding wake events or cancel an armed wake first"));
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
            retain_finished(tx, &wake.room_id, &wake.agent_id)?;
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
                // A full/corrupt recipient never rolls back another timer.
                // Keep its original due time and occurrence retryable.
                tx.execute_batch("SAVEPOINT wake_fire").map_err(sql)?;
                match fire_one(tx, wake.clone(), due, now) {
                    Ok(result) => {
                        tx.execute_batch("RELEASE wake_fire").map_err(sql)?;
                        fired.extend(result);
                    }
                    Err(_) => {
                        tx.execute_batch("ROLLBACK TO wake_fire; RELEASE wake_fire")
                            .map_err(sql)?;
                        if wake.last_delivery.as_deref() != Some("backpressured") {
                            wake.last_delivery = Some("backpressured".into());
                            save_wake(tx, &wake)?;
                            fired.push(wake);
                        }
                    }
                }
            }
            Ok(Outcome::Wakes(fired))
        }
        Operation::ProcessStarted { id, pid, now } => {
            let mut wake = load_wake(tx, &id)?;
            if wake.state != "starting" || pid <= 1 {
                return Err(error("process start receipt does not match its intent"));
            }
            let mut t = load(tx, &wake.task_id)?;
            if t.state != ExecutionState::Working {
                return Err(error(
                    "process start authority ended; physical cancellation must settle",
                ));
            }
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
            if wake.state == "cancelling" && exit_code.is_some() {
                let mut t = load(tx, &wake.task_id)?;
                if let Some(o) = t.obligations.iter_mut().find(|o| o.id == id) {
                    o.status = "cancelled".into();
                }
                t.revision += 1;
                save(tx, &t)?;
                wake.exit_code = exit_code;
                cancel_settle(tx, &mut wake, now)?;
                return Ok(Outcome::Wakes(vec![wake]));
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
                true,
            )?;
            Ok(Outcome::Wakes(vec![wake]))
        }
        Operation::CancelWake { id, task, prompt } => {
            let mut wake = load_wake(tx, &id)?;
            let caller = load(tx, &task)?;
            let mut t = load(tx, &wake.task_id)?;
            if let Some(prompt) = &prompt {
                current(&caller, prompt)?;
                // A fresh user turn is its own task. It may cancel a retained
                // source of this same agent, without taking over that task.
                if caller.room_id != t.room_id
                    || caller.agent_id != t.agent_id
                    || caller.owner_user_id.is_empty()
                    || caller.owner_user_id != t.owner_user_id
                {
                    return Err(error(
                        "wake cancellation requires the owning agent and room",
                    ));
                }
            } else if caller.task_id != wake.task_id || t.state != ExecutionState::Cancelled {
                return Err(error(
                    "only the owning turn or task cancellation cancels a wake",
                ));
            }
            if terminal(&wake) {
                return Ok(Outcome::Wakes(vec![wake]));
            }
            if wake.kind == "process" {
                // A signal request cannot settle an executing resource.
                wake.state = "cancelling".into();
                if let Some(o) = t.obligations.iter_mut().find(|o| o.id == id) {
                    o.dispatch_state = "cancel_requested".into();
                }
                t.revision += 1;
                save(tx, &t)?;
                save_wake(tx, &wake)?;
                return Ok(Outcome::Wakes(vec![wake]));
            }
            wake.state = "cancelled".into();
            wake.next_due_ms = None;
            match prompt {
                Some(_) => {
                    if let Some(o) = t.obligations.iter_mut().find(|o| o.id == id) {
                        o.status = "cancelled".into();
                    }
                    t.revision += 1;
                    save(tx, &t)?;
                    // Invalidate the original wait through its normal source
                    // outcome. Otherwise a successor cancels the clock while
                    // leaving the owning task asleep on its dead registration.
                    cancel_settle(tx, &mut wake, crate::session::unix_epoch_ms())?;
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
            super::wake_receipts::alerted(tx, &id, sequence)?;
            if wake.last_sequence == Some(sequence) {
                wake.alerted_sequence = Some(sequence);
            }
            save_wake(tx, &wake)?;
            Ok(Outcome::Wakes(vec![wake]))
        }
        Operation::RetireWakes { room, agent, now } => {
            let removed = |r: &str, a: &str| r == room && agent.as_deref().is_none_or(|x| x == a);
            // The recipient is going away, so its tasks can never progress.
            for mut t in tasks(tx)? {
                if removed(&t.room_id, &t.agent_id) {
                    if matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled) {
                        match registrations(tx, &t.task_id) {
                            Ok(regs) => {
                                for mut reg in regs {
                                    if reg.live {
                                        reg.live = false;
                                        tx.execute(
                                            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                                            params![reg.id, encode(&reg)?],
                                        )
                                        .map_err(sql)?;
                                    }
                                }
                            }
                            Err(_) => {
                                quarantine::registrations(tx, &t.task_id)?;
                                tx.execute(
                                    "DELETE FROM agent_registrations WHERE task_id=?1",
                                    [&t.task_id],
                                )
                                .map_err(sql)?;
                            }
                        }
                        continue;
                    }
                    super::supervision::cancel_intent(tx, &mut t)?;
                    t.reason = "Room ended or agent removed".into();
                    t.revision += 1;
                    save(tx, &t)?;
                }
            }
            let mut retired = vec![];
            for mut wake in wakes_in(tx, ARMED)? {
                if !removed(&wake.room_id, &wake.agent_id) {
                    continue;
                }
                if wake.kind == "timer" {
                    cancel_settle(tx, &mut wake, now)?;
                } else if wake.state != "cancelling" {
                    wake.state = "cancelling".into();
                    save_wake(tx, &wake)?;
                }
                retired.push(wake);
            }
            // Settle unresolved deliveries, including in-flight attempts, before
            // the timeout sweep can create a Blocked task for a removed recipient.
            let mut q = tx.prepare("SELECT agent_id,sequence FROM agent_inbox WHERE room_id=?1 AND json_extract(payload,'$.state') IN ('pending','submitting','uncertain','blocked')").map_err(sql)?;
            let unsettled = q
                .query_map([&room], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            drop(q);
            for (a, seq) in unsettled.into_iter().filter(|(a, _)| removed(&room, a)) {
                let seq = u64::try_from(seq).map_err(|_| error("corrupt inbox sequence"))?;
                let mut e = get_event(tx, &room, &a, seq)?;
                e.state = "expired".into();
                save_event(tx, &e)?;
                record_delivery(tx, &e)?;
            }
            Ok(Outcome::Wakes(retired))
        }
        _ => Err(error("invalid wake operation")),
    }
}

/// Mirrors the inbox receipt of a wake's latest occurrence onto the wake.
pub(super) fn record_delivery(tx: &Transaction<'_>, e: &InboxEvent) -> Result<(), DaemonError> {
    let now = crate::session::unix_epoch_ms();
    super::wake_receipts::delivery(tx, e, now)?;
    let payload:Option<String>=tx.query_row("SELECT payload FROM agent_wakes WHERE room_id=?1 AND agent_id=?2 AND id=?3 AND json_extract(payload,'$.last_sequence')=?4",params![e.room_id,e.agent_id,e.source_id,sql_integer(e.sequence)?],|r|r.get(0)).optional().map_err(sql)?;
    let Some(payload) = payload else {
        return Ok(());
    };
    let mut wake: AgentWake = decode(&payload)?;
    wake.last_delivery = Some(e.state.clone());
    if matches!(e.state.as_str(), "accepted" | "acknowledged" | "handled") {
        wake.last_delivered_at_ms.get_or_insert(now);
    }
    if matches!(e.state.as_str(), "acknowledged" | "handled") {
        wake.last_acknowledged_at_ms.get_or_insert(now);
    }
    save_wake(tx, &wake)
}
