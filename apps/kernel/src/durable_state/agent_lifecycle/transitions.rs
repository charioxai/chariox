//! MP-08 / MP-09 / MP-10 / MP-11 A02: transactional lifecycle policy.
use super::*;
pub(super) fn apply(tx: &Transaction<'_>, op: Operation) -> Result<Outcome, DaemonError> {
    match op {
        Operation::LeasedProcess(mutation) => super::leased_processes::apply(tx, mutation),
        Operation::CreateLeasedWake {
            task,
            prompt,
            wake,
            binding,
        } => super::leased_processes::create_home_wake(tx, task, prompt, wake, binding),
        Operation::BeginContinuation {
            task,
            room,
            agent,
            prompt,
            run,
            now,
        } => super::continuations::begin(tx, task, room, agent, prompt, run, now),
        Operation::PrepareCatalogContinuation {
            room,
            agent,
            prompt,
            run,
            next_prompt,
            now,
        } => super::continuations::prepare_catalog(tx, room, agent, prompt, run, next_prompt, now),
        Operation::AdoptGoal {
            owner,
            room,
            agent,
            run,
            nodes,
            objective,
            now,
        } => super::goals::adopt(tx, owner, room, agent, run, nodes, objective, now),
        Operation::TimeoutGoal { task, prompt, now } => {
            super::goals::timeout(tx, task, prompt, now)
        }
        Operation::BindGoalCheckLease {
            task,
            prompt,
            lease,
        } => super::goals::bind_check_lease(tx, task, prompt, lease),
        Operation::RetryGoalCheck {
            task,
            prompt,
            lease,
            now,
        } => super::goals::retry_check(tx, task, prompt, lease, now),
        Operation::BeginGoalApproval { task, prompt, now } => {
            super::goals::native_approval::begin(tx, task, prompt, now)
        }
        Operation::AnswerGoalApproval {
            task,
            prompt,
            revision,
            now,
        } => super::goals::native_approval::answer(tx, task, prompt, revision, now),
        Operation::BeginOwnerWait { task, prompt, now } => {
            super::goals::begin_owner_wait(tx, task, prompt, now)
        }
        Operation::EndOwnerWait {
            task,
            prompt,
            refusal,
            now,
        } => super::goals::end_owner_wait(tx, task, prompt, refusal, now),
        Operation::StartGoal {
            task,
            owner,
            objective,
            minutes,
            now,
        } => super::goals::start(tx, task, owner, objective, minutes, now),
        Operation::EvaluateGoal {
            task,
            prompt,
            result,
            now,
        } => super::goals::evaluate(tx, task, prompt, result, now),
        Operation::ControlGoal {
            task,
            owner,
            revision,
            action,
            value,
            now,
        } => super::goals::control(tx, task, owner, revision, action, value, now),
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
                    t.provider_run_id = None;
                    t.revision += 1;
                    t
                } else {
                    new_admitted_task(tx, room, agent, prompt, run.clone(), now)?
                }
            };
            // Queue recovery can race an owner cancellation/block. Provider
            // dispatch revalidates the durable task immediately before I/O;
            // admission alone is not authority to run it later.
            if run.is_some() && task.state != ExecutionState::Working {
                return Err(error("non-working task cannot dispatch a provider turn"));
            }
            // A rejected local wake retries with the same prompt id, so the
            // existing-task lookup can also carry a new continuation intent.
            if task.pending_prompt_id.as_deref() == Some(task.prompt_id.as_str()) {
                task.pending_prompt_id = None;
                task.provider_run_id = None;
                task.revision += 1;
            }
            // The retained wait is consumed only by an actual provider dispatch;
            // a refused wake before that returns the task to this wait. A named
            // run proves admission, not provider I/O: a wake still unsubmitted
            // keeps its wait until BindSubmission.
            if run.is_some() && !super::goals::is_check(&task, &task.prompt_id) {
                super::goals::clock(&mut task, now);
                if task.wait.is_some() {
                    let unsubmitted_wake: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.prompt_id')=?3 AND json_extract(payload,'$.state')='submitting' AND json_extract(payload,'$.submit_epoch') IS NULL ELSE 0 END)",
                        params![task.room_id, task.agent_id, task.prompt_id],
                        |row| row.get(0),
                    ).map_err(sql)?;
                    if !unsubmitted_wake {
                        task.wait = None;
                    }
                }
            }
            if task.owner_user_id.is_empty() {
                task.owner_user_id = owner;
            }
            task.provider_run_id = run.or(task.provider_run_id);
            save(tx, &task)?;
            Ok(Outcome::Task(task))
        }
        Operation::BindDelegate {
            parent_task,
            child_task,
        } => {
            super::delegation::bind(tx, &parent_task, &child_task)?;
            Ok(Outcome::Saved)
        }
        Operation::Withdraw { task } => {
            let t = load(tx, &task)?;
            // Only an untouched fresh admission can be withdrawn; any recorded
            // work keeps the task supervised.
            if t.task_id != t.prompt_id
                || t.state != ExecutionState::Working
                || t.revision
                    != if t.goal.as_ref().is_some_and(|g| {
                        g.check_count == 0 && g.continuation_sequence == 0 && !g.paused
                    }) && t.provider_run_id.is_none()
                    {
                        2
                    } else {
                        1
                    }
                || !t.obligations.is_empty()
                || t.wait.is_some()
                || t.pending_prompt_id.is_some()
            {
                return Err(error("task already holds supervised work"));
            }
            tx.execute("DELETE FROM agent_tasks WHERE task_id=?1", [&task])
                .map_err(sql)?;
            tx.execute("DELETE FROM agent_task_workflows WHERE task_id=?1", [&task])
                .map_err(sql)?;
            Ok(Outcome::Saved)
        }
        Operation::ReconcileFailedWorkflow {
            owner_id,
            room,
            run,
        } => {
            for mut task in tasks(tx)?.into_iter().filter(|t| t.room_id == room) {
                if super::workflow_tasks::settle_unstarted_delegates(
                    tx, &mut task, &owner_id, &run,
                )? {
                    task.revision += 1;
                    save(tx, &task)?;
                }
            }
            Ok(Outcome::Saved)
        }
        Operation::BindWorkflow {
            task,
            prompt,
            run,
            node,
        } => super::workflow_tasks::bind(tx, &task, &prompt, &run, &node),
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
                    // Receipts are monotonic: a duplicate or late receipt for a
                    // settled obligation cannot rewrite its outcome.
                    if o.status != "open"
                        || matches!(o.dispatch_state.as_str(), "accepted" | "rejected")
                    {
                        return Ok(Outcome::Task(task));
                    }
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
                    if accepted
                        && (o.kind == "delegate" || o.kind == "hand_off" || o.tracks_workflow_run())
                    {
                        if let Some(source) = o.resource_id.clone() {
                            let reg = Registration {
                                id: format!("completion-{}", o.id),
                                task_id: task.task_id.clone(),
                                source_id: source,
                                obligation_id: Some(o.id.clone()),
                                // Spawn has no prompt: every prior agent-ID
                                // outcome belongs to an earlier incarnation.
                                source_cursor: if o.kind == "delegate" {
                                    let seq: i64 = tx.query_row("SELECT COALESCE(MAX(sequence),0) FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2", params![task.room_id, o.resource_id], |r| r.get(0)).map_err(sql)?;
                                    u64::try_from(seq)
                                        .map_err(|_| error("corrupt source sequence"))?
                                } else {
                                    0
                                },
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
            mut registration,
        } => {
            let t = load(tx, &task)?;
            current(&t, &prompt)?;
            if registration.task_id != task || !registration.live {
                return Err(error("source is not live"));
            }
            super::subscriptions::bind(tx, &t, &mut registration)?;
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
            if t.wait.is_some() {
                return Err(error("Wait already accepted. End this provider turn now without more tools; the kernel delivers the next source or deadline turn."));
            }
            if ids.is_empty()
                && t.obligations
                    .iter()
                    .all(|o| !matches!(o.status.as_str(), "open" | "settling" | "failed"))
            {
                return Err(error(
                    "nothing to wait on: every tracked obligation is resolved; reply with the final answer",
                ));
            }
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
                return Err(error(yield_refusal(tx, &t, &ids, &regs)?));
            }
            // Goal check-ins have a separate clock. Preserve the admitted wait
            // deadline so an early check-in cannot turn a live source into a timeout.
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
            super::goals::clock(&mut t, crate::session::unix_epoch_ms());
            t.state = ExecutionState::Blocked;
            t.blocked_revision = t.revision + 1;
            t.reason = crate::secret_redaction::redact_secrets(&reason).into_owned();
            t.revision += 1;
            save(tx, &t)?;
            Ok(Outcome::Task(t))
        }
        Operation::EndFailedTurn {
            task,
            prompt,
            reason,
        } => {
            let mut t = load(tx, &task)?;
            if t.prompt_id != prompt || t.state != ExecutionState::Working {
                return Err(error("stale failed-turn intent"));
            }
            super::goals::clock(&mut t, crate::session::unix_epoch_ms());
            t.reason = crate::secret_redaction::redact_secrets(&reason).into_owned();
            if is_plain(tx, &t)? {
                t.state = ExecutionState::Cancelled;
                t.pending_prompt_id = None;
            } else {
                t.state = ExecutionState::Blocked;
                t.blocked_revision = t.revision + 1;
            }
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
            let valid_wait = if let Some(w) = t
                .wait
                .as_ref()
                .filter(|_| !cancelled && t.state != ExecutionState::Blocked)
            {
                let regs = registrations(tx, &t.task_id)?;
                let pending:i64 = tx.query_row("SELECT count(*) FROM agent_inbox WHERE room_id=?1 AND agent_id=?2 AND sequence>?3 AND json_extract(payload,'$.state') NOT IN ('handled','expired','failed') AND json_extract(payload,'$.kind') IN ('source_completed','source_lost') AND (json_extract(payload,'$.payload.task_id')=?4 OR EXISTS(SELECT 1 FROM json_each(json_extract(payload,'$.payload.task_ids')) WHERE value=?4))",params![t.room_id,t.agent_id,sql_integer(w.inbox_cursor)?,t.task_id],|r|r.get(0)).map_err(sql)?;
                // MP-08/MP-09/MP-10/MP-11 F03: the deadline was checked
                // at admission. A late native boundary commits that same
                // wait; the normal sweep delivers its elapsed deadline.
                w.deadline_ms > w.started_at_ms
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
            if !cancelled && t.state == ExecutionState::Working && t.goal.is_some() {
                t.provider_run_id = Some(run);
                let correction = super::goals::work_end(&mut t, has_answer, valid_wait, now);
                t.revision += 1;
                save(tx, &t)?;
                return Ok(Outcome::Settled {
                    task: t,
                    correction,
                });
            }
            let mut correction = false;
            if cancelled
                && (t.goal.as_ref().is_some_and(|g| g.paused)
                    || (super::goals::is_check(&t, &prompt) && t.state == ExecutionState::Blocked))
            {
                return Ok(Outcome::Settled {
                    task: t,
                    correction: false,
                });
            }
            if cancelled {
                super::supervision::cancel_intent(tx, &mut t)?;
            } else if t.state != ExecutionState::Blocked {
                let open = t
                    .obligations
                    .iter()
                    .any(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"));
                let plain = is_plain(tx, &t)?;
                if has_answer && (plain || (!open && t.wait.is_none())) {
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
                    t.pending_prompt_id = Some(format!(
                        "{}{}",
                        super::goals::CORRECTION_PROMPT_PREFIX,
                        t.task_id
                    ));
                    t.reason = if plain {
                        "Turn ended without a final answer; correct once".into()
                    } else {
                        "Turn ended without completing its obligations or a valid live wait; correct once".into()
                    };
                } else {
                    t.state = ExecutionState::Blocked;
                    t.blocked_revision = t.revision + 1;
                    t.wait = None;
                    t.reason = if plain {
                        super::goals::EMPTY_FINAL_ANSWER_REASON.into()
                    } else {
                        "Repeated invalid turn end; owner must resume or cancel".into()
                    };
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
        | Operation::Expire { .. }
        | Operation::Defer { .. }
        | Operation::BindSubmission { .. }
        | Operation::Receipt { .. }
        | Operation::Ack { .. }) => super::delivery::apply(tx, op),
        op @ (Operation::Progress { .. }
        | Operation::SourceOutcome { .. }
        | Operation::Sweep { .. }
        | Operation::CancelTask { .. }
        | Operation::OwnerResponse { .. }) => super::supervision::apply(tx, op),
        op @ (Operation::CreateWake { .. }
        | Operation::ArmWake { .. }
        | Operation::VerifyWakes { .. }
        | Operation::FireWakes { .. }
        | Operation::WakeVerified { .. }
        | Operation::WakeOccurred { .. }
        | Operation::ProcessStarted { .. }
        | Operation::ProcessMatched { .. }
        | Operation::ProcessExited { .. }
        | Operation::CancelWake { .. }
        | Operation::WakeAlerted { .. }
        | Operation::RetireWakes { .. }) => super::wakes::apply(tx, op),
    }
}

pub(super) fn recover_source(
    tx: &Transaction<'_>,
    task: &mut AgentTaskExecution,
    mut registration: Registration,
) -> Result<(), DaemonError> {
    let source: Option<(i64,String,bool,Option<String>)> = tx.query_row("SELECT sequence,occurrence_id,success,public_answer FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2 AND sequence>?3 ORDER BY sequence DESC LIMIT 1",params![task.room_id,registration.source_id,sql_integer(registration.source_cursor)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(sql)?;
    if let Some((seq, id, success, answer)) = source {
        if success && super::delegation::unbound(task, &registration) {
            return Ok(());
        }
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

/// A02-S02: a message wakes a yielded agent in a new turn. Waiting again on
/// the registrations of its earlier task is not this turn's wait: that task
/// still waits, so the turn only has to end.
fn yield_refusal(
    tx: &Transaction<'_>,
    t: &AgentTaskExecution,
    ids: &[String],
    own: &[Registration],
) -> Result<String, DaemonError> {
    let foreign: Vec<&String> = ids
        .iter()
        .filter(|id| !own.iter().any(|r| &r.id == *id))
        .collect();
    let unresolved = t
        .obligations
        .iter()
        .any(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"));
    if !foreign.is_empty() && !unresolved {
        for other in tasks(tx)?.into_iter().filter(|o| {
            o.task_id != t.task_id
                && o.room_id == t.room_id
                && o.agent_id == t.agent_id
                && o.state == ExecutionState::Waiting
        }) {
            let theirs = registrations(tx, &other.task_id)?;
            if foreign.iter().all(|id| theirs.iter().any(|r| &r.id == *id)) {
                return Ok("Your earlier task is still waiting on these registrations; this turn has nothing of its own to wait on. Do not yield: finish this turn now (an answer is optional) and the earlier wait continues.".into());
            }
        }
    }
    Ok("yield sources must cover every unresolved obligation".into())
}
