//! MP-08 / MP-09 / MP-10 / MP-11 A02: source outcomes, sweeps and owner disposition.
use super::*;
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
                t.no_progress_wakes = 0;
                t.last_progress_at_ms = now;
                t.progress_sequence += 1;
                t.revision += 1;
                save(tx, &t)?;
            }
            Ok(Outcome::Task(t))
        }
        Operation::SourceOutcome {
            mut public_answer,
            room,
            source,
            occurrence: id,
            success,
            now: _,
        } => {
            if let Some(answer) = public_answer.as_mut() {
                crate::secret_redaction::redact_json_secrets(answer);
                if encode(answer)?.len() > 8_192 {
                    return Err(error("public answer excerpt limit"));
                }
            }
            tx.execute("INSERT INTO agent_source_occurrences(room_id,source_id,occurrence_id,success,public_answer) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(room_id,source_id,occurrence_id) DO NOTHING",params![room,source,id,success,public_answer.as_ref().map(encode).transpose()?]).map_err(sql)?;
            let stored:(bool,Option<String>)=tx.query_row("SELECT success,public_answer FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2 AND occurrence_id=?3",params![room,source,id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
            if stored.0 != success {
                return Err(error("conflicting terminal source occurrence"));
            }
            public_answer = stored
                .1
                .map(|v| decode::<serde_json::Value>(&v))
                .transpose()?;
            for mut t in tasks(tx)? {
                if t.room_id != room {
                    continue;
                }
                let registrations = match registrations(tx, &t.task_id) {
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
                let mut changed = false;
                for o in &mut t.obligations {
                    if o.completion_source() == Some(&source) && o.status == "open" {
                        o.status = if t.state == ExecutionState::Cancelled {
                            "cancelled"
                        } else if success {
                            "settling"
                        } else {
                            "failed"
                        }
                        .into();
                        changed = true;
                    }
                }
                for mut reg in registrations {
                    if reg.source_id == source && reg.live {
                        reg.live = false;
                        tx.execute(
                            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                            params![reg.id, encode(&reg)?],
                        )
                        .map_err(sql)?;
                        changed = true;
                    }
                }
                if changed {
                    let e = occurrence(
                        &room,
                        &t.agent_id,
                        &source,
                        &id,
                        if success {
                            "source_completed"
                        } else {
                            "source_lost"
                        },
                        serde_json::json!({"task_id":t.task_id,"source_id":source,"public_history_ref":source,"occurrence_id":id,"success":success,"public_answer":public_answer}),
                    );
                    event(tx, e)?;
                    t.revision += 1;
                    save(tx, &t)?;
                }
            }
            Ok(Outcome::Saved)
        }
        Operation::Sweep { now } => {
            let mut changed = vec![];
            for mut t in tasks(tx)? {
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
                    if let Some(w) = t.wait.as_mut() {
                        if !w.long_wait_notified
                            && now.saturating_sub(t.last_progress_at_ms) >= LONG_WAIT_MS
                        {
                            w.long_wait_notified = true;
                            t.revision += 1;
                            changed.push(t.clone());
                        }
                        let w = t.wait.as_mut().unwrap();
                        let clock_rollback = now < w.last_checked_at_ms;
                        w.last_checked_at_ms = w.last_checked_at_ms.max(now);
                        let dead = w
                            .registration_ids
                            .iter()
                            .any(|id| !regs.iter().any(|r| &r.id == id && r.live));
                        let unread:i64=tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence>?3 AND json_extract(payload,'$.state') NOT IN ('handled','expired','failed') AND json_extract(payload,'$.kind') IN ('source_completed','source_lost') AND (json_extract(payload,'$.payload.task_id')=?4 OR EXISTS(SELECT 1 FROM json_each(json_extract(payload,'$.payload.task_ids')) WHERE value=?4))",params![t.room_id,t.agent_id,sql_integer(w.inbox_cursor)?,t.task_id],|r|r.get(0)).map_err(sql)?;
                        if (dead && unread == 0)
                            || w.deadline_ms <= now
                            || now < w.started_at_ms
                            || clock_rollback
                        {
                            let kind = if dead {
                                "wait_recheck"
                            } else {
                                "deadline_reached"
                            };
                            let e = occurrence(
                                &t.room_id,
                                &t.agent_id,
                                &t.task_id,
                                &format!("wait-{}-{kind}", w.started_at_ms),
                                kind,
                                serde_json::json!({"task_id":t.task_id,"reason":"re-evaluate"}),
                            );
                            event(tx, e)?;
                        }
                        save(tx, &t)?;
                    }
                }
            }
            let mut q=tx.prepare("SELECT sequence,room_id,agent_id,source_id,occurrence_id,payload FROM agent_inbox WHERE CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state') NOT IN ('accepted','acknowledged','handled','expired','failed') ELSE 1 END").map_err(sql)?;
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
                if matches!(e.state.as_str(), "submitting" | "uncertain")
                    && e.attempted_at_ms
                        .is_some_and(|at| now < at || now.saturating_sub(at) >= DELIVERY_TIMEOUT_MS)
                {
                    e.state = "blocked".into();
                    save_event(tx, &e)?;
                    let mut matched = false;
                    for mut t in tasks(tx)? {
                        if t.room_id == e.room_id
                            && t.agent_id == e.agent_id
                            && !matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
                        {
                            matched = true;
                            t.state = ExecutionState::Blocked;
                            t.blocked_revision = t.revision + 1;
                            t.reason=format!("Unconfirmed delivery {}: owner must reconcile the original attempt",e.sequence);
                            t.revision += 1;
                            save(tx, &t)?;
                            changed.push(t);
                        }
                    }
                    if !matched {
                        let id = format!("delivery-{}", e.sequence);
                        let mut task = new_task(e.room_id, e.agent_id, id, e.provider_run_id, now);
                        task.state = ExecutionState::Blocked;
                        task.blocked_revision = task.revision;
                        task.reason = format!("Unconfirmed delivery {}: reconcile the original attempt or explicitly cancel",e.sequence);
                        save(tx, &task)?;
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
        } => {
            let mut t = load(tx, &task)?;
            if t.owner_user_id.is_empty() || t.owner_user_id != owner || t.revision != revision {
                return Err(error("stale or foreign owner cancellation"));
            }
            if !matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled) {
                cancel_intent(tx, &mut t)?;
                t.revision += 1;
                save(tx, &t)?;
            }
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
                return Err(error("quarantined obligation coverage must be restored before resume; explicit cancellation remains available"));
            }
            if resume {
                registrations(tx, &t.task_id)?;
            }
            let mut q = tx.prepare("SELECT payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND json_extract(payload,'$.state') IN ('blocked','submitting','uncertain')").map_err(sql)?;
            let deliveries = q
                .query_map(params![t.room_id, t.agent_id], |r| r.get::<_, String>(0))
                .map_err(sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql)?;
            drop(q);
            if resume && !deliveries.is_empty() {
                return Err(error("unconfirmed delivery must receive its exact provider receipt or be explicitly cancelled before resume"));
            }
            if !resume {
                for payload in deliveries {
                    let mut event: InboxEvent = decode(&payload)?;
                    event.state = "failed".into();
                    save_event(tx, &event)?;
                }
            }
            t.state = if resume {
                ExecutionState::Working
            } else {
                ExecutionState::Cancelled
            };
            if !resume {
                cancel_intent(tx, &mut t)?;
            }
            t.no_progress_wakes = 0;
            t.wait = None;
            t.revision += 1;
            if resume {
                t.pending_prompt_id = Some(format!("task-resume-{}-{}", t.task_id, t.revision));
                t.last_progress_at_ms = now;
            }
            save(tx, &t)?;
            Ok(Outcome::Task(t))
        }
        _ => Err(error("invalid supervision operation")),
    }
}
pub(super) fn cancel_intent(
    tx: &Transaction<'_>,
    task: &mut AgentTaskExecution,
) -> Result<(), DaemonError> {
    task.state = ExecutionState::Cancelled;
    task.wait = None;
    task.reason =
        "Owner cancelled; owned resource cancellation remains supervised until physical settlement"
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
        registration.live = false;
        tx.execute(
            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
            params![registration.id, encode(&registration)?],
        )
        .map_err(sql)?;
    }
    Ok(())
}
