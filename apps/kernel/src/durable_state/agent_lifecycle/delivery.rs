//! MP-08 / MP-09 / MP-10 / MP-11 A02: inbox ordering and exact receipt policy.
use super::*;
pub(super) fn apply(tx: &Transaction<'_>, op: Operation) -> Result<Outcome, DaemonError> {
    match op {
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
            let earlier:i64=tx.query_row(&format!("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence<>?3 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state') IN ('submitting','uncertain','blocked') OR (sequence<?3 AND json_extract(payload,'$.state')='pending' AND (?5 IS NULL OR {correlated}) AND (?4=0 OR (json_extract(payload,'$.urgent')=1 AND json_extract(payload,'$.attempted_at_ms') IS NULL))) ELSE 1 END"),params![room,agent,sql_integer(sequence)?,urgent_steer,work],|r|r.get(0)).map_err(sql)?;
            if earlier != 0 {
                return Err(error("earlier recipient delivery must settle first"));
            }
            // Owner Resume/correction already owns the next turn. Do not let
            // an idle inbox attempt race its admission into a second task.
            if target.is_none()
                && e.kind != "message"
                && tasks(tx)?.iter().any(|t| {
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
            e.state = "submitting".into();
            e.prompt_id = Some(prompt.clone());
            e.target_prompt_id = target;
            e.provider_run_id = run;
            // Every admitted attempt gets its full receipt window.
            e.attempted_at_ms = Some(now);
            save_event(tx, &e)?;
            super::wakes::record_delivery(tx, &e)?;
            // Wake retains the original task. Progress is not an ACK/cursor or deadline edit.
            for mut t in tasks(tx)? {
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
                    // Settle blocks at the limit, so a Waiting task is always below it.
                    t.no_progress_wakes += 1;
                    t.pending_prompt_id = Some(prompt);
                    t.state = ExecutionState::Working;
                    t.revision += 1;
                    save(tx, &t)?;
                    break;
                }
            }
            Ok(Outcome::Event(e))
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
                    revert_refused_wake(tx, &e.room_id, &e.agent_id, prompt)?;
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
            if matches!(e.state.as_str(), "handled" | "expired" | "failed") {
                return Ok(Outcome::Event(e));
            }
            e.state = if handled { "handled" } else { "acknowledged" }.into();
            save_event(tx, &e)?;
            super::wakes::record_delivery(tx, &e)?;
            // A recurring check-in ACK is bookkeeping, not useful progress.
            // The existing three-no-progress-wake guard requires owner action.
            if handled && matches!(e.kind.as_str(), "source_completed" | "source_lost") {
                for mut t in tasks(tx)? {
                    if t.room_id != room
                        || t.agent_id != agent
                        || t.state == ExecutionState::Blocked
                    {
                        continue;
                    }
                    let mut changed = false;
                    for o in &mut t.obligations {
                        if o.completion_source() == Some(&e.source_id)
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
                    if changed || belongs {
                        let timer_outcome = e.payload["public_answer"]["kind"].as_str() == Some("timer")
                            || tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_wakes WHERE id=?1 AND json_extract(payload,'$.kind')='timer')", [&e.source_id], |r| r.get::<_,bool>(0)).map_err(sql)?;
                        // Closing a timer obligation is required bookkeeping,
                        // but even rearming one-shot timers cannot buy progress.
                        if !timer_outcome {
                            t.no_progress_wakes = 0;
                            t.progress_sequence += 1;
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

// A wake the provider never accepted did not run: the task keeps its wait and
// the attempt is not a no-progress strike. The pending event retries later.
fn revert_refused_wake(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    prompt: &str,
) -> Result<(), DaemonError> {
    for mut t in tasks(tx)? {
        if t.room_id == room
            && t.agent_id == agent
            && t.state == ExecutionState::Working
            && t.wait.is_some()
            && (t.pending_prompt_id.as_deref() == Some(prompt) || t.prompt_id == prompt)
        {
            t.state = ExecutionState::Waiting;
            t.pending_prompt_id = None;
            t.no_progress_wakes = t.no_progress_wakes.saturating_sub(1);
            t.revision += 1;
            save(tx, &t)?;
        }
    }
    Ok(())
}
