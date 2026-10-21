//! MP-08 / MP-09 / MP-10 / MP-11 A12: goal policy on the shared task ledger.
use super::*;
mod adoption;
mod controls;
pub(super) use adoption::adopt;
pub(super) use controls::control;
use serde::{Deserialize, Serialize};

pub(crate) const CHECK_BOUND_MS: u64 = 60_000;
pub(crate) const RESULT_LIMIT: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentGoal {
    pub schema_version: u32,
    pub id: String,
    pub owner_user_id: String,
    pub room_id: String,
    pub agent_id: String,
    pub objective: String,
    pub original_prompt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_run_id: Option<String>,
    pub objective_revision: u64,
    pub created_at_ms: u64,
    pub active_elapsed_ms: u64,
    pub waiting_elapsed_ms: u64,
    pub state_since_ms: u64,
    pub check_in_minutes: u32,
    pub next_check_in_ms: Option<u64>,
    pub paused: bool,
    pub continuation_sequence: u64,
    pub check_count: u64,
    pub evaluation: Option<GoalEvaluation>,
    pub final_answer: Option<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalEvaluation {
    pub origin: String,
    pub prompt_id: String,
    pub work_prompt_id: String,
    pub work_provider_run_id: Option<String>,
    pub objective_revision: u64,
    pub ledger_revision: u64,
    pub started_at_ms: u64,
    pub deadline_ms: u64,
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Assessment {
    Achieved {
        final_answer: String,
        evidence: Vec<String>,
    },
    Continue {
        reason: String,
    },
    Waiting {
        reason: String,
        registration_ids: Vec<String>,
        deadline_ms: u64,
    },
    Blocked {
        reason: String,
    },
}

pub(super) fn start(
    tx: &Transaction<'_>,
    task: String,
    owner: String,
    objective: String,
    minutes: u32,
    now: u64,
) -> Result<Outcome, DaemonError> {
    let mut t = load(tx, &task)?;
    if t.owner_user_id != owner
        || owner.is_empty()
        || !(1..=60).contains(&minutes)
        || objective.trim().is_empty()
        || objective.len() > RESULT_LIMIT
    {
        return Err(error(
            "goal needs an owner objective and a 1–60 minute interval",
        ));
    }
    if let Some(g) = &t.goal {
        return if g.objective == objective && g.check_in_minutes == minutes {
            Ok(Outcome::Task(t))
        } else {
            Err(error(
                "conflicting goal admission; edit the objective explicitly",
            ))
        };
    }
    if t.state != ExecutionState::Working || t.task_id != t.prompt_id || !t.obligations.is_empty() {
        return Err(error("goal admission must precede work"));
    }
    t.goal = Some(AgentGoal {
        schema_version: 1,
        id: format!("goal:{}", t.task_id),
        owner_user_id: owner,
        room_id: t.room_id.clone(),
        agent_id: t.agent_id.clone(),
        objective: crate::secret_redaction::redact_secrets(&objective).into_owned(),
        original_prompt_id: t.prompt_id.clone(),
        legacy_run_id: None,
        objective_revision: 1,
        created_at_ms: now,
        active_elapsed_ms: 0,
        waiting_elapsed_ms: 0,
        state_since_ms: now,
        check_in_minutes: minutes,
        next_check_in_ms: None,
        paused: false,
        continuation_sequence: 0,
        check_count: 0,
        evaluation: None,
        final_answer: None,
        evidence: vec![],
    });
    t.revision += 1;
    save(tx, &t)?;
    Ok(Outcome::Task(t))
}

pub(super) fn clock(t: &mut AgentTaskExecution, now: u64) {
    if let Some(g) = &mut t.goal {
        let elapsed = now.saturating_sub(g.state_since_ms);
        match t.state {
            ExecutionState::Working => {
                g.active_elapsed_ms = g.active_elapsed_ms.saturating_add(elapsed)
            }
            ExecutionState::Waiting => {
                g.waiting_elapsed_ms = g.waiting_elapsed_ms.saturating_add(elapsed)
            }
            _ => (),
        }
        g.state_since_ms = now;
    }
}

pub(super) fn blocked(t: &mut AgentTaskExecution, reason: &str) {
    t.state = ExecutionState::Blocked;
    t.blocked_revision = t.revision + 1;
    t.pending_prompt_id = None;
    t.reason = reason.into();
    if let Some(g) = &mut t.goal {
        g.next_check_in_ms = None;
    }
}

/// One persisted intent per work turn; duplicate EOF/ACK cannot spend again.
pub(super) fn work_end(t: &mut AgentTaskExecution, has_answer: bool, now: u64) -> bool {
    clock(t, now);
    if !has_answer {
        blocked(
            t,
            "Goal provider ended without a public answer; reconcile failure and explicitly resume",
        );
        return false;
    }
    if t.goal.as_ref().is_some_and(|g| g.paused) {
        return false;
    }
    if completion::evaluation(t).is_some_and(|e| e.work_prompt_id == t.prompt_id) {
        return false;
    }
    let revision = completion::objective_revision(t);
    let count = t.goal.as_ref().map_or(0, |g| g.check_count) + 1;
    let prompt = format!("goal-check-{}-{}-{}", t.task_id, revision, count);
    let check = GoalEvaluation {
        origin: "completion_check".into(),
        prompt_id: prompt.clone(),
        work_prompt_id: t.prompt_id.clone(),
        work_provider_run_id: t.provider_run_id.clone(),
        objective_revision: revision,
        ledger_revision: t.revision + 2,
        started_at_ms: now,
        deadline_ms: now.saturating_add(CHECK_BOUND_MS),
        result: None,
    };
    if let Some(g) = &mut t.goal {
        g.check_count = count;
        g.evaluation = Some(check);
    }
    t.pending_prompt_id = Some(prompt);
    t.state = ExecutionState::Working;
    t.reason = "Evaluating original goal objective".into();
    true
}

pub(crate) fn is_check(t: &AgentTaskExecution, prompt: &str) -> bool {
    completion::evaluation(t).is_some_and(|e| e.prompt_id == prompt)
}

pub(super) fn evaluate(
    tx: &Transaction<'_>,
    task: String,
    prompt: String,
    mut result: String,
    now: u64,
) -> Result<Outcome, DaemonError> {
    let mut t = load(tx, &task)?;
    let e = completion::evaluation(&t).ok_or_else(|| error("no evaluation intent"))?;
    if e.prompt_id != prompt {
        return Err(error("uncorrelated completion check"));
    }
    if e.result.is_some()
        || t.state != ExecutionState::Working
        || t.pending_prompt_id.as_deref().unwrap_or(&t.prompt_id) != prompt
    {
        return Ok(Outcome::Settled {
            task: t,
            correction: false,
        });
    }
    clock(&mut t, now);
    let e = completion::evaluation(&t).unwrap();
    let valid = t.prompt_id == prompt
        && e.objective_revision == completion::objective_revision(&t)
        && e.ledger_revision <= t.revision
        && now <= e.deadline_ms
        && result.len() <= RESULT_LIMIT;
    result = crate::secret_redaction::redact_secrets(&result).into_owned();
    let result = result.trim();
    let result = result
        .strip_prefix("```json")
        .or_else(|| result.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(result)
        .trim();
    let assessment = if valid && result.len() <= RESULT_LIMIT {
        serde_json::from_str::<Assessment>(result).ok()
    } else {
        None
    };
    let mut again = false;
    match assessment {
        None => blocked(&mut t, "Goal evaluation timed out, was malformed or stale; inspect the receipt and explicitly resume"),
        Some(Assessment::Blocked { reason }) if !reason.trim().is_empty() => blocked(&mut t, &reason),
        Some(Assessment::Achieved { final_answer, evidence }) if !final_answer.trim().is_empty() && !evidence.is_empty() && evidence.iter().all(|e| !e.trim().is_empty()) => {
            if t.obligations.iter().any(|o| matches!(o.status.as_str(), "open" | "settling" | "failed")) {
                if t.correction_used {
                    blocked(&mut t, "Repeated achieved claim with unresolved obligations; owner must resume or cancel");
                } else {
                    again = continue_work(&mut t, "Completion was rejected: resolve every open obligation, then wait, continue or block");
                    t.correction_used = true;
                }
            } else {
                t.state = ExecutionState::Done;
                t.wait = None;
                t.reason.clear();
                if let Some(g) = &mut t.goal { g.final_answer = Some(final_answer); g.evidence = evidence; g.next_check_in_ms = None; }
            }
        }
        Some(Assessment::Continue { reason }) if !reason.trim().is_empty() => {
            again = continue_work(&mut t, &reason);
        }
        Some(Assessment::Waiting { reason, registration_ids, deadline_ms }) if !reason.trim().is_empty() => {
            let interval = t.goal.as_ref().map_or(10, |g| g.check_in_minutes) as u64 * 60_000;
            let deadline = deadline_ms.min(now.saturating_add(interval))
                .min(t.wait.as_ref().map_or(u64::MAX, |w| w.deadline_ms))
                .min(wakes::source_deadline(tx, &t, &registration_ids)?.unwrap_or(u64::MAX));
            let regs = registrations(tx, &t.task_id)?;
            let valid_sources = !registration_ids.is_empty() && deadline > now
                && registration_ids.iter().all(|id| regs.iter().any(|r| &r.id == id && r.live))
                && t.obligations.iter().filter(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"))
                    .all(|o| regs.iter().any(|r| r.live && registration_ids.contains(&r.id) && r.obligation_id.as_deref() == Some(&o.id)));
            if !valid_sources {
                blocked(&mut t, "Goal waiting needs live sources covering all obligations and a finite deadline");
            } else if t.no_progress_wakes >= NO_PROGRESS_LIMIT {
                blocked(&mut t, "Goal check-ins made no progress; owner must inspect and explicitly resume");
            } else {
                let prior = t.wait.take();
                t.wait = Some(AgentWait {
                    registration_ids, deadline_ms: deadline, started_at_ms: prior.as_ref().map_or(now, |w| w.started_at_ms),
                    inbox_cursor: prior.as_ref().map_or(0, |w| w.inbox_cursor),
                    long_wait_notified: prior.as_ref().is_some_and(|w| w.long_wait_notified), last_checked_at_ms: now,
                });
                t.reason = reason;
                t.state = ExecutionState::Waiting;
                if let Some(g) = &mut t.goal { g.next_check_in_ms = Some(deadline); }
            }
        }
        _ => blocked(&mut t, "Goal evaluation omitted its required answer/evidence or owner action; explicitly resume"),
    }
    let receipt = if result.len() > RESULT_LIMIT {
        serde_json::json!({"invalid":true,"reason":"completion_check result exceeds 8192 bytes"})
    } else {
        serde_json::from_str(result).unwrap_or_else(|_| serde_json::json!({"invalid":true}))
    };
    completion::evaluation_mut(&mut t).unwrap().result = Some(receipt);
    t.revision += 1;
    migration::audit(
        tx,
        "goal.evaluation",
        &prompt,
        serde_json::json!({"task_id":t.task_id,"goal_id":t.goal.as_ref().map(|g| &g.id),"objective_revision":completion::objective_revision(&t),"evaluation":completion::evaluation(&t),"disposition":t.state,"task_revision":t.revision}),
    )?;
    save(tx, &t)?;
    Ok(Outcome::Settled {
        task: t,
        correction: again,
    })
}

fn continue_work(t: &mut AgentTaskExecution, reason: &str) -> bool {
    if t.no_progress_wakes >= NO_PROGRESS_LIMIT {
        blocked(t, "Goal repeated continuation without handled progress; owner must inspect and explicitly resume");
        return false;
    }
    t.no_progress_wakes += 1;
    t.wait = None;
    t.reason = reason.into();
    let g = t.goal.as_mut().unwrap();
    g.continuation_sequence += 1;
    g.next_check_in_ms = None;
    t.pending_prompt_id = Some(format!(
        "goal-continue-{}-{}",
        t.task_id, g.continuation_sequence
    ));
    true
}

pub(super) fn timeout(
    tx: &Transaction<'_>,
    task: String,
    prompt: String,
    now: u64,
) -> Result<Outcome, DaemonError> {
    let mut t = load(tx, &task)?;
    let Some(check) = completion::evaluation(&t) else {
        return Ok(Outcome::Saved);
    };
    if t.state != ExecutionState::Working
        || check.prompt_id != prompt
        || t.pending_prompt_id.as_deref().unwrap_or(&t.prompt_id) != prompt
        || check.result.is_some()
        || now < check.deadline_ms
    {
        return Ok(Outcome::Saved);
    }
    clock(&mut t, now);
    blocked(&mut t, "Goal completion check exceeded 60 seconds; inspect the exact delivery receipt and explicitly resume");
    completion::evaluation_mut(&mut t).unwrap().result = Some(serde_json::json!({
        "status":"blocked","reason":"completion_check service bound exceeded","received_at_ms":now }));
    t.revision += 1;
    migration::audit(
        tx,
        "goal.evaluation",
        &prompt,
        serde_json::json!({"task_id":t.task_id,"goal_id":t.goal.as_ref().map(|g| &g.id),
        "evaluation":completion::evaluation(&t),"disposition":t.state,"task_revision":t.revision}),
    )?;
    save(tx, &t)?;
    Ok(Outcome::Task(t))
}
