//! A native approval blocks the goal while its existing provider turn waits.
use super::*;

pub(in crate::durable_state::agent_lifecycle) fn begin(
    tx: &Transaction<'_>,
    task: String,
    prompt: String,
    now: u64,
) -> Result<Outcome, DaemonError> {
    let mut t = load(tx, &task)?;
    if t.goal.is_none()
        || t.state != ExecutionState::Working
        || t.goal.as_ref().is_some_and(|g| g.paused)
        || t.pending_prompt_id.as_deref().unwrap_or(&t.prompt_id) != prompt
    {
        return Ok(Outcome::Saved);
    }
    clock(&mut t, now);
    t.state = ExecutionState::Blocked;
    t.reason = "Waiting for the owner to answer the native approval".into();
    t.revision += 1;
    t.blocked_revision = t.revision;
    t.goal.as_mut().unwrap().next_check_in_ms = None;
    // Keep the prompt binding: answering continues this turn, without dispatching another.
    save(tx, &t)?;
    Ok(Outcome::Task(t))
}

pub(in crate::durable_state::agent_lifecycle) fn answer(
    tx: &Transaction<'_>,
    task: String,
    prompt: String,
    revision: u64,
    now: u64,
) -> Result<Outcome, DaemonError> {
    let mut t = load(tx, &task)?;
    if t.goal.is_none()
        || t.state != ExecutionState::Blocked
        || t.blocked_revision != revision
        || t.goal.as_ref().is_some_and(|g| g.paused)
        || t.pending_prompt_id.as_deref().unwrap_or(&t.prompt_id) != prompt
    {
        return Ok(Outcome::Saved);
    }
    clock(&mut t, now);
    t.state = ExecutionState::Working;
    t.blocked_revision = 0;
    t.reason = "Owner answered the native approval; continuing the same turn".into();
    t.revision += 1;
    save(tx, &t)?;
    Ok(Outcome::Task(t))
}
