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
        } => {
            let mut e = get_event(tx, &room, &agent, sequence)?;
            if e.state != "pending" {
                return Err(error(
                    "delivery is already admitted; reconcile its exact receipt",
                ));
            }
            let urgent_steer = e.urgent && target.is_some() && e.attempted_at_ms.is_none();
            let earlier:i64=tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence<>?3 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.state') IN ('submitting','uncertain','blocked') OR (sequence<?3 AND json_extract(payload,'$.state')='pending' AND (?4=0 OR (json_extract(payload,'$.urgent')=1 AND json_extract(payload,'$.attempted_at_ms') IS NULL))) ELSE 1 END",params![room,agent,sql_integer(sequence)?,urgent_steer],|r|r.get(0)).map_err(sql)?;
            if earlier != 0 {
                return Err(error("earlier recipient delivery must settle first"));
            }
            replies::bind(tx, &e, target.as_deref())?;
            e.state = "submitting".into();
            e.prompt_id = Some(prompt.clone());
            e.target_prompt_id = target;
            e.provider_run_id = run;
            e.attempted_at_ms = Some(now);
            save_event(tx, &e)?;
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
            if !matches!(e.state.as_str(), "submitting" | "uncertain" | "blocked") {
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
        _ => Err(error("invalid delivery operation")),
    }
}
