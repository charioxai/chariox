//! Row 470: an agent that got several prompts has a task per prompt, and an earlier prompt's task
//! can still wait on a timer or watched process. A newer wait of the same agent takes those over, so
//! one source firing, or one wait timing out, is one wake; the earlier task's own timeout never
//! starts a turn beside a newer task. Earlier tasks that wait on anything else keep their wait.
use super::*;

/// A live registration of a timer or watched process: the kernel runs the source by itself, and its
/// outcome reaches the agent whichever of its tasks holds the registration.
fn is_wake_source(tx: &Transaction<'_>, reg: &Registration) -> Result<bool, DaemonError> {
    let payload: Option<String> = tx
        .query_row(
            "SELECT payload FROM agent_wakes WHERE id=?1",
            [&reg.source_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql)?;
    let Some(payload) = payload else {
        return Ok(false);
    };
    let wake: AgentWake = decode(&payload)?;
    Ok(reg.id == format!("completion-{}", wake.id)
        && matches!(wake.kind.as_str(), "timer" | "process")
        // A source being cancelled stays with its holder: adoption refuses it until it ends.
        && matches!(wake.state.as_str(), "scheduled" | "starting" | "running"))
}

/// The live registrations of `holder`'s wait when it waits on timers and watched processes only
/// and nothing else of the task needs it: no goal, no unread outcome, no work to settle beyond
/// those sources.
fn foldable(
    tx: &Transaction<'_>,
    holder: &AgentTaskExecution,
) -> Result<Option<Vec<Registration>>, DaemonError> {
    let Some(wait) = holder.wait.as_ref() else {
        return Ok(None);
    };
    if holder.state != ExecutionState::Waiting
        || holder.goal.is_some()
        || holder.pending_prompt_id.is_some()
    {
        return Ok(None);
    }
    let live: Vec<Registration> = registrations(tx, &holder.task_id)?
        .into_iter()
        .filter(|r| r.live)
        .collect();
    if live.is_empty() || settled_waits::unread_outcome(tx, holder, wait)? {
        return Ok(None);
    }
    for reg in &live {
        if !is_wake_source(tx, reg)? {
            return Ok(None);
        }
    }
    let covered = |o: &AgentObligation| {
        live.iter()
            .any(|r| r.obligation_id.as_deref() == Some(o.id.as_str()))
    };
    if holder
        .obligations
        .iter()
        .any(|o| o.requires_resolution() && !covered(o))
    {
        return Ok(None);
    }
    Ok(Some(live))
}

/// Moves what `holder` waits on into `newer` and ends the earlier task quietly. Returns the moved
/// registration ids.
fn take_over(
    tx: &Transaction<'_>,
    newer: &mut AgentTaskExecution,
    own: &mut Vec<Registration>,
    live: Vec<Registration>,
) -> Result<Vec<String>, DaemonError> {
    let mut moved = Vec::new();
    for reg in live {
        if own.iter().any(|o| o.id == reg.id) {
            continue;
        }
        let before = own.len();
        watch_adoption::transfer(tx, newer, own, reg)?;
        moved.extend(own[before..].iter().map(|r| r.id.clone()));
    }
    Ok(moved)
}

/// A wait `newer` just admitted takes over the earlier tasks' timer and process waits.
pub(super) fn fold_older(
    tx: &Transaction<'_>,
    newer: &mut AgentTaskExecution,
    own: &mut Vec<Registration>,
) -> Result<Vec<String>, DaemonError> {
    let mut moved = Vec::new();
    for holder in live_tasks(tx, &newer.room_id, &newer.agent_id)? {
        if holder.task_id == newer.task_id {
            break;
        }
        if holder.owner_user_id != newer.owner_user_id {
            continue;
        }
        if let Some(live) = foldable(tx, &holder)? {
            moved.extend(take_over(tx, newer, own, live)?);
        }
    }
    Ok(moved)
}

/// What the sweep does with an earlier task whose wait reached its deadline.
pub(super) enum Timeout {
    /// Nothing newer holds the agent: the task times out as before.
    Own,
    /// A newer waiting task took its sources over.
    Folded,
    /// A newer task is working and will wait or end; no turn for this timeout.
    Quiet,
}

pub(super) fn timed_out(
    tx: &Transaction<'_>,
    holder: &AgentTaskExecution,
) -> Result<Timeout, DaemonError> {
    let Some(live) = foldable(tx, holder)? else {
        return Ok(Timeout::Own);
    };
    let mut newest = None;
    let mut after = false;
    for other in live_tasks(tx, &holder.room_id, &holder.agent_id)? {
        if other.task_id == holder.task_id {
            after = true;
        } else if after
            && other.owner_user_id == holder.owner_user_id
            && matches!(
                other.state,
                ExecutionState::Working | ExecutionState::Waiting
            )
        {
            newest = Some(other);
        }
    }
    let Some(mut newer) = newest else {
        return Ok(Timeout::Own);
    };
    let waiting = newer.state == ExecutionState::Waiting && newer.wait.is_some();
    if !waiting {
        return Ok(Timeout::Quiet);
    }
    let mut own = registrations(tx, &newer.task_id)?;
    let moved = take_over(tx, &mut newer, &mut own, live)?;
    if let Some(wait) = newer.wait.as_mut() {
        wait.registration_ids.extend(moved.iter().cloned());
    }
    newer.revision += 1;
    save(tx, &newer)?;
    Ok(Timeout::Folded)
}
