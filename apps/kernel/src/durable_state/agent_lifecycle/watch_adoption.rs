//! Transfer a retained watch and its source binding in the transaction that admits the wait.
use super::*;

pub(super) fn adopt(
    tx: &Transaction<'_>,
    task: &mut AgentTaskExecution,
    ids: &[String],
    own: &mut Vec<Registration>,
) -> Result<(), DaemonError> {
    let mut claims = Vec::new();
    let mut retry = Vec::new();
    let mut refusals = Vec::new();
    let mut unknown = Vec::new();
    for id in ids {
        if let Some(reg) = own.iter().find(|r| &r.id == id) {
            if reg.live {
                retry.push(id);
            }
            continue;
        }
        let Some(reg) = transitions::registration_by_id(tx, id)? else {
            unknown.push(id);
            continue;
        };
        let holder = load(tx, &reg.task_id)?;
        if holder.room_id != task.room_id
            || holder.agent_id != task.agent_id
            || holder.owner_user_id != task.owner_user_id
        {
            unknown.push(id);
            continue;
        }
        if let Some(refusal) = why_unavailable(tx, &holder, &reg)? {
            refusals.push(refusal);
        } else {
            retry.push(id);
            claims.push(reg);
        }
    }
    if !unknown.is_empty() {
        refusals.push(format!("unknown registration_ids {}.", encode(&unknown)?));
    }
    if !refusals.is_empty() {
        let next = if !retry.is_empty() {
            format!("Retry with registration_ids {}.", encode(&retry)?)
        } else if own.iter().any(|r| r.live) {
            "Use only this task's live registrations from chariox.events.subscriptions if you have work to wait on.".into()
        } else {
            "If you were only waiting on the earlier watches, finish this turn. They keep their owners.".into()
        };
        return Err(error(format!("{} {next}", refusals.join(" "))));
    }

    for reg in claims {
        // A second named registration for the same obligation moved with the first.
        if !own.iter().any(|owned| owned.id == reg.id) {
            transfer(tx, task, own, reg)?;
        }
    }
    Ok(())
}

fn why_unavailable(
    tx: &Transaction<'_>,
    holder: &AgentTaskExecution,
    reg: &Registration,
) -> Result<Option<String>, DaemonError> {
    let watch = format!("`{}` in task `{}`", reg.id, holder.task_id);
    if !reg.live {
        return Ok(Some(format!(
            "Registration {watch} has ended. Read its outcome in chariox.events.inbox."
        )));
    }
    if let Some(wait) = &holder.wait {
        if !settled_waits::sources_settled(tx, holder, wait)? {
            return Ok(Some(format!(
                "Your earlier task is still waiting. It owns registration {watch}; its wait continues."
            )));
        }
    }
    if holder.pending_prompt_id.is_some()
        || holder.goal.is_some()
        || matches!(
            holder.state,
            ExecutionState::Working | ExecutionState::Blocked
        )
    {
        return Ok(Some(format!(
            "Registration {watch} belongs to a task that still has a turn or owner action pending. Let that task continue."
        )));
    }
    if holder.state == ExecutionState::Waiting
        && holder.obligations.iter().any(|o| {
            o.requires_resolution()
                && !o.outlives_turn()
                && Some(&o.id) != reg.obligation_id.as_ref()
        })
    {
        return Ok(Some(format!(
            "Registration {watch} belongs to a task with work still to settle. Let that task continue."
        )));
    }
    let pending: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_inbox
         WHERE room_id=?1 AND agent_id=?2 AND source_id=?3
         AND json_extract(payload,'$.state') NOT IN ('handled','expired','failed')
         AND (json_extract(payload,'$.payload.task_id')=?4
              OR EXISTS(SELECT 1 FROM json_each(json_extract(payload,'$.payload.task_ids')) WHERE value=?4)))",
        params![holder.room_id, holder.agent_id, reg.source_id, holder.task_id],
        |row| row.get(0),
    ).map_err(sql)?;
    if pending {
        return Ok(Some(format!(
            "Registration {watch} has an unread outcome. Read it in chariox.events.inbox and handle it before retrying."
        )));
    }
    if delegation::unbound(holder, reg) {
        return Ok(Some(format!(
            "Registration {watch} is still binding its child task. Wait for the earlier task's result and reuse the same registration if it is still live."
        )));
    }
    Ok(None)
}

fn transfer(
    tx: &Transaction<'_>,
    task: &mut AgentTaskExecution,
    own: &mut Vec<Registration>,
    reg: Registration,
) -> Result<(), DaemonError> {
    let mut holder = load(tx, &reg.task_id)?;
    let mut moved = if let Some(id) = &reg.obligation_id {
        let index = holder
            .obligations
            .iter()
            .position(|o| &o.id == id)
            .ok_or_else(|| error("registration has no bound obligation"))?;
        if task.obligations.iter().any(|o| &o.id == id) {
            return Err(error(
                "registration obligation already belongs to this task",
            ));
        }
        task.obligations.push(holder.obligations.remove(index));
        registrations(tx, &holder.task_id)?
            .into_iter()
            .filter(|r| r.live && r.obligation_id.as_ref() == Some(id))
            .collect()
    } else {
        vec![reg]
    };
    for registration in &mut moved {
        wakes::adopt(tx, &holder, task, registration)?;
        registration.task_id.clone_from(&task.task_id);
        tx.execute(
            "UPDATE agent_registrations SET task_id=?2,payload=?3 WHERE id=?1",
            params![registration.id, registration.task_id, encode(registration)?],
        )
        .map_err(sql)?;
    }
    own.extend(moved);
    let reason = format!("adopted by task {}", task.task_id);
    if holder.wait.is_some() {
        settled_waits::close(tx, &mut holder, reason)?;
    } else {
        if holder.state == ExecutionState::Waiting {
            holder.state = ExecutionState::Done;
        }
        if holder.state != ExecutionState::Cancelled {
            holder.reason = reason;
        }
    }
    holder.revision += 1;
    save(tx, &holder)
}
