//! Completion of a retained wait after its last outcome has been handled.
use super::*;

pub(super) fn finish(
    tx: &Transaction<'_>,
    task: &mut AgentTaskExecution,
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
    // A message turn runs beside this task (it never continues the wait), so an outcome it
    // handled does not mean the agent's work is done: its parent hears the end when the agent
    // stops working, not while it still does.
    if tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_tasks WHERE room_id=?1 AND agent_id=?2 AND task_id<>?3 AND state='working')",
            params![task.room_id, task.agent_id, task.task_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(sql)?
    {
        return Ok(false);
    }
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
    if unhandled {
        return Ok(false);
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
    for mut task in live_tasks(tx, room, agent)? {
        if finish(tx, &mut task)? {
            task.revision += 1;
            save(tx, &task)?;
        }
    }
    Ok(())
}
