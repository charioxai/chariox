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
            // The retained wait is consumed only by an actual provider dispatch;
            // a refused wake before that returns the task to this wait.
            if run.is_some() {
                task.wait = None;
            }
            if task.owner_user_id.is_empty() {
                task.owner_user_id = owner;
            }
            task.provider_run_id = run.or(task.provider_run_id);
            save(tx, &task)?;
            Ok(Outcome::Task(task))
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
                return Err(error(
                    "yield sources must cover every unresolved obligation",
                ));
            }
            let deadline = deadline.min(now.saturating_add(MAX_WAIT_MS));
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
                super::supervision::cancel_intent(tx, &mut t)?;
            } else if t.state != ExecutionState::Blocked {
                let open = t
                    .obligations
                    .iter()
                    .any(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"));
                let valid_wait = if let Some(w) = &t.wait {
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
                // A turn that owns no obligation and no wait ends as on any
                // plain turn; only a task that registered one is held to a
                // valid end.
                if !open && t.wait.is_none() {
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
                    t.pending_prompt_id = Some(format!("{CORRECTION_PROMPT_PREFIX}{}", t.task_id));
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
        #[cfg(test)]
        Operation::Occur(e) => Ok(Outcome::Event(event(tx, e)?)),
        op @ (Operation::Attempt { .. }
        | Operation::Expire { .. }
        | Operation::BindSubmission { .. }
        | Operation::Receipt { .. }
        | Operation::Ack { .. }) => super::delivery::apply(tx, op),
        op @ (Operation::Progress { .. }
        | Operation::SourceOutcome { .. }
        | Operation::Sweep { .. }
        | Operation::CancelTask { .. }
        | Operation::OwnerResponse { .. }) => super::supervision::apply(tx, op),
        op @ (Operation::ArmWake { .. }
        | Operation::VerifyWakes { .. }
        | Operation::FireWakes { .. }
        | Operation::WakeVerified { .. }
        | Operation::WakeOccurred { .. }
        | Operation::CancelWake { .. }
        | Operation::WakeAlerted { .. }
        | Operation::RetireWakes { .. }) => super::wakes::apply(tx, op),
    }
}
