//! MP-08 / MP-10 / MP-11: catalog refresh retains the admitted working task.
use super::*;

pub(super) fn prepare_catalog(
    tx: &Transaction<'_>,
    room: String,
    agent: String,
    prompt: String,
    run: String,
    next_prompt: String,
    now: u64,
) -> Result<Outcome, DaemonError> {
    let mut task = for_turn(tx, &room, &agent, &prompt)?
        .ok_or_else(|| error("catalog continuation has no admitted source turn"))?;
    if task.provider_run_id.as_deref().is_some_and(|id| id != run) {
        return Err(error("stale catalog continuation provider run"));
    }
    // A catalog change grants no authority to resume an explicit block, wait,
    // cancellation, completed task, or a different retained dispatch intent.
    if task.state != ExecutionState::Working
        || task.wait.is_some()
        || task.pending_prompt_id.is_some()
    {
        return Ok(Outcome::Task(task));
    }
    if next_prompt != format!("task-catalog:{prompt}") {
        return Err(error("catalog continuation must bind its source prompt"));
    }
    // Reloading tools is another wake, not handled task progress. Reuse the
    // durable budget so repeated catalog changes cannot keep a task alive forever.
    if task.no_progress_wakes >= NO_PROGRESS_LIMIT {
        goals::blocked(
            &mut task,
            "Tools changed repeatedly without task progress; finish updating tools, then resume or cancel",
        );
        task.revision += 1;
        save(tx, &task)?;
        return Ok(Outcome::Task(task));
    }
    task.no_progress_wakes += 1;
    if let Some(evaluation) = completion::evaluation_mut(&mut task) {
        if evaluation.prompt_id == prompt && evaluation.result.is_none() {
            evaluation.prompt_id = next_prompt.clone();
        }
    }
    // This newly committed dispatch has its own bounded executor admission
    // window. Completion checks remain separate from the no-progress wake budget.
    task.last_progress_at_ms = task.last_progress_at_ms.max(now);
    task.pending_prompt_id = Some(next_prompt);
    task.revision += 1;
    save(tx, &task)?;
    Ok(Outcome::Task(task))
}

// Revalidate inside the Begin transaction: a timeout can retire a previously
// reserved prompt while the provider reload awaits I/O. That ID must never
// fall through ordinary Begin and manufacture a new task.
pub(super) fn begin(
    tx: &Transaction<'_>,
    task: String,
    room: String,
    agent: String,
    prompt: String,
    run: Option<String>,
    now: u64,
) -> Result<Outcome, DaemonError> {
    let task = load(tx, &task)?;
    if task.room_id != room
        || task.agent_id != agent
        || task.state != ExecutionState::Working
        || (task.prompt_id != prompt && task.pending_prompt_id.as_deref() != Some(&prompt))
    {
        return Err(error("stale or non-working task continuation"));
    }
    super::transitions::apply(
        tx,
        Operation::Begin {
            owner: task.owner_user_id,
            room,
            agent,
            prompt,
            run,
            now,
        },
    )
}
