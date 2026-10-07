//! MP-08 / MP-09 / MP-10 / MP-11 A02: transactional lifecycle policy.
use super::*;
pub(super) fn apply(tx: &Transaction<'_>, op: Operation) -> Result<Outcome, DaemonError> {
    match op {
        Operation::Begin {
            owner,
            room,
            agent,
            prompt,
            run,
            now,
        } => {
            let mut task = if let Some(existing) = for_turn(tx, &room, &agent, &prompt)? {
                existing
            } else {
                let resumed = tasks(tx)?.into_iter().find(|t| {
                    t.room_id == room
                        && t.agent_id == agent
                        && t.pending_prompt_id.as_deref() == Some(&prompt)
                });
                if let Some(mut t) = resumed {
                    if t.state != ExecutionState::Working {
                        return Err(error(
                            "cancelled or blocked continuation cannot resume automatically",
                        ));
                    }
                    t.prompt_id = prompt;
                    t.pending_prompt_id = None;
                    t.state = ExecutionState::Working;
                    t.wait = None;
                    t.provider_run_id = None;
                    t.revision += 1;
                    t
                } else {
                    new_admitted_task(tx, room, agent, prompt, run.clone(), now)?
                }
            };
            if task.owner_user_id.is_empty() {
                task.owner_user_id = owner;
            }
            task.provider_run_id = run.or(task.provider_run_id);
            save(tx, &task)?;
            bind_first_delegate_task(tx, &task)?;
            Ok(Outcome::Task(task))
        }
        Operation::RegisterObligation {
            owner,
            room,
            agent,
            prompt,
            run,
            id,
            kind,
            resource,
            now,
        } => {
            let mut task = match for_turn(tx, &room, &agent, &prompt)? {
                Some(task) => task,
                None => new_admitted_task(tx, room, agent, prompt.clone(), run, now)?,
            };
            current(&task, &prompt)?;
            if task.owner_user_id.is_empty() {
                task.owner_user_id = owner;
            }
            migration::audit(
                tx,
                "room.obligation.registered",
                &id,
                serde_json::json!({"schema_version":1,"id":id,"room_id":task.room_id,"creating_agent_id":task.agent_id,"creating_provider_run_id":task.provider_run_id,"creating_prompt_id":task.prompt_id,"kind":kind,"resource_ref":resource,"status":"open","dispatch_state":"intent","created_at_ms":now}),
            )?;
            task.obligations.push(AgentObligation {
                id,
                kind,
                completion_task_id: None,
                resource_id: resource,
                status: "open".into(),
                dispatch_state: "intent".into(),
            });
            task.revision += 1;
            save(tx, &task)?;
            Ok(Outcome::Task(task))
        }
        Operation::DispatchReceipt {
            id,
            accepted,
            resource,
        } => {
            for mut task in tasks(tx)? {
                if let Some(o) = task.obligations.iter_mut().find(|o| o.id == id) {
                    migration::audit(
                        tx,
                        "room.obligation.dispatch_receipt",
                        &id,
                        serde_json::json!({"schema_version":1,"id":id,"dispatch_state":if accepted{"accepted"}else{"rejected"},"status":if accepted{"open"}else{"failed"},"resource_id":resource,"recorded_at_ms":crate::session::unix_epoch_ms()}),
                    )?;
                    o.dispatch_state = if accepted && task.state == ExecutionState::Cancelled {
                        "cancel_requested"
                    } else if accepted {
                        "accepted"
                    } else {
                        "rejected"
                    }
                    .into();
                    if resource.is_some() {
                        o.resource_id = resource;
                    }
                    // Default no-reply messages are delivery obligations, not perpetual reply waits.
                    if !accepted {
                        o.status = "failed".into();
                    } else if o.kind == "message" {
                        o.status = "satisfied".into();
                    }
                    if accepted && matches!(o.kind.as_str(), "delegate" | "workflow") {
                        if let Some(source) = o.resource_id.clone() {
                            let reg = Registration {
                                id: format!("completion-{}", o.id),
                                task_id: task.task_id.clone(),
                                source_id: source,
                                obligation_id: Some(o.id.clone()),
                                source_cursor: 0,
                                live: true,
                            };
                            tx.execute("INSERT INTO agent_registrations VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![reg.id,reg.task_id,encode(&reg)?]).map_err(sql)?;
                        }
                    }
                    task.revision += 1;
                    save(tx, &task)?;
                    if let Some(reg) = registrations(tx, &task.task_id)?
                        .into_iter()
                        .find(|r| r.id == format!("completion-{id}"))
                    {
                        recover_source(tx, &mut task, reg)?;
                    }
                    return Ok(Outcome::Task(task));
                }
            }
            Err(error("obligation unavailable"))
        }
        Operation::Subscribe {
            task,
            prompt,
            registration,
        } => {
            let t = load(tx, &task)?;
            current(&t, &prompt)?;
            if registration.task_id != task || !registration.live {
                return Err(error("source is not live"));
            }
            if let Some(id) = &registration.obligation_id {
                if !t.obligations.iter().any(|o| {
                    &o.id == id
                        && o.status == "open"
                        && o.completion_source() == Some(&registration.source_id)
                }) {
                    return Err(error("source does not cover an open obligation"));
                }
            }
            tx.execute("INSERT INTO agent_registrations VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![registration.id,task,encode(&registration)?]).map_err(sql)?;
            recover_source(tx, &mut load(tx, &task)?, registration)?;
            Ok(Outcome::Saved)
        }
        Operation::Unsubscribe {
            task,
            prompt,
            registration,
        } => {
            let t = load(tx, &task)?;
            current(&t, &prompt)?;
            let mut reg = registrations(tx, &task)?
                .into_iter()
                .find(|r| r.id == registration)
                .ok_or_else(|| error("registration unavailable in this task"))?;
            reg.live = false;
            tx.execute(
                "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                params![reg.id, encode(&reg)?],
            )
            .map_err(sql)?;
            // An invalidated wait must wake, rather than sleep on a dead registration.
            if t.wait
                .as_ref()
                .is_some_and(|w| w.registration_ids.contains(&reg.id))
            {
                event(
                    tx,
                    occurrence(
                        &t.room_id,
                        &t.agent_id,
                        &reg.source_id,
                        &format!("unsubscribe-{}-{}", reg.id, t.revision),
                        "source_lost",
                        serde_json::json!({"task_id":t.task_id,"registration_id":reg.id}),
                    ),
                )?;
            }
            Ok(Outcome::Saved)
        }
        Operation::Yield {
            task,
            prompt,
            registrations: ids,
            cursor,
            deadline,
            reason,
            now,
        } => {
            let mut t = load(tx, &task)?;
            current(&t, &prompt)?;
            if ids.is_empty()
                || deadline <= now
                || deadline > i64::MAX as u64
                || reason.trim().is_empty()
            {
                return Err(error(
                    "yield requires named live sources and a finite future deadline",
                ));
            }
            let regs = registrations(tx, &task)?;
            let mut selected = vec![];
            for r in regs.iter().filter(|r| ids.contains(&r.id)) {
                let pending:i64=tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND source_id=?3 AND sequence>?4 AND json_extract(payload,'$.state') NOT IN ('handled','expired','failed') AND json_extract(payload,'$.kind') IN ('source_completed','source_lost')",params![t.room_id,t.agent_id,r.source_id,sql_integer(cursor)?],|row|row.get(0)).map_err(sql)?;
                if r.live || pending > 0 {
                    selected.push(r);
                }
            }
            if selected.len() != ids.len()
                || t.obligations
                    .iter()
                    .filter(|o| {
                        o.status == "open" || o.status == "failed" || o.status == "settling"
                    })
                    .any(|o| {
                        !selected
                            .iter()
                            .any(|r| r.obligation_id.as_deref() == Some(&o.id))
                    })
            {
                return Err(error(
                    "yield sources must cover every unresolved obligation",
                ));
            }
            t.wait = Some(AgentWait {
                registration_ids: ids,
                deadline_ms: deadline,
                started_at_ms: now,
                inbox_cursor: cursor,
                long_wait_notified: false,
                last_checked_at_ms: now,
            });
            t.reason = crate::secret_redaction::redact_secrets(&reason).into_owned();
            t.revision += 1;
            // State stays working until the native turn settlement, including racing events.
            save(tx, &t)?;
            Ok(Outcome::Task(t))
        }
        Operation::Block {
            task,
            prompt,
            reason,
        } => {
            let mut t = load(tx, &task)?;
            if t.prompt_id != prompt || t.state != ExecutionState::Working {
                return Err(error("stale blocked intent"));
            }
            if reason.trim().is_empty() {
                return Err(error("blocked requires an owner action"));
            }
            t.state = ExecutionState::Blocked;
            t.blocked_revision = t.revision + 1;
            t.reason = crate::secret_redaction::redact_secrets(&reason).into_owned();
            t.revision += 1;
            save(tx, &t)?;
            Ok(Outcome::Task(t))
        }
        Operation::Settle {
            room,
            agent,
            prompt,
            run,
            has_answer,
            cancelled,
            now,
        } => {
            let mut t = match for_turn(tx, &room, &agent, &prompt)? {
                Some(task) => task,
                None => new_admitted_task(tx, room, agent, prompt.clone(), Some(run.clone()), now)?,
            };
            if t.provider_run_id.as_deref().is_some_and(|r| r != run) {
                return Err(error("stale provider settlement"));
            }
            if matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
                || (!cancelled && t.pending_prompt_id.is_some())
            {
                return Ok(Outcome::Settled {
                    task: t,
                    correction: false,
                });
            }
            let mut correction = false;
            if cancelled {
                cancel_intent(tx, &mut t)?;
            } else if t.state != ExecutionState::Blocked {
                let open = t
                    .obligations
                    .iter()
                    .any(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"));
                let valid_wait = if let Some(w) = &t.wait {
                    let regs = registrations(tx, &t.task_id)?;
                    let pending:i64 = tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence>?3 AND json_extract(payload,'$.state') NOT IN ('handled','expired','failed') AND json_extract(payload,'$.kind') IN ('source_completed','source_lost') AND (json_extract(payload,'$.payload.task_id')=?4 OR EXISTS(SELECT 1 FROM json_each(json_extract(payload,'$.payload.task_ids')) WHERE value=?4))",params![t.room_id,t.agent_id,sql_integer(w.inbox_cursor)?,t.task_id],|r|r.get(0)).map_err(sql)?;
                    w.deadline_ms > now
                        && !w.registration_ids.is_empty()
                        && w.registration_ids
                            .iter()
                            .all(|id| regs.iter().any(|r| &r.id == id && (r.live || pending > 0)))
                        && t.obligations
                            .iter()
                            .filter(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"))
                            .all(|o| {
                                regs.iter().any(|r| {
                                    w.registration_ids.contains(&r.id)
                                        && r.obligation_id.as_deref() == Some(&o.id)
                                })
                            })
                } else {
                    false
                };
                if !open && has_answer && t.wait.is_none() {
                    t.state = ExecutionState::Done;
                    t.reason.clear();
                } else if t.no_progress_wakes >= NO_PROGRESS_LIMIT {
                    t.state = ExecutionState::Blocked;
                    t.blocked_revision = t.revision + 1;
                    t.reason="Three consecutive wakes without handled progress; owner must resume or cancel".into();
                } else if valid_wait {
                    t.state = ExecutionState::Waiting;
                } else if !t.correction_used {
                    t.correction_used = true;
                    correction = true;
                    t.pending_prompt_id = Some(format!("task-correction-{}", t.task_id));
                    t.reason="Turn ended without completing its obligations or a valid live wait; correct once".into();
                } else {
                    t.state = ExecutionState::Blocked;
                    t.blocked_revision = t.revision + 1;
                    t.wait = None;
                    t.reason = "Repeated invalid turn end; owner must resume or cancel".into();
                }
            }
            t.provider_run_id = Some(run);
            t.revision += 1;
            save(tx, &t)?;
            Ok(Outcome::Settled {
                task: t,
                correction,
            })
        }
        op @ (Operation::Send { .. }
        | Operation::Occur(_)
        | Operation::Attempt { .. }
        | Operation::Defer { .. }
        | Operation::BindAttempt { .. }
        | Operation::Receipt { .. }
        | Operation::Ack { .. }) => super::delivery::apply(tx, op),
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
            for mut t in tasks(tx)? {
                if t.room_id != room {
                    continue;
                }
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
                for mut reg in registrations(tx, &t.task_id)? {
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
    }
}

fn recover_source(
    tx: &Transaction<'_>,
    task: &mut AgentTaskExecution,
    mut registration: Registration,
) -> Result<(), DaemonError> {
    let source: Option<(i64,String,bool,Option<String>)> = tx.query_row("SELECT sequence,occurrence_id,success,public_answer FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2 AND sequence>?3 ORDER BY sequence DESC LIMIT 1",params![task.room_id,registration.source_id,sql_integer(registration.source_cursor)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?;
    if let Some((seq, id, success, answer)) = source {
        let public_answer = answer
            .map(|v| decode::<serde_json::Value>(&v))
            .transpose()?;
        event(
            tx,
            occurrence(
                &task.room_id,
                &task.agent_id,
                &registration.source_id,
                &id,
                if success {
                    "source_completed"
                } else {
                    "source_lost"
                },
                serde_json::json!({"task_id":task.task_id,"source_id":registration.source_id,"public_history_ref":registration.source_id,"occurrence_id":id,"success":success,"public_answer":public_answer}),
            ),
        )?;
        registration.live = false;
        registration.source_cursor =
            u64::try_from(seq).map_err(|_| error("corrupt source sequence"))?;
        tx.execute(
            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
            params![registration.id, encode(&registration)?],
        )
        .map_err(sql)?;
        if let Some(o) = task
            .obligations
            .iter_mut()
            .find(|o| Some(&o.id) == registration.obligation_id.as_ref() && o.status == "open")
        {
            o.status = if success { "settling" } else { "failed" }.into();
            task.revision += 1;
            save(tx, task)?;
        }
    }
    Ok(())
}

// Bind once: a later independent task on the same child cannot settle the first delegation.
fn bind_first_delegate_task(
    tx: &Transaction<'_>,
    child: &AgentTaskExecution,
) -> Result<(), DaemonError> {
    for mut parent in tasks(tx)? {
        if parent.room_id != child.room_id
            || parent.owner_user_id.is_empty()
            || parent.owner_user_id != child.owner_user_id
        {
            continue;
        }
        let mut changed = false;
        for obligation in &mut parent.obligations {
            if obligation.kind == "delegate"
                && obligation.status == "open"
                && obligation.dispatch_state == "accepted"
                && obligation.resource_id.as_deref() == Some(&child.agent_id)
                && obligation.completion_task_id.is_none()
            {
                obligation.completion_task_id = Some(child.task_id.clone());
                let id = format!("completion-{}", obligation.id);
                for mut registration in registrations(tx, &parent.task_id)?
                    .into_iter()
                    .filter(|r| r.id == id || r.obligation_id.as_deref() == Some(&obligation.id))
                {
                    registration.source_id = child.task_id.clone();
                    tx.execute(
                        "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                        params![id, encode(&registration)?],
                    )
                    .map_err(sql)?;
                }
                changed = true;
            }
        }
        if changed {
            parent.revision += 1;
            save(tx, &parent)?;
        }
    }
    Ok(())
}

fn cancel_intent(tx: &Transaction<'_>, task: &mut AgentTaskExecution) -> Result<(), DaemonError> {
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
    for mut registration in registrations(tx, &task.task_id)? {
        registration.live = false;
        tx.execute(
            "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
            params![registration.id, encode(&registration)?],
        )
        .map_err(sql)?;
    }
    Ok(())
}
