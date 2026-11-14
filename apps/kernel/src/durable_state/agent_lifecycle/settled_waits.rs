//! Completion of a retained wait after its last outcome has been handled.
use super::*;

/// Whether `task` waits on nothing more: every source it waited on ended and every outcome was
/// handled. It still ends only when the agent holds nothing else (`finish`).
pub(super) fn is_settled(
    tx: &Transaction<'_>,
    task: &AgentTaskExecution,
) -> Result<bool, DaemonError> {
    if task.state != ExecutionState::Waiting
        || task.goal.is_some()
        || task.pending_prompt_id.is_some()
        || task
            .obligations
            .iter()
            .any(AgentObligation::requires_resolution)
    {
        return Ok(false);
    }
    let Some(wait) = task.wait.as_ref() else {
        return Ok(false);
    };
    let regs = registrations(tx, &task.task_id)?;
    if !wait
        .registration_ids
        .iter()
        .all(|id| regs.iter().any(|reg| &reg.id == id && !reg.live))
    {
        return Ok(false);
    }
    // A source can have no obligation (an observation), or its obligation can
    // already be cancelled. Its outcome still belongs to the waiting agent.
    let unhandled: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_inbox i
         WHERE i.room_id=?1 AND i.agent_id=?2
         AND json_extract(i.payload,'$.kind') IN ('source_completed','source_lost','child_finished','peer_event')
         AND json_extract(i.payload,'$.state') NOT IN ('handled','expired','failed')
         AND (json_extract(i.payload,'$.payload.task_id')=?3
              OR EXISTS(SELECT 1 FROM json_each(json_extract(i.payload,'$.payload.task_ids')) WHERE value=?3)
              OR EXISTS(SELECT 1 FROM agent_registrations r WHERE r.task_id=?3
                  AND r.id IN (SELECT value FROM json_each(?4))
                  AND (r.source_id=i.source_id OR r.source_id=json_extract(i.payload,'$.payload.child_task_id')))))",
        params![task.room_id, task.agent_id, task.task_id, encode(&wait.registration_ids)?],
        |row| row.get(0),
    ).map_err(sql)?;
    Ok(!unhandled)
}

pub(super) fn finish(
    tx: &Transaction<'_>,
    task: &mut AgentTaskExecution,
) -> Result<bool, DaemonError> {
    if !is_settled(tx, task)? {
        return Ok(false);
    }
    // The agent's parent hears the end when the agent stops working, not while it still does: a
    // turn or a message turn runs beside this task, another task still waits on a source or
    // owes its owner an answer. A task that is itself settled holds nothing back.
    for other in live_tasks(tx, &task.room_id, &task.agent_id)? {
        if other.task_id != task.task_id && !is_settled(tx, &other)? {
            return Ok(false);
        }
    }
    // A pending deadline/recheck is no longer work once the wait has ended.
    // Admitted or uncertain deliveries retain their normal reconciliation.
    let mut query = tx.prepare(
        "SELECT sequence FROM agent_inbox WHERE room_id=?1 AND agent_id=?2
         AND json_extract(payload,'$.kind') IN ('deadline_reached','wait_recheck')
         AND json_extract(payload,'$.state')='pending'
         AND (json_extract(payload,'$.payload.task_id')=?3
              OR EXISTS(SELECT 1 FROM json_each(json_extract(payload,'$.payload.task_ids')) WHERE value=?3))"
    ).map_err(sql)?;
    let sequences = query
        .query_map(params![task.room_id, task.agent_id, task.task_id], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    drop(query);
    for sequence in sequences {
        let sequence = u64::try_from(sequence).map_err(|_| error("corrupt inbox sequence"))?;
        super::delivery::apply(
            tx,
            Operation::Expire {
                room: task.room_id.clone(),
                agent: task.agent_id.clone(),
                sequence,
            },
        )?;
    }
    task.state = ExecutionState::Done;
    task.wait = None;
    task.reason = RESOLVED_WAIT_REASON.into();
    Ok(true)
}

/// A turn that ends may have been the work that held back another task's finished wait.
pub(super) fn finish_agent_waits(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
) -> Result<(), DaemonError> {
    loop {
        let mut finished = false;
        for mut task in live_tasks(tx, room, agent)? {
            if finish(tx, &mut task)? {
                task.revision += 1;
                save(tx, &task)?;
                finished = true;
            }
        }
        if !finished {
            return Ok(());
        }
    }
}
