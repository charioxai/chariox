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
                    t.prompt_id = prompt;
                    t.pending_prompt_id = None;
                    t.state = ExecutionState::Working;
                    t.wait = None;
                    t.provider_run_id = None;
                    t.revision += 1;
                    t
                } else {
                    new_task(room, agent, prompt, run.clone(), now)
                }
            };
            if task.owner_user_id.is_empty() {
                task.owner_user_id = owner;
            }
            task.provider_run_id = run.or(task.provider_run_id);
            save(tx, &task)?;
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
            let mut task = for_turn(tx, &room, &agent, &prompt)?
                .unwrap_or_else(|| new_task(room, agent, prompt.clone(), run, now));
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
                    o.dispatch_state = if accepted { "accepted" } else { "rejected" }.into();
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
                        && o.resource_id.as_deref() == Some(&registration.source_id)
                }) {
                    return Err(error("source does not cover an open obligation"));
                }
            }
            tx.execute("INSERT INTO agent_registrations VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![registration.id,task,encode(&registration)?]).map_err(sql)?;
            let source:Option<(u64,String,bool)>=tx.query_row("SELECT sequence,occurrence_id,success FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2 AND sequence>?3 ORDER BY sequence DESC LIMIT 1",params![t.room_id,registration.source_id,registration.source_cursor],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
            if let Some((seq, id, success)) = source {
                let e = occurrence(
                    &t.room_id,
                    &t.agent_id,
                    &registration.source_id,
                    &id,
                    if success {
                        "source_completed"
                    } else {
                        "source_lost"
                    },
                    serde_json::json!({"task_id":t.task_id,"source_id":registration.source_id,"success":success}),
                );
                event(tx, e)?;
                let mut registration = registration;
                registration.live = false;
                registration.source_cursor = seq;
                tx.execute(
                    "UPDATE agent_registrations SET payload=?2 WHERE id=?1",
                    params![registration.id, encode(&registration)?],
                )
                .map_err(sql)?;
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
                let pending:i64=tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND source_id=?3 AND sequence>?4 AND json_extract(payload,'$.kind') IN ('source_completed','source_lost')",params![t.room_id,t.agent_id,r.source_id,cursor],|row|row.get(0)).map_err(sql)?;
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
            let mut t = for_turn(tx, &room, &agent, &prompt)?
                .unwrap_or_else(|| new_task(room, agent, prompt.clone(), Some(run.clone()), now));
            if t.provider_run_id.as_deref().is_some_and(|r| r != run) {
                return Err(error("stale provider settlement"));
            }
            if matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
                || t.pending_prompt_id.is_some()
            {
                return Ok(Outcome::Settled {
                    task: t,
                    correction: false,
                });
            }
            let mut correction = false;
            if cancelled {
                t.state = ExecutionState::Cancelled;
                t.wait = None;
                t.reason = "owner cancelled; resource settlement remains supervised".into();
            } else if t.state != ExecutionState::Blocked {
                let open = t
                    .obligations
                    .iter()
                    .any(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"));
                let valid_wait = if let Some(w) = &t.wait {
                    let regs = registrations(tx, &t.task_id)?;
                    let pending:i64 = tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence>?3 AND json_extract(payload,'$.kind') IN ('source_completed','source_lost') AND json_extract(payload,'$.payload.task_id')=?4",params![t.room_id,t.agent_id,w.inbox_cursor,t.task_id],|r|r.get(0)).map_err(sql)?;
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
                if t.no_progress_wakes >= NO_PROGRESS_LIMIT {
                    t.state = ExecutionState::Blocked;
                    t.blocked_revision = t.revision + 1;
                    t.reason="Three consecutive wakes without handled progress; owner must resume or cancel".into();
                } else if valid_wait {
                    t.state = ExecutionState::Waiting;
                } else if !open && has_answer && t.wait.is_none() {
                    t.state = ExecutionState::Done;
                    t.reason.clear();
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
        Operation::Send {
            task,
            prompt,
            event: e,
        } => {
            let mut t = load(tx, &task)?;
            current(&t, &prompt)?;
            if e.source_id != t.agent_id || e.room_id != t.room_id || e.kind != "message" {
                return Err(error("message source binding invalid"));
            }
            let admitted = event(tx, e)?;
            if admitted.reply_requested {
                let id = format!("reply-{}", admitted.sequence);
                if !t.obligations.iter().any(|o| o.id == id) {
                    let source = format!("agent-event-{}-{}", admitted.agent_id, admitted.sequence);
                    t.obligations.push(AgentObligation {
                        id: id.clone(),
                        kind: "reply".into(),
                        resource_id: Some(source.clone()),
                        status: "open".into(),
                        dispatch_state: "accepted".into(),
                    });
                    let reg = Registration {
                        id: format!("completion-{id}"),
                        task_id: t.task_id.clone(),
                        source_id: source,
                        obligation_id: Some(id),
                        source_cursor: 0,
                        live: true,
                    };
                    tx.execute(
                        "INSERT INTO agent_registrations VALUES(?1,?2,?3)",
                        params![reg.id, reg.task_id, encode(&reg)?],
                    )
                    .map_err(sql)?;
                    t.revision += 1;
                    save(tx, &t)?;
                }
            }
            Ok(Outcome::Event(admitted))
        }
        Operation::Occur(e) => Ok(Outcome::Event(event(tx, e)?)),
        Operation::Attempt {
            room,
            agent,
            sequence,
            prompt,
            target,
            run,
            now,
        } => {
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if e.state != "pending" {
                return Err(error(
                    "delivery is already admitted; reconcile its exact receipt",
                ));
            }
            let earlier:i64=tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence<?3 AND json_extract(payload,'$.state') IN ('pending','submitting','uncertain')",params![room,agent,sequence],|r|r.get(0)).map_err(sql)?;
            if earlier != 0 {
                return Err(error("earlier recipient delivery must settle first"));
            }
            e.state = "submitting".into();
            e.prompt_id = Some(prompt.clone());
            e.target_prompt_id = target;
            e.provider_run_id = run;
            e.attempted_at_ms = Some(now);
            save_event(tx, &e)?;
            // Wake retains the original task. Progress is not an ACK/cursor or deadline edit.
            for mut t in tasks(tx)? {
                if t.room_id == room && t.agent_id == agent && t.state == ExecutionState::Waiting {
                    t.no_progress_wakes += 1;
                    if t.no_progress_wakes > NO_PROGRESS_LIMIT {
                        t.state = ExecutionState::Blocked;
                        t.blocked_revision = t.revision + 1;
                        t.reason="Three consecutive wakes without a handled result; owner must resume or cancel".into();
                        e.state = "blocked".into();
                        save_event(tx, &e)?;
                    } else {
                        t.pending_prompt_id = Some(prompt);
                        t.state = ExecutionState::Working;
                    }
                    t.revision += 1;
                    save(tx, &t)?;
                    break;
                }
            }
            Ok(Outcome::Event(e))
        }
        Operation::Defer {
            room,
            agent,
            sequence,
            now,
        } => {
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if e.state != "pending" {
                return Err(error("cannot defer an admitted attempt"));
            }
            e.attempted_at_ms = Some(now);
            save_event(tx, &e)?;
            Ok(Outcome::Event(e))
        }
        Operation::BindAttempt {
            room,
            agent,
            sequence,
            run,
            submit_epoch,
        } => {
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if e.state != "submitting" || e.provider_run_id.as_deref().is_some_and(|id| id != run) {
                return Err(error("attempt binding changed"));
            }
            e.provider_run_id = Some(run);
            e.submit_epoch = Some(submit_epoch);
            save_event(tx, &e)?;
            Ok(Outcome::Event(e))
        }
        Operation::Receipt {
            room,
            agent,
            sequence,
            state,
        } => {
            if !matches!(state.as_str(), "accepted" | "rejected" | "uncertain") {
                return Err(error("invalid delivery receipt"));
            }
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if !matches!(e.state.as_str(), "submitting" | "uncertain") {
                return Err(error("receipt does not match a pending delivery"));
            }
            e.state = if state == "rejected" {
                "pending".into()
            } else {
                state
            };
            if e.state == "pending" {
                e.prompt_id = None;
                e.target_prompt_id = None;
                e.provider_run_id = None;
                // Keep the rejected-at marker: native rejection falls back to a later wake.
                e.submit_epoch = None;
            }
            save_event(tx, &e)?;
            Ok(Outcome::Event(e))
        }
        Operation::Ack {
            room,
            agent,
            sequence,
            handled,
            now,
        } => {
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if matches!(e.state.as_str(), "submitting" | "uncertain" | "blocked") {
                return Err(error(
                    "inbox acknowledgement cannot repair uncertain delivery",
                ));
            }
            if e.state == "handled" {
                return Ok(Outcome::Event(e));
            }
            e.state = if handled { "handled" } else { "acknowledged" }.into();
            save_event(tx, &e)?;
            if handled && matches!(e.kind.as_str(), "source_completed" | "source_lost") {
                for mut t in tasks(tx)? {
                    if t.room_id != room || t.agent_id != agent {
                        continue;
                    }
                    let mut changed = false;
                    for o in &mut t.obligations {
                        if o.resource_id.as_deref() == Some(&e.source_id)
                            && matches!(o.status.as_str(), "failed" | "settling")
                        {
                            o.status = "satisfied".into();
                            changed = true;
                        }
                    }
                    // Handling an actual resource outcome is progress; a receipt ACK alone is not.
                    if changed || e.kind == "source_completed" {
                        t.no_progress_wakes = 0;
                        t.progress_sequence += 1;
                        t.last_progress_at_ms = now;
                        t.revision += 1;
                        save(tx, &t)?;
                    }
                }
            }
            Ok(Outcome::Event(e))
        }
        Operation::SourceOutcome {
            room,
            source,
            occurrence: id,
            success,
            now: _,
        } => {
            tx.execute("INSERT INTO agent_source_occurrences(room_id,source_id,occurrence_id,success) VALUES(?1,?2,?3,?4) ON CONFLICT(room_id,source_id,occurrence_id) DO NOTHING",params![room,source,id,success]).map_err(sql)?;
            for mut t in tasks(tx)? {
                if t.room_id != room {
                    continue;
                }
                let mut changed = false;
                for o in &mut t.obligations {
                    if o.resource_id.as_deref() == Some(&source) && o.status == "open" {
                        o.status = if success { "settling" } else { "failed" }.into();
                        changed = true;
                    }
                }
                for mut reg in registrations(tx, &t.task_id)? {
                    if reg.source_id == source {
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
                        serde_json::json!({"task_id":t.task_id,"source_id":source,"public_history_ref":source,"occurrence_id":id,"success":success}),
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
                        let w = t.wait.as_ref().unwrap();
                        let dead = w
                            .registration_ids
                            .iter()
                            .any(|id| !regs.iter().any(|r| &r.id == id && r.live));
                        if dead || w.deadline_ms <= now || now < w.started_at_ms {
                            let kind = if dead {
                                "source_lost"
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
            let mut q=tx.prepare("SELECT sequence,room_id,agent_id,source_id,occurrence_id,payload FROM agent_inbox WHERE json_valid(payload)=0 OR json_extract(payload,'$.state') IN ('submitting','uncertain')").map_err(sql)?;
            let rows = q
                .query_map([], |r| {
                    Ok((
                        r.get::<_, u64>(0)?,
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
                match decode::<InboxEvent>(&payload) {
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
                        e.sequence = sequence;
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
                if e.attempted_at_ms
                    .is_some_and(|at| now.saturating_sub(at) >= DELIVERY_TIMEOUT_MS)
                {
                    e.state = "blocked".into();
                    save_event(tx, &e)?;
                    for mut t in tasks(tx)? {
                        if t.room_id == e.room_id
                            && t.agent_id == e.agent_id
                            && !matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
                        {
                            t.state = ExecutionState::Blocked;
                            t.blocked_revision = t.revision + 1;
                            t.reason=format!("Unconfirmed delivery {}: owner must reconcile the original attempt",e.sequence);
                            t.revision += 1;
                            save(tx, &t)?;
                            changed.push(t);
                        }
                    }
                }
            }
            Ok(Outcome::Swept(changed))
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
            t.state = if resume {
                ExecutionState::Working
            } else {
                ExecutionState::Cancelled
            };
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
