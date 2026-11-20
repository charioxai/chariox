//! MP-08 / MP-09 / MP-10 / MP-11 A02: source outcomes, sweeps and owner disposition.
use super::*;
/// A turn that ended with a watched process, job, terminal watch or delegate still open left it
/// supervised (see `AgentObligation::outlives_turn`); its end closes that work. The agent hears of
/// a process as an event of its own, marked `after_turn`: the task that started it is done. A
/// delegate's end reaches its parent as `child_finished`.
fn close_outlived(
    tx: &Transaction<'_>,
    t: &mut AgentTaskExecution,
    held: Vec<Registration>,
    source: &str,
    occurrence: &str,
    success: bool,
    public_answer: &Option<serde_json::Value>,
) -> Result<(), DaemonError> {
    let mut told_elsewhere = true;
    let mut closed = false;
    for o in &mut t.obligations {
        if o.completion_source() == Some(source)
            && o.outlives_turn()
            && !(success
                && o.kind == "delegate"
                && o.dispatch_state == "accepted"
                && o.completion_task_id.is_none())
        {
            o.status = "satisfied".into();
            told_elsewhere &= o.kind == "delegate" || o.dispatch_state == "cancel_requested";
            closed = true;
        }
    }
    if !closed {
        return Ok(());
    }
    for mut reg in held {
        if reg.source_id == source && reg.live {
            reg.live = false;
            tx.execute(
                "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                params![reg.id, encode(&reg)?],
            )
            .map_err(sql)?;
        }
    }
    if !told_elsewhere {
        let mut told =
            super::transitions::source_end(t, source, occurrence, success, public_answer, false);
        told.payload["after_turn"] = true.into();
        event(tx, told)?;
    }
    t.revision += 1;
    save(tx, t)
}

pub(super) fn apply(tx: &Transaction<'_>, op: Operation) -> Result<Outcome, DaemonError> {
    match op {
        Operation::Progress {
            task,
            prompt,
            receipt,
            now,
        } => {
            let mut t = load(tx, &task)?;
            current(&t, &prompt)?;
            let inserted=tx.execute("INSERT INTO agent_progress_receipts VALUES(?1,?2,?3) ON CONFLICT(task_id,receipt_id) DO NOTHING",params![task,receipt,sql_integer(now)?]).map_err(sql)?;
            if inserted != 0 {
                t.last_progress_at_ms = now;
                t.revision += 1;
                save(tx, &t)?;
            }
            Ok(Outcome::Task(t))
        }
        Operation::DelegateUnavailable {
            room,
            agent,
            reason,
            now,
        } => {
            replies::recipient_lost(tx, &room, &agent, now, Some(&reason.answer(&agent)))?;
            Ok(Outcome::Saved)
        }
        Operation::ReleaseSpawn { room, source } => {
            // An agent that is gone follows nobody.
            for t in room_tasks(tx, &room, Some(&source))? {
                for mut reg in registrations(tx, &t.task_id)? {
                    if reg.peer && reg.live {
                        reg.live = false;
                        tx.execute(
                            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                            params![reg.id, encode(&reg)?],
                        )
                        .map_err(sql)?;
                    }
                }
            }
            for mut t in source_tasks(tx, &room, &source)? {
                let mut released = Vec::new();
                for o in &mut t.obligations {
                    if o.kind == "delegate"
                        && o.status == "open"
                        && o.completion_task_id.is_none()
                        && o.resource_id.as_deref() == Some(source.as_str())
                    {
                        o.status = "cancelled".into();
                        o.dispatch_state = "cancelled".into();
                        released.push(o.id.clone());
                    }
                }
                if released.is_empty() {
                    continue;
                }
                for mut reg in registrations(tx, &t.task_id)? {
                    if reg.live
                        && reg
                            .obligation_id
                            .as_ref()
                            .is_some_and(|id| released.contains(id))
                    {
                        reg.live = false;
                        tx.execute(
                            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                            params![reg.id, encode(&reg)?],
                        )
                        .map_err(sql)?;
                        // A task parked on the registration wakes to see it ended.
                        if t.wait
                            .as_ref()
                            .is_some_and(|w| w.registration_ids.contains(&reg.id))
                        {
                            event(
                                tx,
                                occurrence(
                                    &room,
                                    &t.agent_id,
                                    &source,
                                    &format!("spawn-released-{}-{}", reg.id, t.revision),
                                    "source_lost",
                                    serde_json::json!({"task_id":t.task_id,"registration_id":reg.id}),
                                ),
                            )?;
                        }
                    }
                }
                t.revision += 1;
                save(tx, &t)?;
            }
            Ok(Outcome::Saved)
        }
        Operation::WorkerResultReturned { room, parent, task } => {
            tx.execute(
                "INSERT INTO agent_source_occurrences(room_id,source_id,occurrence_id,success)
                 VALUES(?1,?2,?3,1) ON CONFLICT(room_id,source_id,occurrence_id) DO NOTHING",
                params![room, format!("worker-result-returned:{task}"), parent],
            )
            .map_err(sql)?;
            Ok(Outcome::Saved)
        }
        Operation::SourceOutcome {
            mut public_answer,
            room,
            source,
            occurrence: id,
            success,
            now,
        } => {
            // Source completion does not advance the task's separate progress clock.
            let _completion_observed_at_ms = now;
            if let Some(answer) = public_answer.as_mut() {
                crate::secret_redaction::redact_json_secrets(answer);
                let job: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_wakes WHERE id=?1 AND room_id=?2 AND json_extract(payload,'$.kind')='job')", params![source, room], |row| row.get(0)).map_err(sql)?;
                if encode(answer)?.len() > if job { 256 * 1024 } else { 8_192 } {
                    return Err(error("public answer excerpt limit"));
                }
            }
            tx.execute("INSERT INTO agent_source_occurrences(room_id,source_id,occurrence_id,success,public_answer) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(room_id,source_id,occurrence_id) DO NOTHING",params![room,source,id,success,public_answer.as_ref().map(encode).transpose()?]).map_err(sql)?;
            let stored:(bool,Option<String>)=tx.query_row("SELECT success,public_answer FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2 AND occurrence_id=?3",params![room,source,id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
            if stored.0 != success {
                return Err(error("conflicting terminal source occurrence"));
            }
            if id == format!("task-terminal-{source}") {
                tx.execute(
                    "UPDATE agent_tasks SET terminal_outcome_recorded=1 WHERE room_id=?1 AND task_id=?2",
                    params![room, source],
                ).map_err(sql)?;
            }
            public_answer = stored
                .1
                .map(|v| decode::<serde_json::Value>(&v))
                .transpose()?;
            for mut t in source_tasks(tx, &room, &source)? {
                let held = registrations(tx, &t.task_id);
                // A finished task is done with its sources, unless it still follows this peer or
                // ended while this source was running.
                if t.state == ExecutionState::Done
                    && !held.as_ref().is_ok_and(|held| {
                        held.iter()
                            .any(|reg| reg.peer && reg.live && reg.source_id == source)
                    })
                {
                    if let Ok(held) = held {
                        close_outlived(tx, &mut t, held, &source, &id, success, &public_answer)?;
                    }
                    continue;
                }
                let registrations = match held {
                    Ok(regs) => regs,
                    Err(_) => {
                        quarantine::registrations(tx, &t.task_id)?;
                        t.state = ExecutionState::Blocked;
                        t.blocked_revision = t.revision + 1;
                        t.reason =
                            "Source registration is corrupt; restore the exact receipt or cancel"
                                .into();
                        t.revision += 1;
                        save(tx, &t)?;
                        continue;
                    }
                };
                // The owner was asked only because the turn ended with this work still open; its
                // success answers that, and no model is left to acknowledge it.
                let turn_end_block = success
                    && t.state == ExecutionState::Blocked
                    && t.reason == super::transitions::INVALID_TURN_END_REASON
                    && t.goal.is_none()
                    && t.wait.is_none();
                let mut changed = false;
                for o in &mut t.obligations {
                    if o.completion_source() == Some(&source)
                        && o.status == "open"
                        && !(success
                            && o.kind == "delegate"
                            && o.dispatch_state == "accepted"
                            && o.completion_task_id.is_none())
                    {
                        o.status = if t.state == ExecutionState::Cancelled {
                            "cancelled"
                        } else if turn_end_block {
                            "satisfied"
                        } else if success {
                            "settling"
                        } else {
                            "failed"
                        }
                        .into();
                        changed = true;
                    }
                }
                let obligations_changed = changed;
                let mut peers_ended = false;
                for mut reg in registrations {
                    if reg.source_id == source
                        && reg.live
                        && !(success && super::delegation::unbound(&t, &reg))
                    {
                        peers_ended |= reg.peer;
                        if !super::peer_follows::outlives(tx, &reg, &id)? {
                            reg.live = false;
                            tx.execute(
                                "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                                params![reg.id, encode(&reg)?],
                            )
                            .map_err(sql)?;
                        }
                        changed = true;
                    }
                }
                if turn_end_block
                    && !t
                        .obligations
                        .iter()
                        .any(AgentObligation::requires_resolution)
                {
                    t.state = ExecutionState::Done;
                    t.reason.clear();
                }
                if changed {
                    let e = super::transitions::source_end(
                        &t,
                        &source,
                        &id,
                        success,
                        &public_answer,
                        peers_ended && !obligations_changed,
                    );
                    let told = event(tx, e)?;
                    // The child's end was already handled through its child_finished, so its
                    // receipt arrives handled: it settles what it reports as a handled ack does.
                    if told.state == "handled" && folded_into_handled_child_end(tx, &told)? {
                        for o in &mut t.obligations {
                            if o.completion_source() == Some(&source)
                                && matches!(o.status.as_str(), "failed" | "settling")
                            {
                                o.status = "satisfied".into();
                            }
                        }
                    }
                    t.revision += 1;
                    save(tx, &t)?;
                }
            }
            super::peer_follows::turn_ended(tx, &room, &source, &id, success, now)?;
            replies::forward_to_sender_room(
                tx,
                &room,
                &source,
                &id,
                success,
                public_answer.clone(),
                now,
            )?;
            replies::reconcile_source(tx, &room, &source)?;
            if !success {
                replies::recipient_lost(tx, &room, &source, now, public_answer.as_ref())?;
            }
            Ok(Outcome::Saved)
        }
        Operation::Sweep {
            now,
            busy_recipients,
            vault_wait_recipients,
            held_work,
        } => {
            let mut changed = vec![];
            for mut t in waiting_tasks(tx)? {
                if t.state == ExecutionState::Waiting {
                    let regs = match registrations(tx, &t.task_id) {
                        Ok(regs) => regs,
                        Err(_) => {
                            quarantine::registrations(tx, &t.task_id)?;
                            t.state = ExecutionState::Blocked;
                            t.blocked_revision = t.revision + 1;
                            t.reason="Source registration is corrupt; owner must restore the exact source receipt".into();
                            t.revision += 1;
                            save(tx, &t)?;
                            changed.push(t);
                            continue;
                        }
                    };
                    if super::settled_waits::finish(tx, &mut t)? {
                        t.revision += 1;
                        save(tx, &t)?;
                        changed.push(t);
                        continue;
                    }
                    let next_check_in = t.goal.as_ref().and_then(|goal| goal.next_check_in_ms);
                    let source_deadline = if let Some(wait) = t.wait.as_ref() {
                        wakes::source_deadline(tx, &t, &wait.registration_ids)?
                    } else {
                        None
                    };
                    let settled = super::settled_waits::is_settled(tx, &t)?;
                    let unread = match t.wait.as_ref() {
                        Some(w) => super::settled_waits::unread_outcome(tx, &t, w)?,
                        None => false,
                    };
                    if let Some(w) = t.wait.as_mut() {
                        let clock_rollback = now < w.last_checked_at_ms;
                        w.last_checked_at_ms = w.last_checked_at_ms.max(now);
                        let dead = w
                            .registration_ids
                            .iter()
                            .any(|id| !regs.iter().any(|r| &r.id == id && r.live));
                        let check_in = next_check_in.filter(|due| {
                            *due <= now
                                && source_deadline.is_none_or(|deadline| deadline > now)
                                && !dead
                                && now >= w.started_at_ms
                                && !clock_rollback
                        });
                        // An unread outcome is the wake when its source already ended.
                        let needs_wake = if dead {
                            !unread
                        } else {
                            w.deadline_ms.is_some_and(|deadline| deadline <= now)
                                || now < w.started_at_ms
                                || clock_rollback
                                || check_in.is_some()
                        };
                        // A settled wait held back by the agent's other work has nothing to re-check.
                        if !settled && needs_wake {
                            let kind = if dead {
                                "wait_recheck"
                            } else {
                                "deadline_reached"
                            };
                            let occurrence_id = check_in.map_or_else(
                                || format!("wait-{}-{kind}", w.started_at_ms),
                                |due| format!("{}{due}", super::goals::CHECK_IN_OCCURRENCE_PREFIX),
                            );
                            let e = occurrence(
                                &t.room_id,
                                &t.agent_id,
                                &t.task_id,
                                &occurrence_id,
                                kind,
                                serde_json::json!({"task_id":t.task_id,"reason":"re-evaluate"}),
                            );
                            event(tx, e)?;
                        }
                        save(tx, &t)?;
                    }
                }
            }
            // Outside recipients (room '') are pulled by their holders, never delivered here.
            let mut q=tx.prepare("SELECT sequence,room_id,agent_id,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id>'' AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state') NOT IN ('accepted','acknowledged','handled','expired','failed') ELSE 1 END ORDER BY sequence").map_err(sql)?;
            let rows = q
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                })
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            drop(q);
            let mut events = vec![];
            for (sequence, room, agent, source, id, payload) in rows {
                match decode_inbox(sequence, &room, &agent, &source, &id, &payload) {
                    Ok(e) => events.push(e),
                    Err(_) => {
                        quarantine::retain(tx, "delivery", &sequence.to_string(), &payload)?;
                        let mut e = occurrence(
                            &room,
                            &agent,
                            &source,
                            &id,
                            "delivery_corrupt",
                            serde_json::json!({"diagnostic":"receipt quarantined; owner reconciliation required"}),
                        );
                        e.sequence =
                            u64::try_from(sequence).map_err(|_| error("corrupt inbox sequence"))?;
                        e.state = "blocked".into();
                        save_event(tx, &e)?;
                        let mut t = quarantine::task(
                            format!("delivery-{sequence}"),
                            room,
                            agent,
                            format!("delivery-{sequence}"),
                        );
                        t.reason="Delivery receipt is quarantined; never replay it without exact reconciliation".into();
                        save(tx, &t)?;
                        changed.push(t);
                    }
                }
            }
            for mut e in events {
                if e.state == "pending" && e.kind == "task_cancelled" {
                    continue;
                }
                if e.state == "submitting"
                    && e.submit_epoch.is_none()
                    && e.provider_run_id.is_none()
                    && e.target_prompt_id.is_none()
                    && vault_wait_recipients
                        .iter()
                        .any(|(room, agent)| room == &e.room_id && agent == &e.agent_id)
                {
                    // Waiting for owner input before I/O does not consume a provider receipt window.
                    e.attempted_at_ms = Some(now);
                    save_event(tx, &e)?;
                    continue;
                }
                let mut owner_held = false;
                if refused_wake(&e) {
                    for task in live_tasks(tx, &e.room_id, &e.agent_id)? {
                        if task.state == ExecutionState::Blocked
                            && task.wait.is_some()
                            && delivery_belongs_to_task(tx, &e, &task)?
                        {
                            owner_held = true;
                            break;
                        }
                    }
                }
                if owner_held {
                    continue;
                }
                let mut clock_quarantined = false;
                let deadline_start = if e.state == "pending" {
                    // Busy refusals are normal queueing, not missing receipts.
                    // The next idle refusal starts a new bounded window.
                    if busy_recipients
                        .iter()
                        .any(|(room, agent)| room == &e.room_id && agent == &e.agent_id)
                        || held_work.iter().any(|(room, agent, work)| {
                            room == &e.room_id
                                && agent == &e.agent_id
                                && !super::work_correlated(&e, work)
                        })
                    {
                        tx.execute(
                            "DELETE FROM agent_inbox_refusals WHERE sequence=?1",
                            [sql_integer(e.sequence)?],
                        )
                        .map_err(sql)?;
                        continue;
                    }
                    let at = tx.query_row(
                        "SELECT first_refused_at_ms, typeof(first_refused_at_ms) || ':' || hex(CAST(first_refused_at_ms AS BLOB)) FROM agent_inbox_refusals WHERE sequence=?1",
                        [sql_integer(e.sequence)?],
                        |r| Ok((r.get::<_, i64>(0), r.get::<_, String>(1)?)),
                    ).optional().map_err(sql)?;
                    match at {
                        None => None,
                        Some((Ok(at), _)) if at >= 0 => Some(at as u64),
                        Some((_, raw)) => {
                            // Preserve type and bytes, even for unreadable text.
                            // Only this event enters the existing owner block.
                            quarantine::retain(tx, "refusal-clock", &e.sequence.to_string(), &raw)?;
                            clock_quarantined = true;
                            None
                        }
                    }
                } else {
                    e.attempted_at_ms
                };
                if e.state == "pending"
                    && (clock_quarantined
                        || deadline_start.is_some_and(|at| {
                            // Receipt time can advance during Sweep's awaited pre-work.
                            // Tolerate that race (and small clock rollbacks), but
                            // still block clocks beyond the delivery window.
                            at > now.saturating_add(DELIVERY_TIMEOUT_MS)
                                || now.saturating_sub(at) >= DELIVERY_TIMEOUT_MS
                        }))
                {
                    // A delivery with a provider attempt but no receipt is settled by the
                    // kernel before this sweep (`settle_interrupted_deliveries`). What is
                    // left refused before any provider I/O: nothing is unconfirmed, so it
                    // ends as failed and cannot fence the recipient's later wakes.
                    if !clock_quarantined {
                        super::delivery_failure::fail_undelivered(
                            tx,
                            &mut e,
                            DelegateGoneReason::FailedStart,
                            now,
                        )?;
                        continue;
                    }
                    let reason = format!(
                        "Delivery {} refusal clock is quarantined; owner must restore authoritative state or cancel",
                        e.sequence
                    );
                    e.state = "blocked".into();
                    save_event(tx, &e)?;
                    // Only the task this delivery belongs to waits on its
                    // reconciliation; unrelated work of the recipient continues.
                    let mut matched = false;
                    for mut t in live_tasks(tx, &e.room_id, &e.agent_id)? {
                        if t.room_id == e.room_id
                            && t.agent_id == e.agent_id
                            && !matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
                            && delivery_belongs_to_task(tx, &e, &t)?
                        {
                            matched = true;
                            if t.state == ExecutionState::Blocked {
                                continue;
                            }
                            t.state = ExecutionState::Blocked;
                            t.blocked_revision = t.revision + 1;
                            t.reason = reason.clone();
                            t.revision += 1;
                            save(tx, &t)?;
                            record_block_cause(tx, &t, true)?;
                            changed.push(t);
                        }
                    }
                    if !matched {
                        let id = format!("delivery-{}", e.sequence);
                        let mut task = new_task(e.room_id, e.agent_id, id, e.provider_run_id, now);
                        task.state = ExecutionState::Blocked;
                        task.blocked_revision = task.revision;
                        task.reason = reason;
                        save(tx, &task)?;
                        record_block_cause(tx, &task, true)?;
                        changed.push(task);
                    }
                }
            }
            Ok(Outcome::Swept(changed))
        }
        Operation::CancelTask {
            task,
            owner,
            revision,
        } => cancel_task(tx, &task, &owner, revision, false),
        Operation::CancelTaskByAgent {
            task,
            owner,
            revision,
        } => cancel_task(tx, &task, &owner, revision, true),
        Operation::NoteTurnCancel { task, initiator } => {
            let mut t = load(tx, &task)?;
            let noted = match initiator {
                CancelInitiator::Owner => OWNER_CANCELLED_REASON,
                CancelInitiator::Agent if t.reason == OWNER_CANCELLED_REASON => {
                    OWNER_CANCELLED_REASON
                }
                CancelInitiator::Agent => AGENT_CANCELLED_REASON,
            };
            if !matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
                && t.reason != noted
            {
                t.reason = noted.into();
                save(tx, &t)?;
            }
            Ok(Outcome::Task(t))
        }
        Operation::RetireRestartFailure {
            task,
            revision,
            later_prompt,
            now,
        } => {
            let mut t = load(tx, &task)?;
            // Admission must not abandon a goal, owned work, or an uncertain receipt. The rows of
            // the later turn itself are its own delivery, not work left behind.
            let unresolved: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.prompt_id') IS NOT ?3 AND (json_extract(payload,'$.state') IN ('blocked','submitting','uncertain') OR (json_extract(payload,'$.state')='pending' AND json_extract(payload,'$.kind') IS NOT 'task_cancelled')) ELSE 1 END)",
                params![t.room_id, t.agent_id, later_prompt], |r| r.get(0),
            ).map_err(sql)?;
            // A Blocked task is not waiting: its pending prompt and wait are what its ended
            // attempt left behind, and a wait on a live source shows as a live registration.
            // Only an interrupted attempt is superseded: a block the agent raised for its owner
            // (a question, a decision) stays until the owner answers it.
            if t.state != ExecutionState::Blocked
                || t.blocked_revision != revision
                || !is_interrupted_block(tx, &t)?
                || t.owns_unfinished_work()
                || unresolved
                || registrations(tx, &t.task_id)?.iter().any(|r| r.live)
            {
                return Ok(Outcome::Task(t));
            }
            // An interrupted wake turn keeps the wait and wake prompt it was dispatched with;
            // with its source settled and handled nothing can consume them.
            t.wait = None;
            t.pending_prompt_id = None;
            t.state = ExecutionState::Cancelled;
            t.reason = "Superseded by a later turn of the same agent; it holds no open work".into();
            t.pending_prompt_id = None;
            t.wait = None;
            t.last_progress_at_ms = now;
            t.revision += 1;
            save(tx, &t)?;
            Ok(Outcome::Task(t))
        }
        Operation::OwnerResponse {
            task,
            revision,
            resume,
            now,
        } => {
            let mut t = load(tx, &task)?;
            if t.state != ExecutionState::Blocked || t.blocked_revision != revision {
                return Err(error("owner response is stale"));
            }
            let quarantined:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_lifecycle_quarantine WHERE kind='task' AND id=?1)",[&task],|r|r.get(0)).map_err(sql)?;
            if resume && quarantined {
                return Err(error(
                    "quarantined obligation coverage must be restored before resume; explicit cancellation remains available",
                ));
            }
            if resume {
                registrations(tx, &t.task_id)?;
            }
            let mut q = tx.prepare("SELECT sequence FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state') IN ('blocked','submitting','uncertain')").map_err(sql)?;
            let deliveries = q
                .query_map(params![t.room_id, t.agent_id], |r| r.get::<_, i64>(0))
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            drop(q);
            // Only this task's own unsettled deliveries hold its Resume: another turn's message
            // on its way in keeps its place in the agent's queue either way.
            for sequence in deliveries {
                let mut event = get_event(
                    tx,
                    &t.room_id,
                    &t.agent_id,
                    u64::try_from(sequence).map_err(|_| error("corrupt delivery sequence"))?,
                )?;
                if !delivery_belongs_to_task(tx, &event, &t)? {
                    continue;
                }
                if resume {
                    return Err(error(format!(
                        "unconfirmed delivery of inbox event {} to task {} must receive its exact provider receipt or be explicitly cancelled before resume",
                        event.sequence, t.task_id
                    )));
                }
                event.state = "failed".into();
                save_event(tx, &event)?;
            }
            let retry_wake = resume && t.wait.is_some() && pending_refused_wake(tx, &t)?;
            let blocked_revision = t.blocked_revision;
            super::goals::clock(&mut t, now);
            t.state = if retry_wake {
                ExecutionState::Waiting
            } else if resume {
                ExecutionState::Working
            } else {
                ExecutionState::Cancelled
            };
            if !resume {
                cancel_intent(tx, &mut t, false)?;
            }
            t.owner_action = None;
            if !retry_wake {
                t.wait = None;
            }
            t.revision += 1;
            if resume {
                if let Some(g) = &mut t.goal {
                    g.paused = false;
                    g.continuation_sequence += 1;
                    g.next_check_in_ms = None;
                }
                t.pending_prompt_id = if retry_wake {
                    None
                } else {
                    Some(format!(
                        "{}{}-{}",
                        super::goals::OWNER_RESUME_PROMPT_PREFIX,
                        t.task_id,
                        t.revision
                    ))
                };
                t.last_progress_at_ms = now;
                if let Some(goal) = &t.goal {
                    audit::audit(
                        tx,
                        "goal.owner_resume",
                        &t.task_id,
                        serde_json::json!({
                            "task_id":t.task_id, "goal_id":goal.id,
                            "objective_revision":goal.objective_revision,
                            "prompt_id":t.pending_prompt_id, "action":"resume",
                            "blocked_revision":blocked_revision, "resumed_revision":t.revision,
                            "at_ms":now,
                        }),
                    )?;
                }
            }
            save(tx, &t)?;
            Ok(Outcome::Task(t))
        }
        _ => Err(error("invalid supervision operation")),
    }
}
/// Shown to the person on the Stopped stripe; the supervised cancellation state itself stays in the task's obligations.
pub(super) const OWNER_CANCELLED_REASON: &str = "Stopped by you";
/// An agent's cancel ends the turn; the agent stays reachable, so this is not `stopped_by_owner`.
pub(super) const AGENT_CANCELLED_REASON: &str = "Stopped by an agent";
fn cancel_task(
    tx: &Transaction<'_>,
    task: &str,
    owner: &str,
    revision: u64,
    by_agent: bool,
) -> Result<Outcome, DaemonError> {
    let mut t = load(tx, task)?;
    if t.owner_user_id.is_empty() || t.owner_user_id != owner || t.revision != revision {
        return Err(error(
            "stale or foreign owner cancellation; reload the task in its owning room and retry",
        ));
    }
    match t.state {
        // A failure can have cancelled it already and left its diagnostic as the reason. An
        // accepted Stop is still a decision to stop: record it, or nothing that goes on with a
        // failed turn (a usage-limit resume) can tell the cancel from the failure.
        ExecutionState::Cancelled => {
            if !t.cancelled_on_purpose() {
                t.reason = if by_agent {
                    AGENT_CANCELLED_REASON
                } else {
                    OWNER_CANCELLED_REASON
                }
                .into();
                t.revision += 1;
                save(tx, &t)?;
            }
        }
        // The turn is over and stays so: Stop only ends the work it left running. The owner's Stop
        // still keeps the agent stopped, as for any turn it ends.
        ExecutionState::Done => {
            if cancel_retained_work(&mut t) {
                if !by_agent {
                    t.reason = OWNER_CANCELLED_REASON.into();
                    keep_agent_stopped(tx, &t)?;
                }
                t.revision += 1;
                save(tx, &t)?;
            }
        }
        _ => {
            cancel_intent(tx, &mut t, by_agent)?;
            t.revision += 1;
            save(tx, &t)?;
        }
    }
    Ok(Outcome::Task(t))
}
/// Message delivery reads the stop from the agent's newest task, so an owner's Stop of an older
/// task that retains work marks the newest finished one too; the owner's next prompt starts a
/// newer task and ends the stop.
fn keep_agent_stopped(
    tx: &Transaction<'_>,
    stopped: &AgentTaskExecution,
) -> Result<(), DaemonError> {
    let Some(mut newest) = super::tasks_where(
        tx,
        "room_id=?1 AND agent_id=?2",
        params![stopped.room_id, stopped.agent_id],
    )?
    .pop() else {
        return Ok(());
    };
    if newest.task_id != stopped.task_id
        && matches!(
            newest.state,
            ExecutionState::Done | ExecutionState::Cancelled
        )
        && newest.reason != OWNER_CANCELLED_REASON
    {
        newest.reason = OWNER_CANCELLED_REASON.into();
        newest.revision += 1;
        save(tx, &newest)?;
    }
    Ok(())
}
/// Asks for the end of the work a Done task retains; its end closes the obligation without
/// waking the agent, whom Stop told to stand down.
fn cancel_retained_work(task: &mut AgentTaskExecution) -> bool {
    let mut any = false;
    for obligation in task.obligations.iter_mut().filter(|o| o.outlives_turn()) {
        obligation.dispatch_state = "cancel_requested".into();
        any = true;
    }
    any
}
/// `by_agent`: an agent, not the owner, cancels the task; it keeps the agent reachable.
pub(super) fn cancel_intent(
    tx: &Transaction<'_>,
    task: &mut AgentTaskExecution,
    by_agent: bool,
) -> Result<(), DaemonError> {
    super::goals::clock(task, crate::session::unix_epoch_ms());
    if let Some(g) = &mut task.goal {
        g.next_check_in_ms = None;
    }
    task.state = ExecutionState::Cancelled;
    task.wait = None;
    task.reason = if by_agent {
        AGENT_CANCELLED_REASON
    } else {
        OWNER_CANCELLED_REASON
    }
    .into();
    for obligation in &mut task.obligations {
        if matches!(obligation.status.as_str(), "failed" | "settling")
            || matches!(obligation.kind.as_str(), "reply" | "message")
        {
            obligation.status = "cancelled".into();
        } else if matches!(obligation.status.as_str(), "open" | "settling") {
            obligation.dispatch_state = "cancel_requested".into();
        }
    }
    let regs = match registrations(tx, &task.task_id) {
        Ok(regs) => regs,
        Err(_) => {
            quarantine::registrations(tx, &task.task_id)?;
            // Explicit cancellation invalidates subscriptions, while the task's
            // resource obligations remain supervised until physical settlement.
            tx.execute(
                "DELETE FROM agent_registrations WHERE task_id=?1",
                [&task.task_id],
            )
            .map_err(sql)?;
            Vec::new()
        }
    };
    for mut registration in regs {
        // Cancelling the turn does not stop the agent following a peer.
        if registration.peer {
            continue;
        }
        registration.live = false;
        tx.execute(
            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
            params![registration.id, encode(&registration)?],
        )
        .map_err(sql)?;
    }
    let notice = occurrence(
        &task.room_id,
        &task.agent_id,
        &task.task_id,
        "owner-cancelled",
        "task_cancelled",
        serde_json::json!({
            "cancelled_task_id": task.task_id,
            "reason": task.reason,
            "message": if by_agent {
                "Another agent cancelled this task and its retained work. Keep it cancelled; a message from an agent is new work."
            } else {
                "The owner cancelled this task and its retained work. Keep it cancelled unless the owner starts new work."
            },
        }),
    );
    // A cancellation is context for the next turn, never a new turn that
    // revives stopped work. It remains in the inbox until the agent handles it.
    event(tx, notice)?;
    Ok(())
}

// An uncertain receipt blocks recipient dispatch, but abandoning it is an
// exact task disposition. Cancelling unrelated work cannot release this fence.
fn delivery_belongs_to_task(
    tx: &Transaction<'_>,
    event: &InboxEvent,
    task: &AgentTaskExecution,
) -> Result<bool, DaemonError> {
    if task.task_id == format!("delivery-{}", event.sequence)
        || event
            .payload
            .get("task_id")
            .and_then(serde_json::Value::as_str)
            == Some(task.task_id.as_str())
        || event
            .payload
            .get("task_ids")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|ids| {
                ids.iter()
                    .any(|id| id.as_str() == Some(task.task_id.as_str()))
            })
    {
        return Ok(true);
    }
    for prompt in [
        event.target_prompt_id.as_deref(),
        event.prompt_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if for_turn(tx, &event.room_id, &event.agent_id, prompt)?
            .is_some_and(|bound| bound.task_id == task.task_id)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn refused_wake(event: &InboxEvent) -> bool {
    event.state == "pending"
        && event.kind != "message"
        && event.attempted_at_ms.is_some()
        && event.submit_epoch.is_none()
}

fn pending_refused_wake(
    tx: &Transaction<'_>,
    task: &AgentTaskExecution,
) -> Result<bool, DaemonError> {
    let mut query = tx.prepare("SELECT sequence FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state')='pending'").map_err(sql)?;
    let sequences = query
        .query_map(params![task.room_id, task.agent_id], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    for sequence in sequences {
        let event = get_event(
            tx,
            &task.room_id,
            &task.agent_id,
            u64::try_from(sequence).map_err(|_| error("corrupt inbox sequence"))?,
        )?;
        if refused_wake(&event) && delivery_belongs_to_task(tx, &event, task)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Records why `t` is Blocked at its current `blocked_revision`: an interrupted attempt the kernel
/// blocked (a later turn may supersede it), or a block for the owner (never superseded).
pub(super) fn record_block_cause(
    tx: &Transaction<'_>,
    t: &AgentTaskExecution,
    interrupted: bool,
) -> Result<(), DaemonError> {
    if interrupted && t.state == ExecutionState::Blocked {
        tx.execute(
            "INSERT INTO agent_interrupted_blocks(task_id,blocked_revision) VALUES(?1,?2) ON CONFLICT(task_id) DO UPDATE SET blocked_revision=excluded.blocked_revision",
            params![t.task_id, sql_integer(t.blocked_revision)?],
        )
    } else {
        tx.execute(
            "DELETE FROM agent_interrupted_blocks WHERE task_id=?1",
            params![t.task_id],
        )
    }
    .map_err(sql)?;
    Ok(())
}

/// Whether `t`'s current block is an interrupted attempt the kernel recorded.
pub(super) fn is_interrupted_block(
    tx: &Transaction<'_>,
    t: &AgentTaskExecution,
) -> Result<bool, DaemonError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_interrupted_blocks WHERE task_id=?1 AND blocked_revision=?2)",
        params![t.task_id, sql_integer(t.blocked_revision)?],
        |r| r.get(0),
    )
    .map_err(sql)
}
