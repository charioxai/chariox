//! MP-08 / MP-09 / MP-10 / MP-11 A02: inbox ordering and exact receipt policy.
use super::*;
/// A receipt or submission binding for one row settles every row of its
/// batch: they share one prompt, so they share its exact outcome.
pub(super) fn apply(tx: &Transaction<'_>, op: Operation) -> Result<Outcome, DaemonError> {
    let (room, agent, sequence) = match &op {
        Operation::Receipt {
            room,
            agent,
            sequence,
            ..
        }
        | Operation::BindSubmission {
            room,
            agent,
            sequence,
            ..
        } => (room.clone(), agent.clone(), *sequence),
        _ => return apply_row(tx, op),
    };
    let members = siblings(tx, &room, &agent, sequence)?;
    let lead = apply_row(tx, op.clone())?;
    for member in members {
        let Some(row) = find_event(tx, &room, &agent, member)? else {
            continue;
        };
        if !matches!(row.state.as_str(), "submitting" | "uncertain" | "blocked") {
            continue;
        }
        let op = match op.clone() {
            Operation::Receipt {
                room,
                agent,
                state,
                now,
                ..
            } => Operation::Receipt {
                room,
                agent,
                sequence: member,
                state,
                now,
            },
            Operation::BindSubmission {
                room,
                agent,
                prompt,
                target,
                run,
                submit_epoch,
                now,
                ..
            } => Operation::BindSubmission {
                room,
                agent,
                sequence: member,
                prompt,
                target,
                run,
                submit_epoch,
                now,
            },
            _ => unreachable!(),
        };
        apply_row(tx, op)?;
    }
    Ok(lead)
}

fn apply_row(tx: &Transaction<'_>, op: Operation) -> Result<Outcome, DaemonError> {
    match op {
        Operation::Send {
            task,
            prompt,
            event: e,
        } => {
            let mut t = load(tx, &task)?;
            current(&t, &prompt)?;
            // A message to another room lives in the recipient's room and names the sender's.
            let sender_room = e.payload["sender_room_id"].as_str().unwrap_or(&e.room_id);
            if e.source_id != t.agent_id || sender_room != t.room_id || e.kind != "message" {
                return Err(error("message source binding invalid"));
            }
            let admitted = event(tx, e)?;
            tx.execute("INSERT INTO agent_delegation_messages VALUES(?1,?2) ON CONFLICT(sequence) DO NOTHING",params![sql_integer(admitted.sequence)?,t.task_id]).map_err(sql)?;
            if admitted.reply_requested {
                let id = format!("reply-{}", admitted.sequence);
                if !t.obligations.iter().any(|o| o.id == id) {
                    let source = format!("agent-event-{}-{}", admitted.agent_id, admitted.sequence);
                    t.obligations.push(AgentObligation {
                        id: id.clone(),
                        kind: "reply".into(),
                        completion_task_id: None,
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
                        peer: false,
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
            work,
            through,
        } => {
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if work
                .as_deref()
                .is_some_and(|work| !super::work_correlated(&e, work))
            {
                return Err(error("event is outside the authorized work binding"));
            }
            if e.state != "pending" {
                return Err(error(
                    "delivery is already admitted; reconcile its exact receipt",
                ));
            }
            let urgent_steer = e.urgent && target.is_some() && e.attempted_at_ms.is_none();
            let correlated = super::WORK_CORRELATED.replace("?3", "?5");
            let earlier:i64=tx.query_row(&format!("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence<>?3 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state') IN ('submitting','uncertain','blocked') OR (sequence<?3 AND json_extract(payload,'$.state')='pending' AND json_extract(payload,'$.kind') IS NOT 'task_cancelled' AND (?5 IS NULL OR {correlated}) AND (?4=0 OR (json_extract(payload,'$.urgent')=1 AND json_extract(payload,'$.attempted_at_ms') IS NULL))) ELSE 1 END"),params![room,agent,sql_integer(sequence)?,urgent_steer,work],|r|r.get(0)).map_err(sql)?;
            if earlier != 0 {
                return Err(error("earlier recipient delivery must settle first"));
            }
            // Owner Resume/correction already owns the next turn. Do not let
            // an idle inbox attempt race its admission into a second task.
            if target.is_none()
                && e.kind != "message"
                && continuing_tasks(tx, &room, &agent)?.iter().any(|t| {
                    t.room_id == room
                        && t.agent_id == agent
                        && t.pending_prompt_id.is_some()
                        && (e.payload["task_id"].as_str() == Some(t.task_id.as_str())
                            || e.payload["task_ids"].as_array().is_some_and(|ids| {
                                ids.iter().any(|id| id.as_str() == Some(t.task_id.as_str()))
                            }))
                })
            {
                return Err(error("task continuation must settle before inbox delivery"));
            }
            replies::bind(tx, &e, target.as_deref(), &prompt)?;
            admit(tx, &mut e, &prompt, target.as_deref(), run.as_deref(), now)?;
            // One wake covers every row it may deliver; the rows share this
            // prompt and settle together through its receipt. A reply
            // request is answered by its own turn, so it is never batched.
            if !e.reply_requested {
                for mut member in recipient_events_between(tx, &room, &agent, e.sequence, through)?
                {
                    if member.reply_requested {
                        break;
                    }
                    if batch_member(&member, target.is_some(), work.as_deref()) {
                        admit(
                            tx,
                            &mut member,
                            &prompt,
                            target.as_deref(),
                            run.as_deref(),
                            now,
                        )?;
                    }
                }
            }
            Ok(Outcome::Event(e))
        }
        Operation::MarkShown {
            room,
            agent,
            sequences,
        } => {
            for sequence in sequences {
                tx.execute(
                    "INSERT OR IGNORE INTO agent_inbox_shown(sequence) SELECT sequence FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence=?3",
                    params![room, agent, sql_integer(sequence)?],
                )
                .map_err(sql)?;
            }
            Ok(Outcome::Saved)
        }
        Operation::Expire {
            room,
            agent,
            sequence,
        } => {
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if e.state != "pending" {
                return Err(error("cannot expire an admitted or uncertain delivery"));
            }
            e.state = "expired".into();
            save_event(tx, &e)?;
            super::wakes::record_delivery(tx, &e)?;
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
            // Unsupported steering stays queued for idle delivery; this marker
            // prevents repeated steering, but is not an admitted-attempt clock.
            e.attempted_at_ms.get_or_insert(now);
            save_event(tx, &e)?;
            super::wakes::record_delivery(tx, &e)?;
            Ok(Outcome::Event(e))
        }
        Operation::BindSubmission {
            room,
            agent,
            sequence,
            prompt,
            target,
            run,
            submit_epoch,
            now,
        } => {
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if e.prompt_id.as_deref() != Some(&prompt)
                || e.target_prompt_id != target
                || !matches!(
                    e.state.as_str(),
                    "submitting"
                        | "uncertain"
                        | "blocked"
                        | "accepted"
                        | "acknowledged"
                        | "handled"
                )
            {
                return Err(error("submission does not match the admitted event prompt"));
            }
            // Provider I/O consumes the wait that Begin kept for this wake;
            // a reconciled submission cannot consume a newer wait.
            if e.submit_epoch.is_none() && e.state == "submitting" {
                if let Some(mut task) = for_turn(tx, &room, &agent, &prompt)? {
                    if task.state == ExecutionState::Working
                        && !super::goals::is_check(&task, &prompt)
                        && task.wait.take().is_some()
                    {
                        task.revision += 1;
                        save(tx, &task)?;
                    }
                }
            }
            if e.provider_run_id.as_deref() != Some(&run) || e.submit_epoch != Some(submit_epoch) {
                e.provider_run_id = Some(run);
                e.submit_epoch = Some(submit_epoch);
                e.attempted_at_ms = Some(now);
                // A handled outcome remains handled across provider recovery.
                if !matches!(e.state.as_str(), "acknowledged" | "handled") {
                    e.state = "submitting".into();
                }
                save_event(tx, &e)?;
            }
            Ok(Outcome::Event(e))
        }
        Operation::Receipt {
            room,
            agent,
            sequence,
            state,
            now,
        } => {
            if !matches!(state.as_str(), "accepted" | "rejected" | "uncertain") {
                return Err(error("invalid delivery receipt"));
            }
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if !matches!(e.state.as_str(), "submitting" | "uncertain" | "blocked") {
                return Err(error("receipt does not match a pending delivery"));
            }
            // A timeout-only task is an owner projection of this delivery,
            // not provider work. Keep a terminal row so the next runtime sweep
            // retracts its interaction, without suppressing the pending retry.
            if e.state == "blocked" {
                let id = format!("delivery-{}", e.sequence);
                if let Some(mut task) = for_turn(tx, &e.room_id, &e.agent_id, &id)? {
                    if task.task_id == id
                        && task.state == ExecutionState::Blocked
                        && task.obligations.is_empty()
                    {
                        task.state = ExecutionState::Done;
                        task.reason = "Delivery receipt reconciled".into();
                        task.revision += 1;
                        save(tx, &task)?;
                    }
                }
            }
            e.state = if state == "rejected" {
                "pending".into()
            } else {
                state
            };
            if e.state == "pending" {
                if let Some(prompt) = e.prompt_id.as_deref() {
                    revert_refused_wake(tx, &e, prompt)?;
                }
                // Steering is a busy refusal even when its receipt arrives
                // after the turn ends or the admitted attempt times out.
                if e.target_prompt_id.is_none() {
                    tx.execute(
                        "INSERT INTO agent_inbox_refusals VALUES(?1,?2) ON CONFLICT(sequence) DO NOTHING",
                        params![sql_integer(e.sequence)?, sql_integer(now)?],
                    ).map_err(sql)?;
                }
                e.prompt_id = None;
                e.target_prompt_id = None;
                e.provider_run_id = None;
                e.submit_epoch = None;
            }
            save_event(tx, &e)?;
            super::wakes::record_delivery(tx, &e)?;
            super::delegation::bind_message(tx, &e)?;
            replies::reconcile_event(tx, &e)?;
            if e.state == "accepted" {
                super::unread_steers::revert_late_receipt(tx, &mut e)?;
            }
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
            let already_handled = e.state == "handled";
            // Reading a pending check-in during independent work is not admission
            // of its goal continuation. Keep it deliverable through the shared owner.
            if e.state == "pending"
                && matches!(e.kind.as_str(), "deadline_reached" | "wait_recheck")
                && live_tasks(tx, &room, &agent)?.iter().any(|task| {
                    task.goal.is_some()
                        && task.state == ExecutionState::Waiting
                        && task.room_id == room
                        && task.agent_id == agent
                        && (e.payload["task_id"].as_str() == Some(task.task_id.as_str())
                            || e.payload["task_ids"].as_array().is_some_and(|ids| {
                                ids.iter()
                                    .any(|id| id.as_str() == Some(task.task_id.as_str()))
                            }))
                })
            {
                return Err(error(
                    "goal check-in must be admitted to its task before acknowledgement",
                ));
            }
            if (e.state == "failed" || (already_handled && !handled))
                || (e.state == "expired"
                    && (!handled
                        || !live_tasks(tx, &room, &agent)?
                            .iter()
                            .any(|task| unresolved_source_outcome(task, &e))))
            {
                return Ok(Outcome::Event(e));
            }
            if !already_handled {
                e.state = if handled { "handled" } else { "acknowledged" }.into();
                save_event(tx, &e)?;
                super::wakes::record_delivery(tx, &e)?;
            }
            // A loss handled while Blocked keeps its failed obligation until
            // owner Resume. A repeated ACK can then reconcile that disposition
            // without replaying delivery or advancing the same receipt's clock.
            if handled && e.is_source_outcome() {
                for mut t in live_tasks(tx, &room, &agent)? {
                    if t.room_id != room
                        || t.agent_id != agent
                        // MP-08/MP-10/MP-11: a loss ACK cannot override the owner's Blocked decision.
                        || (t.state == ExecutionState::Blocked && e.is_loss())
                        || matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
                    {
                        continue;
                    }
                    let mut changed = false;
                    for o in &mut t.obligations {
                        if o.completion_source()
                            .is_some_and(|source| e.reports_source(source))
                            && matches!(o.status.as_str(), "failed" | "settling")
                        {
                            o.status = "satisfied".into();
                            changed = true;
                        }
                    }
                    // Handling an actual resource outcome is progress; a receipt ACK alone is not.
                    let belongs = e.payload["task_id"].as_str() == Some(t.task_id.as_str())
                        || e.payload["task_ids"].as_array().is_some_and(|ids| {
                            ids.iter().any(|id| id.as_str() == Some(t.task_id.as_str()))
                        });
                    // A Blocked task awaits its owner: another task's outcome that merely
                    // names it as a subscriber must not move its revision.
                    let finished_wait = super::settled_waits::finish(tx, &mut t)?;
                    if changed
                        || finished_wait
                        || (!already_handled && belongs && t.state != ExecutionState::Blocked)
                    {
                        let timer_outcome = e.payload["public_answer"]["kind"].as_str() == Some("timer")
                            || tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_wakes WHERE id=?1 AND json_extract(payload,'$.kind')='timer')", [&e.source_id], |r| r.get::<_,bool>(0)).map_err(sql)?;
                        // Closing a timer obligation is required bookkeeping,
                        // but even rearming one-shot timers cannot buy progress.
                        if !timer_outcome && t.state != ExecutionState::Blocked {
                            t.last_progress_at_ms = now;
                        }
                        t.revision += 1;
                        save(tx, &t)?;
                    }
                }
            }
            Ok(Outcome::Event(e))
        }
        _ => Err(error("invalid delivery operation")),
    }
}

// A wake the provider never accepted did not run: the task keeps its wait.
// The pending event retries later.
fn revert_refused_wake(
    tx: &Transaction<'_>,
    event: &InboxEvent,
    prompt: &str,
) -> Result<(), DaemonError> {
    for mut t in live_tasks(tx, &event.room_id, &event.agent_id)? {
        if t.room_id == event.room_id
            && t.agent_id == event.agent_id
            && t.state == ExecutionState::Working
            && t.wait.is_some()
            && (t.pending_prompt_id.as_deref() == Some(prompt) || t.prompt_id == prompt)
        {
            t.state = ExecutionState::Waiting;
            t.pending_prompt_id = None;
            t.revision += 1;
            save(tx, &t)?;
        }
    }
    Ok(())
}

/// Whether a pending row may join the batch of a wake: a steer wake carries
/// only fresh urgent rows, a sudo-bound wake only rows of the work task.
pub(crate) fn batch_member(event: &InboxEvent, steer: bool, work: Option<&str>) -> bool {
    event.state == "pending"
        && event.kind != "task_cancelled"
        && !(steer && (!event.urgent || event.attempted_at_ms.is_some()))
        && work.is_none_or(|work| super::work_correlated(event, work))
}

/// The recipient's rows with `after < sequence <= through`, in order. The sequence is global, so
/// the range is read through the recipient's index, never one lookup per number.
fn recipient_events_between(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    after: u64,
    through: u64,
) -> Result<Vec<InboxEvent>, DaemonError> {
    let mut statement = tx
        .prepare_cached(
            "SELECT sequence,source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence>?3 AND sequence<=?4 ORDER BY sequence",
        )
        .map_err(sql)?;
    let rows = statement
        .query_map(
            params![room, agent, sql_integer(after)?, sql_integer(through)?],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    rows.into_iter()
        .map(|(sequence, source, id, payload)| {
            decode_inbox(sequence, room, agent, &source, &id, &payload)
        })
        .collect()
}

fn find_event(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    sequence: u64,
) -> Result<Option<InboxEvent>, DaemonError> {
    let row: Option<(String, String, String)> = tx
        .query_row(
            "SELECT source_id,occurrence_id,payload FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence=?3",
            params![room, agent, sql_integer(sequence)?],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql)?;
    row.map(|(source, id, payload)| {
        decode_inbox(sql_integer(sequence)?, room, agent, &source, &id, &payload)
    })
    .transpose()
}

/// Rows that share `sequence`'s prompt: the other members of its batch.
fn siblings(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    sequence: u64,
) -> Result<Vec<u64>, DaemonError> {
    let mut q = tx
        .prepare("SELECT sequence FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence<>?3 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.prompt_id') END IS NOT NULL AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.prompt_id') END=(SELECT CASE WHEN json_valid(payload) THEN json_extract(payload,'$.prompt_id') END FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence=?3) ORDER BY sequence")
        .map_err(sql)?;
    let rows = q
        .query_map(params![room, agent, sql_integer(sequence)?], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    #[cfg(test)]
    INBOX_LOOKUP_STEPS.with(|n| n.set(n.get() + q.get_status(rusqlite::StatementStatus::VmStep)));
    rows.into_iter()
        .map(|s| u64::try_from(s).map_err(|_| error("corrupt delivery sequence")))
        .collect()
}

/// Admits one pending row to the attempt: it takes the prompt, and the task
/// whose wait it wakes keeps that wait until provider I/O.
fn admit(
    tx: &Transaction<'_>,
    e: &mut InboxEvent,
    prompt: &str,
    target: Option<&str>,
    run: Option<&str>,
    now: u64,
) -> Result<(), DaemonError> {
    let (room, agent) = (e.room_id.clone(), e.agent_id.clone());
    e.state = "submitting".into();
    e.prompt_id = Some(prompt.into());
    e.target_prompt_id = target.map(str::to_owned);
    e.provider_run_id = run.map(str::to_owned);
    // Every admitted attempt gets its full receipt window.
    e.attempted_at_ms = Some(now);
    save_event(tx, e)?;
    super::wakes::record_delivery(tx, e)?;
    // A wake retains the original task and its progress clock.
    for mut t in live_tasks(tx, &room, &agent)? {
        if t.room_id == room
            && t.agent_id == agent
            && t.state == ExecutionState::Waiting
            && e.kind != "message"
            && (e.payload.get("task_id").and_then(serde_json::Value::as_str)
                == Some(t.task_id.as_str())
                || e.payload["task_ids"].as_array().is_some_and(|ids| {
                    ids.iter().any(|id| id.as_str() == Some(t.task_id.as_str()))
                }))
        {
            super::goals::clock(&mut t, now);
            if let Some(g) = &mut t.goal {
                g.continuation_sequence += 1;
                g.next_check_in_ms = None;
            }
            t.pending_prompt_id = Some(prompt.into());
            t.state = ExecutionState::Working;
            t.revision += 1;
            save(tx, &t)?;
            break;
        }
    }
    Ok(())
}
